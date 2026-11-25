//! GNOME Shell on Wayland. A Wayland client cannot move other applications'
//! windows or receive global keys, so the `gnome/sash@sash.local` Shell
//! extension does both and this module drives it over D-Bus (`local.sash.Shell`).

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Sender, SyncSender};
use std::time::{Duration, Instant};

use anyhow::Context;
use log::{info, warn};
use serde_json::json;
use zbus::blocking::{Connection, Proxy};

use crate::config::{Color, Config};
use crate::daemon::{Daemon, Input};
use crate::geometry::{Monitor, Rect};
use crate::model::{Axis, CommandSet};
use crate::session::Session;
use crate::shortcut::Shortcut;

mod service;

const BUS_NAME: &str = "local.sash.Shell";
const OBJECT_PATH: &str = "/local/sash/Shell";
const INTERFACE: &str = "local.sash.Shell";
/// Owned while `sash` runs; the extension shows the status icon only then, and
/// its settings page talks to Sash through the object served on it.
const DAEMON_NAME: &str = "local.sash.Daemon";
const DAEMON_PATH: &str = "/local/sash/Daemon";

const EXTENSION_UUID: &str = "sash@sash.local";
/// Where the Debian package puts the extension for every user. The Shell reads
/// it at login, and a copy in the user's own directory would override it.
const SYSTEM_EXTENSION_DIR: &str = "/usr/share/gnome-shell/extensions";
/// The `Version` the bundled extension reports. Bump it together with
/// `EXTENSION_VERSION` in `extension.js` whenever the extension changes: a Shell
/// that still runs an older copy only learns the new one at the next login.
const EXTENSION_VERSION: u32 = 2;
const EXTENSION_METADATA: &str = include_str!("../gnome/sash@sash.local/metadata.json");
const EXTENSION_SCRIPT: &str = include_str!("../gnome/sash@sash.local/extension.js");
const EXTENSION_PREFS: &str = include_str!("../gnome/sash@sash.local/prefs.js");
const EXTENSION_ICON: &str = include_str!("../gnome/sash@sash.local/icons/sash-symbolic.svg");

/// `exists, reserved, instance, class, x, y, w, h` as the extension reports a window.
type WindowInfo = (bool, bool, String, String, i32, i32, i32, i32);
/// `x, y, w, h` of a monitor, then `x, y, w, h` of its work area.
type MonitorInfo = (i32, i32, i32, i32, i32, i32, i32, i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Preference {
    Auto,
    Gnome,
    X11,
}

fn preference() -> Preference {
    match std::env::var("SASH_BACKEND").as_deref() {
        Ok("gnome") => Preference::Gnome,
        Ok("x11") => Preference::X11,
        _ => Preference::Auto,
    }
}

fn is_gnome_wayland() -> bool {
    let wayland = std::env::var("XDG_SESSION_TYPE")
        .is_ok_and(|t| t.eq_ignore_ascii_case("wayland"))
        || std::env::var_os("WAYLAND_DISPLAY").is_some();
    let gnome = std::env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|d| d.to_ascii_lowercase().contains("gnome"));
    wayland && gnome
}

/// The GNOME Shell session to use, or `None` to use X11. `SASH_BACKEND=gnome`
/// or `x11` overrides the choice. On a GNOME Wayland session whose extension is
/// not running, Sash warns and falls back to X11 (XWayland).
pub fn session(config: &Config, patience: Duration) -> anyhow::Result<Option<GnomeSession>> {
    match preference() {
        Preference::X11 => Ok(None),
        Preference::Gnome => GnomeSession::connect(config, patience).map(Some),
        Preference::Auto if is_gnome_wayland() => match GnomeSession::connect(config, patience) {
            Ok(session) => Ok(Some(session)),
            Err(error) => {
                warn!("{error:#}");
                warn!("falling back to X11: only XWayland windows can be managed");
                Ok(None)
            }
        },
        Preference::Auto => Ok(None),
    }
}

