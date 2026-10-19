use std::sync::Arc;

use super::length::{FontContext, Length, LengthContext, Viewport};

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
        matches!(
            self,
            FlexDirection::RowReverse | FlexDirection::ColumnReverse
        )
    }
}

#[doc(hidden)]
/// Parse a `flex-direction` keyword.
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
/// Parse a `flex-wrap` keyword.
pub fn parse_flex_wrap(s: &str) -> Option<FlexWrap> {
    match s.trim().to_ascii_lowercase().as_str() {
        "nowrap" => Some(FlexWrap::NoWrap),
        "wrap" => Some(FlexWrap::Wrap),
        "wrap-reverse" => Some(FlexWrap::WrapReverse),
        _ => None,
    }
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
/// Parse a `justify-content` keyword.
pub fn parse_justify_content(s: &str) -> Option<JustifyContent> {
    match s.trim().to_ascii_lowercase().as_str() {
        // `left` / `right` are valid justify-content keywords; the engine is
        // LTR-only, so `left` = `flex-start`, `right` = `flex-end`.
        "flex-start" | "start" | "left" => Some(JustifyContent::FlexStart),
        "flex-end" | "end" | "right" => Some(JustifyContent::FlexEnd),
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
/// Parse an `align-items` keyword.
pub fn parse_align_items(s: &str) -> Option<AlignItems> {
    match s.trim().to_ascii_lowercase().as_str() {
        "stretch" => Some(AlignItems::Stretch),
        // `self-start` / `self-end` resolve against the writing mode; the
        // engine is LTR-only, so they match the flex-start / flex-end side.
        "flex-start" | "start" | "self-start" => Some(AlignItems::FlexStart),
        "flex-end" | "end" | "self-end" => Some(AlignItems::FlexEnd),
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
/// Parse an `align-content` keyword.
pub fn parse_align_content(s: &str) -> Option<AlignContent> {
    match s.trim().to_ascii_lowercase().as_str() {
        "stretch" => Some(AlignContent::Stretch),
        // `left` / `right` map to the flex-start / flex-end side in LTR.
        "flex-start" | "start" | "left" => Some(AlignContent::FlexStart),
        "flex-end" | "end" | "right" => Some(AlignContent::FlexEnd),
        "center" => Some(AlignContent::Center),
        "space-between" => Some(AlignContent::SpaceBetween),
        "space-around" => Some(AlignContent::SpaceAround),
        "space-evenly" => Some(AlignContent::SpaceEvenly),
        _ => None,
    }
}

/// Whether `name` is one of the `place-*` box-alignment shorthands.
pub(crate) fn is_place_shorthand_name(name: &str) -> bool {
    matches!(name, "place-content" | "place-items" | "place-self")
}

/// Expand a `place-*` box-alignment shorthand declaration into its longhand
/// components at declaration-parse time.
///
/// Browsers cascade shorthands as their expanded longhands, so the *later*
/// declaration wins regardless of shorthand / longhand form — expanding here
/// (instead of falling back in the `CssAttr` getters) gives the cascade that
/// ordering for free. Returns `None` when the value fails to parse (the whole
/// declaration is then dropped, like a browser parse error).
///
/// Component rules:
/// - `place-content`: the first token sets `align-content`; a second token
///   (or the first, when it is also valid there — `stretch` is not) sets
///   `justify-content`.
/// - `place-items` / `place-self`: the first token sets `align-items` /
///   `align-self`. The second (`justify-items` / `justify-self`) is
///   syntax-checked only — those properties have no layout effect in this
///   engine and are not emitted.
pub(crate) fn expand_place_shorthand(
    name: &str,
    value: &str,
) -> Option<Vec<(&'static str, Arc<str>)>> {
    let mut tokens = value.split_whitespace();
    let first = tokens.next()?;
    let second = tokens.next();
    if tokens.next().is_some() {
        return None;
    }
    match name {
        "place-content" => {
            parse_align_content(first)?;
            let justify_token = match second {
                Some(s) => {
                    parse_justify_content(s)?;
                    Some(s)
                }
                // A single value doubles as `justify-content` when it is also
                // valid there; `stretch` is not, so `place-content: stretch`
                // only sets `align-content`.
                None => parse_justify_content(first).map(|_| first),
            };
            let mut out = vec![("align-content", Arc::from(first))];
            if let Some(token) = justify_token {
                out.push(("justify-content", Arc::from(token)));
            }
            Some(out)
        }
        "place-items" => {
            parse_align_items(first)?;
            if let Some(s) = second {
                parse_align_items(s)?;
            }
            Some(vec![("align-items", Arc::from(first))])
        }
        "place-self" => {
            // `align-self` additionally accepts `auto` (defer to the
            // container's `align-items`).
            if parse_align_items(first).is_none() && !first.eq_ignore_ascii_case("auto") {
                return None;
            }
            if let Some(s) = second {
                parse_align_items(s)?;
            }
            Some(vec![("align-self", Arc::from(first))])
        }
        _ => None,
    }
}

/// Whether `name` is one of the alignment longhands the `place-*` shorthands
/// expand into.
pub(crate) fn is_alignment_longhand(name: &str) -> bool {
    matches!(
        name,
        "align-items" | "align-self" | "align-content" | "justify-content"
    )
}

/// Whether `value` parses for the alignment longhand `name`.
///
/// Used at declaration-parse time so an invalid longhand value is dropped
/// (the browser's parse-error behaviour) instead of silently overriding an
/// earlier `place-*` expansion via the per-property cascade.
pub(crate) fn alignment_longhand_valid(name: &str, value: &str) -> bool {
    let value = value.trim();
    match name {
        "align-items" => parse_align_items(value).is_some(),
        "align-self" => value.eq_ignore_ascii_case("auto") || parse_align_items(value).is_some(),
        "align-content" => parse_align_content(value).is_some(),
        "justify-content" => parse_justify_content(value).is_some(),
        _ => true,
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
/// Parse a `flex-basis` value (`auto`, `content`, or a length).
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
/// chain (item's `width` → content size → distribute equally). A percentage
/// basis against an indefinite (`None`) containing main size also resolves
/// to `None` (CSS: percentages against an indefinite axis compute to
/// `auto`).
pub fn resolve_flex_basis(
    basis: FlexBasis,
    containing_main: Option<usize>,
    font_ctx: FontContext,
    viewport: Viewport,
) -> Option<usize> {
    match basis {
        FlexBasis::Auto | FlexBasis::Content => None,
        FlexBasis::Length(Length::Pct(_)) if containing_main.is_none() => None,
        FlexBasis::Length(len) => Some(len.resolve(LengthContext {
            containing: containing_main.unwrap_or(0),
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
/// Parse the `flex` shorthand into grow / shrink / basis.
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
    let zero_basis = FlexBasis::Length(Length::Px(0.0));
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
            if dir.replace(d).is_some() {
                return None;
            }
            continue;
        }
        let w = parse_flex_wrap(tok)?;
        if wrap.replace(w).is_some() {
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
/// Parse the 1- or 2-value `gap` shorthand into row and column sizes.
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
