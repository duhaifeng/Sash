//! Moves windows through the Accessibility API, so Sash can run in a normal
//! macOS session without an X server.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use anyhow::{Context, bail};

use block2::RcBlock;
use log::{info, warn};
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType,
    NSBezierPath, NSColor, NSCompositingOperation, NSGraphicsContext, NSImage, NSMenu,
    NSMenuDelegate, NSMenuItem, NSPopUpMenuWindowLevel, NSRunningApplication, NSScreen,
    NSStatusBar, NSStatusItem, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
    NSWorkspace,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSData, NSNotification, NSObject, NSObjectProtocol, NSString};

use crate::config::{Color, Config};
use crate::daemon::{Daemon, Input};
use crate::geometry::{Monitor, Rect};
use crate::session::Session;
use crate::shortcut::Shortcut;

mod login;
mod menu;
mod settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MacWindow {
    pid: i32,
    id: u32,
}

pub struct MacSession {
    overlay: Retained<NSWindow>,
    primary_height: Cell<f64>,
    mtm: MainThreadMarker,
}

impl MacSession {
    /// Colours the drag preview, so edited settings show without a restart.
    fn set_highlight(&self, highlight: &crate::config::Highlight) {
        self.overlay
            .setBackgroundColor(Some(&ns_color(highlight.fill)));
    }

    pub fn connect(config: &Config) -> anyhow::Result<Self> {
        ensure_trusted()?;
        let mtm = MainThreadMarker::new().context_main()?;
        let app = NSApplication::sharedApplication(mtm);
        // Sash lives only in the menu bar, without a Dock icon.
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        let overlay = overlay_window(mtm, &config.highlight);
        Ok(Self {
            overlay,
            primary_height: Cell::new(0.0),
            mtm,
        })
    }
}

trait MainThreadContext {
    fn context_main(self) -> anyhow::Result<MainThreadMarker>;
}

impl MainThreadContext for Option<MainThreadMarker> {
    fn context_main(self) -> anyhow::Result<MainThreadMarker> {
        self.ok_or_else(|| anyhow::anyhow!("AppKit must be used on the main thread"))
    }
}

impl Session for MacSession {
    type Window = MacWindow;

