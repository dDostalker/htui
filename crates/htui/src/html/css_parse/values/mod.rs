mod color;
mod length;

pub use color::parse_color_str;
use length::parse_length_token;
pub use length::{FontContext, Length, LengthContext, PX_PER_CELL, Viewport};
pub(crate) use length::{
    parse_border_side, parse_border_shorthand, parse_border_style, parse_border_width,
    parse_edge_auto, parse_edge_sizes,
};
pub(crate) use length::{merge_border_longhand};

use crate::html::box_edges::Border;

use indexmap::IndexMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// Font weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontWeight {
    Normal,
    Bold,
}

/// Font style (CSS `font-style`).
///
/// Mirrors the CSS values. Terminal output collapses `Italic` and `Oblique`
/// to the same renderer flag, but the distinction is preserved here so the
/// raw CSS decision is not lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontStyleKind {
    Normal,
    Italic,
    Oblique,
}

/// Text decoration (CSS `text-decoration`).
///
/// The CSS property is space-separated and can combine multiple values; we
/// model the subset we render (`underline`, `line-through`) as discrete
/// variants so `==` matches expected outputs without set semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDecoration {
    None,
    Underline,
    LineThrough,
    UnderlineLineThrough,
}

/// Text case transformation (CSS `text-transform`).
///
/// The transform is applied to inline text before wrapping and painting, so
/// width measurement reflects the transformed characters. `capitalize`
/// uppercases the first character of each whitespace-delimited word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextTransform {
    /// `none` — leave the text untouched.
    #[default]
    None,
    /// `uppercase` — map every character to its uppercase form.
    Uppercase,
    /// `lowercase` — map every character to its lowercase form.
    Lowercase,
    /// `capitalize` — uppercase the first letter of each word.
    Capitalize,
}

#[doc(hidden)]
pub fn parse_text_transform(s: &str) -> Option<TextTransform> {
    match s.trim().to_ascii_lowercase().as_str() {
        "none" => Some(TextTransform::None),
        "uppercase" => Some(TextTransform::Uppercase),
        "lowercase" => Some(TextTransform::Lowercase),
        "capitalize" => Some(TextTransform::Capitalize),
        _ => None,
    }
}

/// Line-box height (CSS `line-height`).
///
/// Inherited. Resolved into a concrete row count per visual line by
/// [`resolve_line_height_cells`]; on a terminal grid every value collapses to
/// a whole number of rows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineHeight {
    /// `normal` — one row per font-size cell (the terminal's natural line
    /// height).
    Normal,
    /// A unitless multiplier (CSS `1.5`): `n * font_size_cells` rows.
    Number(f32),
    /// An absolute length (`24px`, `2em`, `150%`). Percentages resolve
    /// against the element's own font-size, mirroring the CSS spec.
    Length(Length),
}

#[doc(hidden)]
pub fn parse_line_height(s: &str) -> Option<LineHeight> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("normal") {
        return Some(LineHeight::Normal);
    }
    // A bare number is a unitless multiplier; everything else (including
    // `0px`) falls through to `Length::parse`.
    if let Ok(n) = s.parse::<f32>() {
        return Some(LineHeight::Number(n));
    }
    Length::parse(s).map(LineHeight::Length)
}

/// Resolve a [`LineHeight`] into the number of canvas rows one visual line
/// occupies, given the element's effective font-size in cells.
///
/// `normal` maps to `font_size_cells`; a number multiplies it; a length
/// resolves like any other CSS length (`%` against the font-size). The result
/// is rounded to a whole row count (min 1) — terminals cannot render partial
/// rows.
pub fn resolve_line_height_cells(
    line_height: LineHeight,
    font_size_cells: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> usize {
    let font = font_size_cells.max(1);
    match line_height {
        LineHeight::Normal => font,
        LineHeight::Number(n) => (n * font as f32).round().max(1.0) as usize,
        LineHeight::Length(len) => match len {
            // `line-height: 150%` resolves against the element's font-size,
            // not the containing block — handle the percentage here instead
            // of routing through `Length::resolve`.
            Length::Pct(p) => ((p / 100.0) * font as f32).round().max(1.0) as usize,
            other => other
                .resolve(LengthContext {
                    containing: 0,
                    font: font_ctx,
                    viewport,
                })
                .max(1),
        },
    }
}

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
pub fn parse_visibility(s: &str) -> Option<Visibility> {
    match s.trim().to_ascii_lowercase().as_str() {
        "visible" => Some(Visibility::Visible),
        "hidden" | "collapse" => Some(Visibility::Hidden),
        _ => None,
    }
}

/// CSS `top` / `right` / `bottom` / `left`, resolved to cells.
///
/// Each side is `None` when the property is unset / `auto`, in which case
/// the positioning pass keeps the box's static position on that axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InsetSides {
    pub top: Option<usize>,
    pub right: Option<usize>,
    pub bottom: Option<usize>,
    pub left: Option<usize>,
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
pub(super) fn parse_border_radius(
    decls: &IndexMap<String, String>,
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
                parse_length_token(Some(raw.as_str()), containing_width, font_ctx, viewport)
        {
            *slot = len;
        }
    }
    radii
}

