use std::collections::HashMap;

use anyhow::{Context, bail};
use log::{debug, info, warn};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

use crate::config::Highlight;
use crate::geometry::{Extents, Monitor, Rect};
use crate::shortcut::Shortcut;

x11rb::atom_manager! {
    pub Atoms: AtomsCookie {
        _NET_SUPPORTED,
        _NET_ACTIVE_WINDOW,
        _NET_CLIENT_LIST,
        _NET_CURRENT_DESKTOP,
        _NET_WORKAREA,
        _NET_FRAME_EXTENTS,
        _GTK_FRAME_EXTENTS,
        _NET_MOVERESIZE_WINDOW,
        _NET_WM_STATE,
        _NET_WM_STATE_MAXIMIZED_VERT,
        _NET_WM_STATE_MAXIMIZED_HORZ,
        _NET_WM_STATE_FULLSCREEN,
        _NET_WM_STATE_ABOVE,
        _NET_WM_STRUT,
        _NET_WM_STRUT_PARTIAL,
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_DESKTOP,
        _NET_WM_WINDOW_TYPE_DOCK,
    }
}

pub struct X11 {
    pub conn: RustConnection,
    pub root: Window,
    screen: usize,
    pub atoms: Atoms,
    moveresize_supported: bool,
}

const NET_WM_STATE_REMOVE: u32 = 0;
const NET_WM_STATE_TOGGLE: u32 = 2;
const SOURCE_PAGER: u32 = 2;
const STATIC_GRAVITY: u32 = 10;
const XI_ALL_MASTER_DEVICES: xinput::DeviceId = 1;

impl X11 {
    pub fn connect() -> anyhow::Result<Self> {
        let (conn, screen) = RustConnection::connect(None).context(
            "cannot connect to an X server; Sash needs an X11 session (DISPLAY) or XWayland",
        )?;
        let root = conn.setup().roots[screen].root;
        let atoms = Atoms::new(&conn)?.reply()?;
        let mut x = Self {
            conn,
            root,
            screen,
            atoms,
            moveresize_supported: false,
        };
        x.moveresize_supported = x
            .cardinals(root, x.atoms._NET_SUPPORTED)?
            .contains(&x.atoms._NET_MOVERESIZE_WINDOW);
        Ok(x)
    }

    fn screen(&self) -> &Screen {
        &self.conn.setup().roots[self.screen]
    }

    fn cardinals(&self, window: Window, property: Atom) -> anyhow::Result<Vec<u32>> {
        let reply = self
            .conn
            .get_property(false, window, property, AtomEnum::ANY, 0, 4096)?
            .reply()?;
        Ok(reply.value32().map(Iterator::collect).unwrap_or_default())
    }

    fn extents(&self, window: Window, property: Atom) -> Extents {
        match self.cardinals(window, property).as_deref() {
            Ok([l, r, t, b, ..]) => Extents {
                left: *l as i32,
                right: *r as i32,
                top: *t as i32,
                bottom: *b as i32,
            },
            _ => Extents::default(),
        }
    }

    pub fn active_window(&self) -> anyhow::Result<Option<Window>> {
        let window = self
            .cardinals(self.root, self.atoms._NET_ACTIVE_WINDOW)?
            .first()
            .copied();
        Ok(window.filter(|&w| w != 0 && w != self.root))
    }

    pub fn client_list(&self) -> anyhow::Result<Vec<Window>> {
        self.cardinals(self.root, self.atoms._NET_CLIENT_LIST)
    }

    /// Whether the window is the desktop or a panel, which Sash never moves.
    pub fn is_desktop_or_dock(&self, window: Window) -> bool {
        let types = self
            .cardinals(window, self.atoms._NET_WM_WINDOW_TYPE)
            .unwrap_or_default();
        types.contains(&self.atoms._NET_WM_WINDOW_TYPE_DESKTOP)
            || types.contains(&self.atoms._NET_WM_WINDOW_TYPE_DOCK)
    }

