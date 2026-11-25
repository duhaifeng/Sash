//! The settings window, after Magnet's: an icon sidebar with a Preferences
//! page and a Commands page (list on the left, the selected command's cards on
//! the right). Edits change the running daemon at once and are written to
//! `config.toml` and the commands file.

use std::cell::RefCell;
use std::collections::HashSet;

use block2::RcBlock;
use log::warn;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertStyle, NSApplication, NSAutoresizingMaskOptions, NSBackingStoreType, NSButton,
    NSCellImagePosition, NSColor, NSColorPanel, NSColorSpace, NSColorWell, NSControl,
    NSControlSize, NSImageScaling, NSImageView, NSMenu, NSScreen, NSScrollView, NSStepper,
    NSSwitch, NSTextField, NSView, NSWindow, NSWindowButton, NSWindowStyleMask,
    NSWindowTitleVisibility,
};
use objc2_core_foundation::{CGPoint, CGSize};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};

use self::views::{
    ChipButton, FlippedView, GridKind, GridView, Panel, RowKind, RowView, ShortcutField, Tall,
    Tone, font, label, rect, right_aligned, symbol,
};
use super::{
    MacSession, register_hotkeys, set_snapping, set_status_item_visible, unregister_hotkeys,
    with_daemon,
};
use crate::config::{Color, Config};
use crate::daemon::Daemon;
use crate::geometry::GridRect;
use crate::model::{Action, Axis, Command, CommandSet, Special};
use crate::shortcut::Shortcut;

mod views;

const SIDEBAR_W: f64 = 70.0;
const LIST_W: f64 = 275.0;
const DETAIL_W: f64 = 530.0;
const HEADER_H: f64 = 52.0;
const CARD_X: f64 = 22.0;
const CARD_W: f64 = 486.0;
const CARD_PAD: f64 = 14.0;
const ROW: f64 = 30.0;
const CARD_GAP: f64 = 18.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Preferences,
    Commands,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pref {
    Login,
    HideIcon,
    Snap,
    RestoreSize,
    EdgeWidth,
    Scale,
    Padding,
    Fit,
    Fill,
}

/// The control a preference reads its value from.
#[derive(Clone)]
enum Control {
    Check(Retained<NSSwitch>),
    Stepper(Retained<NSStepper>, Retained<NSTextField>),
    Color(Retained<NSColorWell>),
}

/// What a control does when it fires; its tag indexes `Settings::actions`.
#[derive(Clone)]
enum Act {
    Page(Page),
    Pref(Pref, Control),
    RestoreDefaults,
    Collapse(Axis),
    Remove(u64),
    AddMenu(Axis, Retained<NSButton>),
    AddCommand(Axis),
    AddDivider(Axis),
    /// The chip in the selected row: flips visibility in the menu bar menu.
    ToggleMenu(u64),
    MenuSwitch(u64, Retained<NSSwitch>),
    ShortcutSwitch(u64, Retained<NSSwitch>),
    ActivationSwitch(u64, Retained<NSSwitch>),
    Rename(u64, Retained<NSTextField>),
}

struct Settings {
    window: Retained<NSWindow>,
    target: Retained<ActionTarget>,
    /// The sidebar buttons and the highlights behind them.
    sidebar: Vec<(Retained<NSButton>, Retained<Panel>)>,
    list_pane: Retained<Panel>,
    list_scroll: Retained<NSScrollView>,
    divider: Retained<Panel>,
    detail_pane: Retained<Panel>,
    detail_header: Retained<Panel>,
    detail_scroll: Retained<NSScrollView>,
    title: Option<Retained<NSView>>,
    page: Page,
    /// The command (or, with `DIVIDER` set, the divider after a command) the
    /// right-hand side shows.
    selected: u64,
    shown: u64,
    collapsed: [bool; 2],
    /// Commands whose shortcut or activation card is switched on but empty.
    shortcut_open: HashSet<u64>,
    activation_open: HashSet<u64>,
    actions: Vec<Act>,
    /// Replaced views stay alive until the next render, because the control
    /// that triggered the change may still be running.
    retired: Vec<Retained<NSView>>,
}

thread_local! {
    static SETTINGS: RefCell<Option<Settings>> = const { RefCell::new(None) };
}

/// Tags 0 and 1 belong to the sidebar; page controls start after them.
const FIRST_PAGE_TAG: usize = 2;

/// Marks an id as aimed at the divider after a command.
const DIVIDER: u64 = 1 << 63;

fn with<R>(f: impl FnOnce(&mut Settings) -> R) -> Option<R> {
    SETTINGS.with(|slot| slot.try_borrow_mut().ok()?.as_mut().map(f))
}

fn slot(axis: Axis) -> usize {
    match axis {
        Axis::Horizontal => 0,
        Axis::Vertical => 1,
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct ActionTarget;

    unsafe impl NSObjectProtocol for ActionTarget {}

    impl ActionTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            dispatch(tag);
        }

        #[unsafe(method(rerender))]
        fn rerender(&self) {
            if let Some(mtm) = MainThreadMarker::new() {
                with(|s| render(s, mtm));
            }
        }
    }
);

impl ActionTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

/// Opens the window, or brings it forward, showing current values.
pub(super) fn show(mtm: MainThreadMarker) {
    if SETTINGS.with(|slot| slot.borrow().is_none()) {
        let settings = create(mtm);
        SETTINGS.with(|slot| *slot.borrow_mut() = Some(settings));
        with(|s| s.window.center());
    }
    with(|s| {
        render(s, mtm);
        s.window.makeKeyAndOrderFront(None);
    });
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
}

/// Magnet's window is 830 points tall; shorter screens get what fits.
fn default_height(mtm: MainThreadMarker) -> f64 {
    let visible =
        NSScreen::mainScreen(mtm).map_or(900.0, |screen| screen.visibleFrame().size.height);
    (visible - 40.0).clamp(480.0, 830.0)
}