struct OverlayStyle {
    fill: String,
    outline: String,
    width: i32,
}

fn css(color: Color) -> String {
    format!(
        "rgba({}, {}, {}, {:.3})",
        color.r,
        color.g,
        color.b,
        f64::from(color.a) / 255.0
    )
}

impl OverlayStyle {
    fn new(config: &Config) -> Self {
        Self {
            fill: css(config.highlight.fill),
            outline: css(config.highlight.outline),
            width: i32::from(config.highlight.outline_width.max(1)),
        }
    }
}

pub struct GnomeSession {
    proxy: Proxy<'static>,
    overlay: RefCell<OverlayStyle>,
    /// The window a status-menu choice applies to: the one that was active when
    /// the menu opened, since opening it may have taken the focus.
    pinned: Cell<Option<u64>>,
    /// The connection that serves `DAEMON_NAME`; a separate one, so that a
    /// settings request waiting for the main loop never blocks the calls the
    /// main loop makes to the Shell.
    daemon_connection: OnceCell<Connection>,
}

impl GnomeSession {
    /// Connects to the extension, waiting up to `patience` for it to appear: at
    /// login the Shell may still be loading it.
    pub fn connect(config: &Config, patience: Duration) -> anyhow::Result<Self> {
        let connection = Connection::session().context("cannot connect to the session bus")?;
        let proxy = Proxy::new(&connection, BUS_NAME, OBJECT_PATH, INTERFACE)?;
        let session = Self {
            proxy,
            overlay: RefCell::new(OverlayStyle::new(config)),
            pinned: Cell::new(None),
            daemon_connection: OnceCell::new(),
        };
        let deadline = Instant::now() + patience;
        // Only worth waiting when the extension is already set up, which means the
        // Shell is probably still loading it. Otherwise it needs a new login.
        let mut setup: Option<anyhow::Result<bool>> = None;
        let failure = loop {
            match session.proxy.call::<_, _, u64>("ActiveWindow", &()) {
                Ok(_) => break None,
                Err(error) => {
                    let result = setup.get_or_insert_with(prepare_extension);
                    let may_still_load = matches!(result, Ok(false));
                    if !may_still_load || Instant::now() >= deadline {
                        break Some(error);
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        };
        if let Some(error) = failure {
            let message = match setup.unwrap_or_else(prepare_extension) {
                Ok(true) => {
                    desktop_notify("Sash", "Log out and back in once to finish setting up Sash.");
                    "the Sash GNOME Shell extension is not loaded yet; log out and back in once \
                     to finish setting up Sash"
                        .to_string()
                }
                Ok(false) => "the Sash GNOME Shell extension is installed and enabled, but the \
                              Shell did not load it; see `gnome-extensions info sash@sash.local` \
                              (the extension targets GNOME Shell 50)"
                    .to_string(),
                Err(prepare) => format!(
                    "the Sash GNOME Shell extension is not running and could not be enabled \
                     ({prepare:#}); run `sash install-extension`, then log out and back in"
                ),
            };
            return Err(anyhow::Error::new(error).context(message));
        }
        Ok(session)
    }

    /// Warns when the Shell still runs an older extension than this binary
    /// expects, which only a new login fixes.
    fn check_extension_version(&self) {
        match self.proxy.call::<_, _, u32>("Version", &()) {
            Ok(EXTENSION_VERSION) => {}
            found => {
                warn!(
                    "the GNOME Shell still runs an older Sash extension ({found:?}, expected \
                     {EXTENSION_VERSION}); log out and back in to load the new one"
                );
                self.notify(
                    "Sash was updated",
                    "Log out and back in once so GNOME loads the new version.",
                );
            }
        }
    }

    fn info(&self, window: u64) -> anyhow::Result<WindowInfo> {
        Ok(self.proxy.call("WindowInfo", &window)?)
    }

    /// Grabs the accelerators and returns the ones GNOME refused.
    fn set_shortcuts(&self, accelerators: &[String]) -> anyhow::Result<Vec<String>> {
        Ok(self.proxy.call("SetShortcuts", &accelerators)?)
    }

    /// Takes the name that tells the extension Sash is running, and refuses to
    /// start a second copy. Settings requests arrive on `events` from then on.
    fn serve(&self, events: Sender<Event>) -> anyhow::Result<()> {
        let connection = zbus::blocking::connection::Builder::session()?
            .serve_at(DAEMON_PATH, service::Service::new(events))?
            .name(DAEMON_NAME)
            .and_then(zbus::blocking::connection::Builder::build)
            .context("Sash is already running in this session")?;
        let _ = self.daemon_connection.set(connection);
        Ok(())
    }

    fn set_menu(&self, json: &str) -> anyhow::Result<()> {
        self.proxy.call::<_, _, ()>("SetMenu", &json)?;
        Ok(())
    }

    fn notify(&self, title: &str, body: &str) {
        if let Err(error) = self.proxy.call::<_, _, ()>("Notify", &(title, body)) {
            warn!("could not show a notification: {error}");
        }
    }

    fn set_overlay_style(&self, config: &Config) {
        *self.overlay.borrow_mut() = OverlayStyle::new(config);
    }

    fn listen(&self, tx: Sender<Event>) -> anyhow::Result<()> {
        let signals = self.proxy.receive_all_signals()?;
        std::thread::spawn(move || {
            for message in signals {
                let header = message.header();
                let Some(member) = header.member() else {
                    continue;
                };
                let body = message.body();
                let event = match member.as_str() {
                    "Ready" => Event::Ready,
                    "Shortcut" => match body.deserialize::<String>() {
                        Ok(accelerator) => Event::Shortcut(accelerator),
                        Err(_) => continue,
                    },
                    "Input" => match body.deserialize::<String>() {
                        Ok(kind) => Event::Input(kind),
                        Err(_) => continue,
                    },
                    "MenuAction" => match body.deserialize::<(String, String, u64)>() {
                        Ok((action, arg, window)) => Event::Menu(MenuAction {
                            action,
                            arg,
                            window: (window != 0).then_some(window),
                        }),
                        Err(_) => continue,
                    },
                    _ => continue,
                };
                if tx.send(event).is_err() {
                    break;
                }
            }
        });
        Ok(())
    }
}

/// A status-menu item that was chosen.
struct MenuAction {
    action: String,
    arg: String,
    /// The window that was active when the menu opened.
    window: Option<u64>,
}

enum Event {
    Ready,
    Shortcut(String),
    Input(String),
    Menu(MenuAction),
    /// A request from the settings page, with where to send the JSON answer.
    Settings(Request, SyncSender<Result<String, String>>),
}

/// What the settings page asks Sash to do.
enum Request {
    Get,
    /// A preference by its `config.toml` name, and its value as JSON.
    SetPreference(String, String),
    /// Adds or removes an application of the menu's ignored list.
    SetIgnored(String, bool),
    /// The whole command set, in the shape of a commands file.
    SetCommands(String),
    /// `preferences`, `commands` or `all`.
    Restore(String),
    OpenConfig,
}

impl Event {
    fn is_motion(&self) -> bool {
        matches!(self, Event::Input(kind) if kind == "motion")
    }
}

impl Session for GnomeSession {
    type Window = u64;

    fn active_window(&self) -> anyhow::Result<Option<u64>> {
        if let Some(window) = self.pinned.get() {
            return Ok(Some(window));
        }
        let id: u64 = self.proxy.call("ActiveWindow", &())?;
        Ok((id != 0).then_some(id))
    }

    fn is_reserved(&self, window: u64) -> bool {
        self.info(window).map_or(true, |(_, reserved, ..)| reserved)
    }

    fn app_names(&self, window: u64) -> (String, String) {
        self.info(window)
            .map(|(_, _, instance, class, ..)| (instance, class))
            .unwrap_or_default()
    }

    fn frame(&self, window: u64) -> anyhow::Result<Rect> {
        let (exists, _, _, _, x, y, w, h) = self.info(window)?;
        anyhow::ensure!(exists, "window {window} is gone");
        Ok(Rect::new(x, y, w, h))
    }

    fn set_frame(&self, window: u64, frame: Rect) -> anyhow::Result<()> {
        self.proxy
            .call::<_, _, ()>("SetFrame", &(window, frame.x, frame.y, frame.w, frame.h))?;
        Ok(())
    }

    fn toggle_above(&self, window: u64) -> anyhow::Result<()> {
        self.proxy.call::<_, _, ()>("ToggleAbove", &window)?;
        Ok(())
    }

    fn monitors(&self) -> anyhow::Result<Vec<Monitor>> {
        let monitors: Vec<MonitorInfo> = self.proxy.call("Monitors", &())?;
        Ok(monitors
            .into_iter()
            .map(|(x, y, w, h, wx, wy, ww, wh)| Monitor {
                frame: Rect::new(x, y, w, h),
                work: Rect::new(wx, wy, ww, wh),
            })
            .collect())
    }

    fn pointer(&self) -> anyhow::Result<(i32, i32)> {
        Ok(self.proxy.call("Pointer", &())?)
    }

    fn window_under_pointer(&self) -> anyhow::Result<Option<u64>> {
        let id: u64 = self.proxy.call("WindowUnderPointer", &())?;
        Ok((id != 0).then_some(id))
    }

    fn exists(&self, window: u64) -> bool {
        self.proxy.call("Exists", &window).unwrap_or(false)
    }

    fn show_overlay(&self, frame: Rect) -> anyhow::Result<()> {
        let style = self.overlay.borrow();
        self.proxy.call::<_, _, ()>(
            "ShowOverlay",
            &(
                frame.x,
                frame.y,
                frame.w.max(1),
                frame.h.max(1),
                style.fill.as_str(),
                style.outline.as_str(),
                style.width,
            ),
        )?;
        Ok(())
    }

    fn hide_overlay(&self) -> anyhow::Result<()> {
        self.proxy.call::<_, _, ()>("HideOverlay", &())?;
        Ok(())
    }
}

/// The accelerator -> shortcut table, and the shortcuts GNOME refused.
type Registered = (HashMap<String, Shortcut>, Vec<String>);

/// Grabs every shortcut in the extension and shows the status menu's content.
fn register(daemon: &Daemon<GnomeSession>) -> anyhow::Result<Registered> {
    let table: HashMap<String, Shortcut> = daemon
        .c
        .commands
        .shortcuts()
        .into_iter()
        .map(|shortcut| (shortcut.gnome_accelerator(), shortcut.clone()))
        .collect();
    let accelerators: Vec<String> = table.keys().cloned().collect();
    let failed = daemon.c.session.set_shortcuts(&accelerators)?;
    let mut refused = Vec::new();
    for accelerator in &failed {
        let name = table
            .get(accelerator)
            .map_or_else(|| accelerator.clone(), ToString::to_string);
        warn!(
            "{accelerator}: GNOME refused the shortcut ({name}); it may be used by the desktop. \
             Give the command another shortcut in the settings"
        );
        refused.push(name);
    }
    info!("{} shortcuts active", accelerators.len() - failed.len());
    push_menu(daemon);
    Ok((table, refused))
}

/// The status menu's content for the extension: the commands of both
/// orientations (it picks the one for the active window's display), the
/// ignored applications and the switches.
fn menu_json(config: &Config, commands: &CommandSet) -> String {
    let list = |axis| {
        commands
            .for_axis(axis)
            .filter(|command| command.in_menu)
            .map(|command| {
                json!({
                    "name": command.name,
                    "shortcut": command.shortcut.as_ref().map(ToString::to_string),
                    "separator": command.separator_after,
                })
            })
            .collect::<Vec<_>>()
    };
    json!({
        "horizontal": list(Axis::Horizontal),
        "vertical": list(Axis::Vertical),
        "ignored": config.user_ignored,
        "ignored_by_preferences": config.ignored_apps,
        "snap_windows": config.snap_windows,
        "restore_to_original_size": config.restore_to_original_size,
        "hide_icon": config.hide_menu_bar_icon,
    })
    .to_string()
}

fn push_menu(daemon: &Daemon<GnomeSession>) {
    let json = menu_json(&daemon.c.config, &daemon.c.commands);
    if let Err(error) = daemon.c.session.set_menu(&json) {
        warn!("could not update the status menu: {error}");
    }
}

/// The boolean preference a menu switch stands for.
fn switch<'a>(config: &'a mut Config, key: &str) -> Option<&'a mut bool> {
    match key {
        "snap_windows" => Some(&mut config.snap_windows),
        "restore_to_original_size" => Some(&mut config.restore_to_original_size),
        _ => None,
    }
}

/// Opens the preferences file in the default editor, creating it from the
/// commented example the first time.
fn open_settings(config: &Config) -> anyhow::Result<()> {
    let path = config
        .path
        .clone()
        .or_else(Config::default_path)
        .context("no preferences location")?;
    if !path.exists() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&path, Config::EXAMPLE)
            .with_context(|| format!("writing {}", path.display()))?;
    }
    let mut child = Command::new("xdg-open")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("running xdg-open")?;
    // Reap it when the editor hands over, so it does not linger as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Opens the extension's settings page.
