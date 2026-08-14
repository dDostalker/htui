//! Block-flow row allocator used while laying out siblings.

/// Tracks the next free row when laying out block-level siblings.
///
/// Allocated by [`Self::allocate`], which both records the start `y` and
/// advances the cursor by the box's height in a single call so every box is
/// placed at a deterministic row.
#[derive(Debug, Clone, Copy, Default)]
pub struct Cursor {
    pub y: usize,
}

impl Cursor {
    pub fn new() -> Self {
        Self { y: 0 }
    }

    /// Reserve `height` rows starting at the current cursor; returns the
    /// starting `y` and advances the cursor by `height`.
    pub fn allocate(&mut self, height: usize) -> usize {
        let start = self.y;
        self.y = self.y.saturating_add(height);
        start
    }

    /// Advance the cursor by `rows` without returning a value. Use this for
    /// margin-top / margin-bottom gaps where the reserved rows don't belong
    /// to any specific box — [`allocate`](Self::allocate) would mistakenly
    /// treat those rows as the box's own row.
    pub fn skip(&mut self, rows: usize) {
        self.y = self.y.saturating_add(rows);
    }
}