fn scroll_view(
    mtm: MainThreadMarker,
    frame: objc2_core_foundation::CGRect,
) -> Retained<NSScrollView> {
    let scroll = NSScrollView::new(mtm);
    scroll.setFrame(frame);
    scroll.setHasVerticalScroller(true);
    scroll.setDrawsBackground(false);
    scroll.setAutohidesScrollers(true);
    scroll.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);
    scroll
}

fn create(mtm: MainThreadMarker) -> Settings {
    let target = ActionTarget::new(mtm);
    let height = default_height(mtm);
    let width = SIDEBAR_W + DETAIL_W;
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            rect(0.0, 0.0, width, height),
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable
                | NSWindowStyleMask::FullSizeContentView,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str("Sash"));
    window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    window.setTitlebarAppearsTransparent(true);
    if let Some(zoom) = window.standardWindowButton(NSWindowButton::ZoomButton) {
        zoom.setHidden(true);
    }

    let root = FlippedView::new(mtm, rect(0.0, 0.0, width, height));
    window.setContentView(Some(&root));

    // Sidebar: one icon per page.
    let sidebar_panel =
        Panel::new(mtm, Tone::Sidebar, 0.0, rect(0.0, 0.0, SIDEBAR_W, height)).tall();
    let edge = Panel::new(
        mtm,
        Tone::Hairline,
        0.0,
        rect(SIDEBAR_W - 1.0, 0.0, 1.0, height),
    )
    .tall();
    sidebar_panel.addSubview(&edge);
    let mut sidebar = Vec::new();
    for (index, (name, tooltip)) in [("switch.2", "Preferences"), ("command", "Commands")]
        .into_iter()
        .enumerate()
    {
        let frame = rect(13.0, 62.0 + 49.0 * index as f64, 44.0, 44.0);
        let highlight = Panel::new(mtm, Tone::Accent, 10.0, frame);
        sidebar_panel.addSubview(&highlight);
        let button = icon_button(mtm, name, 22.0, tooltip, frame);
        button.setTag(index as isize);
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(Some(sel!(fire:)));
        }
        sidebar_panel.addSubview(&button);
        sidebar.push((button, highlight));
    }
    root.addSubview(&sidebar_panel);

    // The command list.
    let list_x = SIDEBAR_W;
    let list_pane = Panel::new(mtm, Tone::List, 0.0, rect(list_x, 0.0, LIST_W, height)).tall();
    let list_header = Panel::new(mtm, Tone::Header, 0.0, rect(0.0, 0.0, LIST_W, HEADER_H));
    let list_title = label(mtm, "Commands", 15.0, true, false);
    list_title.setFrameOrigin(CGPoint::new(
        24.0,
        (HEADER_H - list_title.frame().size.height) / 2.0,
    ));
    list_header.addSubview(&list_title);
    let list_scroll = scroll_view(mtm, rect(0.0, HEADER_H, LIST_W, height - HEADER_H));
    list_pane.addSubview(&list_scroll);
    list_pane.addSubview(&list_header);
    root.addSubview(&list_pane);
    let divider = Panel::new(
        mtm,
        Tone::Hairline,
        0.0,
        rect(list_x + LIST_W, 0.0, 1.0, height),
    )
    .tall();
    root.addSubview(&divider);

    // The detail side.
    let detail_pane = Panel::new(
        mtm,
        Tone::Detail,
        0.0,
        rect(SIDEBAR_W, 0.0, DETAIL_W, height),
    )
    .tall();
    let detail_header = Panel::new(mtm, Tone::Header, 0.0, rect(0.0, 0.0, DETAIL_W, HEADER_H));
    let detail_scroll = scroll_view(mtm, rect(0.0, HEADER_H, DETAIL_W, height - HEADER_H));
    detail_pane.addSubview(&detail_scroll);
    detail_pane.addSubview(&detail_header);
    root.addSubview(&detail_pane);

    Settings {
        window,
        target,
        sidebar,
        list_pane,
        list_scroll,
        divider,
        detail_pane,
        detail_header,
        detail_scroll,
        title: None,
        page: Page::Preferences,
        selected: 0,
        shown: u64::MAX,
        collapsed: [false; 2],
        shortcut_open: HashSet::new(),
        activation_open: HashSet::new(),
        actions: vec![Act::Page(Page::Preferences), Act::Page(Page::Commands)],
        retired: Vec::new(),
    }
}

/// A borderless button showing only a symbol.
fn icon_button(
    mtm: MainThreadMarker,
    name: &str,
    points: f64,
    tooltip: &str,
    frame: objc2_core_foundation::CGRect,
) -> Retained<NSButton> {
    let button = match symbol(name, points) {
        Some(image) => unsafe { NSButton::buttonWithImage_target_action(&image, None, None, mtm) },
        None => unsafe {
            NSButton::buttonWithTitle_target_action(&NSString::from_str(tooltip), None, None, mtm)
        },
    };
    button.setFrame(frame);
    button.setBordered(false);
    button.setImagePosition(NSCellImagePosition::ImageOnly);
    button.setImageScaling(NSImageScaling::ScaleNone);
    button.setToolTip(Some(&NSString::from_str(tooltip)));
    button
}

/// Attaches `act` to `control` through the next free tag.
fn bind(s: &mut Settings, control: &NSControl, act: Act) {
    control.setTag(s.actions.len() as isize);
    unsafe {
        control.setTarget(Some(&s.target));
        control.setAction(Some(sel!(fire:)));
    }
    s.actions.push(act);
}

fn defer_render() {
    with(|s| unsafe {
        let _: () = msg_send![
            &*s.target,
            performSelector: sel!(rerender),
            withObject: std::ptr::null::<AnyObject>(),
            afterDelay: 0.0f64
        ];
    });
}

