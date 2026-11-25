//! AppKit building blocks for the settings window: drawing helpers, the panels
//! the layout is made of, the list rows, the shortcut recorder and the grid
//! editor. Everything is laid out with plain frames.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSAutoresizingMaskOptions, NSBezierPath, NSButton, NSColor, NSEvent, NSEventModifierFlags,
    NSFont, NSFontWeightRegular, NSFontWeightSemibold, NSImage, NSImageSymbolConfiguration,
    NSImageView, NSTextAlignment, NSTextField, NSView,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSArray, NSString};

use crate::geometry::GridRect;
use crate::model::Axis;
use crate::shortcut::{Modifiers, Shortcut};

/// Distance between the centres of two grid cells.
const PITCH: f64 = 16.0;
/// Side of one drawn cell; the rest of the pitch is the gap.
const CELL: f64 = 13.0;
/// Space above the grid for the fraction badge.
const GRID_TOP: f64 = 33.0;

/// An owned `NSView` handle for any view type.
macro_rules! view {
    ($x:expr) => {{
        let v: &objc2_app_kit::NSView = &$x;
        objc2::Message::retain(v)
    }};
}
pub(crate) use view;

pub fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
    CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
}

/// Redraws a view whose class (a control) hides the plain `NSView` method.
fn redraw(view: &NSView) {
    view.setNeedsDisplay(true);
}

pub fn is_dark(view: &NSView) -> bool {
    let appearance = view.effectiveAppearance();
    let names = unsafe { NSArray::from_slice(&[NSAppearanceNameAqua, NSAppearanceNameDarkAqua]) };
    appearance
        .bestMatchFromAppearancesWithNames(&names)
        .is_some_and(|name| name.isEqualToString(unsafe { NSAppearanceNameDarkAqua }))
}

/// The surfaces of the window. Dark values were sampled from Magnet's settings
/// window; the light ones are our own guesses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Sidebar,
    List,
    Detail,
    Header,
    Card,
    Hairline,
    Field,
    Cell,
    Bar,
    Accent,
}

fn hex(value: u32) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from((value >> 16) & 0xff) / 255.0,
        f64::from((value >> 8) & 0xff) / 255.0,
        f64::from(value & 0xff) / 255.0,
        1.0,
    )
}

pub fn tone_color(tone: Tone, dark: bool) -> Retained<NSColor> {
    let (dark_value, light_value) = match tone {
        Tone::Sidebar => (0x2e2a2b, 0xe3e0e1),
        Tone::List => (0x2f2f2f, 0xf0f0f0),
        Tone::Detail => (0x383838, 0xe9e9e9),
        Tone::Header => (0x212121, 0xf7f7f7),
        Tone::Card => (0x2b2b2b, 0xffffff),
        Tone::Hairline => (0x3d3d3d, 0xd4d4d4),
        Tone::Field => (0x4c4c4c, 0xf4f4f4),
        Tone::Cell => (0x484848, 0xdcdcdc),
        Tone::Bar => (0x525252, 0xb8b8b8),
        Tone::Accent => return NSColor::controlAccentColor(),
    };
    hex(if dark { dark_value } else { light_value })
}

fn rounded(frame: CGRect, radius: f64) -> Retained<NSBezierPath> {
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(frame, radius, radius)
}

pub fn font(size: f64, bold: bool) -> Retained<NSFont> {
    if bold {
        NSFont::systemFontOfSize_weight(size, unsafe { NSFontWeightSemibold })
    } else {
        NSFont::systemFontOfSize(size)
    }
}

/// A label of its natural size, at the origin.
pub fn label(
    mtm: MainThreadMarker,
    text: &str,
    size: f64,
    bold: bool,
    secondary: bool,
) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(&font(size, bold)));
    if secondary {
        field.setTextColor(Some(&NSColor::secondaryLabelColor()));
    }
    field.sizeToFit();
    field
}

pub fn right_aligned(field: &NSTextField, right: f64, y: f64, width: f64) {
    field.setAlignment(NSTextAlignment::Right);
    let height = field.frame().size.height;
    field.setFrame(rect(right - width, y, width, height));
}