// ============================================================
// Text-wrapping property values (CSS Text Level 3 subset)
//
// Parsers and enums for `white-space`, `word-break`, and
// `overflow-wrap`. These properties do NOT inherit per the CSS
// spec, so they live on `CssAttr` lookup, not in `INHERITED_PROPS`.
// The renderer honours the defaults that browsers ship:
//   - white-space: normal   (collapse ws + wrap)
//   - word-break:  normal   (allow CJK character breaks)
//   - overflow-wrap: normal (no mid-word breaks unless the word
//     itself doesn't fit on a single line)
// ============================================================

/// Whitespace handling (CSS `white-space`).
///
/// Mirrors the CSS Text Module Level 3 grammar. The two orthogonal
/// axes are:
///
/// - **collapsing** — whether runs of whitespace are folded into a
///   single space (`normal` / `nowrap` / `pre-line`) or preserved
///   verbatim (`pre` / `pre-wrap`).
/// - **wrapping** — whether soft-wrap opportunities exist
///   (`normal` / `pre-wrap` / `pre-line`) or the text is forced to
///   a single line (`nowrap` / `pre`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WhiteSpace {
    /// `normal` — collapse whitespace, wrap at soft-wrap opportunities.
    #[default]
    Normal,
    /// `pre` — preserve whitespace and newlines verbatim, no wrapping.
    Pre,
    /// `nowrap` — collapse whitespace but never wrap.
    NoWrap,
    /// `pre-wrap` — preserve whitespace and newlines, allow wrapping.
    PreWrap,
    /// `pre-line` — collapse spaces / tabs but preserve newlines, allow
    /// wrapping.
    PreLine,
}

impl WhiteSpace {
    /// `true` when the configured mode preserves whitespace runs and
    /// newlines verbatim (instead of collapsing them).
    pub const fn preserves_whitespace(self) -> bool {
        matches!(self, WhiteSpace::Pre | WhiteSpace::PreWrap)
    }

    /// `true` when the configured mode allows wrapping at soft-wrap
    /// opportunities (spaces, CJK character boundaries).
    pub const fn allows_wrap(self) -> bool {
        matches!(
            self,
            WhiteSpace::Normal | WhiteSpace::PreWrap | WhiteSpace::PreLine
        )
    }
}

#[doc(hidden)]
pub fn parse_white_space(s: &str) -> Option<WhiteSpace> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(WhiteSpace::Normal),
        "pre" => Some(WhiteSpace::Pre),
        "nowrap" => Some(WhiteSpace::NoWrap),
        "pre-wrap" => Some(WhiteSpace::PreWrap),
        "pre-line" => Some(WhiteSpace::PreLine),
        _ => None,
    }
}

/// Word-break behaviour for CJK / long-word overflow (CSS `word-break`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WordBreak {
    /// `normal` — break at word boundaries; between CJK characters the
    /// breaker may also break (mirrors browser default).
    #[default]
    Normal,
    /// `break-all` — break between any two adjacent characters.
    BreakAll,
    /// `keep-all` — for CJK text, do not break between CJK characters;
    /// breaks only happen at spaces / western word boundaries.
    KeepAll,
}

#[doc(hidden)]
pub fn parse_word_break(s: &str) -> Option<WordBreak> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(WordBreak::Normal),
        "break-all" => Some(WordBreak::BreakAll),
        "keep-all" => Some(WordBreak::KeepAll),
        _ => None,
    }
}

