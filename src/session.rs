use std::fmt::Debug;
use std::hash::Hash;

use crate::geometry::{Monitor, Rect};

/// Window-manager operations the snapping logic needs. X11 and macOS each
/// provide one.
pub trait Session {
    type Window: Copy + Eq + Hash + Debug;

    fn active_window(&self) -> anyhow::Result<Option<Self::Window>>;
    /// Desktop, dock, panel, or another surface that must not be moved.
    fn is_reserved(&self, window: Self::Window) -> bool;
    /// Names compared with `ignored_apps` (instance and class, or app name and bundle id).
    fn app_names(&self, window: Self::Window) -> (String, String);
    fn frame(&self, window: Self::Window) -> anyhow::Result<Rect>;
    fn set_frame(&self, window: Self::Window, frame: Rect) -> anyhow::Result<()>;
    fn toggle_above(&self, window: Self::Window) -> anyhow::Result<()>;
    fn monitors(&self) -> anyhow::Result<Vec<Monitor>>;
    fn pointer(&self) -> anyhow::Result<(i32, i32)>;
    fn window_under_pointer(&self) -> anyhow::Result<Option<Self::Window>>;
    fn exists(&self, window: Self::Window) -> bool;
    fn show_overlay(&self, frame: Rect) -> anyhow::Result<()>;
    fn hide_overlay(&self) -> anyhow::Result<()>;
}
