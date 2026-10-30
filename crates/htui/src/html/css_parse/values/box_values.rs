use std::sync::Arc;

use super::length::parse_length_token;
use super::length::{FontContext, Viewport};

use indexmap::IndexMap;

/// Box sizing model (CSS `box-sizing`).
///
/// The engine's historical default applies `width` / `height` to the
/// **border-box** (the terminal cell grid has no separate box-model
/// primitive), so unset behaves like [`BoxSizing::BorderBox`]; declaring
/// `content-box` switches those properties back to CSS semantics where
/// padding + border are added on top.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoxSizing {
    /// `border-box` — `width` / `height` include padding and border.
    #[default]
    BorderBox,
    /// `content-box` — `width` / `height` apply to the content box only;
    /// padding + border are added on top.
    ContentBox,
}

#[doc(hidden)]
/// Parse a `box-sizing` keyword (`content-box` / `border-box`).
pub fn parse_box_sizing(s: &str) -> Option<BoxSizing> {
    match s.trim().to_ascii_lowercase().as_str() {
        "content-box" => Some(BoxSizing::ContentBox),
        "border-box" => Some(BoxSizing::BorderBox),
        _ => None,
    }
}

/// CSS `position` value (CSS Positioned Layout).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Position {
    /// `static` — normal flow; `top` / `right` / `bottom` / `left` have no
    /// effect.
    #[default]
    Static,
    /// `relative` — keeps its normal-flow slot, then offsets by the insets.
    Relative,
    /// `absolute` — removed from flow and positioned against the nearest
    /// positioned ancestor (or the initial containing block when there is
    /// none).
    Absolute,
    /// `fixed` — removed from flow and positioned against the viewport.
    Fixed,
    /// `sticky` — keeps its normal-flow slot; while scrolling, the box is
    /// clamped into the viewport once its flow position would move past the
    /// inset offset (v1 subset: the constraint rectangle is the viewport,
    /// not the nearest scrolling ancestor's content box).
    Sticky,
}

/// CSS `visibility` value.
///
/// Inherited: `hidden` on a parent hides its own content but a descendant
/// with an explicit `visible` still paints (each node decides at paint time).
/// `collapse` behaves like `hidden` except on table rows / columns, which a
/// terminal table renderer does not distinguish — it is treated as `hidden`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Visibility {
    #[default]
    Visible,
    /// `hidden` / `collapse` — the box keeps its layout space but paints
    /// nothing of its own chrome / inline content.
    Hidden,
}

#[doc(hidden)]
/// Parse a `visibility` keyword (`hidden` / `collapse` map to `Hidden`).
pub fn parse_visibility(s: &str) -> Option<Visibility> {
    match s.trim().to_ascii_lowercase().as_str() {
        "visible" => Some(Visibility::Visible),
        "hidden" | "collapse" => Some(Visibility::Hidden),
        _ => None,
    }
}

/// CSS `table-layout` algorithm selector.
///
/// `auto` sizes each column to its widest cell (the existing algorithm);
/// `fixed` divides the available width evenly across the first row's column
/// count — the terminal's take on the spec's first-row-driven layout (no
/// `<col>` width / first-row `width` declaration weighting).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TableLayout {
    /// `auto` — column widths follow the widest content.
    #[default]
    Auto,
    /// `fixed` — columns share the available width evenly.
    Fixed,
}

/// Parse a `table-layout` keyword.
pub fn parse_table_layout(s: &str) -> Option<TableLayout> {
    match s.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(TableLayout::Auto),
        "fixed" => Some(TableLayout::Fixed),
        _ => None,
    }
}

/// CSS `caption-side` — which side of the table the `<caption>` box renders
/// on. Only the two horizontal sides exist in a terminal grid.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CaptionSide {
    /// `top` (the default).
    #[default]
    Top,
    /// `bottom`.
    Bottom,
}

/// Parse a `caption-side` keyword.
pub fn parse_caption_side(s: &str) -> Option<CaptionSide> {
    match s.trim().to_ascii_lowercase().as_str() {
        "top" => Some(CaptionSide::Top),
        "bottom" => Some(CaptionSide::Bottom),
        _ => None,
    }
}

/// CSS `empty-cells` — whether cells with no visible content paint their
/// borders / background.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EmptyCells {
    /// `show` (the default).
    #[default]
    Show,
    /// `hide` — empty cells paint nothing.
    Hide,
}

/// Parse an `empty-cells` keyword.
pub fn parse_empty_cells(s: &str) -> Option<EmptyCells> {
    match s.trim().to_ascii_lowercase().as_str() {
        "show" => Some(EmptyCells::Show),
        "hide" => Some(EmptyCells::Hide),
        _ => None,
    }
}