/// Shows or hides the list and resizes the window to the page's width.
fn fit_page(s: &Settings) {
    let commands = s.page == Page::Commands;
    s.list_pane.setHidden(!commands);
    s.divider.setHidden(!commands);
    let detail_x = if commands {
        SIDEBAR_W + LIST_W + 1.0
    } else {
        SIDEBAR_W
    };
    s.detail_pane.setFrameOrigin(CGPoint::new(detail_x, 0.0));
    let total = detail_x + DETAIL_W;
    let mut frame = s.window.frame();
    if (frame.size.width - total).abs() > 0.5 {
        s.window.setMaxSize(CGSize::new(10_000.0, 10_000.0));
        s.window.setMinSize(CGSize::new(100.0, 420.0));
        frame.size.width = total;
        s.window.setFrame_display_animate(frame, true, false);
    }
    s.window.setMinSize(CGSize::new(total, 420.0));
    s.window.setMaxSize(CGSize::new(total, 10_000.0));
}

/// Puts `document` in `scroll`, keeping the scroll position when asked.
fn show_document(
    scroll: &NSScrollView,
    retired: &mut Vec<Retained<NSView>>,
    document: &NSView,
    keep_position: bool,
) {
    let clip = scroll.contentView();
    let previous = if keep_position {
        clip.bounds().origin.y
    } else {
        0.0
    };
    if let Some(old) = scroll.documentView() {
        retired.push(old);
    }
    scroll.setDocumentView(Some(document));
    let limit = (document.frame().size.height - clip.bounds().size.height).max(0.0);
    clip.scrollToPoint(CGPoint::new(0.0, previous.clamp(0.0, limit)));
    scroll.reflectScrolledClipView(&clip);
}

/// The big title in the detail header; a text field when the name can be edited.
fn set_title(s: &mut Settings, mtm: MainThreadMarker, name: &str, editable: Option<u64>) {
    if let Some(old) = s.title.take() {
        old.removeFromSuperview();
        s.retired.push(old);
    }
    let view: Retained<NSView> = match editable {
        Some(id) => {
            let field = NSTextField::textFieldWithString(&NSString::from_str(name), mtm);
            field.setBezeled(false);
            field.setDrawsBackground(false);
            field.setFont(Some(&font(15.0, true)));
            field.setFrame(rect(34.0, (HEADER_H - 22.0) / 2.0 + 2.0, 420.0, 22.0));
            field.setToolTip(Some(&NSString::from_str("Rename")));
            bind(s, &field, Act::Rename(id, field.clone()));
            views::view!(field)
        }
        None => {
            let text = label(mtm, name, 15.0, true, false);
            text.setFrameOrigin(CGPoint::new(
                36.0,
                (HEADER_H - text.frame().size.height) / 2.0,
            ));
            views::view!(text)
        }
    };
    s.detail_header.addSubview(&view);
    s.title = Some(view);
}

/// Rebuilds the current page from the live configuration and commands.
fn render(s: &mut Settings, mtm: MainThreadMarker) {
    s.actions.truncate(FIRST_PAGE_TAG);
    s.retired.clear();
    let Some((config, commands)) = with_daemon(|d| (d.c.config.clone(), d.c.commands.clone()))
    else {
        return;
    };
    for (index, (button, highlight)) in s.sidebar.iter().enumerate() {
        let on = (index == 1) == (s.page == Page::Commands);
        highlight.setHidden(!on);
        let tint = if on {
            NSColor::whiteColor()
        } else {
            NSColor::labelColor()
        };
        button.setContentTintColor(Some(&tint));
    }
    fit_page(s);
    let selection_valid = commands
        .commands()
        .iter()
        .any(|c| c.id == s.selected & !DIVIDER && (s.selected & DIVIDER == 0 || c.separator_after));
    if !selection_valid {
        s.selected = commands.commands().first().map_or(0, |c| c.id);
    }

    match s.page {
        Page::Preferences => {
            set_title(s, mtm, "Preferences", None);
            let document = preferences_document(s, mtm, &config);
            show_document(
                &s.detail_scroll,
                &mut s.retired,
                &document,
                s.shown == u64::MAX - 1,
            );
            s.shown = u64::MAX - 1;
        }
        Page::Commands => {
            let list = list_document(s, mtm, &commands);
            show_document(&s.list_scroll, &mut s.retired, &list, true);
            let same = s.shown == s.selected;
            let real = s.selected & !DIVIDER;
            let selected = commands.commands().iter().find(|c| c.id == real).cloned();
            let document = match selected {
                Some(command) if s.selected & DIVIDER != 0 => {
                    set_title(s, mtm, "Divider", None);
                    divider_document(mtm, &command)
                }
                Some(command) => {
                    let builtin = commands
                        .commands()
                        .iter()
                        .position(|c| c.id == command.id)
                        .is_some_and(|index| commands.is_builtin(index));
                    set_title(s, mtm, &command.name, (!builtin).then_some(command.id));
                    command_document(s, mtm, &command)
                }
                None => FlippedView::new(mtm, rect(0.0, 0.0, DETAIL_W, 10.0)).into_super(),
            };
            show_document(&s.detail_scroll, &mut s.retired, &document, same);
            s.shown = s.selected;
        }
    }
}

// MARK: Preferences

struct StepperSpec {
    min: f64,
    max: f64,
    step: f64,
}

fn format_pref(pref: Pref, value: f64) -> String {
    match pref {
        Pref::Scale => format!("{value:.0} %"),
        _ => format!("{value:.0} pt"),
    }
}

fn card_at(mtm: MainThreadMarker, document: &NSView, y: f64, height: f64) -> Retained<Panel> {
    let card = Panel::new(mtm, Tone::Card, 10.0, rect(CARD_X, y, CARD_W, height));
    document.addSubview(&card);
    card
}