    fn active_window(&self) -> anyhow::Result<Option<MacWindow>> {
        let Some(app) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
            return Ok(None);
        };
        let pid = app.processIdentifier();
        if pid == std::process::id() as i32 {
            return Ok(None);
        }
        let application = ax_app(pid);
        let Some(focused) = copy_attr(application, c"AXFocusedWindow") else {
            release(application);
            return Ok(None);
        };
        release(application);
        let id = window_number(focused);
        release(focused);
        Ok(id.map(|id| MacWindow { pid, id }))
    }

    fn is_reserved(&self, window: MacWindow) -> bool {
        if window.pid == std::process::id() as i32 {
            return true;
        }
        let Some(element) = element(window) else {
            return false;
        };
        let desktop = string_attr_eq(element, c"AXSubrole", c"AXDesktop");
        release(element);
        desktop
    }

    fn app_names(&self, window: MacWindow) -> (String, String) {
        let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(window.pid)
        else {
            return (String::new(), String::new());
        };
        (
            ns_string(app.localizedName().as_deref()),
            ns_string(app.bundleIdentifier().as_deref()),
        )
    }

    fn frame(&self, window: MacWindow) -> anyhow::Result<Rect> {
        let Some(element) = element(window) else {
            bail!("window is gone");
        };
        let frame = element_frame(element).ok_or_else(|| anyhow::anyhow!("window has no frame"))?;
        release(element);
        Ok(frame)
    }

    fn set_frame(&self, window: MacWindow, frame: Rect) -> anyhow::Result<()> {
        let Some(element) = element(window) else {
            bail!("window is gone");
        };
        set_bool(element, c"AXFullScreen", false);
        let point = CGPoint::new(f64::from(frame.x), f64::from(frame.y));
        let size = CGSize::new(f64::from(frame.w.max(1)), f64::from(frame.h.max(1)));
        let position = ax_value(1, &point);
        let dimensions = ax_value(2, &size);
        let attr_size = cf_string(c"AXSize");
        let attr_pos = cf_string(c"AXPosition");
        let size_error = unsafe { AXUIElementSetAttributeValue(element, attr_size, dimensions) };
        let pos_error = unsafe { AXUIElementSetAttributeValue(element, attr_pos, position) };
        unsafe { AXUIElementSetAttributeValue(element, attr_size, dimensions) };
        release(attr_size);
        release(attr_pos);
        release(dimensions);
        release(position);
        release(element);
        if size_error != 0 || pos_error != 0 {
            bail!("the window refused the new frame (position {pos_error}, size {size_error})");
        }
        Ok(())
    }

    fn toggle_above(&self, window: MacWindow) -> anyhow::Result<()> {
        toggle_window_level(window.id)
    }

    fn monitors(&self) -> anyhow::Result<Vec<Monitor>> {
        let screens = NSScreen::screens(self.mtm);
        let mut primary_height = 0.0;
        let mut frames = Vec::new();
        for index in 0..screens.count() {
            let screen = screens.objectAtIndex(index);
            let frame = screen.frame();
            if frame.origin.x == 0.0 && frame.origin.y == 0.0 {
                primary_height = frame.size.height;
            }
            frames.push((frame, screen.visibleFrame()));
        }
        if primary_height == 0.0 {
            primary_height = frames
                .first()
                .map(|(frame, _)| frame.size.height)
                .unwrap_or(0.0);
        }
        self.primary_height.set(primary_height);
        let mut monitors: Vec<_> = frames
            .into_iter()
            .map(|(frame, visible)| Monitor {
                frame: cocoa_to_ax(frame, primary_height),
                work: cocoa_to_ax(visible, primary_height),
            })
            .collect();
        monitors.sort_by_key(|monitor| (monitor.frame.x, monitor.frame.y));
        Ok(monitors)
    }

    fn pointer(&self) -> anyhow::Result<(i32, i32)> {
        Ok(pointer())
    }

    fn window_under_pointer(&self) -> anyhow::Result<Option<MacWindow>> {
        let (x, y) = pointer();
        let system = unsafe { AXUIElementCreateSystemWide() };
        let mut found = std::ptr::null_mut();
        let status =
            unsafe { AXUIElementCopyElementAtPosition(system, x as f32, y as f32, &mut found) };
        release(system);
        if status != 0 || found.is_null() {
            return Ok(None);
        }
        let Some(window) = containing_window(found) else {
            return Ok(None);
        };
        let mut pid = 0;
        unsafe { AXUIElementGetPid(window, &mut pid) };
        let id = window_number(window);
        release(window);
        Ok(id.map(|id| MacWindow { pid, id }))
    }

    fn exists(&self, window: MacWindow) -> bool {
        if let Some(element) = element(window) {
            release(element);
            true
        } else {
            false
        }
    }

    fn show_overlay(&self, frame: Rect) -> anyhow::Result<()> {
        let cocoa = ax_to_cocoa(frame, self.primary_height.get());
        self.overlay.setFrame_display(cocoa, true);
        self.overlay.orderFrontRegardless();
        Ok(())
    }

    fn hide_overlay(&self) -> anyhow::Result<()> {
        self.overlay.orderOut(None);
        Ok(())
    }
}