/// A symbol image whose glyph is `points` tall.
pub fn symbol(name: &str, points: f64) -> Option<Retained<NSImage>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        None,
    )?;
    let configuration =
        NSImageSymbolConfiguration::configurationWithPointSize_weight(points, unsafe {
            NSFontWeightRegular
        });
    image.imageWithSymbolConfiguration(&configuration)
}

/// Key symbols the way menus draw them, for example `⌃⌥←`.
pub fn symbolic(shortcut: &Shortcut) -> String {
    let m = shortcut.modifiers;
    let mut out = String::new();
    for (on, symbol) in [(m.ctrl, "⌃"), (m.alt, "⌥"), (m.shift, "⇧"), (m.super_, "⌘")] {
        if on {
            out.push_str(symbol);
        }
    }
    out.push_str(match shortcut.key.as_str() {
        "Left" => "←",
        "Up" => "↑",
        "Right" => "→",
        "Down" => "↓",
        "Return" => "↩",
        "BackSpace" => "⌫",
        "Delete" => "⌦",
        "Tab" => "⇥",
        "space" => "Space",
        "Home" => "↖",
        "End" => "↘",
        "Page_Up" => "⇞",
        "Page_Down" => "⇟",
        other => other,
    });
    out
}

define_class!(
    /// Top-left origin, so scrolled pages start at their first row.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    pub struct FlippedView;

    impl FlippedView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> Bool {
            Bool::YES
        }
    }
);

impl FlippedView {
    pub fn new(mtm: MainThreadMarker, frame: CGRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

pub struct PanelIvars {
    tone: Cell<Tone>,
    radius: f64,
}

define_class!(
    /// A flat surface: a filled, optionally rounded rectangle. Cards also get
    /// a hairline border.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = PanelIvars]
    pub struct Panel;

    impl Panel {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> Bool {
            Bool::YES
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: CGRect) {
            let dark = is_dark(self);
            let ivars = self.ivars();
            let bounds = self.bounds();
            tone_color(ivars.tone.get(), dark).setFill();
            rounded(bounds, ivars.radius).fill();
            if ivars.tone.get() == Tone::Card {
                tone_color(Tone::Hairline, dark).setStroke();
                let inset = rect(0.5, 0.5, bounds.size.width - 1.0, bounds.size.height - 1.0);
                let border = rounded(inset, ivars.radius - 0.5);
                border.setLineWidth(1.0);
                border.stroke();
            }
        }
    }
);

/// Stretches with the window's height.
pub trait Tall {
    fn tall(self) -> Self;
}

impl Tall for Retained<Panel> {
    fn tall(self) -> Self {
        self.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);
        self
    }
}

impl Panel {
    pub fn new(mtm: MainThreadMarker, tone: Tone, radius: f64, frame: CGRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PanelIvars {
            tone: Cell::new(tone),
            radius,
        });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

define_class!(
    /// A small translucent-white button for the selected row: a symbol in the
    /// accent colour on a rounded square or a disc.
    #[unsafe(super = NSButton)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ChipIvars]
    pub struct ChipButton;

    impl ChipButton {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: CGRect) {
            NSColor::whiteColor().colorWithAlphaComponent(0.7).setFill();
            let bounds = self.bounds();
            let radius = if self.ivars().round { bounds.size.width / 2.0 } else { 5.0 };
            rounded(bounds, radius).fill();
        }
    }
);

pub struct ChipIvars {
    round: bool,
}

impl ChipButton {
    pub fn new(mtm: MainThreadMarker, frame: CGRect, glyph: &str, round: bool) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ChipIvars { round });
        let button: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        button.setBordered(false);
        if let Some(image) = symbol(glyph, 11.0) {
            let icon = NSImageView::imageViewWithImage(&image, mtm);
            icon.setFrame(rect(0.0, 0.0, frame.size.width, frame.size.height));
            icon.setContentTintColor(Some(&NSColor::controlAccentColor()));
            icon.setEnabled(false);
            button.addSubview(&icon);
        }
        button
    }
}

/// What a list row shows next to its name.
#[derive(Clone, Copy)]
pub enum RowKind {
    /// A miniature of the screen with the command's target area filled in.
    Area(GridRect, Axis),
    /// The icon is a separate image view.
    Special,
    Divider,
}

/// Size of the miniature screen in a list row.
pub fn mini_size(axis: Axis) -> CGSize {
    match axis {
        Axis::Horizontal => CGSize::new(25.0, 15.0),
        Axis::Vertical => CGSize::new(15.0, 25.0),
    }
}