fn hairline(mtm: MainThreadMarker, card: &Panel, y: f64) {
    card.addSubview(&Panel::new(
        mtm,
        Tone::Hairline,
        0.0,
        rect(CARD_PAD, y, CARD_W - 2.0 * CARD_PAD, 1.0),
    ));
}

/// Adds a switch whose right edge sits in the card's padding, level with a
/// 16-point line of text at `y`.
fn add_switch(
    s: &mut Settings,
    mtm: MainThreadMarker,
    card: &Panel,
    y: f64,
    on: bool,
    act: impl FnOnce(Retained<NSSwitch>) -> Act,
) -> Retained<NSSwitch> {
    let switch = NSSwitch::new(mtm);
    switch.setControlSize(NSControlSize::Mini);
    switch.setState(isize::from(on));
    switch.sizeToFit();
    let size = switch.frame().size;
    switch.setFrameOrigin(CGPoint::new(
        CARD_W - CARD_PAD - size.width,
        y + (16.0 - size.height) / 2.0,
    ));
    bind(s, &switch, act(switch.clone()));
    card.addSubview(&switch);
    switch
}

/// Lays the rows of a preferences card out one under another.
struct Rows<'a> {
    s: &'a mut Settings,
    mtm: MainThreadMarker,
    card: &'a Panel,
    y: f64,
}

impl Rows<'_> {
    /// Starts a row: a hairline above it, then its name on the left.
    fn begin(&mut self, text: &str) {
        if self.y > 0.0 {
            hairline(self.mtm, self.card, self.y);
        }
        let name = label(self.mtm, text, 13.0, false, false);
        name.setFrameOrigin(CGPoint::new(CARD_PAD, self.y + 12.0));
        self.card.addSubview(&name);
    }

    fn switch(&mut self, text: &str, note: Option<&str>, on: bool, pref: Pref) {
        self.begin(text);
        add_switch(self.s, self.mtm, self.card, self.y + 12.0, on, |switch| {
            Act::Pref(pref, Control::Check(switch))
        });
        match note {
            None => self.y += 39.0,
            Some(note) => {
                let lines = note.lines().count() as f64;
                let note = label(self.mtm, note, 11.0, false, true);
                note.setFrameOrigin(CGPoint::new(CARD_PAD, self.y + 30.0));
                self.card.addSubview(&note);
                self.y += 30.0 + lines * 14.0 + 12.0;
            }
        }
    }

    fn stepper(&mut self, text: &str, pref: Pref, value: f64, spec: &StepperSpec) {
        self.begin(text);
        let stepper = NSStepper::new(self.mtm);
        stepper.setControlSize(NSControlSize::Small);
        stepper.setMinValue(spec.min);
        stepper.setMaxValue(spec.max);
        stepper.setIncrement(spec.step);
        stepper.setDoubleValue(value);
        stepper.sizeToFit();
        let size = stepper.frame().size;
        let left = CARD_W - CARD_PAD - size.width;
        stepper.setFrameOrigin(CGPoint::new(left, self.y + (39.0 - size.height) / 2.0));
        let shown = label(self.mtm, &format_pref(pref, value), 13.0, false, false);
        right_aligned(&shown, left - 6.0, self.y + 12.0, 80.0);
        bind(
            self.s,
            &stepper,
            Act::Pref(pref, Control::Stepper(stepper.clone(), shown.clone())),
        );
        self.card.addSubview(&shown);
        self.card.addSubview(&stepper);
        self.y += 39.0;
    }

    fn color(&mut self, text: &str, pref: Pref, color: Color) {
        self.begin(text);
        let well = NSColorWell::new(self.mtm);
        well.setColor(&to_ns_color(color));
        well.setFrame(rect(CARD_W - CARD_PAD - 46.0, self.y + 8.0, 46.0, 23.0));
        bind(self.s, &well, Act::Pref(pref, Control::Color(well.clone())));
        NSColorPanel::sharedColorPanel(self.mtm).setShowsAlpha(true);
        self.card.addSubview(&well);
        self.y += 39.0;
    }
}

/// A heading and a card whose rows `rows` adds.
fn section(
    s: &mut Settings,
    mtm: MainThreadMarker,
    document: &NSView,
    y: &mut f64,
    name: &str,
    rows: impl FnOnce(&mut Rows),
) {
    let heading = label(mtm, name, 13.0, true, false);
    heading.setFrameOrigin(CGPoint::new(CARD_X + CARD_PAD, *y));
    document.addSubview(&heading);
    let card = card_at(mtm, document, *y + 27.0, 0.0);
    let mut laid_out = Rows {
        s,
        mtm,
        card: &card,
        y: 0.0,
    };
    rows(&mut laid_out);
    let height = laid_out.y;
    card.setFrameSize(CGSize::new(CARD_W, height));
    *y += 27.0 + height + 22.0;
}

