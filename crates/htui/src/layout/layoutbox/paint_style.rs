//! Paint-only visual chrome carried by every [`crate::layout::LayoutBox`].

use crate::html::box_edges::BoxEdges;

use super::inline_run::InlineRun;
use crate::html::css_parse::CornerRadii;

/// Paint-only data carried by every [`crate::layout::LayoutBox`].
///
/// This is the visual chrome of a box: padding / border / margin for box
/// geometry, inline runs for text, font-size for expansion, and the flags
/// that drive background-fill behaviour. Keeping it on a dedicated struct
/// (rather than as fields of [`crate::layout::LayoutBox`]) keeps the layout
/// tree focused on classification and lets the paint phase read every visual
/// property from a single place.
#[derive(Debug, Clone)]
pub struct PaintStyle {
    /// Resolved padding / border / margin edges.
    pub box_edges: BoxEdges,
    /// Inline runs for the box's own text content, grouped by visual line
    /// after wrapping. Each inner `Vec<InlineRun>` is one line that fits
    /// inside the content-box; the outer `Vec` has one entry per visual
    /// line. `None` when the box has no inline content of its own
    /// (e.g. a structural `<aside>` whose text lives in nested block
    /// children, or a replaced element like `<img>`).
    pub runs: Option<Vec<Vec<InlineRun>>>,
    /// Effective font-size of this box in terminal cells (≥ 1). Drives the
    /// `font_size_cells` expansion the paint phase applies so an `<h1>` with
    /// the default 2em font-size renders each character across 2 cells.
    pub font_size_cells: usize,
    /// Rows one visual line occupies (CSS `line-height`, resolved to cells).
    /// `normal` yields `font_size_cells` — one line box per font cell; larger
    /// values spread wrapped lines apart vertically.
    pub line_height_cells: usize,
    /// Corner radii (CSS `border-radius`) in cells. Any non-zero corner
    /// paints with the rounded box-drawing corner glyphs.
    pub border_radius: CornerRadii,
    /// Whether the rect's background should be painted. `true` when the box
    /// has an explicit CSS `background-color`.
    pub fill_bg: bool,
    /// Trailing blank rows reserved below the rect (default 0). `<hr>` uses 1
    /// to leave breathing room before the next block. These rows are not
    /// filled with the rect's background.
    pub spacing_after: usize,
}

impl Default for PaintStyle {
    fn default() -> Self {
        Self {
            box_edges: BoxEdges::default(),
            runs: None,
            font_size_cells: 1,
            line_height_cells: 1,
            border_radius: CornerRadii::default(),
            fill_bg: false,
            spacing_after: 0,
        }
    }
}