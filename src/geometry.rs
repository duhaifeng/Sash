use crate::model::Axis;

/// A display: the full area the pointer can reach, and the work area left
/// after the menu bar, dock or panels.
#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    pub frame: Rect,
    pub work: Rect,
}

/// A rectangle in root-window pixel coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub const fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
    }

    pub fn overlap_area(&self, other: &Rect) -> i64 {
        self.intersection(other)
            .map_or(0, |r| i64::from(r.w) * i64::from(r.h))
    }

    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    /// Grows each side by the given extents (left, right, top, bottom).
    #[allow(dead_code)]
    pub fn outset(&self, e: Extents) -> Rect {
        Rect::new(
            self.x - e.left,
            self.y - e.top,
            self.w + e.left + e.right,
            self.h + e.top + e.bottom,
        )
    }

    /// Shrinks each side by the given extents (left, right, top, bottom).
    pub fn inset(&self, e: Extents) -> Rect {
        Rect::new(
            self.x + e.left,
            self.y + e.top,
            (self.w - e.left - e.right).max(1),
            (self.h - e.top - e.bottom).max(1),
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Extents {
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
}

/// A rectangle measured in grid cells, written as `[[x, y], [w, h]]` in command files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl GridRect {
    pub fn fits(&self, axis: Axis) -> bool {
        let (cols, rows) = axis.grid();
        self.w > 0 && self.h > 0 && self.x + self.w <= cols && self.y + self.h <= rows
    }

    fn touches(&self, axis: Axis) -> Touches {
        let (cols, rows) = axis.grid();
        Touches {
            left: self.x == 0,
            right: self.x + self.w == cols,
            top: self.y == 0,
            bottom: self.y + self.h == rows,
        }
    }
}

impl From<[[u32; 2]; 2]> for GridRect {
    fn from([[x, y], [w, h]]: [[u32; 2]; 2]) -> Self {
        Self { x, y, w, h }
    }
}

impl From<GridRect> for [[u32; 2]; 2] {
    fn from(GridRect { x, y, w, h }: GridRect) -> Self {
        [[x, y], [w, h]]
    }
}

#[derive(Clone, Copy, Debug)]
struct Touches {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

impl Touches {
    fn any(&self) -> bool {
        self.left || self.right || self.top || self.bottom
    }
}

/// Maps grid cells onto `area`, rounding each edge independently so that
/// adjacent cells share edges without gaps.
pub fn grid_rect(area: Rect, axis: Axis, g: GridRect) -> Rect {
    let (cols, rows) = axis.grid();
    let edge = |origin: i32, length: i32, cells: u32, index: u32| {
        origin + (f64::from(length) * f64::from(index) / f64::from(cells)).round() as i32
    };
    let left = edge(area.x, area.w, cols, g.x);
    let right = edge(area.x, area.w, cols, g.x + g.w);
    let top = edge(area.y, area.h, rows, g.y);
    let bottom = edge(area.y, area.h, rows, g.y + g.h);
    Rect::new(left, top, right - left, bottom - top)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Spacing {
    pub padding: i32,
    pub fit_tightly_to_screen_edges: bool,
}

/// The frame a window receives for a grid target inside the monitor work area.
///
/// Sides on the work-area edge receive the full padding (or none when fitting
/// tightly); interior sides receive half, so neighbouring windows are
/// separated by exactly one padding.
pub fn target_frame(work: Rect, axis: Axis, g: GridRect, spacing: Spacing) -> Rect {
    let frame = grid_rect(work, axis, g);
    if spacing.padding <= 0 {
        return frame;
    }
    let touches = g.touches(axis);
    let side = |on_edge: bool| match (on_edge, spacing.fit_tightly_to_screen_edges) {
        (true, true) => 0,
        (true, false) => spacing.padding,
        (false, _) => spacing.padding / 2,
    };
    frame.inset(Extents {
        left: side(touches.left),
        right: side(touches.right),
        top: side(touches.top),
        bottom: side(touches.bottom),
    })
}

#[derive(Clone, Copy, Debug)]
pub struct ActivationTuning {
    /// Thickness in pixels of activation segments that lie on a monitor edge.
    pub edge_width: i32,
    /// Scale applied around the centre of segments that touch no monitor edge.
    pub interior_scale: f64,
}

/// Whether the pointer at `(px, py)` is inside an activation segment.
///
/// Segments touching a monitor edge only react within `edge_width` pixels of
/// that edge, so a window dragged through the middle of the screen does not
/// trigger them. Interior segments are scaled around their centre.
pub fn activation_hit(
    monitor: Rect,
    axis: Axis,
    segment: GridRect,
    (px, py): (i32, i32),
    tuning: ActivationTuning,
) -> bool {
    let cell = grid_rect(monitor, axis, segment);
    let touches = segment.touches(axis);
    if touches.any() {
        if !cell.contains(px, py) {
            return false;
        }
        let ew = tuning.edge_width.max(1);
        return (touches.left && px < monitor.x + ew)
            || (touches.right && px >= monitor.right() - ew)
            || (touches.top && py < monitor.y + ew)
            || (touches.bottom && py >= monitor.bottom() - ew);
    }
    let scale = tuning.interior_scale.max(0.1);
    let (cx, cy) = (
        f64::from(cell.x) + f64::from(cell.w) / 2.0,
        f64::from(cell.y) + f64::from(cell.h) / 2.0,
    );
    let (hw, hh) = (
        f64::from(cell.w) * scale / 2.0,
        f64::from(cell.h) * scale / 2.0,
    );
    let (px, py) = (f64::from(px), f64::from(py));
    px >= cx - hw && px < cx + hw && py >= cy - hh && py < cy + hh
}

/// Keeps the window size (clamped to the work area) and centres it.
pub fn centered(frame: Rect, work: Rect) -> Rect {
    let w = frame.w.min(work.w);
    let h = frame.h.min(work.h);
    Rect::new(work.x + (work.w - w) / 2, work.y + (work.h - h) / 2, w, h)
}

/// Moves a frame to another work area, preserving its relative position and size.
pub fn transfer(frame: Rect, from: Rect, to: Rect) -> Rect {
    let sx = f64::from(to.w) / f64::from(from.w.max(1));
    let sy = f64::from(to.h) / f64::from(from.h.max(1));
    let x = to.x + (f64::from(frame.x - from.x) * sx).round() as i32;
    let y = to.y + (f64::from(frame.y - from.y) * sy).round() as i32;
    let w = ((f64::from(frame.w) * sx).round() as i32).clamp(1, to.w);
    let h = ((f64::from(frame.h) * sy).round() as i32).clamp(1, to.h);
    Rect::new(
        x.clamp(to.x, to.right() - w),
        y.clamp(to.y, to.bottom() - h),
        w,
        h,
    )
}

/// The frame for restoring the original size while the pointer drags `current`,
/// keeping the pointer at the same relative horizontal position on the window.
pub fn unsnapped(original: Rect, current: Rect, (px, _py): (i32, i32)) -> Rect {
    let fraction = f64::from(px - current.x) / f64::from(current.w.max(1));
    let x = px - (fraction * f64::from(original.w)).round() as i32;
    Rect::new(x, current.y, original.w, original.h)
}

/// Window managers may adjust a requested frame (size increments, minimum
/// sizes), so a frame we applied is compared loosely.
pub fn roughly_same(applied: Rect, observed: Rect) -> bool {
    (applied.x - observed.x).abs() <= 4
        && (applied.y - observed.y).abs() <= 4
        && (applied.w - observed.w).abs() <= 32
        && (applied.h - observed.h).abs() <= 32
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect::new(0, 0, 1920, 1080);
    const TUNING: ActivationTuning = ActivationTuning {
        edge_width: 6,
        interior_scale: 1.0,
    };

    fn g(x: u32, y: u32, w: u32, h: u32) -> GridRect {
        GridRect { x, y, w, h }
    }

    #[test]
    fn halves_and_thirds_tile_without_gaps() {
        let work = Rect::new(0, 27, 1920, 1053);
        let left = grid_rect(work, Axis::Horizontal, g(0, 0, 12, 12));
        let right = grid_rect(work, Axis::Horizontal, g(12, 0, 12, 12));
        assert_eq!(left, Rect::new(0, 27, 960, 1053));
        assert_eq!(left.right(), right.x);
        assert_eq!(right.right(), 1920);

        let thirds: Vec<_> = [0, 8, 16]
            .map(|x| grid_rect(Rect::new(0, 0, 1366, 768), Axis::Horizontal, g(x, 0, 8, 12)))
            .into();
        assert_eq!(thirds[0].right(), thirds[1].x);
        assert_eq!(thirds[1].right(), thirds[2].x);
        assert_eq!(thirds[2].right(), 1366);
    }

    #[test]
    fn padding_is_full_on_edges_and_half_between_windows() {
        let spacing = Spacing {
            padding: 10,
            fit_tightly_to_screen_edges: false,
        };
        let left = target_frame(SCREEN, Axis::Horizontal, g(0, 0, 12, 12), spacing);
        assert_eq!(left, Rect::new(10, 10, 960 - 15, 1080 - 20));

        let tight = Spacing {
            fit_tightly_to_screen_edges: true,
            ..spacing
        };
        let left = target_frame(SCREEN, Axis::Horizontal, g(0, 0, 12, 12), tight);
        let right = target_frame(SCREEN, Axis::Horizontal, g(12, 0, 12, 12), tight);
        assert_eq!(left, Rect::new(0, 0, 955, 1080));
        assert_eq!(right.x - left.right(), 10);
    }

    #[test]
    fn edge_segments_only_react_near_the_edge() {
        let left_edge = g(0, 3, 1, 6);
        assert!(activation_hit(
            SCREEN,
            Axis::Horizontal,
            left_edge,
            (0, 540),
            TUNING
        ));
        assert!(activation_hit(
            SCREEN,
            Axis::Horizontal,
            left_edge,
            (5, 540),
            TUNING
        ));
        assert!(!activation_hit(
            SCREEN,
            Axis::Horizontal,
            left_edge,
            (6, 540),
            TUNING
        ));
        assert!(!activation_hit(
            SCREEN,
            Axis::Horizontal,
            left_edge,
            (0, 100),
            TUNING
        ));

        let corner = g(23, 0, 1, 1);
        assert!(activation_hit(
            SCREEN,
            Axis::Horizontal,
            corner,
            (1919, 40),
            TUNING
        ));
        assert!(activation_hit(
            SCREEN,
            Axis::Horizontal,
            corner,
            (1890, 0),
            TUNING
        ));
        assert!(!activation_hit(
            SCREEN,
            Axis::Horizontal,
            corner,
            (1890, 40),
            TUNING
        ));
    }

    #[test]
    fn interior_segments_scale_around_their_centre() {
        let segment = g(11, 10, 2, 1);
        let cell = grid_rect(SCREEN, Axis::Horizontal, segment);
        let (cx, cy) = cell.center();
        assert!(activation_hit(
            SCREEN,
            Axis::Horizontal,
            segment,
            (cell.x, cy),
            TUNING
        ));
        let half = ActivationTuning {
            interior_scale: 0.5,
            ..TUNING
        };
        assert!(!activation_hit(
            SCREEN,
            Axis::Horizontal,
            segment,
            (cell.x, cy),
            half
        ));
        assert!(activation_hit(
            SCREEN,
            Axis::Horizontal,
            segment,
            (cx, cy),
            half
        ));
    }

    #[test]
    fn transfer_keeps_relative_frame() {
        let from = Rect::new(0, 0, 1920, 1080);
        let to = Rect::new(1920, 0, 2560, 1440);
        let moved = transfer(Rect::new(0, 0, 960, 1080), from, to);
        assert_eq!(moved, Rect::new(1920, 0, 1280, 1440));
    }

    #[test]
    fn centered_clamps_to_work_area() {
        let work = Rect::new(0, 30, 1000, 700);
        assert_eq!(
            centered(Rect::new(5, 5, 400, 300), work),
            Rect::new(300, 230, 400, 300)
        );
        assert_eq!(centered(Rect::new(5, 5, 1200, 900), work), work);
    }

    #[test]
    fn unsnapped_keeps_pointer_fraction() {
        let current = Rect::new(0, 0, 960, 1080);
        let restored = unsnapped(Rect::new(100, 100, 480, 300), current, (480, 10));
        assert_eq!(restored, Rect::new(240, 0, 480, 300));
    }
}
