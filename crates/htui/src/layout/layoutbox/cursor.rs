//! Block-flow row allocator used while laying out siblings.

/// Tracks the next free row when laying out block-level siblings.
///
/// Allocated by [`Self::allocate`], which both records the start `y` and
/// advances the cursor by the box's height in a single call so every box is
/// placed at a deterministic row.
///
/// [`Cursor::prev_bottom_margin`] carries the previous in-flow sibling's
/// bottom margin so adjacent vertical margins can collapse to the larger of
/// the two (CSS margin collapsing), instead of stacking.
#[derive(Debug, Clone, Copy, Default)]
pub struct Cursor {
    pub y: usize,
    /// Bottom margin left by the previous in-flow block sibling. Consumed by
    /// the next block's top-margin collapse (larger wins); only meaningful
    /// while laying out siblings in a block formatting context.
    pub prev_bottom_margin: usize,
}

impl Cursor {
    pub fn new() -> Self {
        Self {
            y: 0,
            prev_bottom_margin: 0,
        }
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

    /// Skip the vertical gap between two in-flow block siblings, collapsing
    /// `margin_top` with the previous sibling's bottom margin (CSS vertical
    /// margin collapsing: the larger wins). The previous bottom margin is
    /// already included in [`Cursor::y`] (the sibling's layout advanced past
    /// it), so only the excess of `margin_top` over it adds rows. Consumes
    /// the pending bottom margin regardless of which side won.
    pub fn collapse_and_skip(&mut self, margin_top: usize) {
        if margin_top > self.prev_bottom_margin {
            self.skip(margin_top - self.prev_bottom_margin);
        }
        self.prev_bottom_margin = 0;
    }

    /// Leave this block's bottom margin for the next in-flow sibling to
    /// collapse its top margin against.
    pub fn leave_bottom_margin(&mut self, margin_bottom: usize) {
        self.prev_bottom_margin = margin_bottom;
    }
}
