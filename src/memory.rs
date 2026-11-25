use std::collections::HashMap;
use std::hash::Hash;

use crate::geometry::{Rect, roughly_same};

/// Remembers each window's frame from before Sash first moved it, so that
/// Restore and drag-to-unsnap can return to it.
pub struct FrameMemory<W> {
    entries: HashMap<W, Entry>,
}

#[derive(Clone, Copy)]
struct Entry {
    original: Rect,
    applied: Rect,
}

impl<W: Eq + Hash + Copy> FrameMemory<W> {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Records that `window`, currently at `current`, is being moved to `applied`.
    /// The original frame is kept while the window stays where Sash put it;
    /// once the user moves it by other means, `current` becomes the new original.
    pub fn record(&mut self, window: W, current: Rect, applied: Rect) {
        match self.entries.get_mut(&window) {
            Some(entry) if roughly_same(entry.applied, current) => entry.applied = applied,
            _ => {
                self.entries.insert(
                    window,
                    Entry {
                        original: current,
                        applied,
                    },
                );
            }
        }
    }

    /// The original frame of a window that is still where Sash put it.
    pub fn snapped_original(&self, window: W, current: Rect) -> Option<Rect> {
        self.entries
            .get(&window)
            .filter(|entry| roughly_same(entry.applied, current))
            .map(|entry| entry.original)
    }

    pub fn take_original(&mut self, window: W) -> Option<Rect> {
        self.entries.remove(&window).map(|entry| entry.original)
    }

    pub fn forget(&mut self, window: W) {
        self.entries.remove(&window);
    }

    pub fn retain(&mut self, alive: impl Fn(&W) -> bool) {
        self.entries.retain(|window, _| alive(window));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGINAL: Rect = Rect::new(100, 100, 800, 600);
    const LEFT: Rect = Rect::new(0, 0, 960, 1080);
    const RIGHT: Rect = Rect::new(960, 0, 960, 1080);

    #[test]
    fn keeps_first_original_across_consecutive_snaps() {
        let mut memory = FrameMemory::new();
        memory.record(1, ORIGINAL, LEFT);
        memory.record(1, LEFT, RIGHT);
        assert_eq!(memory.snapped_original(1, RIGHT), Some(ORIGINAL));
        assert_eq!(memory.take_original(1), Some(ORIGINAL));
        assert_eq!(memory.take_original(1), None);
    }

    #[test]
    fn manual_moves_reset_the_original() {
        let mut memory = FrameMemory::new();
        memory.record(1, ORIGINAL, LEFT);
        let moved = Rect::new(300, 200, 500, 400);
        assert_eq!(memory.snapped_original(1, moved), None);
        memory.record(1, moved, RIGHT);
        assert_eq!(memory.take_original(1), Some(moved));
    }
}