fn open_preferences() -> anyhow::Result<()> {
    let mut child = Command::new("gnome-extensions")
        .args(["prefs", EXTENSION_UUID])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("running gnome-extensions")?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Reads the preferences and commands again and applies them. The old ones stay
/// in force when the new ones are invalid.
fn reload(
    daemon: &mut Daemon<GnomeSession>,
    shortcuts: &mut HashMap<String, Shortcut>,
) -> anyhow::Result<()> {
    let config = Config::load(daemon.c.config.path.as_deref())?;
    let commands = config.commands()?;
    daemon.c.session.set_overlay_style(&config);
    daemon.c.config = config;
    daemon.c.commands = commands;
    *shortcuts = register(daemon)?.0;
    Ok(())
}

/// What the settings page reads: the preferences, the ignored applications and
/// the commands of both orientations.
fn settings_json(daemon: &Daemon<GnomeSession>) -> anyhow::Result<serde_json::Value> {
    let config = &daemon.c.config;
    Ok(json!({
        "config": serde_json::to_value(config).context("reading preferences")?,
        "config_path": config.path.clone().or_else(Config::default_path),
        "ignored": { "preferences": config.ignored_apps, "menu": config.user_ignored },
        "commands": daemon.c.commands.editor_json(),
    }))
}

/// Makes `commands` the running set and keeps them: saved, shortcuts grabbed,
/// menu updated. Everything stays as it was when saving fails.
fn apply_commands(
    daemon: &mut Daemon<GnomeSession>,
    shortcuts: &mut HashMap<String, Shortcut>,
    commands: CommandSet,
) -> anyhow::Result<Vec<String>> {
    let previous = daemon.c.config.clone();
    let saved = daemon.c.config.save_commands(&commands).and_then(|newly| {
        if newly {
            daemon.c.config.save()?;
        }
        Ok(())
    });
    if let Err(error) = saved {
        daemon.c.config = previous;
        return Err(error);
    }
    daemon.c.commands = commands;
    let (table, refused) = register(daemon)?;
    *shortcuts = table;
    Ok(refused)
}

/// Saves the preferences and applies what the running Sash holds in memory.
fn apply_preferences(daemon: &mut Daemon<GnomeSession>, previous: Config) -> anyhow::Result<()> {
    if let Err(error) = daemon.c.config.save() {
        daemon.c.config = previous;
        return Err(error);
    }
    daemon.c.session.set_overlay_style(&daemon.c.config);
    push_menu(daemon);
    Ok(())
}

fn settings_request(
    daemon: &mut Daemon<GnomeSession>,
    shortcuts: &mut HashMap<String, Shortcut>,
    request: Request,
) -> anyhow::Result<serde_json::Value> {
    let mut refused = Vec::new();
    match request {
        Request::Get => return settings_json(daemon),
        Request::SetPreference(key, value) => {
            let value = serde_json::from_str(&value).context("the value is not JSON")?;
            let previous = daemon.c.config.clone();
            daemon.c.config.set_preference(&key, value)?;
            apply_preferences(daemon, previous)?;
        }
        Request::SetIgnored(app, ignored) => {
            daemon.c.config.set_ignored_by_menu(&app, ignored)?;
            push_menu(daemon);
        }
        Request::SetCommands(json) => {
            let commands = CommandSet::parse(&json)?;
            commands.check_replacement()?;
            refused = apply_commands(daemon, shortcuts, commands)?;
        }
        Request::Restore(what) => {
            let (preferences, commands) = match what.as_str() {
                "preferences" => (true, false),
                "commands" => (false, true),
                "all" => (true, true),
                other => anyhow::bail!("cannot restore `{other}`"),
            };
            if preferences {
                let previous = daemon.c.config.clone();
                daemon.c.config.restore_preferences();
                apply_preferences(daemon, previous)?;
            }
            if commands {
                refused = apply_commands(daemon, shortcuts, CommandSet::builtin())?;
            }
        }
        Request::OpenConfig => open_settings(&daemon.c.config)?,
    }
    Ok(json!({ "refused": refused }))
}

fn run_menu_action(
    daemon: &mut Daemon<GnomeSession>,
    shortcuts: &mut HashMap<String, Shortcut>,
    menu: &MenuAction,
) -> anyhow::Result<()> {
    match menu.action.as_str() {
        "command" => daemon
            .c
            .run_on_active(|set, axis| set.by_name(axis, &menu.arg)),
        "ignore" => {
            let window = menu.window.context("no window to ignore")?;
            let (instance, class) = daemon.c.session.app_names(window);
            let config = &mut daemon.c.config;
            if config.ignored_by_menu(&instance, &class) {
                for id in [&instance, &class].into_iter().filter(|id| !id.is_empty()) {
                    config.set_ignored_by_menu(id, false)?;
                }
            } else {
                let id = if class.is_empty() { &instance } else { &class };
                anyhow::ensure!(
                    !id.is_empty(),
                    "cannot tell which application this window belongs to"
                );
                config.set_ignored_by_menu(id, true)?;
            }
            push_menu(daemon);
            Ok(())
        }
        "toggle" => {
            let flag = switch(&mut daemon.c.config, &menu.arg)
                .with_context(|| format!("unknown setting `{}`", menu.arg))?;
            *flag = !*flag;
            if let Err(error) = daemon.c.config.save() {
                if let Some(flag) = switch(&mut daemon.c.config, &menu.arg) {
                    *flag = !*flag;
                }
                push_menu(daemon);
                return Err(error);
            }
            push_menu(daemon);
            Ok(())
        }
        // What the extension sent before it opened the settings page itself.
        "settings" => open_preferences(),
        "reload" => reload(daemon, shortcuts),
        "quit" => {
            info!("quit from the status menu");
            std::process::exit(0);
        }
        other => {
            warn!("unknown status menu action `{other}`");
            Ok(())
        }
    }
}

/// Runs a status-menu choice against the window that was active when the menu
/// opened, and tells the user when it fails.
fn menu_action(
    daemon: &mut Daemon<GnomeSession>,
    shortcuts: &mut HashMap<String, Shortcut>,
    menu: MenuAction,
) {
    daemon.c.session.pinned.set(menu.window);
    let result = run_menu_action(daemon, shortcuts, &menu);
    daemon.c.session.pinned.set(None);
    match result {
        Ok(()) if menu.action == "reload" => daemon.c.session.notify("Sash", "Settings reloaded"),
        Ok(()) => {}
        Err(error) => {
            warn!("{error:#}");
            daemon.c.session.notify("Sash", &format!("{error:#}"));
        }
    }
}

pub fn launch(session: GnomeSession, config: Config) -> anyhow::Result<()> {
    let (tx, events) = mpsc::channel();
    session.listen(tx.clone())?;
    session.serve(tx)?;
    session.check_extension_version();
    let mut daemon = Daemon::start(session, config).context("starting Sash")?;
    let mut shortcuts = register(&daemon)?.0;
    loop {
        let mut batch = vec![events.recv().context("lost the GNOME Shell connection")?];
        batch.extend(events.try_iter());
        // A slow handler must not leave a backlog of stale pointer samples.
        batch.dedup_by(|a, b| a.is_motion() && b.is_motion());
        for event in batch {
            let result = match event {
                Event::Ready => register(&daemon).map(|(table, _)| shortcuts = table),
                Event::Shortcut(accelerator) => match shortcuts.get(&accelerator).cloned() {
                    Some(shortcut) => daemon.handle_input(Input::Shortcut(shortcut)),
                    None => Ok(()),
                },
                Event::Menu(menu) => {
                    menu_action(&mut daemon, &mut shortcuts, menu);
                    Ok(())
                }
                Event::Settings(request, reply) => {
                    let answer = settings_request(&mut daemon, &mut shortcuts, request)
                        .map(|value| value.to_string())
                        .map_err(|error| format!("{error:#}"));
                    if let Err(message) = &answer {
                        warn!("{message}");
                    }
                    let _ = reply.send(answer);
                    Ok(())
                }
                Event::Input(_) if !daemon.c.config.snap_windows => Ok(()),
                Event::Input(kind) => match kind.as_str() {
                    "press" => daemon.handle_input(Input::Press),
                    "release" => daemon.handle_input(Input::Release),
                    "motion" => daemon.handle_input(Input::Motion),
                    _ => Ok(()),
                },
            };
            if let Err(error) = result {
                daemon.recover(error);
            }
        }
    }
}

fn user_extension_dir() -> anyhow::Result<PathBuf> {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .context("neither XDG_DATA_HOME nor HOME is set")?;
    Ok(data_home.join("gnome-shell/extensions").join(EXTENSION_UUID))
}

/// Whether the Debian package (or an administrator) installed the extension for all users.
fn system_extension_installed() -> bool {
    std::path::Path::new(SYSTEM_EXTENSION_DIR)
        .join(EXTENSION_UUID)
        .join("metadata.json")
        .is_file()
}

/// Writes the bundled extension into the user's directory. Returns whether any
/// file changed.
fn write_user_extension(dir: &std::path::Path) -> anyhow::Result<bool> {
    std::fs::create_dir_all(dir.join("icons"))
        .with_context(|| format!("creating {}", dir.join("icons").display()))?;
    let mut changed = false;
    for (name, contents) in [
        ("metadata.json", EXTENSION_METADATA),
        ("extension.js", EXTENSION_SCRIPT),
        ("prefs.js", EXTENSION_PREFS),
        ("icons/sash-symbolic.svg", EXTENSION_ICON),
    ] {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).is_ok_and(|current| current == contents) {
            continue;
        }
        std::fs::write(&path, contents)
            .with_context(|| format!("writing {}", path.display()))?;
        changed = true;
    }
    Ok(changed)
}