pub fn launch(config: Config) -> anyhow::Result<()> {
    let session = MacSession::connect(&config)?;
    let mut daemon = Daemon::start(session, config).context("starting Sash")?;
    let shortcuts: Vec<Shortcut> = daemon.c.commands.shortcuts().into_iter().cloned().collect();
    let active = register_hotkeys(&shortcuts);
    if daemon.c.config.snap_windows {
        install_mouse_tap()?;
    }
    info!("{active} shortcuts active");
    let mtm = MainThreadMarker::new().context_main()?;
    let app = NSApplication::sharedApplication(mtm);
    let delegate = AppDelegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    DAEMON.with(|slot| slot.set(&mut daemon));
    app.run();
    Ok(())
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            let mtm = self.mtm();
            let app = NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
            // The icon only shows in the About panel; there is no Dock icon.
            if let Some(icon) = app_icon() {
                unsafe { app.setApplicationIconImage(Some(&icon)) };
            }
            menu::install_main_menu(mtm, self);
            install_status_item(mtm, self);
        }
    }

    impl AppDelegate {
        /// Starting Sash again, from Spotlight or the Applications folder, opens
        /// the settings; this is how a hidden menu bar icon is reached.
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(&self, _app: &NSApplication, _visible: Bool) -> Bool {
            settings::show(self.mtm());
            Bool::YES
        }
    }

    unsafe impl NSMenuDelegate for AppDelegate {
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self, menu: &NSMenu) {
            menu::rebuild(menu, self.mtm(), self);
        }
    }

    impl AppDelegate {
        #[unsafe(method(runCommand:))]
        fn run_command(&self, sender: &NSMenuItem) {
            // Item titles are command names, and the menu only lists the active
            // window's own commands.
            enqueue(Input::Command(sender.title().to_string()));
        }

        #[unsafe(method(toggleIgnore:))]
        fn toggle_ignore(&self, _sender: &NSMenuItem) {
            with_daemon(|daemon| {
                let Some(app) = daemon.c.menu_context().app else {
                    return;
                };
                if let Err(error) = daemon.c.config.set_ignored_by_menu(&app.id, !app.ignored_by_menu) {
                    warn!("{error:#}");
                }
            });
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: &NSMenuItem) {
            settings::show(self.mtm());
        }

        #[unsafe(method(authorize:))]
        fn authorize(&self, _sender: &NSMenuItem) {
            open_with_system(&[
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            ]);
        }

        #[unsafe(method(showAbout:))]
        fn show_about(&self, _sender: &NSMenuItem) {
            let app = NSApplication::sharedApplication(self.mtm());
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
            app.orderFrontStandardAboutPanel(None);
        }
    }
);

fn open_with_system(args: &[&str]) {
    if let Err(error) = std::process::Command::new("/usr/bin/open")
        .args(args)
        .status()
    {
        warn!("open {args:?}: {error}");
    }
}

fn with_daemon<R>(f: impl FnOnce(&mut Daemon<MacSession>) -> R) -> Option<R> {
    DAEMON.with(|slot| {
        let daemon = slot.get();
        // SAFETY: the pointer is set in `launch` to a daemon that outlives the run
        // loop, and every caller runs on the main thread outside `enqueue`.
        (!daemon.is_null()).then(|| f(unsafe { &mut *daemon }))
    })
}

fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

impl AppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

fn install_status_item(mtm: MainThreadMarker, delegate: &AppDelegate) {
    let bar = NSStatusBar::systemStatusBar();
    let item = bar.statusItemWithLength(bar.thickness());
    // A stable name keeps this out of the hidden Item-N slots macOS assigns
    // to unnamed extras when the menu bar is crowded.
    item.setAutosaveName(Some(&NSString::from_str("local.sash")));
    if let Some(button) = item.button(mtm) {
        button.setImage(Some(&status_icon()));
    }
    let menu = NSMenu::new(mtm);
    menu.setDelegate(Some(ProtocolObject::from_ref(delegate)));
    item.setMenu(Some(&menu));
    item.setVisible(with_daemon(|d| !d.c.config.hide_menu_bar_icon).unwrap_or(true));
    STATUS.with(|slot| *slot.borrow_mut() = Some(item));
}