pub struct RowIvars {
    id: u64,
    selected: Cell<bool>,
    kind: RowKind,
}

define_class!(
    /// One line of the command list: selection highlight and icon. Clicking it
    /// selects the command.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = RowIvars]
    pub struct RowView;

    impl RowView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> Bool {
            Bool::YES
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: &NSEvent) -> Bool {
            Bool::YES
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            super::select(self.ivars().id);
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: CGRect) {
            self.draw();
        }
    }
);

impl RowView {
    pub fn new(
        mtm: MainThreadMarker,
        frame: CGRect,
        id: u64,
        selected: bool,
        kind: RowKind,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RowIvars {
            id,
            selected: Cell::new(selected),
            kind,
        });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn draw(&self) {
        let ivars = self.ivars();
        let dark = is_dark(self);
        let height = self.bounds().size.height;
        let selected = ivars.selected.get();
        let accent = NSColor::controlAccentColor();
        let white = NSColor::whiteColor();
        if selected {
            accent.setFill();
            rounded(
                rect(14.0, 1.0, self.bounds().size.width - 30.0, height - 2.0),
                7.0,
            )
            .fill();
        }
        match ivars.kind {
            RowKind::Area(area, axis) => {
                let size = mini_size(axis);
                let (cols, rows) = axis.grid();
                let origin = CGPoint::new(25.0, (height - size.height) / 2.0);
                let base = CGRect::new(origin, size);
                if selected {
                    white.colorWithAlphaComponent(0.35).setFill();
                } else {
                    accent
                        .colorWithAlphaComponent(if dark { 0.28 } else { 0.22 })
                        .setFill();
                }
                rounded(base, 3.0).fill();
                let inner = rect(
                    origin.x + 1.5 + f64::from(area.x) / f64::from(cols) * (size.width - 3.0),
                    origin.y + 1.5 + f64::from(area.y) / f64::from(rows) * (size.height - 3.0),
                    (f64::from(area.w) / f64::from(cols) * (size.width - 3.0)).max(2.0),
                    (f64::from(area.h) / f64::from(rows) * (size.height - 3.0)).max(2.0),
                );
                if selected {
                    white.setFill();
                } else {
                    accent.setFill();
                }
                rounded(inner, 2.0).fill();
            }
            RowKind::Special => {}
            RowKind::Divider => {
                if selected {
                    white.colorWithAlphaComponent(0.7).setFill();
                } else {
                    tone_color(Tone::Bar, dark).setFill();
                }
                rounded(rect(25.0, (height - 3.0) / 2.0, 26.0, 3.0), 1.5).fill();
            }
        }
    }
}

pub struct ShortcutIvars {
    command: u64,
    current: RefCell<Option<Shortcut>>,
    recording: Cell<bool>,
}

define_class!(
    /// Click, then press the combination to bind. Delete clears, Escape cancels.
    #[unsafe(super = NSButton)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ShortcutIvars]
    pub struct ShortcutField;

    impl ShortcutField {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> Bool {
            Bool::YES
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty: CGRect) {
            let dark = is_dark(self);
            let bounds = self.bounds();
            tone_color(Tone::Field, dark).setFill();
            rounded(bounds, 7.0).fill();
            let (colour, width) = if self.ivars().recording.get() {
                (NSColor::controlAccentColor(), 2.0)
            } else {
                (tone_color(Tone::Hairline, dark), 1.0)
            };
            colour.setStroke();
            let inset = rect(width / 2.0, width / 2.0, bounds.size.width - width, bounds.size.height - width);
            let border = rounded(inset, 7.0);
            border.setLineWidth(width);
            border.stroke();
            unsafe { msg_send![super(self), drawRect: dirty] }
        }

        /// The button's own action, so a click and an accessibility press both start it.
        #[unsafe(method(beginRecording:))]
        fn begin_recording(&self, _sender: &AnyObject) {
            if let Some(window) = self.window() {
                window.makeFirstResponder(Some(self));
            }
            self.ivars().recording.set(true);
            super::recording(true);
            self.setTitle(&NSString::from_str("Start typing your shortcut"));
            redraw(self);
        }

        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> Bool {
            if self.ivars().recording.replace(false) {
                super::recording(false);
            }
            self.show_current();
            redraw(self);
            Bool::YES
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if !self.handle(event) {
                unsafe { msg_send![super(self), keyDown: event] }
            }
        }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> Bool {
            Bool::new(self.ivars().recording.get() && self.handle(event))
        }
    }
);