/// Makes the Shell load the current extension at the next login: the system copy
/// when the package provides one (a copy in the user's directory would override
/// it, so a stale one is removed), otherwise a copy in the user's directory, and
/// in both cases an entry in `enabled-extensions`. Returns whether anything had
/// to change, which means a login is needed.
fn prepare_extension() -> anyhow::Result<bool> {
    let user_dir = user_extension_dir()?;
    let mut changed = false;
    if system_extension_installed() {
        if user_dir.exists() {
            std::fs::remove_dir_all(&user_dir)
                .with_context(|| format!("removing {}", user_dir.display()))?;
            changed = true;
        }
    } else {
        changed |= write_user_extension(&user_dir)?;
    }
    changed |= enable_in_settings()?;
    Ok(changed)
}

/// Copies the Shell extension where the Shell finds it and turns it on.
pub fn install_extension() -> anyhow::Result<()> {
    if system_extension_installed() {
        println!(
            "The extension is installed for all users in {SYSTEM_EXTENSION_DIR}/{EXTENSION_UUID}."
        );
    }
    prepare_extension()?;
    // A running Shell only discovers a new extension at login, so asking it to
    // enable the extension cannot be relied on. The setting above is what the
    // Shell reads at startup; this also loads an update at once when the Shell
    // already knows the extension.
    let _ = Command::new("gnome-extensions")
        .args(["enable", EXTENSION_UUID])
        .output();
    println!(
        "Enabled. GNOME Shell on Wayland cannot be restarted: log out and back in to load the \
         extension (also after updating it), then run `sash`."
    );
    Ok(())
}