fn preferences_document(s: &mut Settings, mtm: MainThreadMarker, c: &Config) -> Retained<NSView> {
    let document = FlippedView::new(mtm, rect(0.0, 0.0, DETAIL_W, 10.0));
    let mut y = 22.0;

    section(s, mtm, &document, &mut y, "General", |rows| {
        rows.switch(
            "Add Sash to login items",
            None,
            super::login::enabled(),
            Pref::Login,
        );
        rows.switch(
            "Hide menu bar icon",
            Some(
                "If the menu bar icon is hidden and you want to access it again,\nlaunch Sash from your Applications folder or via Spotlight.",
            ),
            c.hide_menu_bar_icon,
            Pref::HideIcon,
        );
    });

    section(s, mtm, &document, &mut y, "Mouse", |rows| {
        rows.switch("Snap windows by dragging", None, c.snap_windows, Pref::Snap);
        rows.switch(
            "Restore windows to original size",
            Some(
                "When active, dragging windows away from target positions\nrestores them to original size.",
            ),
            c.restore_to_original_size,
            Pref::RestoreSize,
        );
    });

    section(s, mtm, &document, &mut y, "Activation Areas", |rows| {
        rows.stepper(
            "Adjust scale of regular activation areas",
            Pref::Scale,
            (c.activation_area_scale * 100.0).round(),
            &StepperSpec {
                min: 10.0,
                max: 300.0,
                step: 5.0,
            },
        );
        rows.stepper(
            "Set width of screen edge activation areas",
            Pref::EdgeWidth,
            f64::from(c.activation_edge_width),
            &StepperSpec {
                min: 1.0,
                max: 200.0,
                step: 1.0,
            },
        );
    });

    section(s, mtm, &document, &mut y, "Target Areas", |rows| {
        rows.color(
            "Choose target areas' appearance",
            Pref::Fill,
            c.highlight.fill,
        );
    });

    section(s, mtm, &document, &mut y, "Spacing", |rows| {
        rows.stepper(
            "Add margins around windows",
            Pref::Padding,
            f64::from(c.padding),
            &StepperSpec {
                min: 0.0,
                max: 200.0,
                step: 1.0,
            },
        );
        rows.switch(
            "Fit tightly to screen edges",
            Some("When active, margins are not applied near screen edges."),
            c.fit_tightly_to_screen_edges,
            Pref::Fit,
        );
    });

    section(s, mtm, &document, &mut y, "Defaults", |rows| {
        let text = label(mtm, "Restore to defaults", 13.0, false, false);
        text.setTextColor(Some(&NSColor::controlAccentColor()));
        let size = text.frame().size;
        text.setFrameOrigin(CGPoint::new(
            (CARD_W - size.width) / 2.0,
            (39.0 - size.height) / 2.0,
        ));
        rows.card.addSubview(&text);
        // A transparent button over the text takes the clicks.
        let button = NSButton::new(mtm);
        button.setTitle(&NSString::from_str(""));
        button.setBordered(false);
        button.setFrame(rect(0.0, 0.0, CARD_W, 39.0));
        bind(rows.s, &button, Act::RestoreDefaults);
        rows.card.addSubview(&button);
        rows.y += 39.0;
    });

    document.setFrameSize(CGSize::new(DETAIL_W, y + 6.0));
    document.into_super()
}

// MARK: Commands

fn list_document(
    s: &mut Settings,
    mtm: MainThreadMarker,
    commands: &CommandSet,
) -> Retained<NSView> {
    let document = FlippedView::new(mtm, rect(0.0, 0.0, LIST_W, 10.0));
    let mut y = 15.0;
    for (axis, name) in [
        (Axis::Horizontal, "Horizontal"),
        (Axis::Vertical, "Vertical"),
    ] {
        let title = label(mtm, name, 15.0, true, false);
        title.setFrameOrigin(CGPoint::new(24.0, y));
        document.addSubview(&title);
        let collapsed = s.collapsed[slot(axis)];
        let chevron = icon_button(
            mtm,
            if collapsed {
                "chevron.right.circle.fill"
            } else {
                "chevron.down.circle.fill"
            },
            17.0,
            if collapsed { "Expand" } else { "Collapse" },
            rect(213.0, y, 20.0, 20.0),
        );
        chevron.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
        bind(s, &chevron, Act::Collapse(axis));
        document.addSubview(&chevron);
        let plus = icon_button(
            mtm,
            "plus.circle.fill",
            17.0,
            "Add",
            rect(233.0, y, 20.0, 20.0),
        );
        plus.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
        bind(s, &plus, Act::AddMenu(axis, plus.clone()));
        document.addSubview(&plus);

        y += 37.0;
        if !collapsed {
            for index in commands.indices(axis) {
                let command = &commands.commands()[index];
                document.addSubview(&command_row(s, mtm, commands, index, y));
                y += ROW;
                if command.separator_after {
                    document.addSubview(&divider_row(s, mtm, command.id, y));
                    y += ROW;
                }
            }
            y += 20.0;
        } else {
            y += 15.0;
        }
    }
    document.setFrameSize(CGSize::new(LIST_W, y + 10.0));
    document.into_super()
}

/// A translucent button on the selected row.
fn chip(
    mtm: MainThreadMarker,
    row: &RowView,
    name: &str,
    tooltip: &str,
    x: f64,
    round: bool,
) -> Retained<ChipButton> {
    let button = ChipButton::new(mtm, rect(x, 5.0, 20.0, 20.0), name, round);
    button.setToolTip(Some(&NSString::from_str(tooltip)));
    row.addSubview(&button);
    button
}

fn command_row(
    s: &mut Settings,
    mtm: MainThreadMarker,
    commands: &CommandSet,
    index: usize,
    y: f64,
) -> Retained<RowView> {
    let command = &commands.commands()[index];
    let selected = s.selected == command.id;
    let kind = match command.action {
        Action::Area(area) => RowKind::Area(area, command.axis),
        Action::Special(_) => RowKind::Special,
    };
    let row = RowView::new(mtm, rect(0.0, y, LIST_W, ROW), command.id, selected, kind);
    match command.action {
        Action::Area(_) => {}
        Action::Special(special) => {
            let name = match special {
                Special::NextDisplay => "arrowshape.right.fill",
                Special::PreviousDisplay => "arrowshape.left.fill",
                Special::Center => "arrow.down.right.and.arrow.up.left",
                Special::Restore => "arrow.uturn.backward",
                Special::StayOnTop => "pin.fill",
            };
            if let Some(image) = symbol(name, 13.0) {
                let icon = NSImageView::imageViewWithImage(&image, mtm);
                icon.setFrame(rect(25.0, (ROW - 15.0) / 2.0, 25.0, 15.0));
                let tint = if selected {
                    NSColor::whiteColor()
                } else {
                    NSColor::controlAccentColor()
                };
                icon.setContentTintColor(Some(&tint));
                row.addSubview(&icon);
            }
        }
    }
    let name = label(mtm, &command.name, 13.0, false, false);
    if selected {
        name.setTextColor(Some(&NSColor::whiteColor()));
    }
    let size = name.frame().size;
    name.setFrameOrigin(CGPoint::new(60.0, (ROW - size.height) / 2.0));
    row.addSubview(&name);
    if selected {
        let builtin = commands.is_builtin(index);
        let menu = chip(
            mtm,
            &row,
            "menubar.rectangle",
            if command.in_menu {
                "Visible in menu bar"
            } else {
                "Not visible in menu bar"
            },
            if builtin { 231.0 } else { 207.0 },
            false,
        );
        menu.setAlphaValue(if command.in_menu { 1.0 } else { 0.55 });
        bind(s, &menu, Act::ToggleMenu(command.id));
        if !builtin {
            let remove = chip(mtm, &row, "xmark", "Remove", 231.0, true);
            bind(s, &remove, Act::Remove(command.id));
        }
    }
    row
}