/// The About panel icon is `assets/sash.svg`, so the packaged app and the
/// running process cannot drift apart.
fn app_icon() -> Option<Retained<NSImage>> {
    let data = NSData::with_bytes(include_bytes!("../assets/sash.svg"));
    NSImage::initWithData(NSImage::alloc(), &data)
}

/// Menu-bar template: a window in each corner and one in the middle. Each window
/// is a title band separated from its body by a thin gap.
fn status_icon() -> Retained<NSImage> {
    let draw = RcBlock::new(|_bounds: CGRect| {
        let Some(context) = NSGraphicsContext::currentContext() else {
            return Bool::NO;
        };
        let clear = |rect: CGRect, radius: f64| {
            context.setCompositingOperation(NSCompositingOperation::Clear);
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, radius, radius).fill();
            context.setCompositingOperation(NSCompositingOperation::SourceOver);
        };
        let window = |x: f64, y: f64, size: f64, radius: f64| {
            NSColor::colorWithCalibratedWhite_alpha(0.0, 1.0).setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                CGRect::new(CGPoint::new(x, y), CGSize::new(size, size)),
                radius,
                radius,
            )
            .fill();
            // The gap sits below the title band; Y grows upward in this image.
            clear(
                CGRect::new(
                    CGPoint::new(x - 0.5, y + size - 2.9),
                    CGSize::new(size + 1.0, 0.9),
                ),
                0.0,
            );
        };
        for (x, y) in [(0.7, 10.3), (10.3, 10.3), (0.7, 0.7), (10.3, 0.7)] {
            window(x, y, 7.0, 1.4);
        }
        // Cut a margin around the middle window so it reads as lying on top.
        clear(
            CGRect::new(CGPoint::new(4.4, 4.4), CGSize::new(9.2, 9.2)),
            2.2,
        );
        window(5.4, 5.4, 7.2, 1.6);
        Bool::YES
    });
    let image =
        NSImage::imageWithSize_flipped_drawingHandler(CGSize::new(18.0, 18.0), false, &draw);
    // The drawing handler is vector, so shrinking the point size scales the art.
    image.setSize(CGSize::new(16.0, 16.0));
    image.setTemplate(true);
    image
}

thread_local! {
    static DAEMON: Cell<*mut Daemon<MacSession>> = const { Cell::new(std::ptr::null_mut()) };
    static QUEUE: RefCell<VecDeque<Input>> = const { RefCell::new(VecDeque::new()) };
    static SHORTCUTS: RefCell<Vec<Shortcut>> = const { RefCell::new(Vec::new()) };
    static STATUS: RefCell<Option<Retained<NSStatusItem>>> = const { RefCell::new(None) };
    static HOTKEYS: RefCell<Vec<*mut c_void>> = const { RefCell::new(Vec::new()) };
}

static HOTKEY_HANDLER: AtomicBool = AtomicBool::new(false);

static BUSY: AtomicBool = AtomicBool::new(false);
static TAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

fn enqueue(input: Input) {
    QUEUE.with(|queue| queue.borrow_mut().push_back(input));
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    loop {
        let next = QUEUE.with(|queue| queue.borrow_mut().pop_front());
        let Some(next) = next else { break };
        DAEMON.with(|slot| {
            let daemon = slot.get();
            if daemon.is_null() {
                return;
            }
            unsafe {
                if let Err(error) = (*daemon).handle_input(next) {
                    (*daemon).recover(error);
                }
            }
        });
    }
    BUSY.store(false, Ordering::SeqCst);
}

fn ensure_trusted() -> anyhow::Result<()> {
    let key = unsafe { kAXTrustedCheckOptionPrompt };
    let value = unsafe { kCFBooleanTrue };
    let options = unsafe {
        CFDictionaryCreate(
            std::ptr::null(),
            &key,
            &value,
            1,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        )
    };
    let trusted = unsafe { AXIsProcessTrustedWithOptions(options) } != 0;
    release(options);
    if trusted {
        return Ok(());
    }
    let path = std::env::current_exe()
        .map_or_else(|_| "sash".to_string(), |path| path.display().to_string());
    bail!(
        "需要辅助功能权限才能移动窗口。系统设置已经打开权限面板，请允许这个程序后重新运行：\n  {path}"
    )
}