    /// WM_CLASS as (instance, class).
    pub fn wm_class(&self, window: Window) -> (String, String) {
        let value = self
            .conn
            .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| reply.value)
            .unwrap_or_default();
        let mut parts = value
            .split(|&b| b == 0)
            .map(|p| String::from_utf8_lossy(p).into_owned());
        (
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default(),
        )
    }

    /// The visible frame of a managed client window, including window-manager
    /// decorations and excluding client-side shadows.
    pub fn frame(&self, window: Window) -> anyhow::Result<Rect> {
        let geometry = self.conn.get_geometry(window)?.reply()?;
        let origin = self
            .conn
            .translate_coordinates(window, self.root, 0, 0)?
            .reply()?;
        let client = Rect::new(
            i32::from(origin.dst_x),
            i32::from(origin.dst_y),
            i32::from(geometry.width),
            i32::from(geometry.height),
        );
        Ok(client
            .outset(self.extents(window, self.atoms._NET_FRAME_EXTENTS))
            .inset(self.extents(window, self.atoms._GTK_FRAME_EXTENTS)))
    }

    /// Moves and resizes a client window so its visible frame becomes `frame`.
    pub fn set_frame(&self, window: Window, frame: Rect) -> anyhow::Result<()> {
        let a = &self.atoms;
        self.send_root_message(
            window,
            a._NET_WM_STATE,
            [
                NET_WM_STATE_REMOVE,
                a._NET_WM_STATE_MAXIMIZED_VERT,
                a._NET_WM_STATE_MAXIMIZED_HORZ,
                SOURCE_PAGER,
                0,
            ],
        )?;
        self.send_root_message(
            window,
            a._NET_WM_STATE,
            [
                NET_WM_STATE_REMOVE,
                a._NET_WM_STATE_FULLSCREEN,
                0,
                SOURCE_PAGER,
                0,
            ],
        )?;
        let client = frame
            .inset(self.extents(window, a._NET_FRAME_EXTENTS))
            .outset(self.extents(window, a._GTK_FRAME_EXTENTS));
        debug!("window {window:#x}: frame {frame:?}, client {client:?}");
        if self.moveresize_supported {
            let flags = STATIC_GRAVITY | 0b1111 << 8 | SOURCE_PAGER << 12;
            self.send_root_message(
                window,
                a._NET_MOVERESIZE_WINDOW,
                [
                    flags,
                    client.x as u32,
                    client.y as u32,
                    client.w.max(1) as u32,
                    client.h.max(1) as u32,
                ],
            )?;
        } else {
            let aux = ConfigureWindowAux::new()
                .x(client.x)
                .y(client.y)
                .width(client.w.max(1) as u32)
                .height(client.h.max(1) as u32);
            self.conn.configure_window(window, &aux)?;
        }
        self.conn.flush()?;
        Ok(())
    }

    pub fn toggle_above(&self, window: Window) -> anyhow::Result<()> {
        let a = &self.atoms;
        self.send_root_message(
            window,
            a._NET_WM_STATE,
            [
                NET_WM_STATE_TOGGLE,
                a._NET_WM_STATE_ABOVE,
                0,
                SOURCE_PAGER,
                0,
            ],
        )?;
        self.conn.flush()?;
        Ok(())
    }

    fn send_root_message(&self, window: Window, kind: Atom, data: [u32; 5]) -> anyhow::Result<()> {
        let event = ClientMessageEvent::new(32, window, kind, data);
        let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
        self.conn.send_event(false, self.root, mask, event)?;
        Ok(())
    }

    pub fn pointer(&self) -> anyhow::Result<(i32, i32)> {
        let reply = self.conn.query_pointer(self.root)?.reply()?;
        Ok((i32::from(reply.root_x), i32::from(reply.root_y)))
    }

    /// The managed client whose top-level frame is under the pointer.
    pub fn client_under_pointer(&self) -> anyhow::Result<Option<Window>> {
        let toplevel = self.conn.query_pointer(self.root)?.reply()?.child;
        if toplevel == x11rb::NONE {
            return Ok(None);
        }
        for client in self.client_list()? {
            if self.toplevel_of(client)? == Some(toplevel) {
                return Ok(Some(client));
            }
        }
        Ok(None)
    }

    fn toplevel_of(&self, mut window: Window) -> anyhow::Result<Option<Window>> {
        loop {
            let Ok(reply) = self.conn.query_tree(window)?.reply() else {
                return Ok(None);
            };
            if reply.parent == self.root || reply.parent == x11rb::NONE {
                return Ok(Some(window));
            }
            window = reply.parent;
        }
    }

    pub fn monitors(&self) -> anyhow::Result<Vec<Monitor>> {
        let screen = self.screen();
        let (root_w, root_h) = (
            i32::from(screen.width_in_pixels),
            i32::from(screen.height_in_pixels),
        );
        let mut frames: Vec<Rect> = match self
            .conn
            .randr_get_monitors(self.root, true)
            .map(|c| c.reply())
        {
            Ok(Ok(reply)) => reply
                .monitors
                .iter()
                .map(|m| {
                    Rect::new(
                        i32::from(m.x),
                        i32::from(m.y),
                        i32::from(m.width),
                        i32::from(m.height),
                    )
                })
                .collect(),
            _ => Vec::new(),
        };
        if frames.is_empty() {
            frames.push(Rect::new(0, 0, root_w, root_h));
        }
        frames.sort_by_key(|r| (r.x, r.y));

        let struts = self.struts(root_w, root_h);
        let desktop_work = self.desktop_work_area();
        Ok(frames
            .into_iter()
            .map(|frame| {
                let mut work = struts.iter().fold(frame, |work, strut| strut.clip(work));
                if let Some(desktop) = desktop_work {
                    work = work.intersection(&desktop).unwrap_or(work);
                }
                Monitor { frame, work }
            })
            .collect())
    }

    fn desktop_work_area(&self) -> Option<Rect> {
        let desktop = self
            .cardinals(self.root, self.atoms._NET_CURRENT_DESKTOP)
            .ok()?
            .first()
            .copied()
            .unwrap_or(0);
        let areas = self.cardinals(self.root, self.atoms._NET_WORKAREA).ok()?;
        let i = desktop as usize * 4;
        match areas.get(i..i + 4) {
            Some(&[x, y, w, h]) if w > 0 && h > 0 => {
                Some(Rect::new(x as i32, y as i32, w as i32, h as i32))
            }
            _ => None,
        }
    }

    fn struts(&self, root_w: i32, root_h: i32) -> Vec<Strut> {
        let clients = self.client_list().unwrap_or_default();
        let mut out = Vec::new();
        for client in clients {
            let partial = self
                .cardinals(client, self.atoms._NET_WM_STRUT_PARTIAL)
                .unwrap_or_default();
            let values: Vec<i32> = if partial.len() >= 12 {
                partial.iter().map(|&v| v as i32).collect()
            } else {
                let plain = self
                    .cardinals(client, self.atoms._NET_WM_STRUT)
                    .unwrap_or_default();
                if plain.len() < 4 {
                    continue;
                }
                let s: Vec<i32> = plain.iter().map(|&v| v as i32).collect();
                vec![
                    s[0], s[1], s[2], s[3], 0, root_h, 0, root_h, 0, root_w, 0, root_w,
                ]
            };
            out.push(Strut {
                root_w,
                root_h,
                v: values,
            });
        }
        out
    }
}

