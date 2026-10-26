//! Paint-only visual chrome carried by every [`crate::layout::LayoutBox`].
use crossterm::style::Color;

use crate::html::box_edges::BoxEdges;
use crate::html::css_parse::{CornerRadii, TextAlign};

use super::inline_run::InlineRun;

/// Multi-column fragmentation of a box's own inline content.
///
/// Present when the box fragments its content into side-by-side columns; the
/// normal [`PaintStyle::runs`] stays `None` in that case and the wrapped lines
/// live here, grouped per column. Block children are re-positioned into the
/// columns by the layout phase, so only text needs this table.
#[derive(Debug, Clone)]
pub struct ColumnLayout {
    /// Wrapped inline lines for each column: `[column][line][run]`. A column
    /// with no inline content has an empty inner `Vec`.
    pub lines: Vec<Vec<Vec<InlineRun>>>,
    /// Absolute canvas x of each column's content-left edge.
    pub col_x: Vec<usize>,
    /// Content width of one column in cells.
    pub col_width: usize,
    /// Absolute canvas x of every painted column-rule (one between each
    /// adjacent pair of columns).
    pub rule_x: Vec<usize>,
    /// Colour of the column rules (the element's foreground when unspecified).
    pub rule_color: Option<Color>,
    /// Total column height (rows the columns span, content-box rows).
    pub height: usize,
}

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
    /// Multi-column fragmentation (see [`ColumnLayout`]). `None` for regular
    /// single-column boxes. When set, [`Self::runs`] is `None` and the box's
    /// inline text is painted per column instead.
    pub column_layout: Option<ColumnLayout>,
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
    /// Resolved `text-align` for the box's own inline runs. Carried here (not
    /// re-read from the cascade at paint time) so user-agent defaults that
    /// the layout phase resolved — `<th>`'s `center` — participate even
    /// though they are not stored in the node's `css_attr`.
    pub text_align: TextAlign,
    /// First-line indent (CSS `text-indent`) in cells: the first visual
    /// line's wrap budget shrank by this many columns at layout time, and
    /// the paint phase shifts its origin right accordingly.
    pub text_indent: usize,
}

impl Default for PaintStyle {
    fn default() -> Self {
        Self {
            box_edges: BoxEdges::default(),
            runs: None,
            column_layout: None,
            font_size_cells: 1,
            line_height_cells: 1,
            border_radius: CornerRadii::default(),
            fill_bg: false,
            spacing_after: 0,
            text_align: TextAlign::Left,
            text_indent: 0,
        }
    }
}