fn divider_row(s: &mut Settings, mtm: MainThreadMarker, after: u64, y: f64) -> Retained<RowView> {
    let id = after | DIVIDER;
    let selected = s.selected == id;
    let row = RowView::new(
        mtm,
        rect(0.0, y, LIST_W, ROW),
        id,
        selected,
        RowKind::Divider,
    );
    if selected {
        let remove = chip(mtm, &row, "xmark", "Remove", 231.0, true);
        bind(s, &remove, Act::Remove(id));
    }
    row
}

fn divider_document(mtm: MainThreadMarker, _command: &Command) -> Retained<NSView> {
    let document = FlippedView::new(mtm, rect(0.0, 0.0, DETAIL_W, 10.0));
    let card = card_at(mtm, &document, 22.0, 56.0);
    let text = label(
        mtm,
        "A divider separates the commands above it from\nthe ones below it in the menu bar menu.",
        12.0,
        false,
        true,
    );
    text.setFrameOrigin(CGPoint::new(CARD_PAD, 12.0));
    card.addSubview(&text);
    document.setFrameSize(CGSize::new(DETAIL_W, 100.0));
    document.into_super()
}

fn command_document(
    s: &mut Settings,
    mtm: MainThreadMarker,
    command: &Command,
) -> Retained<NSView> {
    let document = FlippedView::new(mtm, rect(0.0, 0.0, DETAIL_W, 10.0));
    let mut y = 22.0;
    let id = command.id;
    let axis = command.axis;

    if let Action::Area(target) = command.action {
        let height = 40.0 + GridView::height(axis, 44.0);
        let card = card_at(mtm, &document, y, height);
        let title = label(mtm, "Intended window size and position", 13.0, false, false);
        title.setFrameOrigin(CGPoint::new(CARD_PAD, 12.0));
        card.addSubview(&title);
        card.addSubview(&GridView::new(
            mtm,
            rect(0.0, 40.0, CARD_W, height - 40.0),
            id,
            GridKind::Target,
            axis,
            vec![target],
        ));
        y += height + CARD_GAP;
    }

    let shortcut_on = command.shortcut.is_some() || s.shortcut_open.contains(&id);
    let height = if shortcut_on { 86.0 } else { 40.0 };
    let card = card_at(mtm, &document, y, height);
    let title = label(mtm, "Keyboard shortcut", 13.0, false, false);
    title.setFrameOrigin(CGPoint::new(CARD_PAD, 12.0));
    card.addSubview(&title);
    add_switch(s, mtm, &card, 12.0, shortcut_on, |switch| {
        Act::ShortcutSwitch(id, switch)
    });
    if shortcut_on {
        card.addSubview(&ShortcutField::new(
            mtm,
            rect(CARD_PAD, 41.0, CARD_W - 2.0 * CARD_PAD, 27.0),
            id,
            command.shortcut.clone(),
        ));
    }
    y += height + CARD_GAP;

    if matches!(command.action, Action::Area(_)) {
        let on = !command.activation.is_empty() || s.activation_open.contains(&id);
        let height = if on {
            40.0 + GridView::height(axis, 34.0)
        } else {
            40.0
        };
        let card = card_at(mtm, &document, y, height);
        let title = label(mtm, "Activation area for dragging", 13.0, false, false);
        title.setFrameOrigin(CGPoint::new(CARD_PAD, 12.0));
        card.addSubview(&title);
        add_switch(s, mtm, &card, 12.0, on, |switch| {
            Act::ActivationSwitch(id, switch)
        });
        if on {
            card.addSubview(&GridView::new(
                mtm,
                rect(0.0, 40.0, CARD_W, height - 40.0),
                id,
                GridKind::Activation,
                axis,
                command.activation.clone(),
            ));
        }
        y += height + CARD_GAP;
    }

    let card = card_at(mtm, &document, y, 40.0);
    let title = label(mtm, "Visible in menu bar", 13.0, false, false);
    title.setFrameOrigin(CGPoint::new(CARD_PAD, 12.0));
    card.addSubview(&title);
    add_switch(s, mtm, &card, 12.0, command.in_menu, |switch| {
        Act::MenuSwitch(id, switch)
    });
    y += 40.0;

    document.setFrameSize(CGSize::new(DETAIL_W, y + 22.0));
    document.into_super()
}

// MARK: Actions