/// Overflow-wrap behaviour (CSS `overflow-wrap`, formerly `word-wrap`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverflowWrap {
    /// `normal` — break only at normal break opportunities; a too-long
    /// word overflows.
    #[default]
    Normal,
    /// `break-word` — if no normal break opportunity exists on a line,
    /// the unbreakable word may be broken to fit.
    BreakWord,
    /// `anywhere` — same as `break-word`, but soft-wrap opportunities
    /// are considered even between every pair of characters.
    Anywhere,
}

#[doc(hidden)]
pub fn parse_overflow_wrap(s: &str) -> Option<OverflowWrap> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(OverflowWrap::Normal),
        "break-word" => Some(OverflowWrap::BreakWord),
        "anywhere" => Some(OverflowWrap::Anywhere),
        _ => None,
    }
}

// ============================================================
// Flexbox property values
//
// Parsers and enums for the CSS flex properties supported by the
// renderer. Kept in `values.rs` alongside the other typed enums
// because they're plain value-types with no DOM / layout coupling.
// The flex layout itself lives in `layout::flex::flex`.
// ============================================================

/// Main axis direction (CSS `flex-direction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    /// Items laid out left-to-right along the main axis.
    Row,
    /// Items laid out right-to-left along the main axis (main-start is the
    /// right edge).
    RowReverse,
    /// Items laid out top-to-bottom along the main axis.
    Column,
    /// Items laid out bottom-to-top along the main axis (main-start is the
    /// bottom edge).
    ColumnReverse,
}

impl FlexDirection {
    /// Whether the main axis is horizontal (`row` / `row-reverse`).
    pub fn is_row(&self) -> bool {
        matches!(self, FlexDirection::Row | FlexDirection::RowReverse)
    }
    /// Whether the main axis is vertical (`column` / `column-reverse`).
    pub fn is_column(&self) -> bool {
        matches!(self, FlexDirection::Column | FlexDirection::ColumnReverse)
    }
    /// Whether the main axis runs backwards (`row-reverse` / `column-reverse`).
    pub fn is_reverse(&self) -> bool {
        matches!(self, FlexDirection::RowReverse | FlexDirection::ColumnReverse)
    }
}

#[doc(hidden)]
pub fn parse_flex_direction(s: &str) -> Option<FlexDirection> {
    match s.trim().to_ascii_lowercase().as_str() {
        "row" => Some(FlexDirection::Row),
        "row-reverse" => Some(FlexDirection::RowReverse),
        "column" => Some(FlexDirection::Column),
        "column-reverse" => Some(FlexDirection::ColumnReverse),
        _ => None,
    }
}

/// Wrapping behaviour (CSS `flex-wrap`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexWrap {
    /// All items on a single line; items may shrink to fit.
    NoWrap,
    /// Items wrap onto additional lines when the main axis overflows.
    Wrap,
    /// Like `wrap`, but the cross axis is reversed: lines stack from the
    /// cross-end (bottom for rows, right for columns).
    WrapReverse,
}

#[doc(hidden)]
pub fn parse_flex_wrap(s: &str) -> Option<FlexWrap> {
    match s.trim().to_ascii_lowercase().as_str() {
        "nowrap" => Some(FlexWrap::NoWrap),
        "wrap" => Some(FlexWrap::Wrap),
        "wrap-reverse" => Some(FlexWrap::WrapReverse),
        _ => None,
    }
}

/// CSS `column-fill` value (Multi-column Layout).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ColumnFill {
    /// `balance` (default) — columns get an equal share of the content's
    /// height.
    #[default]
    Balance,
    /// `auto` — columns are filled in order; later columns hold whatever is
    /// left over.
    Auto,
}

/// Parse a `column-fill` keyword.
#[doc(hidden)]
pub fn parse_column_fill(s: &str) -> Option<ColumnFill> {
    match s.trim().to_ascii_lowercase().as_str() {
        "balance" | "balance-all" => Some(ColumnFill::Balance),
        "auto" => Some(ColumnFill::Auto),
        _ => None,
    }
}

/// Parse a `column-count` integer (1..=10000).
#[doc(hidden)]
pub fn parse_column_count(s: &str) -> Option<usize> {
    let n = s.trim().parse::<usize>().ok()?;
    (1..=10000).contains(&n).then_some(n)
}

/// The `columns` shorthand components: `<'column-width'> || <'column-count'>`
/// in any order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnsShorthand {
    /// The `column-width` component (`None` when absent).
    pub width: Option<Length>,
    /// The `column-count` component (`None` when absent).
    pub count: Option<usize>,
}