/// CSS `overflow` for a box's content, resolved to the two behaviours the
/// terminal can express.
///
/// `visible` (the CSS default) lets content paint beyond the box's content
/// area, exactly like a browser; `hidden` clips it there. The
/// container-scroll keywords (`auto` / `scroll`) have no terminal
/// counterpart — the engine scrolls the whole document, not boxes — so they
/// degenerate to [`Overflow::Clip`] (see 能力差异清单 "overflow").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Overflow {
    /// Content paints beyond the content box (the browser default).
    #[default]
    Visible,
    /// `hidden` / `auto` / `scroll` / `clip` — content is clipped to the
    /// content box.
    Clip,
}

#[doc(hidden)]
/// Parse an `overflow` keyword (`auto` / `scroll` / `clip` all clip; the
/// terminal has no container-level scrolling).
pub fn parse_overflow(s: &str) -> Option<Overflow> {
    match s.trim().to_ascii_lowercase().as_str() {
        "visible" => Some(Overflow::Visible),
        "hidden" | "auto" | "scroll" | "clip" => Some(Overflow::Clip),
        _ => None,
    }
}

/// CSS `top` / `right` / `bottom` / `left`, resolved to cells.
///
/// Each side is `None` when the property is unset / `auto`, in which case
/// the positioning pass keeps the box's static position on that axis. The
/// values are signed cells — negative insets (common for tooltips/popups:
/// `top: -2`, `left: -10`) shift the box opposite the axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InsetSides {
    pub top: Option<isize>,
    pub right: Option<isize>,
    pub bottom: Option<isize>,
    pub left: Option<isize>,
}

/// Resolved positioning for a layout box: `position` + insets + `z-index`.
///
/// Computed once at layout time (when the length context is available) and
/// stored on the [`LayoutBox`](crate::layout::LayoutBox) so the post-layout
/// positioning pass and the paint phase can read it without re-parsing CSS.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Positioning {
    pub position: Position,
    pub inset: InsetSides,
    /// CSS `z-index` — only meaningful on positioned boxes; higher values
    /// paint on top (negative values paint below static siblings).
    pub z_index: Option<i32>,
}

#[doc(hidden)]
/// Parse a `position` keyword.
pub fn parse_position(s: &str) -> Option<Position> {
    match s.trim().to_ascii_lowercase().as_str() {
        "relative" => Some(Position::Relative),
        "absolute" => Some(Position::Absolute),
        "fixed" => Some(Position::Fixed),
        "sticky" => Some(Position::Sticky),
        "static" => Some(Position::Static),
        _ => None,
    }
}

/// Four corner radii of a box (CSS `border-radius`), in cells.
///
/// `0` means a square corner. The paint phase renders any non-zero radius
/// with the rounded box-drawing corner glyphs (`╭ ╮ ╰ ╯`); radii larger than
/// 1 currently render identically to radius 1 — the terminal grid has no
/// finer rounding primitive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CornerRadii {
    /// Top-left corner radius.
    pub top_left: usize,
    /// Top-right corner radius.
    pub top_right: usize,
    /// Bottom-right corner radius.
    pub bottom_right: usize,
    /// Bottom-left corner radius.
    pub bottom_left: usize,
}

impl CornerRadii {
    /// `true` when at least one corner carries a non-zero radius.
    pub const fn is_rounded(&self) -> bool {
        self.top_left > 0 || self.top_right > 0 || self.bottom_right > 0 || self.bottom_left > 0
    }
}