fn dispatch(tag: isize) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let act = with(|s| {
        usize::try_from(tag)
            .ok()
            .and_then(|i| s.actions.get(i).cloned())
    })
    .flatten();
    let Some(act) = act else { return };
    match act {
        Act::Page(page) => {
            with(|s| {
                s.page = page;
                render(s, mtm);
            });
        }
        Act::Pref(pref, control) => preference_changed(mtm, pref, &control),
        Act::RestoreDefaults => confirm_restore(mtm),
        Act::Collapse(axis) => {
            with(|s| s.collapsed[slot(axis)] ^= true);
            defer_render();
        }
        Act::Remove(id) => confirm_remove(mtm, id, id & DIVIDER != 0),
        Act::AddMenu(axis, button) => add_menu(mtm, axis, &button),
        Act::AddCommand(axis) => {
            let added = with_daemon(|d| {
                let index = d.c.commands.add_command(axis);
                d.c.commands.commands()[index].id
            });
            if let Some(id) = added {
                with(|s| s.selected = id);
                finish_commands(mtm, true);
            }
        }
        Act::AddDivider(axis) => {
            if with_daemon(|d| d.c.commands.add_divider(axis)).unwrap_or(false) {
                finish_commands(mtm, true);
            }
        }
        Act::ToggleMenu(id) => set_in_menu(mtm, id, None),
        Act::MenuSwitch(id, switch) => set_in_menu(mtm, id, Some(switch.state() != 0)),
        Act::ShortcutSwitch(id, switch) => {
            if switch.state() != 0 {
                with(|s| s.shortcut_open.insert(id));
                defer_render();
            } else {
                with(|s| s.shortcut_open.remove(&id));
                let cleared = with_daemon(|d| {
                    let index = index_of(d, id)?;
                    d.c.commands.set_shortcut(index, None).ok()
                })
                .flatten();
                finish_commands(mtm, cleared.is_some());
            }
        }
        Act::ActivationSwitch(id, switch) => {
            if switch.state() != 0 {
                with(|s| s.activation_open.insert(id));
                defer_render();
            } else {
                with(|s| s.activation_open.remove(&id));
                let cleared = with_daemon(|d| {
                    let index = index_of(d, id)?;
                    d.c.commands.set_activation(index, Vec::new()).ok()
                })
                .flatten();
                finish_commands(mtm, cleared.is_some());
            }
        }
        Act::Rename(id, field) => {
            let name = field.stringValue().to_string();
            let renamed = with_daemon(|d| {
                let index = index_of(d, id)?;
                d.c.commands.set_name(index, &name).ok()
            })
            .flatten();
            finish_commands(mtm, renamed.is_some());
        }
    }
}

/// Sets or, with `None`, flips whether a command is in the menu bar menu.
fn set_in_menu(mtm: MainThreadMarker, id: u64, visible: Option<bool>) {
    let changed = with_daemon(|d| {
        let index = index_of(d, id)?;
        let now = visible.unwrap_or(!d.c.commands.commands()[index].in_menu);
        d.c.commands.set_in_menu(index, now);
        Some(())
    })
    .flatten();
    finish_commands(mtm, changed.is_some());
}

/// Called by the list rows.
pub(super) fn select(id: u64) {
    with(|s| s.selected = id);
    defer_render();
}

/// Forgets what the window remembers about a removed command.
fn forget(id: u64) {
    with(|s| {
        s.shortcut_open.remove(&id);
        s.activation_open.remove(&id);
        s.selected = 0;
    });
}

fn index_of(d: &Daemon<MacSession>, id: u64) -> Option<usize> {
    d.c.commands.commands().iter().position(|c| c.id == id)
}

/// Saves changed commands, applies them and redraws the page.
fn finish_commands(mtm: MainThreadMarker, changed: bool) {
    if changed && let Some(Err(error)) = with_daemon(|d| persist(d, true)) {
        report(mtm, &error);
    }
    defer_render();
}

fn persist(d: &mut Daemon<MacSession>, commands: bool) -> anyhow::Result<()> {
    let mut config_changed = !commands;
    if commands {
        let shortcuts: Vec<Shortcut> = d.c.commands.shortcuts().into_iter().cloned().collect();
        register_hotkeys(&shortcuts);
        config_changed = save_commands(&mut d.c.config, &d.c.commands)?;
    }
    if config_changed {
        d.c.config.save()?;
    }
    Ok(())
}

/// Writes the commands file, returning whether `commands_file` was newly set.
fn save_commands(config: &mut Config, commands: &CommandSet) -> anyhow::Result<bool> {
    config.save_commands(commands)
}

fn report(mtm: MainThreadMarker, error: &anyhow::Error) {
    warn!("{error:#}");
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Warning);
    alert.setMessageText(&NSString::from_str("Couldn’t save settings"));
    alert.setInformativeText(&NSString::from_str(&format!("{error:#}")));
    alert.addButtonWithTitle(&NSString::from_str("OK"));
    match with(|s| s.window.clone()) {
        Some(window) => alert.beginSheetModalForWindow_completionHandler(&window, None),
        None => {
            alert.runModal();
        }
    }
}

fn apply_runtime(d: &mut Daemon<MacSession>) {
    if let Err(error) = set_snapping(d.c.config.snap_windows) {
        warn!("{error:#}");
    }
    set_status_item_visible(!d.c.config.hide_menu_bar_icon);
    d.c.session.set_highlight(&d.c.config.highlight);
}

fn preference_changed(mtm: MainThreadMarker, pref: Pref, control: &Control) {
    let on = matches!(control, Control::Check(b) if b.state() != 0);
    let value = match control {
        Control::Stepper(stepper, _) => stepper.doubleValue(),
        _ => 0.0,
    };
    if let Control::Stepper(_, shown) = control {
        shown.setStringValue(&NSString::from_str(&format_pref(pref, value)));
    }
    let outcome = with_daemon(|d| -> anyhow::Result<()> {
        let c = &mut d.c.config;
        match pref {
            Pref::Login => return super::login::set_enabled(on),
            Pref::HideIcon => c.hide_menu_bar_icon = on,
            Pref::Snap => c.snap_windows = on,
            Pref::RestoreSize => c.restore_to_original_size = on,
            Pref::EdgeWidth => c.activation_edge_width = value.round() as i32,
            Pref::Scale => c.activation_area_scale = value.round() / 100.0,
            Pref::Padding => c.padding = value.round() as i32,
            Pref::Fit => c.fit_tightly_to_screen_edges = on,
            Pref::Fill => {
                if let Control::Color(well) = control
                    && let Some(color) = from_ns_color(&well.color())
                {
                    c.highlight.fill = color;
                }
            }
        }
        apply_runtime(d);
        persist(d, false)
    });
    if let Some(Err(error)) = outcome {
        report(mtm, &error);
    }
}

