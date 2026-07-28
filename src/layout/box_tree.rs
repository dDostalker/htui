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

use crossterm::style::Color;
use unicode_segmentation::UnicodeSegmentation;

use crate::html::box_edges::BoxEdges;
use crate::tdom::node::NodeId;
use crate::tui::graph_api::FontStyle;

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
    /// (used by `<header>` / `<footer>`). `chrome` marks chrome elements
    /// (`<aside>`/`<header>`/`<footer>`) whose background always covers the
    /// full border-box — even when an explicit `background-color` is
    /// supplied — so the block reads as a solid chrome strip regardless of
    /// the inline content's text-align / font-size.
    Block {
        fill_bg: bool,
        spacing_after: usize,
        chrome: bool,
    },
    /// `<img>`: rasterized into a fixed-size grid of `cols` × `rows` cells.
    Image { cols: usize, rows: usize },
}

/// One inline segment of a block-level element.
///
/// A block can hold multiple `InlineRun`s when its inline tree contains
/// `<span>`s with distinct styles. Each run carries the resolved style, fg
/// and bg that should be applied to its text. The paint phase walks `runs`
/// in order and emits one styled cell per `char_width` column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineRun {
    pub text: String,
    pub style: FontStyle,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
}

impl InlineRun {
    pub fn new(
        text: impl Into<String>,
        style: FontStyle,
        fg: Option<Color>,
        bg: Option<Color>,
    ) -> Self {
        Self {
            text: text.into(),
            style,
            fg,
            bg,
        }
    }

    /// Returns the cell width this run occupies at the default
    /// (un-expanded) font-size: `grapheme_width` per grapheme cluster, so a
    /// wide CJK glyph (or a 2-column emoji) at the end of one run does not
    /// collide with the first cell of the next. Layouts that need the width
    /// at a larger font-size should call [`Self::width_at`].
    ///
    /// Iterating graphemes rather than `char`s lets multi-codepoint emoji
    /// (`👨‍👩‍👧`, `👍🏽`, `❤️`) count as a single cluster — `char_width`
    /// would either over- or under-count them depending on how many
    /// combining / ZWJ codepoints the cluster contains.
    pub fn width(&self) -> usize {
        use crate::tui::graph_api::grapheme_width;
        self.text.graphemes(true).map(grapheme_width).sum()
    }

    /// Cell width this run occupies at the supplied `font_size_cells`. Each
    /// grapheme is treated as `max(grapheme_width(g), font_size_cells)`
    /// cells wide (matching the expansion the paint phase applies), so a
    /// heading with font-size=2 has its `text-align` measured against the
    /// expanded width — not the single-cell-per-char default.
    pub fn width_at(&self, font_size_cells: usize) -> usize {
        use crate::tui::graph_api::grapheme_width;
        let font_size = font_size_cells.max(1);
        self.text
            .graphemes(true)
            .map(|g| grapheme_width(g).max(font_size))
            .sum()
    }

    /// Returns true when the run has no visible text.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// One rectangle in the layout tree.
///
/// `rect` is the **border-box**: it includes the border, padding and the
/// content area. `box_edges` carries the resolved padding / border / margin
/// values so the paint phase can draw border glyphs around the rect's
/// perimeter and offset inline content into the content area.
///
/// `children` holds *nested block-level* boxes laid out beneath this box.
/// Inline text content lives in [`Self::runs`] as one or more
/// [`InlineRun`]s — a single text node becomes one run, while `<span>`
/// boundaries produce runs with their own style. The field is `None`
/// only for purely structural containers (e.g. an `<aside>` whose text
/// is delegated to nested block children).
///
/// `font_size_cells` is the **effective font-size** of this box in terminal
/// cells. It is used by the paint phase to expand each character cell by
/// this factor (so `<h1>` with the default 2em font-size renders each
/// character across 2 cells, with the second cell filled with the run's
/// background). When `font_size_cells == 1` the rendering is byte-identical
/// to the pre-font-size layout.
#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub node_id: NodeId,
    pub rect: Rect,
    pub kind: BoxKind,
    pub box_edges: BoxEdges,
    pub runs: Option<Vec<InlineRun>>,
    pub image_path: Option<String>,
    pub children: Vec<LayoutBox>,
    /// Effective font-size of this box in terminal cells (≥ 1).
    pub font_size_cells: usize,
}

impl LayoutBox {
    /// Create a block-level box whose inline content is split into one
    /// or more `InlineRun`s (one per `<span>` boundary, one per direct
    /// text node). Pass an empty `Vec` when the block is purely
    /// structural — `runs` will be stored as `None`.
    pub fn new_block(
        node_id: NodeId,
        rect: Rect,
        box_edges: BoxEdges,
        fill_bg: bool,
        spacing_after: usize,
        runs: Vec<InlineRun>,
        children: Vec<LayoutBox>,
        font_size_cells: usize,
    ) -> Self {
        let runs = if runs.is_empty() { None } else { Some(runs) };
        Self {
            node_id,
            rect,
            kind: BoxKind::Block {
                fill_bg,
                spacing_after,
                chrome: false,
            },
            box_edges,
            runs,
            image_path: None,
            children,
            font_size_cells,
        }
    }

    /// Create a block-level box whose inline content is split into one
    /// or more `InlineRun`s (one per `<span>` boundary, one per direct
    /// text node). Pass an empty `Vec` when the block is purely
    /// structural — `runs` will be stored as `None`.
    ///
    /// `chrome` marks the box as a chrome container (`<aside>` /
    /// `<header>` / `<footer>`) whose background always covers the
    /// full border-box regardless of inline content.
    pub fn new_block_with_chrome(
        node_id: NodeId,
        rect: Rect,
        box_edges: BoxEdges,
        fill_bg: bool,
        spacing_after: usize,
        runs: Vec<InlineRun>,
        children: Vec<LayoutBox>,
        font_size_cells: usize,
        chrome: bool,
    ) -> Self {
        let runs = if runs.is_empty() { None } else { Some(runs) };
        Self {
            node_id,
            rect,
            kind: BoxKind::Block {
                fill_bg,
                spacing_after,
                chrome,
            },
            box_edges,
            runs,
            image_path: None,
            children,
            font_size_cells,
        }
    }

    /// Create an `<img>` box. Caller must ensure `path` is loadable.
    pub fn new_image(node_id: NodeId, rect: Rect, cols: usize, rows: usize, path: String) -> Self {
        Self {
            node_id,
            rect,
            kind: BoxKind::Image { cols, rows },
            box_edges: BoxEdges::default(),
            runs: None,
            image_path: Some(path),
            children: vec![],
            font_size_cells: 1,
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

    /// Advance the cursor by `rows` without returning a value. Use this for
    /// margin-top / margin-bottom gaps where the reserved rows don't belong
    /// to any specific box — [`allocate`](Self::allocate) would mistakenly
    /// treat those rows as the box's own row.
    pub fn skip(&mut self, rows: usize) {
        self.y = self.y.saturating_add(rows);
    }
}
