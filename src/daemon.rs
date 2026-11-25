use std::time::{Duration, Instant};

use log::{debug, info, warn};

use crate::config::Config;
use crate::geometry::{self, Rect};
use crate::memory::FrameMemory;
use crate::model::{Action, Axis, Command, CommandSet, Special};
use crate::session::Session;
use crate::shortcut::Shortcut;

/// Pointer travel before a press is treated as a possible window drag.
const MIN_MOVE_DISTANCE: i32 = 8;
/// Pointer travel after which a press that has not moved its window is ignored.
const GIVE_UP_DISTANCE: i32 = 64;
const DRAG_UPDATE_INTERVAL: Duration = Duration::from_millis(16);
/// Lets the window manager finish its move before the snap frame is applied.
const SETTLE_AFTER_RELEASE: Duration = Duration::from_millis(40);

enum DragState<W> {
    Idle,
    Down {
        window: W,
        start_frame: Rect,
        start_pointer: (i32, i32),
    },
    Drag {
        window: W,
        start_frame: Rect,
        target: Option<Rect>,
        updated: Instant,
    },
    /// The press is not a window move; wait for the release.
    Skip,
}

pub(crate) enum Input {
    Shortcut(Shortcut),
    /// A command chosen from the status menu, by name.
    #[cfg(target_os = "macos")]
    Command(String),
    Press,
    Release,
    Motion,
}

/// The active window's application, as the status menu describes it.
#[cfg(target_os = "macos")]
pub struct ActiveApp {
    pub name: String,
    /// Identifier used to match and store ignored applications.
    pub id: String,
    pub ignored_by_preferences: bool,
    pub ignored_by_menu: bool,
}

/// What the status menu needs to know about the window it will act on.
#[cfg(target_os = "macos")]
pub struct MenuContext {
    pub axis: Axis,
    pub app: Option<ActiveApp>,
}

/// Executes commands against windows; shared by the daemon and `sash apply`.
pub struct Controller<S: Session> {
    pub session: S,
    pub config: Config,
    pub commands: CommandSet,
    memory: FrameMemory<S::Window>,
}

impl<S: Session> Controller<S> {
    pub fn new(session: S, config: Config) -> anyhow::Result<Self> {
        let commands = config.commands()?;
        Ok(Self {
            session,
            config,
            commands,
            memory: FrameMemory::new(),
        })
    }

    /// The active window unless it is ignored, a panel, or the desktop.
    fn target_window(&self) -> anyhow::Result<Option<S::Window>> {
        Ok(self
            .session
            .active_window()?
            .filter(|&w| self.manageable(w)))
    }

    fn manageable(&self, window: S::Window) -> bool {
        if self.session.is_reserved(window) {
            return false;
        }
        let (instance, class) = self.session.app_names(window);
        let ignored = self.config.ignores(&instance, &class);
        if ignored {
            debug!("ignoring {instance}/{class}");
        }
        !ignored
    }

    /// The command list and application the status menu applies to. Without a
    /// usable active window the first display chooses the list.
    #[cfg(target_os = "macos")]
    pub fn menu_context(&self) -> MenuContext {
        let window = self
            .session
            .active_window()
            .ok()
            .flatten()
            .filter(|&w| !self.session.is_reserved(w));
        let monitors = self.session.monitors().unwrap_or_default();
        let monitor = match window.and_then(|w| self.session.frame(w).ok()) {
            Some(frame) if !monitors.is_empty() => {
                Some(monitors[monitor_index_for_frame(&monitors, frame)])
            }
            _ => monitors.first().copied(),
        };
        let axis = monitor.map_or(Axis::Horizontal, |m| Axis::for_size(m.frame.w, m.frame.h));
        let app = window.map(|w| {
            let (name, id) = self.session.app_names(w);
            ActiveApp {
                ignored_by_preferences: self.config.ignored_by_preferences(&name, &id),
                ignored_by_menu: self.config.ignored_by_menu(&name, &id),
                id: if id.is_empty() { name.clone() } else { id },
                name,
            }
        });
        MenuContext { axis, app }
    }