/// A panel reservation in `_NET_WM_STRUT_PARTIAL` form, relative to root edges.
struct Strut {
    root_w: i32,
    root_h: i32,
    v: Vec<i32>,
}

impl Strut {
    fn clip(&self, work: Rect) -> Rect {
        let v = &self.v;
        let spans = |start: i32, end: i32, lo: i32, hi: i32| start < hi && end >= lo;
        let (mut left, mut right, mut top, mut bottom) =
            (work.x, work.right(), work.y, work.bottom());
        if v[0] > 0 && spans(v[4], v[5], work.y, work.bottom()) {
            left = left.max(v[0]);
        }
        if v[1] > 0 && spans(v[6], v[7], work.y, work.bottom()) {
            right = right.min(self.root_w - v[1]);
        }
        if v[2] > 0 && spans(v[8], v[9], work.x, work.right()) {
            top = top.max(v[2]);
        }
        if v[3] > 0 && spans(v[10], v[11], work.x, work.right()) {
            bottom = bottom.min(self.root_h - v[3]);
        }
        if right > left && bottom > top {
            Rect::new(left, top, right - left, bottom - top)
        } else {
            work
        }
    }
}

/// Raw pointer events from all devices, delivered even while another client
/// (such as the window manager during a move) holds the pointer grab.
pub fn select_raw_pointer_events(x: &X11) -> anyhow::Result<()> {
    let version = x
        .conn
        .xinput_xi_query_version(2, 2)?
        .reply()
        .context("XInput 2 is not available")?;
    if (version.major_version, version.minor_version) < (2, 1) {
        bail!(
            "XInput {}.{} is too old for raw events",
            version.major_version,
            version.minor_version
        );
    }
    let mask = xinput::XIEventMask::RAW_BUTTON_PRESS
        | xinput::XIEventMask::RAW_BUTTON_RELEASE
        | xinput::XIEventMask::RAW_MOTION;
    x.conn
        .xinput_xi_select_events(
            x.root,
            &[xinput::EventMask {
                deviceid: XI_ALL_MASTER_DEVICES,
                mask: vec![mask],
            }],
        )?
        .check()?;
    Ok(())
}

