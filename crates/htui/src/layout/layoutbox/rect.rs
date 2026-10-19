//! Rectangle in column/row space within the virtual canvas.

/// Rectangle in column/row space within the virtual canvas.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    /// Create a rectangle at `(x, y)` with the given width and height.
    pub fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// True when the canvas point `(x, y)` falls inside the rect
    /// (half-open: `x < x + width`, `y < y + height`).
    pub fn contains(&self, x: u32, y: u32) -> bool {
        let x = x as usize;
        let y = y as usize;
        x >= self.x
            && x < self.x.saturating_add(self.width)
            && y >= self.y
            && y < self.y.saturating_add(self.height)
    }

    /// Number of cells covered by the rect (used for deepest-hit ranking).
    pub fn area(&self) -> usize {
        self.width.saturating_mul(self.height)
    }
}