/// Parse the `columns` shorthand (`12em 3` / `3` / `12em`). Each token is
/// classified as either an integer (count) or a length (width); a duplicate
/// or unrecognised token rejects the whole declaration.
#[doc(hidden)]
pub fn parse_columns_shorthand(s: &str) -> Option<ColumnsShorthand> {
    let mut width: Option<Length> = None;
    let mut count: Option<usize> = None;
    for token in s.split_whitespace() {
        if let Some(c) = parse_column_count(token) {
            if count.is_some() {
                return None;
            }
            count = Some(c);
            continue;
        }
        let l = Length::parse(token)?;
        if width.is_some() {
            return None;
        }
        width = Some(l);
    }
    (width.is_some() || count.is_some()).then_some(ColumnsShorthand { width, count })
}

/// Parse a `column-rule` shorthand (`1px solid red`, `solid red`, …) into a
/// [`Border`]. Reuses the border token classifier so widths / styles / colors
/// may appear in any order. Returns `None` when nothing recognizable is
/// present.
#[doc(hidden)]
pub fn parse_column_rule(
    s: &str,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> Option<Border> {
    parse_border_shorthand(s, containing_width, font_ctx, viewport)
}

/// Fully resolved multi-column layout.
///
/// Produced by [`CssAttr::get_columns_resolved`](crate::html::css_parse::CssAttr)
/// and consumed by the block layout to fragment a box's content into
/// side-by-side columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnsResolved {
    /// Number of columns (`>= 2` — single-column layouts never resolve here).
    pub count: usize,
    /// Content width of one column in cells.
    pub col_width: usize,
    /// Gap between adjacent columns in cells.
    pub gap: usize,
    /// `column-fill` strategy (`balance` by default).
    pub fill: ColumnFill,
    /// `column-rule` (visible when its style is not `none`).
    pub rule: Option<Border>,
}

/// Main-axis distribution (CSS `justify-content`).
///
/// `SpaceAround` / `SpaceEvenly` are distinct because they differ in how the
/// leading / trailing edges are spaced — merging them would lose the visual
/// difference. The `align-content` property reuses this same enum because
/// it has the same value grammar in CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JustifyContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[doc(hidden)]
pub fn parse_justify_content(s: &str) -> Option<JustifyContent> {
    match s.trim().to_ascii_lowercase().as_str() {
        "flex-start" | "start" => Some(JustifyContent::FlexStart),
        "flex-end" | "end" => Some(JustifyContent::FlexEnd),
        "center" => Some(JustifyContent::Center),
        "space-between" => Some(JustifyContent::SpaceBetween),
        "space-around" => Some(JustifyContent::SpaceAround),
        "space-evenly" => Some(JustifyContent::SpaceEvenly),
        _ => None,
    }
}

/// Cross-axis alignment (CSS `align-items` and `align-self`).
///
/// The default `Stretch` only takes effect when the container has a known
/// cross-axis size (e.g. an explicit `height` for row direction). `Baseline`
/// aligns each item's first text row to the line's shared baseline (terminal
/// approximation of the CSS baseline).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignItems {
    Stretch,
    FlexStart,
    FlexEnd,
    Center,
    /// Align the items' first text rows (the terminal cell-grid analogue of
    /// the CSS text baseline).
    Baseline,
}

#[doc(hidden)]
pub fn parse_align_items(s: &str) -> Option<AlignItems> {
    match s.trim().to_ascii_lowercase().as_str() {
        "stretch" => Some(AlignItems::Stretch),
        "flex-start" | "start" => Some(AlignItems::FlexStart),
        "flex-end" | "end" => Some(AlignItems::FlexEnd),
        "center" => Some(AlignItems::Center),
        "baseline" | "first baseline" => Some(AlignItems::Baseline),
        _ => None,
    }
}