fn overlay_window(
    mtm: MainThreadMarker,
    highlight: &crate::config::Highlight,
) -> Retained<NSWindow> {
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            CGRect::new(CGPoint::ZERO, CGSize::new(1.0, 1.0)),
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    window.setOpaque(false);
    window.setHasShadow(false);
    window.setIgnoresMouseEvents(true);
    window.setLevel(NSPopUpMenuWindowLevel);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    window.setBackgroundColor(Some(&ns_color(highlight.fill)));
    unsafe { window.setReleasedWhenClosed(false) };
    window
}

fn ns_color(color: Color) -> Retained<NSColor> {
    NSColor::colorWithRed_green_blue_alpha(
        f64::from(color.r) / 255.0,
        f64::from(color.g) / 255.0,
        f64::from(color.b) / 255.0,
        f64::from(color.a) / 255.0,
    )
}

fn cocoa_to_ax(rect: CGRect, primary_height: f64) -> Rect {
    let top = primary_height - (rect.origin.y + rect.size.height);
    Rect::new(
        rect.origin.x.round() as i32,
        top.round() as i32,
        rect.size.width.round() as i32,
        rect.size.height.round() as i32,
    )
}

fn ax_to_cocoa(frame: Rect, primary_height: f64) -> CGRect {
    let y = primary_height - f64::from(frame.y) - f64::from(frame.h);
    CGRect::new(
        CGPoint::new(f64::from(frame.x), y),
        CGSize::new(f64::from(frame.w.max(1)), f64::from(frame.h.max(1))),
    )
}

fn pointer() -> (i32, i32) {
    unsafe {
        let event = CGEventCreate(std::ptr::null_mut());
        let point = CGEventGetLocation(event);
        release(event);
        (point.x.round() as i32, point.y.round() as i32)
    }
}

fn ax_app(pid: i32) -> *mut c_void {
    let application = unsafe { AXUIElementCreateApplication(pid) };
    unsafe { AXUIElementSetMessagingTimeout(application, 0.4) };
    application
}

fn element(window: MacWindow) -> Option<*mut c_void> {
    let application = ax_app(window.pid);
    let Some(windows) = copy_attr(application, c"AXWindows") else {
        release(application);
        return None;
    };
    let mut found = None;
    let count = unsafe { CFArrayGetCount(windows) };
    for index in 0..count {
        let item = unsafe { CFArrayGetValueAtIndex(windows, index) } as *mut c_void;
        if window_number(item) == Some(window.id) {
            found = Some(unsafe { CFRetain(item) } as *mut c_void);
            break;
        }
    }
    release(windows);
    release(application);
    found
}

fn containing_window(mut element: *mut c_void) -> Option<*mut c_void> {
    for _ in 0..12 {
        if string_attr_eq(element, c"AXRole", c"AXWindow") {
            return Some(element);
        }
        let Some(parent) = copy_attr(element, c"AXParent") else {
            release(element);
            return None;
        };
        release(element);
        element = parent;
    }
    release(element);
    None
}

fn element_frame(element: *mut c_void) -> Option<Rect> {
    let position = copy_attr(element, c"AXPosition")?;
    let size = copy_attr(element, c"AXSize")?;
    let mut point = CGPoint::ZERO;
    let mut dimensions = CGSize::ZERO;
    let point_ok =
        unsafe { AXValueGetValue(position, 1, &mut point as *mut _ as *mut c_void) } != 0;
    let size_ok =
        unsafe { AXValueGetValue(size, 2, &mut dimensions as *mut _ as *mut c_void) } != 0;
    release(position);
    release(size);
    (point_ok && size_ok).then(|| {
        Rect::new(
            point.x.round() as i32,
            point.y.round() as i32,
            dimensions.width.round() as i32,
            dimensions.height.round() as i32,
        )
    })
}