pub struct Hotkeys {
    bindings: HashMap<(Keycode, u16), Shortcut>,
}

const RELEVANT_MODIFIERS: u16 = 0x01 | 0x04 | 0x08 | 0x40; // Shift, Control, Mod1, Mod4
/// Caps Lock (Lock) and Num Lock (Mod2) must not prevent a shortcut from firing.
const LOCK_VARIANTS: [u16; 4] = [0, 0x02, 0x10, 0x02 | 0x10];

impl Hotkeys {
    /// Grabs every shortcut on the root window. Shortcuts already grabbed by
    /// another client are reported and skipped.
    pub fn grab(x: &X11, shortcuts: &[&Shortcut]) -> anyhow::Result<Self> {
        let setup = x.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let mapping = x.conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
        let per = usize::from(mapping.keysyms_per_keycode).max(1);
        let keycode_of = |keysym: u32| {
            let rows = || mapping.keysyms.chunks(per).zip(min..=max);
            rows()
                .find(|(syms, _)| syms[0] == keysym)
                .or_else(|| rows().find(|(syms, _)| syms.contains(&keysym)))
                .map(|(_, code)| code)
        };

        let mut bindings = HashMap::new();
        for &shortcut in shortcuts {
            let Some(keycode) = keycode_of(shortcut.keysym) else {
                warn!("{shortcut}: key is not on this keyboard layout");
                continue;
            };
            let m = shortcut.modifiers;
            let mods = u16::from(m.shift)
                | u16::from(m.ctrl) << 2
                | u16::from(m.alt) << 3
                | u16::from(m.super_) << 6;
            let variants = LOCK_VARIANTS.map(|extra| ModMask::from(mods | extra));
            let mut grabbed = true;
            for variant in variants {
                let result = x
                    .conn
                    .grab_key(
                        true,
                        x.root,
                        variant,
                        keycode,
                        GrabMode::ASYNC,
                        GrabMode::ASYNC,
                    )?
                    .check();
                grabbed &= result.is_ok();
            }
            if grabbed {
                bindings.insert((keycode, mods), shortcut.clone());
            } else {
                warn!(
                    "{shortcut}: already used by another application; rebind it in commands_file"
                );
                for variant in variants {
                    x.conn.ungrab_key(keycode, x.root, variant)?;
                }
            }
        }
        x.conn.flush()?;
        Ok(Self { bindings })
    }

    pub fn ungrab(&self, x: &X11) -> anyhow::Result<()> {
        for &(keycode, mods) in self.bindings.keys() {
            for extra in LOCK_VARIANTS {
                x.conn
                    .ungrab_key(keycode, x.root, ModMask::from(mods | extra))?;
            }
        }
        x.conn.flush()?;
        Ok(())
    }

    pub fn lookup(&self, keycode: Keycode, state: KeyButMask) -> Option<&Shortcut> {
        self.bindings
            .get(&(keycode, u16::from(state) & RELEVANT_MODIFIERS))
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }
}

/// Preview of the target frame while a window is dragged. Never takes input.
pub struct Overlay {
    pub window: Window,
    gc: Gcontext,
    fill: Option<u32>,
    outline: u32,
    outline_width: u16,
    shown: Option<Rect>,
}