fn to_ns_color(color: Color) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from(color.r) / 255.0,
        f64::from(color.g) / 255.0,
        f64::from(color.b) / 255.0,
        f64::from(color.a) / 255.0,
    )
}

fn from_ns_color(color: &NSColor) -> Option<Color> {
    let rgb = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(Color {
        r: byte(rgb.redComponent()),
        g: byte(rgb.greenComponent()),
        b: byte(rgb.blueComponent()),
        a: byte(rgb.alphaComponent()),
    })
}

fn add_menu(mtm: MainThreadMarker, axis: Axis, button: &NSButton) {
    let menu = NSMenu::new(mtm);
    let entries = [
        ("Command", Act::AddCommand(axis)),
        ("Divider", Act::AddDivider(axis)),
    ];
    for (name, act) in entries {
        let item = unsafe {
            menu.addItemWithTitle_action_keyEquivalent(
                &NSString::from_str(name),
                Some(sel!(fire:)),
                &NSString::from_str(""),
            )
        };
        with(|s| {
            item.setTag(s.actions.len() as isize);
            unsafe { item.setTarget(Some(&s.target)) };
            s.actions.push(act);
        });
    }
    menu.popUpMenuPositioningItem_atLocation_inView(
        None,
        CGPoint::new(0.0, button.bounds().size.height),
        Some(button),
    );
}

/// Shows a sheet; `done` receives the index of the clicked button, 0 first.
fn sheet(
    mtm: MainThreadMarker,
    title: &str,
    text: &str,
    buttons: &[&str],
    done: impl Fn(isize) + 'static,
) {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(text));
    for button in buttons {
        alert.addButtonWithTitle(&NSString::from_str(button));
    }
    let block = RcBlock::new(move |response: isize| done(response - 1000));
    if let Some(window) = with(|s| s.window.clone()) {
        alert.beginSheetModalForWindow_completionHandler(&window, Some(&block));
    }
}

fn confirm_remove(mtm: MainThreadMarker, id: u64, divider: bool) {
    let (title, text) = if divider {
        ("Divider", "Delete this divider?")
    } else {
        (
            "Remove Command",
            "You are about to permanently\nremove this command.",
        )
    };
    sheet(mtm, title, text, &["Remove", "Cancel"], move |choice| {
        if choice != 0 {
            return;
        }
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let real = id & !DIVIDER;
        let removed = with_daemon(|d| {
            let index = index_of(d, real)?;
            if divider {
                d.c.commands.set_separator_after(index, false);
                Some(())
            } else {
                d.c.commands.remove(index).ok()
            }
        })
        .flatten();
        forget(real);
        finish_commands(mtm, removed.is_some());
    });
}

fn confirm_restore(mtm: MainThreadMarker) {
    sheet(
        mtm,
        "Restore",
        "Choose which sections\nyou’d like to restore to defaults.",
        &[
            "Restore Preferences",
            "Restore Commands",
            "Restore All",
            "Cancel",
        ],
        |choice| {
            let (prefs, commands) = match choice {
                0 => (true, false),
                1 => (false, true),
                2 => (true, true),
                _ => return,
            };
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            let outcome = with_daemon(|d| -> anyhow::Result<()> {
                if prefs {
                    d.c.config.restore_preferences();
                    apply_runtime(d);
                }
                if commands {
                    d.c.commands = CommandSet::builtin();
                }
                persist(d, commands)?;
                if prefs {
                    d.c.config.save()?;
                }
                Ok(())
            });
            if let Some(Err(error)) = outcome {
                report(mtm, &error);
            }
            defer_render();
        },
    );
}

/// Global shortcuts would swallow the keys being recorded, so they are paused
/// while a recorder waits and registered again afterwards.
pub(super) fn recording(active: bool) {
    if active {
        unregister_hotkeys();
    } else {
        with_daemon(|d| {
            let shortcuts: Vec<Shortcut> = d.c.commands.shortcuts().into_iter().cloned().collect();
            register_hotkeys(&shortcuts);
        });
    }
}

/// Called by the shortcut recorder; the message is shown in its field.
pub(super) fn shortcut_chosen(command: u64, shortcut: Option<Shortcut>) -> Result<(), String> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err("Not on the main thread".into());
    };
    let outcome = with_daemon(|d| {
        let index = index_of(d, command).ok_or_else(|| "This command is gone".to_string())?;
        d.c.commands
            .set_shortcut(index, shortcut)
            .map_err(|_| "This combination is taken".to_string())?;
        persist(d, true).map_err(|error| {
            warn!("{error:#}");
            format!("{error:#}")
        })
    })
    .unwrap_or_else(|| Err("Not running".into()));
    if outcome.is_ok() {
        let _ = mtm;
        defer_render();
    }
    outcome
}

/// Called by the grid editors with the command's complete set of areas.
pub(super) fn grid_changed(command: u64, kind: GridKind, areas: Vec<GridRect>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let outcome = with_daemon(|d| -> anyhow::Result<()> {
        let index = index_of(d, command).ok_or_else(|| anyhow::anyhow!("command is gone"))?;
        match kind {
            GridKind::Target => {
                if let Some(&target) = areas.first() {
                    d.c.commands.set_target(index, target)?;
                }
            }
            GridKind::Activation => d.c.commands.set_activation(index, areas)?,
        }
        persist(d, true)
    });
    if let Some(Err(error)) = outcome {
        report(mtm, &error);
    }
    defer_render();
}