/// Cross-axis line distribution (CSS `align-content`).
///
/// Same value grammar as `justify-content` plus `stretch`, which grows the
/// lines to fill the container's free cross-axis space (implemented as an
/// even `free / line_count` gap between the stacked lines).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AlignContent {
    /// `stretch` — the CSS initial value: lines fill the free cross space.
    #[default]
    Stretch,
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[doc(hidden)]
pub fn parse_align_content(s: &str) -> Option<AlignContent> {
    match s.trim().to_ascii_lowercase().as_str() {
        "stretch" => Some(AlignContent::Stretch),
        "flex-start" | "start" => Some(AlignContent::FlexStart),
        "flex-end" | "end" => Some(AlignContent::FlexEnd),
        "center" => Some(AlignContent::Center),
        "space-between" => Some(AlignContent::SpaceBetween),
        "space-around" => Some(AlignContent::SpaceAround),
        "space-evenly" => Some(AlignContent::SpaceEvenly),
        _ => None,
    }
}

/// Resolved `flex-basis` value.
///
/// `Auto` defers to the item's `width` (for row direction) or `height`
/// (for column direction), then to its measured content size. The variants
/// mirror [`Length`] but keep the `auto` case separate so a missing
/// declaration and an explicit `auto` share a single value type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FlexBasis {
    Auto,
    /// `content` — the item's intrinsic content size (ignores `width`).
    /// Resolved like `auto` (the fallback chain measures the content).
    Content,
    Length(Length),
}

#[doc(hidden)]
pub fn parse_flex_basis(s: &str) -> Option<FlexBasis> {
    let trimmed = s.trim();
    if trimmed.eq_ignore_ascii_case("auto") {
        return Some(FlexBasis::Auto);
    }
    if trimmed.eq_ignore_ascii_case("content") {
        return Some(FlexBasis::Content);
    }
    Length::parse(trimmed).map(FlexBasis::Length)
}

/// Resolve a `flex-basis` against the containing main-axis size and the
/// active font-size context.
///
/// `Auto` / `Content` return `None` so callers can apply their own fallback
/// chain (item's `width` → content size → distribute equally).
pub fn resolve_flex_basis(
    basis: FlexBasis,
    containing_main: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> Option<usize> {
    match basis {
        FlexBasis::Auto | FlexBasis::Content => None,
        FlexBasis::Length(len) => Some(len.resolve(LengthContext {
            containing: containing_main,
            font: font_ctx,
            viewport,
        })),
    }
}

/// Parsed `flex` shorthand: grow / shrink / basis.
///
/// CSS `flex: none` → `0 0 auto`; a lone `<flex-grow>` / `<flex-grow>
/// <flex-shrink>` imply a `0` basis (the item sizes purely from growth); a
/// lone `<flex-basis>` implies `1 1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlexShorthand {
    pub grow: f32,
    pub shrink: f32,
    pub basis: FlexBasis,
}

#[doc(hidden)]
pub fn parse_flex_shorthand(s: &str) -> Option<FlexShorthand> {
    let trimmed = s.trim();
    if trimmed.eq_ignore_ascii_case("none") {
        return Some(FlexShorthand {
            grow: 0.0,
            shrink: 0.0,
            basis: FlexBasis::Auto,
        });
    }
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.is_empty() || tokens.len() > 3 {
        return None;
    }
    let zero_basis = FlexBasis::Length(Length::Cells(0.0));
    match tokens.as_slice() {
        [t] => {
            if let Some(g) = parse_non_negative_number(t) {
                // `flex: <grow>` → basis 0 (size purely from growth).
                Some(FlexShorthand {
                    grow: g,
                    shrink: 1.0,
                    basis: zero_basis,
                })
            } else {
                // `flex: <basis>` → grow / shrink 1.
                Some(FlexShorthand {
                    grow: 1.0,
                    shrink: 1.0,
                    basis: parse_flex_basis(t)?,
                })
            }
        }
        [a, b] => {
            let na = parse_non_negative_number(a);
            let nb = parse_non_negative_number(b);
            match (na, nb) {
                (Some(g), Some(sh)) => Some(FlexShorthand {
                    grow: g,
                    shrink: sh,
                    basis: zero_basis,
                }),
                (Some(g), None) => Some(FlexShorthand {
                    grow: g,
                    shrink: 1.0,
                    basis: parse_flex_basis(b)?,
                }),
                (None, Some(sh)) => Some(FlexShorthand {
                    grow: 1.0,
                    shrink: sh,
                    basis: parse_flex_basis(a)?,
                }),
                (None, None) => None,
            }
        }
        [g, s, b] => Some(FlexShorthand {
            grow: parse_non_negative_number(g)?,
            shrink: parse_non_negative_number(s)?,
            basis: parse_flex_basis(b)?,
        }),
        _ => None,
    }
}