fn window_number(element: *mut c_void) -> Option<u32> {
    let mut id = 0u32;
    (unsafe { _AXUIElementGetWindow(element, &mut id) } == 0 && id != 0).then_some(id)
}

fn string_attr_eq(element: *mut c_void, attribute: &CStr, expected: &CStr) -> bool {
    let Some(value) = copy_attr(element, attribute) else {
        return false;
    };
    let wanted = cf_string(expected);
    let equal = unsafe { CFEqual(value, wanted) } != 0;
    release(value);
    release(wanted);
    equal
}

fn set_bool(element: *mut c_void, attribute: &CStr, value: bool) {
    let attr = cf_string(attribute);
    let boolean = unsafe {
        if value {
            kCFBooleanTrue
        } else {
            kCFBooleanFalse
        }
    };
    unsafe { AXUIElementSetAttributeValue(element, attr, boolean) };
    release(attr);
}

fn copy_attr(element: *mut c_void, name: &CStr) -> Option<*mut c_void> {
    let attribute = cf_string(name);
    let mut value = std::ptr::null_mut();
    let status = unsafe { AXUIElementCopyAttributeValue(element, attribute, &mut value) };
    release(attribute);
    (status == 0 && !value.is_null()).then_some(value)
}

fn ax_value(kind: u32, value: *const impl Sized) -> *mut c_void {
    unsafe { AXValueCreate(kind, value as *const _ as *const c_void) }
}

fn cf_string(text: &CStr) -> *mut c_void {
    unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), 0x0800_0100) }
}

fn ns_string(value: Option<&NSString>) -> String {
    value
        .and_then(|text| unsafe { CStr::from_ptr(text.UTF8String()) }.to_str().ok())
        .unwrap_or("")
        .to_string()
}

fn release(value: *mut c_void) {
    if !value.is_null() {
        unsafe { CFRelease(value) }
    }
}

fn toggle_window_level(window: u32) -> anyhow::Result<()> {
    type Connection = unsafe extern "C" fn() -> i32;
    type GetLevel = unsafe extern "C" fn(i32, u32, *mut i32) -> i32;
    type SetLevel = unsafe extern "C" fn(i32, u32, i32) -> i32;
    let symbol = |name: &[u8]| {
        let found = unsafe { dlsym(-2isize as *mut c_void, name.as_ptr().cast()) };
        (!found.is_null()).then_some(found)
    };
    let connection = symbol(b"CGSMainConnectionID\0");
    let get = symbol(b"CGSGetWindowLevel\0");
    let set = symbol(b"CGSSetWindowLevel\0");
    let (Some(connection), Some(get), Some(set)) = (connection, get, set) else {
        bail!("this macOS version does not allow changing another app's window level");
    };
    let connection: Connection = unsafe { std::mem::transmute(connection) };
    let get: GetLevel = unsafe { std::mem::transmute(get) };
    let set: SetLevel = unsafe { std::mem::transmute(set) };
    let connection = unsafe { connection() };
    let mut level = 0;
    let read = unsafe { get(connection, window, &mut level) };
    if read != 0 {
        bail!("cannot read the window level ({read})");
    }
    let next = if level > 0 { 0 } else { 3 };
    let write = unsafe { set(connection, window, next) };
    if write != 0 {
        bail!("cannot change the window level ({write})");
    }
    Ok(())
}