    /// Runs `select` against the active window's monitor axis, then executes it.
    pub fn run_on_active(
        &mut self,
        select: impl Fn(&CommandSet, Axis) -> Option<&Command>,
    ) -> anyhow::Result<()> {
        let Some(window) = self.target_window()? else {
            debug!("no manageable active window");
            return Ok(());
        };
        let frame = self.session.frame(window)?;
        let monitors = self.session.monitors()?;
        if monitors.is_empty() {
            anyhow::bail!("no displays");
        }
        let index = monitor_index_for_frame(&monitors, frame);
        let monitor = monitors[index];
        let axis = Axis::for_size(monitor.frame.w, monitor.frame.h);
        let Some(command) = select(&self.commands, axis) else {
            debug!("no {axis:?} command matches");
            return Ok(());
        };
        let action = command.action;
        info!("{} on window {window:?}", command.name);
        self.execute(action, axis, window, frame, &monitors, index)
    }

    fn execute(
        &mut self,
        action: Action,
        axis: Axis,
        window: S::Window,
        frame: Rect,
        monitors: &[crate::geometry::Monitor],
        index: usize,
    ) -> anyhow::Result<()> {
        let work = monitors[index].work;
        let next = match action {
            Action::Area(grid) => geometry::target_frame(work, axis, grid, self.config.spacing()),
            Action::Special(Special::Center) => geometry::centered(frame, work),
            Action::Special(special @ (Special::NextDisplay | Special::PreviousDisplay)) => {
                if monitors.len() < 2 {
                    return Ok(());
                }
                let step = if special == Special::NextDisplay {
                    1
                } else {
                    monitors.len() - 1
                };
                geometry::transfer(frame, work, monitors[(index + step) % monitors.len()].work)
            }
            Action::Special(Special::Restore) => {
                if let Some(original) = self.memory.take_original(window) {
                    self.session.set_frame(window, original)?;
                }
                return Ok(());
            }
            Action::Special(Special::StayOnTop) => return self.session.toggle_above(window),
        };
        self.remember(window, frame, next)?;
        self.session.set_frame(window, next)
    }

    fn remember(&mut self, window: S::Window, current: Rect, applied: Rect) -> anyhow::Result<()> {
        self.memory.retain(|w| self.session.exists(*w));
        self.memory.record(window, current, applied);
        Ok(())
    }
}

pub struct Daemon<S: Session> {
    pub(crate) c: Controller<S>,
    drag: DragState<S::Window>,
}

impl<S: Session> Daemon<S> {
    pub fn start(session: S, config: Config) -> anyhow::Result<Self> {
        Ok(Self {
            c: Controller::new(session, config)?,
            drag: DragState::Idle,
        })
    }

    pub(crate) fn handle_input(&mut self, input: Input) -> anyhow::Result<()> {
        match input {
            Input::Shortcut(shortcut) => {
                self.c
                    .run_on_active(|set, axis| set.by_shortcut(axis, &shortcut))?;
            }
            #[cfg(target_os = "macos")]
            Input::Command(name) => {
                self.c.run_on_active(|set, axis| set.by_name(axis, &name))?;
            }
            Input::Press => self.on_press()?,
            Input::Release => self.on_release()?,
            Input::Motion => self.on_motion()?,
        }
        Ok(())
    }

    /// Reports the error, drops the current drag and hides the preview.
    pub(crate) fn recover(&mut self, error: anyhow::Error) {
        warn!("{error:#}");
        self.drag = DragState::Skip;
        if let Err(error) = self.c.session.hide_overlay() {
            warn!("{error:#}");
        }
    }

    fn on_press(&mut self) -> anyhow::Result<()> {
        self.drag = DragState::Skip;
        let Some(window) = self
            .c
            .session
            .window_under_pointer()?
            .filter(|&w| self.c.manageable(w))
        else {
            return Ok(());
        };
        self.drag = DragState::Down {
            window,
            start_frame: self.c.session.frame(window)?,
            start_pointer: self.c.session.pointer()?,
        };
        Ok(())
    }

