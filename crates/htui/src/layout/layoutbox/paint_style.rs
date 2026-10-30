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

/// Layout facts for a box's native scrollbars.
///
/// Layout computes them after measuring overflowing children so the input
/// layer and paint phase share one canonical thumb geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollbarLayout {
    /// Current content offset in rows.
    pub scroll_y: usize,
    /// Maximum valid offset (`content_height - viewport_height`).
    pub max_scroll: usize,
    /// Full scrollable content height in rows.
    pub content_height: usize,
    /// Visible content height in rows, excluding padding / border.
    pub viewport_height: usize,
    /// Absolute one-column track rectangle.
    pub track: crate::layout::Rect,
    /// Absolute thumb rectangle inside `track`.
    pub thumb: crate::layout::Rect,
    /// Current content offset in columns (0 when the box does not scroll
    /// horizontally).
    pub scroll_x: usize,
    /// Maximum valid horizontal offset (`content_width - viewport_width`).
    pub max_scroll_x: usize,
    /// Full scrollable content width in columns.
    pub content_width: usize,
    /// Visible content width in columns, excluding padding / border.
    pub viewport_width: usize,
    /// Absolute one-row bottom track rectangle; zero-height when the box has
    /// no horizontal overflow.
    pub h_track: crate::layout::Rect,
    /// Absolute thumb rectangle inside `h_track`.
    pub h_thumb: crate::layout::Rect,
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
    /// Native scroll state for `overflow-y: auto/scroll`; `None` for ordinary
    /// boxes and non-overflowing `auto` containers.
    pub scrollbar: Option<ScrollbarLayout>,
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
            scrollbar: None,
        }
    }
}