/// Adds the extension to `enabled-extensions`. Returns whether it was missing.
fn enable_in_settings() -> anyhow::Result<bool> {
    let gsettings = |args: &[&str]| -> anyhow::Result<String> {
        let out = Command::new("gsettings")
            .args(args)
            .output()
            .context("running gsettings")?;
        anyhow::ensure!(
            out.status.success(),
            "gsettings {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let current = gsettings(&["get", "org.gnome.shell", "enabled-extensions"])?;
    let mut ids: Vec<&str> = current.split('\'').skip(1).step_by(2).collect();
    if ids.contains(&EXTENSION_UUID) {
        return Ok(false);
    }
    ids.push(EXTENSION_UUID);
    let value = format!(
        "[{}]",
        ids.iter()
            .map(|id| format!("'{id}'"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    gsettings(&["set", "org.gnome.shell", "enabled-extensions", &value])?;
    Ok(true)
}

/// Shows a desktop notification, for when the extension that would otherwise show
/// it is not running. Failing to show one is not an error.
fn desktop_notify(title: &str, body: &str) {
    let result = Connection::session().and_then(|connection| {
        connection.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "Sash",
                0u32,
                "sash",
                title,
                body,
                Vec::<&str>::new(),
                HashMap::<&str, zbus::zvariant::Value<'_>>::new(),
                15_000i32,
            ),
        )
    });
    if let Err(error) = result {
        warn!("could not show a notification: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(config: &Config) -> serde_json::Value {
        serde_json::from_str(&menu_json(config, &CommandSet::builtin())).unwrap()
    }

    #[test]
    fn menu_lists_the_commands_of_both_orientations() {
        let menu = menu(&Config::default());
        assert_eq!(menu["horizontal"][0]["name"], "Left");
        assert_eq!(menu["horizontal"][0]["shortcut"], "Ctrl+Alt+Left");
        assert_eq!(menu["vertical"][4]["name"], "Top Left");
        let bottom = menu["horizontal"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["name"] == "Bottom")
            .unwrap();
        assert_eq!(bottom["separator"], true);
    }

    #[test]
    fn menu_carries_the_switches_and_ignored_applications() {
        let config = Config {
            snap_windows: false,
            hide_menu_bar_icon: true,
            ignored_apps: vec!["Terminal".into()],
            user_ignored: vec!["firefox".into()],
            ..Config::default()
        };
        let menu = menu(&config);
        assert_eq!(menu["snap_windows"], false);
        assert_eq!(menu["restore_to_original_size"], true);
        assert_eq!(menu["hide_icon"], true);
        assert_eq!(menu["ignored_by_preferences"][0], "Terminal");
        assert_eq!(menu["ignored"][0], "firefox");
    }

    #[test]
    fn switches_map_to_preferences() {
        let mut config = Config::default();
        *switch(&mut config, "snap_windows").unwrap() = false;
        assert!(!config.snap_windows);
        assert!(switch(&mut config, "padding").is_none());
    }
}