    fn on_motion(&mut self) -> anyhow::Result<()> {
        match self.drag {
            DragState::Idle | DragState::Skip => Ok(()),
            DragState::Down {
                window,
                start_frame,
                start_pointer,
            } => {
                let pointer = self.c.session.pointer()?;
                let travel = (pointer.0 - start_pointer.0)
                    .abs()
                    .max((pointer.1 - start_pointer.1).abs());
                if travel < MIN_MOVE_DISTANCE {
                    return Ok(());
                }
                let frame = self.c.session.frame(window)?;
                if (frame.w - start_frame.w).abs() > 1 || (frame.h - start_frame.h).abs() > 1 {
                    self.drag = DragState::Skip;
                } else if (frame.x, frame.y) == (start_frame.x, start_frame.y) {
                    if travel > GIVE_UP_DISTANCE {
                        self.drag = DragState::Skip;
                    }
                } else {
                    self.begin_drag(window, start_frame, frame, pointer)?;
                }
                Ok(())
            }
            DragState::Drag {
                window,
                start_frame,
                target,
                updated,
            } => {
                if updated.elapsed() < DRAG_UPDATE_INTERVAL {
                    return Ok(());
                }
                let next = self.snap_target(self.c.session.pointer()?)?;
                if next != target {
                    match next {
                        Some(frame) => self.c.session.show_overlay(frame)?,
                        None => self.c.session.hide_overlay()?,
                    }
                }
                self.drag = DragState::Drag {
                    window,
                    start_frame,
                    target: next,
                    updated: Instant::now(),
                };
                Ok(())
            }
        }
    }

    fn begin_drag(
        &mut self,
        window: S::Window,
        start_frame: Rect,
        frame: Rect,
        pointer: (i32, i32),
    ) -> anyhow::Result<()> {
        let original = self.c.memory.snapped_original(window, start_frame);
        self.c.memory.forget(window);
        if let (true, Some(original)) = (self.c.config.restore_to_original_size, original) {
            self.c
                .session
                .set_frame(window, geometry::unsnapped(original, frame, pointer))?;
        }
        let start_frame = original.unwrap_or(start_frame);
        self.drag = DragState::Drag {
            window,
            start_frame,
            target: None,
            updated: Instant::now(),
        };
        Ok(())
    }

    /// The target frame of the activation area under the pointer, if any.
    fn snap_target(&self, pointer: (i32, i32)) -> anyhow::Result<Option<Rect>> {
        let monitors = self.c.session.monitors()?;
        let Some(monitor) = monitors
            .iter()
            .find(|m| m.frame.contains(pointer.0, pointer.1))
        else {
            return Ok(None);
        };
        let axis = Axis::for_size(monitor.frame.w, monitor.frame.h);
        let tuning = self.c.config.activation();
        let hit = self
            .c
            .commands
            .for_axis(axis)
            .find_map(|command| match command.action {
                Action::Area(grid) => command
                    .activation
                    .iter()
                    .any(|&segment| {
                        geometry::activation_hit(monitor.frame, axis, segment, pointer, tuning)
                    })
                    .then_some(grid),
                Action::Special(_) => None,
            });
        Ok(hit
            .map(|grid| geometry::target_frame(monitor.work, axis, grid, self.c.config.spacing())))
    }

    fn on_release(&mut self) -> anyhow::Result<()> {
        let state = std::mem::replace(&mut self.drag, DragState::Idle);
        let DragState::Drag {
            window,
            start_frame,
            ..
        } = state
        else {
            return Ok(());
        };
        let target = self.snap_target(self.c.session.pointer()?)?;
        self.c.session.hide_overlay()?;
        if let Some(frame) = target {
            std::thread::sleep(SETTLE_AFTER_RELEASE);
            self.c.remember(window, start_frame, frame)?;
            self.c.session.set_frame(window, frame)?;
        }
        Ok(())
    }
}

pub fn apply_command<S: Session>(session: S, config: Config, name: &str) -> anyhow::Result<()> {
    let mut controller = Controller::new(session, config)?;
    if controller
        .commands
        .by_name(Axis::Horizontal, name)
        .is_none()
        && controller.commands.by_name(Axis::Vertical, name).is_none()
    {
        anyhow::bail!("unknown command `{name}`; see `sash list`");
    }
    controller.run_on_active(|set, axis| set.by_name(axis, name))
}

/// The monitor sharing the most area with the frame, or the nearest one.
fn monitor_index_for_frame(monitors: &[crate::geometry::Monitor], frame: Rect) -> usize {
    let best = (0..monitors.len()).max_by_key(|&i| monitors[i].frame.overlap_area(&frame));
    match best {
        Some(i) if monitors[i].frame.overlap_area(&frame) > 0 => i,
        _ => {
            let (cx, cy) = frame.center();
            (0..monitors.len())
                .min_by_key(|&i| {
                    let (mx, my) = monitors[i].frame.center();
                    i64::from(mx - cx).pow(2) + i64::from(my - cy).pow(2)
                })
                .unwrap_or(0)
        }
    }
}