impl ShortcutField {
    pub fn new(
        mtm: MainThreadMarker,
        frame: CGRect,
        command: u64,
        current: Option<Shortcut>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ShortcutIvars {
            command,
            current: RefCell::new(current),
            recording: Cell::new(false),
        });
        let field: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        field.setBordered(false);
        field.setFont(Some(&font(13.0, false)));
        unsafe {
            field.setTarget(Some(&field));
            field.setAction(Some(sel!(beginRecording:)));
        }
        field.show_current();
        field
    }

    fn show_current(&self) {
        let title = match &*self.ivars().current.borrow() {
            Some(shortcut) => symbolic(shortcut),
            None => "Click to record a shortcut".to_string(),
        };
        self.setTitle(&NSString::from_str(&title));
    }

    /// Returns whether the key was used by the recorder.
    fn handle(&self, event: &NSEvent) -> bool {
        if !self.ivars().recording.get() {
            return false;
        }
        let code = event.keyCode();
        let flags = event.modifierFlags();
        let modifiers = Modifiers {
            ctrl: flags.contains(NSEventModifierFlags::Control),
            alt: flags.contains(NSEventModifierFlags::Option),
            shift: flags.contains(NSEventModifierFlags::Shift),
            super_: flags.contains(NSEventModifierFlags::Command),
        };
        let plain = !(modifiers.ctrl || modifiers.alt || modifiers.shift || modifiers.super_);
        if plain && code == 0x35 {
            // Escape
            if let Some(window) = self.window() {
                window.makeFirstResponder(None);
            }
            return true;
        }
        let choice = if plain && matches!(code, 0x33 | 0x75) {
            // Delete clears the binding.
            None
        } else if !(modifiers.ctrl || modifiers.alt || modifiers.super_) {
            self.setTitle(&NSString::from_str("Add Control, Option or Command"));
            return true;
        } else if let Some(shortcut) = Shortcut::from_macos_key(modifiers, code) {
            Some(shortcut)
        } else {
            self.setTitle(&NSString::from_str("This key isn't supported"));
            return true;
        };
        match super::shortcut_chosen(self.ivars().command, choice.clone()) {
            Ok(()) => {
                *self.ivars().current.borrow_mut() = choice;
                if self.ivars().recording.replace(false) {
                    super::recording(false);
                }
                self.show_current();
                if let Some(window) = self.window() {
                    window.makeFirstResponder(None);
                }
            }
            Err(message) => self.setTitle(&NSString::from_str(&message)),
        }
        true
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GridKind {
    /// One area: where the window goes.
    Target,
    /// Several areas: where dragging a window triggers the command.
    Activation,
}

pub struct GridIvars {
    command: u64,
    kind: GridKind,
    axis: Axis,
    areas: RefCell<Vec<GridRect>>,
    drag: Cell<Option<Drag>>,
}

/// Where a drag began, the cell now under the pointer, and the index of the
/// area under the first cell.
type Drag = ((u32, u32), (u32, u32), Option<usize>);

define_class!(
    /// The command's grid. Dragging draws an area; in the activation grid a
    /// click on an area removes it.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = GridIvars]
    pub struct GridView;

    impl GridView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> Bool {
            Bool::YES
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: &NSEvent) -> Bool {
            Bool::YES
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: CGRect) {
            self.draw();
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let cell = self.cell_at(event);
            let hit = self.ivars().areas.borrow().iter().position(|a| contains(*a, cell));
            self.ivars().drag.set(Some((cell, cell, hit)));
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            if let Some((start, _, hit)) = self.ivars().drag.get() {
                self.ivars().drag.set(Some((start, self.cell_at(event), hit)));
                self.setNeedsDisplay(true);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let Some((start, _, hit)) = self.ivars().drag.take() else {
                return;
            };
            let end = self.cell_at(event);
            self.setNeedsDisplay(true);
            let ivars = self.ivars();
            let mut areas = ivars.areas.borrow().clone();
            match (ivars.kind, hit) {
                (GridKind::Activation, Some(index)) if start == end => {
                    areas.remove(index);
                }
                (GridKind::Target, _) => areas = vec![spanning(start, end)],
                (GridKind::Activation, _) => areas.push(spanning(start, end)),
            }
            *ivars.areas.borrow_mut() = areas.clone();
            super::grid_changed(ivars.command, ivars.kind, areas);
        }
    }
);

impl GridView {
    /// Height of a grid view for `axis`, with `bottom` points under the grid.
    pub fn height(axis: Axis, bottom: f64) -> f64 {
        let (_, rows) = axis.grid();
        GRID_TOP + f64::from(rows) * PITCH - (PITCH - CELL) + bottom
    }

    pub fn new(
        mtm: MainThreadMarker,
        frame: CGRect,
        command: u64,
        kind: GridKind,
        axis: Axis,
        areas: Vec<GridRect>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(GridIvars {
            command,
            kind,
            axis,
            areas: RefCell::new(areas),
            drag: Cell::new(None),
        });
        let grid: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        if kind == GridKind::Target {
            grid.add_badges(mtm);
        }
        grid
    }

    fn size(&self) -> CGSize {
        let (cols, rows) = self.ivars().axis.grid();
        CGSize::new(
            f64::from(cols) * PITCH - (PITCH - CELL),
            f64::from(rows) * PITCH - (PITCH - CELL),
        )
    }

    fn origin(&self) -> CGPoint {
        CGPoint::new(
            (self.bounds().size.width - self.size().width) / 2.0,
            GRID_TOP,
        )
    }

    fn cell_at(&self, event: &NSEvent) -> (u32, u32) {
        let point = self.convertPoint_fromView(event.locationInWindow(), None);
        let origin = self.origin();
        let (cols, rows) = self.ivars().axis.grid();
        let clamp = |value: f64, count: u32| {
            (value / PITCH).floor().clamp(0.0, f64::from(count - 1)) as u32
        };
        (
            clamp(point.x - origin.x, cols),
            clamp(point.y - origin.y, rows),
        )
    }

    fn cell_rect(&self, x: u32, y: u32) -> CGRect {
        let origin = self.origin();
        rect(
            origin.x + f64::from(x) * PITCH,
            origin.y + f64::from(y) * PITCH,
            CELL,
            CELL,
        )
    }

    /// The span of cells from `a` to `b`, edges included.
    fn span_rect(&self, area: GridRect, grow: f64) -> CGRect {
        let first = self.cell_rect(area.x, area.y);
        let last = self.cell_rect(area.x + area.w - 1, area.y + area.h - 1);
        rect(
            first.origin.x - grow,
            first.origin.y - grow,
            last.origin.x + CELL - first.origin.x + 2.0 * grow,
            last.origin.y + CELL - first.origin.y + 2.0 * grow,
        )
    }

    /// The fractions of the screen the target covers, as small chips around the grid.
    fn add_badges(&self, mtm: MainThreadMarker) {
        let Some(&area) = self.ivars().areas.borrow().first() else {
            return;
        };
        let (cols, rows) = self.ivars().axis.grid();
        let span = self.span_rect(area, 0.0);
        let origin = self.origin();
        let size = self.size();
        // `x` is where the chip's centre goes.
        let chip = |text: String, x: f64, y: f64| {
            let text = label(mtm, &text, 10.0, true, false);
            text.setTextColor(Some(&NSColor::whiteColor()));
            let size = text.frame().size;
            let width = (size.width + 10.0).max(22.0);
            let panel = Panel::new(
                mtm,
                Tone::Accent,
                4.0,
                rect(x - width / 2.0, y, width, 15.0),
            );
            text.setFrameOrigin(CGPoint::new(
                (width - size.width) / 2.0,
                (15.0 - size.height) / 2.0,
            ));
            panel.addSubview(&text);
            self.addSubview(&panel);
        };
        chip(
            fraction(area.w, cols),
            span.origin.x + span.size.width / 2.0,
            origin.y - 24.0,
        );
        let left_gap = area.x;
        let right_gap = cols - area.x - area.w;
        let x = if left_gap <= right_gap {
            origin.x - 19.0
        } else {
            origin.x + size.width + 19.0
        };
        chip(
            fraction(area.h, rows),
            x,
            span.origin.y + span.size.height / 2.0 - 7.5,
        );
    }

    fn draw(&self) {
        let ivars = self.ivars();
        let dark = is_dark(self);
        let (cols, rows) = ivars.axis.grid();
        let accent = NSColor::controlAccentColor();
        let preview = ivars.drag.get().and_then(|(start, end, hit)| {
            (!(ivars.kind == GridKind::Activation && hit.is_some() && start == end))
                .then(|| spanning(start, end))
        });
        let areas = ivars.areas.borrow();
        match ivars.kind {
            GridKind::Target => {
                let on = preview.or_else(|| areas.first().copied());
                for y in 0..rows {
                    for x in 0..cols {
                        let selected = on.is_some_and(|a| contains(a, (x, y)));
                        if selected {
                            accent.setFill();
                        } else {
                            tone_color(Tone::Cell, dark).setFill();
                        }
                        rounded(self.cell_rect(x, y), 3.0).fill();
                    }
                }
            }
            GridKind::Activation => {
                let zone = tone_color(Tone::Bar, dark).colorWithAlphaComponent(0.4);
                for area in areas.iter().copied().chain(preview) {
                    zone.setFill();
                    rounded(self.span_rect(area, 2.0), (CELL + 4.0) / 2.0).fill();
                }
                for y in 0..rows {
                    for x in 0..cols {
                        let inside = areas
                            .iter()
                            .copied()
                            .chain(preview)
                            .any(|a| contains(a, (x, y)));
                        if inside {
                            accent.setFill();
                        } else {
                            tone_color(Tone::Bar, dark).setFill();
                        }
                        let cell = self.cell_rect(x, y);
                        let dot = if inside { 4.0 } else { 3.0 };
                        NSBezierPath::bezierPathWithOvalInRect(rect(
                            cell.origin.x + (CELL - dot) / 2.0,
                            cell.origin.y + (CELL - dot) / 2.0,
                            dot,
                            dot,
                        ))
                        .fill();
                    }
                }
            }
        }
    }
}

/// `part` of `whole`, reduced and set with small numerals like ⁵⁄₈.
pub fn fraction(part: u32, whole: u32) -> String {
    const UP: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    const DOWN: [char; 10] = ['₀', '₁', '₂', '₃', '₄', '₅', '₆', '₇', '₈', '₉'];
    fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    if part == whole {
        return "1".to_string();
    }
    let divisor = gcd(part, whole).max(1);
    let digits = |value: u32, set: &[char; 10]| {
        value
            .to_string()
            .bytes()
            .map(|d| set[usize::from(d - b'0')])
            .collect::<String>()
    };
    format!(
        "{}⁄{}",
        digits(part / divisor, &UP),
        digits(whole / divisor, &DOWN)
    )
}

fn spanning((x1, y1): (u32, u32), (x2, y2): (u32, u32)) -> GridRect {
    GridRect {
        x: x1.min(x2),
        y: y1.min(y2),
        w: x1.abs_diff(x2) + 1,
        h: y1.abs_diff(y2) + 1,
    }
}

fn contains(area: GridRect, (x, y): (u32, u32)) -> bool {
    x >= area.x && x < area.x + area.w && y >= area.y && y < area.y + area.h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drags_span_from_any_corner() {
        let a = spanning((5, 7), (2, 3));
        assert_eq!((a.x, a.y, a.w, a.h), (2, 3, 4, 5));
        assert!(contains(a, (5, 7)) && !contains(a, (6, 7)));
        assert_eq!(spanning((1, 1), (1, 1)).w, 1);
    }

    #[test]
    fn shortcuts_read_like_menus() {
        assert_eq!(symbolic(&"Ctrl+Alt+Left".parse().unwrap()), "⌃⌥←");
        assert_eq!(symbolic(&"Ctrl+Alt+Super+U".parse().unwrap()), "⌃⌥⌘U");
    }

    #[test]
    fn fractions_are_reduced() {
        assert_eq!(fraction(15, 24), "⁵⁄₈");
        assert_eq!(fraction(8, 12), "²⁄₃");
        assert_eq!(fraction(12, 24), "¹⁄₂");
        assert_eq!(fraction(24, 24), "1");
    }
}