/// Expand the `border-radius` shorthand (1–4 values) into a [`CornerRadii`],
/// then overlay the per-corner longhands (`border-top-left-radius`, …).
///
/// Follows the standard CSS 1-/2-/3-/4-value pattern: one value applies to
/// every corner, two set `(top-left & bottom-right) / (top-right &
/// bottom-left)`, three set top-left / (top-right & bottom-left) /
/// bottom-right, four set each corner in clockwise order. Tokens that cannot
/// be parsed as a [`Length`] are treated as `0`. The per-corner longhands win
/// over the shorthand (mirroring CSS priority); an unparseable longhand
/// leaves that corner untouched.
pub(in crate::html::css_parse) fn parse_border_radius(
    decls: &IndexMap<Arc<str>, Arc<str>>,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> CornerRadii {
    let mut radii: CornerRadii = decls
        .get("border-radius")
        .map(|value| {
            let parsed: Vec<usize> = value
                .split_whitespace()
                .filter_map(|t| parse_length_token(Some(t), containing_width, font_ctx, viewport))
                .collect();
            match parsed.len() {
                0 => CornerRadii::default(),
                1 => CornerRadii {
                    top_left: parsed[0],
                    top_right: parsed[0],
                    bottom_right: parsed[0],
                    bottom_left: parsed[0],
                },
                2 => CornerRadii {
                    top_left: parsed[0],
                    top_right: parsed[1],
                    bottom_right: parsed[0],
                    bottom_left: parsed[1],
                },
                3 => CornerRadii {
                    top_left: parsed[0],
                    top_right: parsed[1],
                    bottom_right: parsed[2],
                    bottom_left: parsed[1],
                },
                _ => CornerRadii {
                    top_left: parsed[0],
                    top_right: parsed[1],
                    bottom_right: parsed[2],
                    bottom_left: parsed[3],
                },
            }
        })
        .unwrap_or_default();
    // Per-corner longhands override the shorthand.
    for (name, slot) in [
        ("border-top-left-radius", &mut radii.top_left),
        ("border-top-right-radius", &mut radii.top_right),
        ("border-bottom-right-radius", &mut radii.bottom_right),
        ("border-bottom-left-radius", &mut radii.bottom_left),
    ] {
        if let Some(raw) = decls.get(name)
            && let Some(len) =
                parse_length_token(Some(raw.as_ref()), containing_width, font_ctx, viewport)
        {
            *slot = len;
        }
    }
    radii
}

// ============================================================
// CSS `display` value
//
// `CssDisplay` is the single source of truth for the CSS `display`
// property. Every other display-derived value (`OuterDisplay`,
// `FormattingContext`, and the inline-vs-block predicates in
// `walk_runs` / `layout_a`) is computed from this enum so that the
// generation phase sees one consistent classification.
//
// The enum covers every keyword CSS Display 3 / 4 defines; keywords
// the renderer doesn't implement yet (e.g. `run-in`) still parse but
// are mapped to their `flow` / `block` / `flex` / `table` equivalents
// at the outer / inner level so generation can hand them to the
// closest supported algorithm without dropping them.
// ============================================================

/// CSS `display` value, parsed from the author-declared string.
///
/// The variant determines both the outer role (block-level vs inline-level)
/// and the inner formatting context (flow / flex / table / grid) the box
/// will use. See the per-variant docs for the exact mapping; helpers
/// [`CssAttr::outer_display`](crate::html::css_parse::CssAttr::outer_display) and [`CssAttr::inner_formatting_context`](crate::html::css_parse::CssAttr::inner_formatting_context)
/// give the per-axis view for callers that need only one piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssDisplay {
    /// `display: block`. Block-level box that establishes a block
    /// formatting context.
    Block,
    /// `display: flow` — CSS Display 3 alias for the default block flow.
    /// Renders identically to `block` in the v1 renderer.
    Flow,
    /// `display: flow-root` — block container that establishes a new
    /// block formatting context. Treated as `block` for v1 layout; the
    /// BFC distinction is exposed via the `container` flag in
    /// [`crate::layout::OuterDisplay`].
    FlowRoot,
    /// `display: inline` — inline-level box that participates in the
    /// parent's inline formatting context. Does not establish a new FC.
    Inline,
    /// `display: inline-block` — inline-level box that establishes a
    /// block formatting context for its children.
    InlineBlock,
    /// `display: list-item` — block-level box that participates in the
    /// parent's flow as a list item. The marker is not rendered in v1;
    /// the box otherwise behaves like a `flow`.
    ListItem,
    /// `display: flex` — block-level box that establishes a flex
    /// formatting context for its children.
    Flex,
    /// `display: inline-flex` — inline-level box that establishes a
    /// flex formatting context for its children.
    InlineFlex,
    /// `display: grid` 鈥?block-level box that establishes a grid
    /// formatting context and is laid out by the grid algorithm.
    Grid,
    /// `display: inline-grid` 鈥?inline-level atomic box that establishes a
    /// grid formatting context.
    InlineGrid,
    /// `display: table` — block-level box that establishes a table
    /// formatting context.
    Table,
    /// `display: inline-table` — inline-level box that establishes a
    /// table formatting context.
    InlineTable,
    /// `display: table-row-group` — table-internal row group (`<tbody>`).
    /// Generated as a normal block flow container in v1 (no group chrome);
    /// the table algorithm expects its children to be `<tr>`s.
    TableRowGroup,
    /// `display: table-header-group` (`<thead>`) — like
    /// [`CssDisplay::TableRowGroup`] but conventionally printed before
    /// `<tbody>` on a re-flow. Treated identically in v1.
    TableHeaderGroup,
    /// `display: table-footer-group` (`<tfoot>`) — like
    /// [`CssDisplay::TableRowGroup`].
    TableFooterGroup,
    /// `display: table-row` — table row.
    TableRow,
    /// `display: table-cell` — table cell.
    TableCell,
    /// `display: table-column-group` (`<colgroup>`).
    TableColumnGroup,
    /// `display: table-column` (`<col>`).
    TableColumn,
    /// `display: table-caption` (`<caption>`) — block-level box that
    /// appears above / below the table.
    TableCaption,
    /// `display: none` — the element and its descendants generate no
    /// boxes. The generation phase drops these entirely.
    None,
    /// `display: contents` — the element itself generates no box, but
    /// its children participate in the element's *parent*'s formatting
    /// context (i.e. are flattened into the parent's children list).
    Contents,
}