/// Parse the `flex-flow` shorthand (`<flex-direction> <flex-wrap>` in any
/// order; either part may be omitted). Returns the direction and the wrap
/// when present.
#[doc(hidden)]
pub fn parse_flex_flow(s: &str) -> Option<(FlexDirection, Option<FlexWrap>)> {
    let mut dir: Option<FlexDirection> = None;
    let mut wrap: Option<FlexWrap> = None;
    for tok in s.split_whitespace() {
        if let Some(d) = parse_flex_direction(tok) {
            if dir.is_some() {
                return None;
            }
            dir = Some(d);
        } else if let Some(w) = parse_flex_wrap(tok) {
            if wrap.is_some() {
                return None;
            }
            wrap = Some(w);
        } else {
            return None;
        }
    }
    Some((dir.unwrap_or(FlexDirection::Row), wrap))
}

/// Parse a non-negative number used by `flex-grow` / `flex-shrink` / `order`.
///
/// Accepts integers (`0`, `1`, `2`) and decimals (`0.5`, `1.25`). Negative
/// values and non-numeric input return `None` so the parser surfaces
/// malformed rules rather than silently defaulting.
#[doc(hidden)]
pub fn parse_non_negative_number(s: &str) -> Option<f32> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    let v: f32 = trimmed.parse().ok()?;
    if v < 0.0 || !v.is_finite() {
        return None;
    }
    Some(v)
}

/// Parse a signed integer used by `order`.
///
/// `order` accepts any integer (positive moves later, negative earlier).
#[doc(hidden)]
pub fn parse_order(s: &str) -> Option<i32> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    let v: i32 = trimmed.parse().ok()?;
    Some(v)
}

/// Row / column gap sizes (CSS `gap` / `row-gap` / `column-gap`).
///
/// `gap: <length>` applies the same value to both axes; `gap: <row>
/// <column>` is two values. `row-gap` / `column-gap` longhands override
/// the corresponding shorthand component.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GapSizes {
    /// Gap between lines (rows for `flex-direction: row`, columns for column).
    pub row: usize,
    /// Gap between items on the same line.
    pub column: usize,
}

#[doc(hidden)]
pub fn parse_gap_shorthand(s: &str, font_ctx: FontContext, viewport: Viewport) -> Option<GapSizes> {
    // 1- or 2-value gap grammar. Each value is a length; unparseable
    // tokens become 0 so a single bad component doesn't kill the rule.
    // `containing` is fixed to `usize::MAX` because the flex container's
    // actual axis size is not in scope here — `vw` / `vh` still resolve
    // correctly via the viewport baked into `font_ctx`. Percentage gap is
    // intentionally broken (see doc/layout.md "百分比 gap").
    let mut parts = s.split_whitespace();
    let first = parts.next()?;
    let first_len = Length::parse(first);
    let Some(second) = parts.next() else {
        let v = first_len?;
        return Some(GapSizes {
            row: v.resolve(LengthContext {
                containing: usize::MAX,
                font: font_ctx,
                viewport,
            }),
            column: v.resolve(LengthContext {
                containing: usize::MAX,
                font: font_ctx,
                viewport,
            }),
        });
    };
    // CSS allows more than two values for row-gap-style properties, but the
    // visual test corpus never uses them. Reject any trailing token so we
    // never silently mis-parse.
    if parts.next().is_some() {
        return None;
    }
    let second_len = Length::parse(second);
    let row = first_len?;
    let column = second_len?;
    Some(GapSizes {
        row: row.resolve(LengthContext {
            containing: usize::MAX,
            font: font_ctx,
            viewport,
        }),
        column: column.resolve(LengthContext {
            containing: usize::MAX,
            font: font_ctx,
            viewport,
        }),
    })
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
/// [`CssDisplay::outer_display`] and [`CssDisplay::inner_formatting_context`]
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
    /// `display: grid` — block-level box that would establish a grid
    /// formatting context. The grid algorithm is reserved; generation
    /// routes these to the geometry dispatcher which currently logs a
    /// debug message and returns `None`.
    Grid,
    /// `display: inline-grid` — inline-level box that would establish a
    /// grid formatting context. Same caveat as [`CssDisplay::Grid`].
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
    /// [`layout_a`](crate::layout::layoutbox::inner_formatting_context::flow::a::layout_a)
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
