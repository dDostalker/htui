//! Layout box tree — the intermediate representation between the DOM and the
//! terminal canvas.
//!
//! A [`LayoutBox`] describes a single rectangular region on the virtual canvas
//! (one block-level element or one inline run), its computed style, and its
//! nested children. The two-phase pipeline is:
//!
//! ```text
//! DOM → LayoutBox tree → Block grid
//! ```
//!
//! - [`layout_tree`](crate::layout::tree::layout_tree) builds the tree and
//!   assigns rectangles from containing-block constraints.
//! - [`paint_layout_tree`](crate::layout::paint::paint_layout_tree) walks the
//!   tree and writes [`Block`] cells into the terminal canvas.

use crate::tdom::node::NodeId;

/// Rectangle in column/row space within the virtual canvas.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    pub fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Distinguishes the rendering shape of a layout box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxKind {
    /// Block-level container. `fill_bg` controls whether the entire rect is
    /// painted with the element's background color (always true for `<aside>`
    /// even without an explicit `background-color`, so it visually forms a
    /// solid stripe). `spacing_after` adds trailing blank rows below the box
    /// (used by `<header>` / `<footer>`).
    Block { fill_bg: bool, spacing_after: usize },
    /// `<img>`: rasterized into a fixed-size grid of `cols` × `rows` cells.
    Image { cols: usize, rows: usize },
}

/// One rectangle in the layout tree.
///
/// `children` holds *nested block-level* boxes laid out beneath this box.
/// Inline text content is collapsed into [`Self::text`] for the common
/// `<p>...</p>` case; the field is `None` for purely structural containers.
#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub node_id: NodeId,
    pub rect: Rect,
    pub kind: BoxKind,
    pub text: Option<String>,
    pub image_path: Option<String>,
    pub children: Vec<LayoutBox>,
}

impl LayoutBox {
    /// Create a block-level box at `rect` with the given text and children.
    pub fn new_block(
        node_id: NodeId,
        rect: Rect,
        fill_bg: bool,
        spacing_after: usize,
        text: Option<String>,
        children: Vec<LayoutBox>,
    ) -> Self {
        Self {
            node_id,
            rect,
            kind: BoxKind::Block {
                fill_bg,
                spacing_after,
            },
            text,
            image_path: None,
            children,
        }
    }

    /// Create an `<img>` box. Caller must ensure `path` is loadable.
    pub fn new_image(node_id: NodeId, rect: Rect, cols: usize, rows: usize, path: String) -> Self {
        Self {
            node_id,
            rect,
            kind: BoxKind::Image { cols, rows },
            text: None,
            image_path: Some(path),
            children: vec![],
        }
    }
}

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

    /// Peek at the next available row without advancing.
    pub fn peek(&self) -> usize {
        self.y
    }
}