impl Overlay {
    pub fn new(x: &X11, highlight: &Highlight) -> anyhow::Result<Self> {
        let screen = x.screen();
        let argb_visual = screen
            .allowed_depths
            .iter()
            .filter(|d| d.depth == 32)
            .flat_map(|d| d.visuals.iter())
            .find(|v| v.class == VisualClass::TRUE_COLOR)
            .map(|v| v.visual_id);
        let compositing = {
            let name = format!("_NET_WM_CM_S{}", x.screen);
            let atom = x.conn.intern_atom(false, name.as_bytes())?.reply()?.atom;
            x.conn.get_selection_owner(atom)?.reply()?.owner != x11rb::NONE
        };

        let window = x.conn.generate_id()?;
        let aux = CreateWindowAux::new()
            .override_redirect(1)
            .event_mask(EventMask::EXPOSURE)
            .border_pixel(0);
        let (fill, outline) = match argb_visual.filter(|_| compositing) {
            Some(visual) => {
                let colormap = x.conn.generate_id()?;
                x.conn
                    .create_colormap(ColormapAlloc::NONE, colormap, x.root, visual)?;
                let aux = aux.background_pixel(0).colormap(colormap);
                x.conn.create_window(
                    32,
                    window,
                    x.root,
                    0,
                    0,
                    1,
                    1,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    visual,
                    &aux,
                )?;
                (
                    Some(highlight.fill.argb_premultiplied()),
                    highlight.outline.argb_premultiplied(),
                )
            }
            None => {
                let aux = aux.background_pixel(screen.black_pixel);
                x.conn.create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    window,
                    x.root,
                    0,
                    0,
                    1,
                    1,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    screen.root_visual,
                    &aux,
                )?;
                (None, highlight.outline.rgb())
            }
        };
        x.conn.shape_rectangles(
            shape::SO::SET,
            shape::SK::INPUT,
            ClipOrdering::UNSORTED,
            window,
            0,
            0,
            &[],
        )?;
        let gc = x.conn.generate_id()?;
        x.conn.create_gc(gc, window, &CreateGCAux::new())?;
        Ok(Self {
            window,
            gc,
            fill,
            outline,
            outline_width: highlight.outline_width.max(1),
            shown: None,
        })
    }

    pub fn show(&mut self, x: &X11, frame: Rect) -> anyhow::Result<()> {
        if self.shown == Some(frame) {
            return Ok(());
        }
        let aux = ConfigureWindowAux::new()
            .x(frame.x)
            .y(frame.y)
            .width(frame.w.max(1) as u32)
            .height(frame.h.max(1) as u32)
            .stack_mode(StackMode::ABOVE);
        x.conn.configure_window(self.window, &aux)?;
        if self.fill.is_none() {
            let ring = self.ring(frame.w, frame.h);
            x.conn.shape_rectangles(
                shape::SO::SET,
                shape::SK::BOUNDING,
                ClipOrdering::UNSORTED,
                self.window,
                0,
                0,
                &ring,
            )?;
        }
        if self.shown.is_none() {
            x.conn.map_window(self.window)?;
        }
        self.shown = Some(frame);
        self.draw(x)?;
        x.conn.flush()?;
        Ok(())
    }

    pub fn hide(&mut self, x: &X11) -> anyhow::Result<()> {
        if self.shown.take().is_some() {
            x.conn.unmap_window(self.window)?;
            x.conn.flush()?;
        }
        Ok(())
    }

    pub fn draw(&self, x: &X11) -> anyhow::Result<()> {
        let Some(frame) = self.shown else {
            return Ok(());
        };
        let full = Rectangle {
            x: 0,
            y: 0,
            width: frame.w.max(1) as u16,
            height: frame.h.max(1) as u16,
        };
        if let Some(fill) = self.fill {
            x.conn
                .change_gc(self.gc, &ChangeGCAux::new().foreground(fill))?;
            x.conn.poly_fill_rectangle(self.window, self.gc, &[full])?;
        }
        x.conn
            .change_gc(self.gc, &ChangeGCAux::new().foreground(self.outline))?;
        x.conn
            .poly_fill_rectangle(self.window, self.gc, &self.ring(frame.w, frame.h))?;
        Ok(())
    }

    fn ring(&self, w: i32, h: i32) -> [Rectangle; 4] {
        let t = self.outline_width;
        let (w, h) = (w.max(1) as u16, h.max(1) as u16);
        [
            Rectangle {
                x: 0,
                y: 0,
                width: w,
                height: t,
            },
            Rectangle {
                x: 0,
                y: h.saturating_sub(t) as i16,
                width: w,
                height: t,
            },
            Rectangle {
                x: 0,
                y: 0,
                width: t,
                height: h,
            },
            Rectangle {
                x: w.saturating_sub(t) as i16,
                y: 0,
                width: t,
                height: h,
            },
        ]
    }
}