fn register_hotkeys(shortcuts: &[Shortcut]) -> usize {
    if NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(
        "com.crowdcafe.windowmagnet",
    ))
    .count()
        > 0
    {
        warn!(
            "Magnet.app 正在运行并占用了相同的快捷键。要让这个程序接收快捷键，请先退出 Magnet.app"
        );
    }
    unregister_hotkeys();
    SHORTCUTS.with(|slot| slot.replace(shortcuts.to_vec()));
    let target = unsafe { GetApplicationEventTarget() };
    if !HOTKEY_HANDLER.swap(true, Ordering::SeqCst) {
        let spec = EventTypeSpec {
            event_class: u32::from_be_bytes(*b"keyb"),
            event_kind: 5,
        };
        unsafe {
            InstallEventHandler(
                target,
                hotkey_pressed,
                1,
                &spec,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
    }
    let mut active = 0;
    for (index, shortcut) in shortcuts.iter().enumerate() {
        let Some(keycode) = shortcut.macos_virtual_key() else {
            warn!("{shortcut}: this key has no macOS key code");
            continue;
        };
        let modifiers = shortcut.modifiers;
        let mut carbon = 0u32;
        if modifiers.shift {
            carbon |= 1 << 9;
        }
        if modifiers.ctrl {
            carbon |= 1 << 12;
        }
        if modifiers.alt {
            carbon |= 1 << 11;
        }
        if modifiers.super_ {
            carbon |= 1 << 8;
        }
        let mut registered = std::ptr::null_mut();
        let id = EventHotKeyID {
            signature: u32::from_be_bytes(*b"SASH"),
            id: index as u32,
        };
        // Exclusive registration fails when another app, such as Magnet, already
        // owns the combination. Registering without it still delivers the key.
        let mut status = unsafe {
            RegisterEventHotKey(u32::from(keycode), carbon, id, target, 1, &mut registered)
        };
        if status != 0 {
            status = unsafe {
                RegisterEventHotKey(u32::from(keycode), carbon, id, target, 0, &mut registered)
            };
        }
        if status != 0 {
            warn!("{shortcut}: already used by another application ({status})");
        } else {
            HOTKEYS.with(|slot| slot.borrow_mut().push(registered));
            active += 1;
        }
    }
    active
}

fn unregister_hotkeys() {
    for hotkey in HOTKEYS.with(|slot| std::mem::take(&mut *slot.borrow_mut())) {
        unsafe { UnregisterEventHotKey(hotkey) };
    }
}

/// Starts or pauses drag-to-snap, creating the event tap on first use.
fn set_snapping(enabled: bool) -> anyhow::Result<()> {
    let port = TAP.load(Ordering::SeqCst);
    if port.is_null() {
        if enabled {
            install_mouse_tap()?;
        }
    } else {
        unsafe { CGEventTapEnable(port, enabled) };
    }
    Ok(())
}

fn set_status_item_visible(visible: bool) {
    STATUS.with(|slot| {
        if let Some(item) = slot.borrow().as_ref() {
            item.setVisible(visible);
        }
    });
}

fn install_mouse_tap() -> anyhow::Result<()> {
    let mask = (1u64 << 1) | (1u64 << 2) | (1u64 << 6);
    let port = unsafe { CGEventTapCreate(1, 0, 1, mask, mouse_tap, std::ptr::null_mut()) };
    if port.is_null() {
        bail!("cannot watch the pointer; the Accessibility permission does not cover mouse events");
    }
    let source = unsafe { CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0) };
    unsafe {
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
        CGEventTapEnable(port, true);
    }
    TAP.store(port, Ordering::SeqCst);
    Ok(())
}

unsafe extern "C" fn hotkey_pressed(
    _next: *mut c_void,
    event: *mut c_void,
    _user: *mut c_void,
) -> i32 {
    let mut id = EventHotKeyID {
        signature: 0,
        id: 0,
    };
    unsafe {
        GetEventParameter(
            event,
            u32::from_be_bytes(*b"----"),
            u32::from_be_bytes(*b"hkid"),
            std::ptr::null_mut(),
            std::mem::size_of::<EventHotKeyID>() as u32,
            std::ptr::null_mut(),
            &mut id as *mut _ as *mut c_void,
        )
    };
    if let Some(shortcut) = SHORTCUTS.with(|slot| slot.borrow().get(id.id as usize).cloned()) {
        enqueue(Input::Shortcut(shortcut));
    }
    0
}