impl CssDisplay {
    /// Parse a `display` value from a CSS string. Trims surrounding
    /// whitespace and lowercases the input. Returns `None` for
    /// unrecognised keywords so callers can fall back to the UA default.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "block" => Some(CssDisplay::Block),
            "flow" => Some(CssDisplay::Flow),
            "flow-root" => Some(CssDisplay::FlowRoot),
            "inline" => Some(CssDisplay::Inline),
            "inline-block" => Some(CssDisplay::InlineBlock),
            "list-item" => Some(CssDisplay::ListItem),
            "flex" => Some(CssDisplay::Flex),
            "inline-flex" => Some(CssDisplay::InlineFlex),
            "grid" => Some(CssDisplay::Grid),
            "inline-grid" => Some(CssDisplay::InlineGrid),
            "table" => Some(CssDisplay::Table),
            "inline-table" => Some(CssDisplay::InlineTable),
            "table-row-group" => Some(CssDisplay::TableRowGroup),
            "table-header-group" => Some(CssDisplay::TableHeaderGroup),
            "table-footer-group" => Some(CssDisplay::TableFooterGroup),
            "table-row" => Some(CssDisplay::TableRow),
            "table-cell" => Some(CssDisplay::TableCell),
            "table-column-group" => Some(CssDisplay::TableColumnGroup),
            "table-column" => Some(CssDisplay::TableColumn),
            "table-caption" => Some(CssDisplay::TableCaption),
            "none" => Some(CssDisplay::None),
            "contents" => Some(CssDisplay::Contents),
            _ => None,
        }
    }

    /// `true` when this value removes the element from the box tree.
    pub fn is_dropped(self) -> bool {
        matches!(self, CssDisplay::None)
    }

    /// `true` when this value removes the element's own box but keeps
    /// its children (which then flatten into the parent).
    pub fn is_transparent(self) -> bool {
        matches!(self, CssDisplay::Contents)
    }

    /// `true` for block-level keywords (the box participates in its
    /// parent's flow as a block). Used by
    /// [`collect_runs`](crate::layout::layoutbox::collect_runs) and
    /// `layout_a`
    /// to decide whether an element should be handled as a block or
    /// remain in the parent's inline runs.
    pub fn is_block_level(self) -> bool {
        matches!(
            self,
            CssDisplay::Block
                | CssDisplay::Flow
                | CssDisplay::FlowRoot
                | CssDisplay::ListItem
                | CssDisplay::Flex
                | CssDisplay::Grid
                | CssDisplay::Table
                | CssDisplay::TableRowGroup
                | CssDisplay::TableHeaderGroup
                | CssDisplay::TableFooterGroup
                | CssDisplay::TableRow
                | CssDisplay::TableCell
                | CssDisplay::TableColumnGroup
                | CssDisplay::TableColumn
                | CssDisplay::TableCaption
        )
    }

    /// `true` for inline-level keywords.
    pub fn is_inline_level(self) -> bool {
        matches!(
            self,
            CssDisplay::Inline
                | CssDisplay::InlineBlock
                | CssDisplay::InlineFlex
                | CssDisplay::InlineGrid
                | CssDisplay::InlineTable
        )
    }

    /// `true` for `display: flex` and `display: inline-flex`. Used by
    /// the inline-runs walker to recognise anchor children of flex
    /// containers as flex items rather than folding them into the
    /// parent's text.
    pub fn is_flex(self) -> bool {
        matches!(self, CssDisplay::Flex | CssDisplay::InlineFlex)
    }

    /// `true` for `display: grid` and `display: inline-grid`. Both establish
    /// grid formatting contexts; the second is an atomic inline-level box.
    pub fn is_grid(self) -> bool {
        matches!(self, CssDisplay::Grid | CssDisplay::InlineGrid)
    }

    /// `true` for `display: table`, `inline-table`, and table-internal
    /// keywords that should route through the table formatting context.
    pub fn is_table_family(self) -> bool {
        matches!(
            self,
            CssDisplay::Table
                | CssDisplay::InlineTable
                | CssDisplay::TableRowGroup
                | CssDisplay::TableHeaderGroup
                | CssDisplay::TableFooterGroup
                | CssDisplay::TableRow
                | CssDisplay::TableCell
                | CssDisplay::TableColumnGroup
                | CssDisplay::TableColumn
                | CssDisplay::TableCaption
        )
    }
}

impl Default for CssDisplay {
    /// UA default for elements that don't carry an explicit `display`:
    /// `flow` (block-level block container). This is the CSS Display 3
    /// name for the legacy `block` behaviour.
    fn default() -> Self {
        CssDisplay::Flow
    }
}