pub struct X11Session {
    pub(crate) x: X11,
    overlay: std::cell::RefCell<Overlay>,
}

impl X11Session {
    pub fn connect(config: &crate::config::Config) -> anyhow::Result<Self> {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            log::warn!("Wayland session: only XWayland windows can be managed");
        }
        let x = X11::connect()?;
        let overlay = Overlay::new(&x, &config.highlight)?;
        Ok(Self {
            x,
            overlay: std::cell::RefCell::new(overlay),
        })
    }
}

impl crate::session::Session for X11Session {
    type Window = Window;

    fn active_window(&self) -> anyhow::Result<Option<Window>> {
        self.x.active_window()
    }

    fn is_reserved(&self, window: Window) -> bool {
        self.x.is_desktop_or_dock(window)
    }

    fn app_names(&self, window: Window) -> (String, String) {
        self.x.wm_class(window)
    }

    fn frame(&self, window: Window) -> anyhow::Result<Rect> {
        self.x.frame(window)
    }

    fn set_frame(&self, window: Window, frame: Rect) -> anyhow::Result<()> {
        self.x.set_frame(window, frame)
    }

    fn toggle_above(&self, window: Window) -> anyhow::Result<()> {
        self.x.toggle_above(window)
    }

    fn monitors(&self) -> anyhow::Result<Vec<Monitor>> {
        self.x.monitors()
    }

    fn pointer(&self) -> anyhow::Result<(i32, i32)> {
        self.x.pointer()
    }

    fn window_under_pointer(&self) -> anyhow::Result<Option<Window>> {
        self.x.client_under_pointer()
    }

    fn exists(&self, window: Window) -> bool {
        self.x
            .client_list()
            .is_ok_and(|clients| clients.contains(&window))
    }

    fn show_overlay(&self, frame: Rect) -> anyhow::Result<()> {
        self.overlay.borrow_mut().show(&self.x, frame)
    }

    fn hide_overlay(&self) -> anyhow::Result<()> {
        self.overlay.borrow_mut().hide(&self.x)
    }
}

pub fn launch(config: crate::config::Config) -> anyhow::Result<()> {
    use x11rb::connection::Connection;
    use x11rb::protocol::Event;

    use crate::daemon::{Daemon, Input};

    let session = X11Session::connect(&config)?;
    let mut daemon = Daemon::start(session, config).context("starting Sash")?;
    let mut hotkeys = Hotkeys::grab(&daemon.c.session.x, &daemon.c.commands.shortcuts())?;
    info!("{} shortcuts active", hotkeys.len());
    if daemon.c.config.snap_windows {
        select_raw_pointer_events(&daemon.c.session.x)
            .context("snapping by dragging needs XInput 2.1")?;
    }
    loop {
        let event = daemon.c.session.x.conn.wait_for_event()?;
        let result = match event {
            Event::KeyPress(key) => {
                if let Some(shortcut) = hotkeys.lookup(key.detail, key.state).cloned() {
                    daemon.handle_input(Input::Shortcut(shortcut))
                } else {
                    Ok(())
                }
            }
            Event::MappingNotify(_) => {
                hotkeys.ungrab(&daemon.c.session.x)?;
                hotkeys = Hotkeys::grab(&daemon.c.session.x, &daemon.c.commands.shortcuts())?;
                Ok(())
            }
            Event::XinputRawButtonPress(press) if press.detail == 1 => {
                daemon.handle_input(Input::Press)
            }
            Event::XinputRawButtonRelease(release) if release.detail == 1 => {
                daemon.handle_input(Input::Release)
            }
            Event::XinputRawMotion(_) => daemon.handle_input(Input::Motion),
            Event::Expose(expose) if expose.window == daemon.c.session.overlay.borrow().window => {
                daemon.c.session.overlay.borrow().draw(&daemon.c.session.x)
            }
            Event::Error(error) => {
                debug!("X error: {error:?}");
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            daemon.recover(error);
        }
    }
}