unsafe extern "C" fn mouse_tap(
    _proxy: *mut c_void,
    kind: u32,
    event: *mut c_void,
    _user: *mut c_void,
) -> *mut c_void {
    if kind == 0xFFFF_FFFE || kind == 0xFFFF_FFFF {
        let port = TAP.load(Ordering::SeqCst);
        if !port.is_null() {
            unsafe { CGEventTapEnable(port, true) };
        }
        return event;
    }
    let input = match kind {
        1 => Input::Press,
        2 => Input::Release,
        6 => Input::Motion,
        _ => return event,
    };
    enqueue(input);
    event
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
struct CFDictionaryKeyCallBacks {
    version: isize,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
    equal: *const c_void,
    hash: *const c_void,
}

#[repr(C)]
struct CFDictionaryValueCallBacks {
    version: isize,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
    equal: *const c_void,
}

#[allow(clippy::duplicated_attributes)]
#[link(name = "CoreFoundation", kind = "framework")]
#[link(name = "ApplicationServices", kind = "framework")]
#[link(name = "CoreGraphics", kind = "framework")]
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    static kAXTrustedCheckOptionPrompt: *const c_void;
    static kCFBooleanTrue: *const c_void;
    static kCFBooleanFalse: *const c_void;
    static kCFTypeDictionaryKeyCallBacks: CFDictionaryKeyCallBacks;
    static kCFTypeDictionaryValueCallBacks: CFDictionaryValueCallBacks;
    static kCFRunLoopCommonModes: *const c_void;

    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateSystemWide() -> *mut c_void;
    fn AXUIElementCreateApplication(pid: i32) -> *mut c_void;
    fn AXUIElementCopyAttributeValue(
        element: *mut c_void,
        attribute: *const c_void,
        value: *mut *mut c_void,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: *mut c_void,
        attribute: *const c_void,
        value: *const c_void,
    ) -> i32;
    fn AXUIElementCopyElementAtPosition(
        application: *mut c_void,
        x: f32,
        y: f32,
        element: *mut *mut c_void,
    ) -> i32;
    fn AXUIElementGetPid(element: *mut c_void, pid: *mut i32) -> i32;
    fn AXUIElementSetMessagingTimeout(element: *mut c_void, seconds: f32) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> *mut c_void;
    fn AXValueGetValue(value: *const c_void, kind: u32, out: *mut c_void) -> u8;
    fn _AXUIElementGetWindow(element: *mut c_void, window: *mut u32) -> i32;

    fn CFRelease(value: *const c_void);
    fn CFRetain(value: *const c_void) -> *const c_void;
    fn CFEqual(left: *const c_void, right: *const c_void) -> u8;
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        text: *const c_char,
        encoding: u32,
    ) -> *mut c_void;
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        count: isize,
        key_callbacks: *const CFDictionaryKeyCallBacks,
        value_callbacks: *const CFDictionaryValueCallBacks,
    ) -> *mut c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: *mut c_void,
        order: isize,
    ) -> *mut c_void;

    fn CGEventCreate(source: *mut c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events: u64,
        callback: unsafe extern "C" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> *mut c_void,
        user: *mut c_void,
    ) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);

    fn GetApplicationEventTarget() -> *mut c_void;
    fn UnregisterEventHotKey(hotkey: *mut c_void) -> i32;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
        count: u32,
        types: *const EventTypeSpec,
        user: *mut c_void,
        handler_ref: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        keycode: u32,
        modifiers: u32,
        id: EventHotKeyID,
        target: *mut c_void,
        options: u32,
        hotkey: *mut *mut c_void,
    ) -> i32;
    fn GetEventParameter(
        event: *mut c_void,
        name: u32,
        kind: u32,
        actual_kind: *mut u32,
        size: u32,
        actual_size: *mut u32,
        data: *mut c_void,
    ) -> i32;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
