use std::str::FromStr;

use crossterm::style::Color;
use cssparser::{Parser as CssParser, ParserInput, Token};
use indexmap::IndexMap;

use crate::html::box_edges::{Border, BorderStyle, EdgeSizes};

/// Number of CSS px represented by one terminal cell on each axis.
/// CSS defines 1px as 1/96 inch; terminals cannot query physical DPI,
/// so we anchor px to the cell grid (1px == 1 cell column or row). This
/// makes every parsed px an integer cell count without rounding. The
/// constant is named so future changes (e.g. a 2x DPI mode) have one
/// edit point. This constant only governs CSS px, not the numeric
/// pixels on img HTML attributes, which describe source-image pixels
/// and are sampled at the ratios defined in tui::graph_api::image
/// (IMAGE_PIXELS_PER_CELL_COL / IMAGE_PIXELS_PER_CELL_ROW).
pub const PX_PER_CELL: f32 = 1.0;

/// Parse a CSS color string into a [`crossterm::style::Color`].
///
/// Supported formats:
/// - Named colors including `dark` variants: `red`, `darkblue`, `DarkBlue`, ...
/// - Hexadecimal: `#RRGGBB`, `#RGB`
/// - Functional: `rgb(255, 0, 0)`
/// - Fallback to `crossterm`'s built-in `Color::from_str`.
#[doc(hidden)]
pub fn parse_color_str(s: &str) -> Option<Color> {
    let lower = s.trim().to_lowercase();

    // 1. Hexadecimal #RRGGBB or #RGB
    if let Some(hex) = lower.strip_prefix('#') {
        return parse_hex_color(hex);
    }

    // 2. Functional rgb(r, g, b)
    if let Some(inner) = lower.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        return parse_rgb_color(inner);
    }

    // 3. Extended named colors including `dark` variants
    if let Some(color) = parse_named_color(&lower) {
        return Some(color);
    }

    // 4. Fall back to crossterm's built-in parser.
    Color::from_str(s).ok()
}

fn parse_hex_color(hex: &str) -> Option<Color> {
    let hex = hex.trim();
    let (r, g, b) = if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        (r, g, b)
    } else if hex.len() == 3 {
        let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
        let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
        let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
        (r, g, b)
    } else {
        return None;
    };
    Some(Color::Rgb { r, g, b })
}

fn parse_rgb_color(inner: &str) -> Option<Color> {
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() != 3 {
        return None;
    }
    let r = parts[0].trim().parse::<u8>().ok()?;
    let g = parts[1].trim().parse::<u8>().ok()?;
    let b = parts[2].trim().parse::<u8>().ok()?;
    Some(Color::Rgb { r, g, b })
}

fn parse_named_color(name: &str) -> Option<Color> {
    match name {
        "black" => Some(Color::Black),
        "darkgrey" | "darkgray" | "grey" | "gray" => Some(Color::Grey),
        "red" => Some(Color::Red),
        "darkred" => Some(Color::DarkRed),
        "green" => Some(Color::Green),
        "darkgreen" => Some(Color::DarkGreen),
        "yellow" => Some(Color::Yellow),
        "darkyellow" => Some(Color::DarkYellow),
        "blue" => Some(Color::Blue),
        "darkblue" => Some(Color::DarkBlue),
        "magenta" => Some(Color::Magenta),
        "darkmagenta" => Some(Color::DarkMagenta),
        "cyan" => Some(Color::Cyan),
        "darkcyan" => Some(Color::DarkCyan),
        "white" => Some(Color::White),
        _ => None,
    }
}

// ============================================================
// Box-model shorthand parsing
// ============================================================

/// Expand `padding` / `padding-{side}` (or `margin` / `margin-{side}`) into an
/// [`EdgeSizes`].
///
/// Each value is parsed by [`Length::parse`]; unparseable tokens become `0`.
/// Per-side longhands are layered on top of the shorthand so `padding: 0;
/// padding-top: 5px;` resolves to top=5, others=0.
pub(super) fn parse_edge_sizes(
    decls: &IndexMap<String, String>,
    shorthand: &str,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> EdgeSizes {
    let shorthand_raw = decls.get(shorthand);
    let mut edges = parse_shorthand_lengths(
        shorthand_raw.map(|s| s.as_str()),
        containing_width,
        font_ctx,
        viewport,
    );

    // Layer per-side longhands on top (they take precedence).
    let top_raw = decls.get(&format!("{shorthand}-top")).map(|s| s.as_str());
    if let Some(v) = parse_length_token(top_raw, containing_width, font_ctx, viewport) {
        edges.top = v;
    }
    let right_raw = decls.get(&format!("{shorthand}-right")).map(|s| s.as_str());
    if let Some(v) = parse_length_token(right_raw, containing_width, font_ctx, viewport) {
        edges.right = v;
    }
    let bottom_raw = decls
        .get(&format!("{shorthand}-bottom"))
        .map(|s| s.as_str());
    if let Some(v) = parse_length_token(bottom_raw, containing_width, font_ctx, viewport) {
        edges.bottom = v;
    }
    let left_raw = decls.get(&format!("{shorthand}-left")).map(|s| s.as_str());
    if let Some(v) = parse_length_token(left_raw, containing_width, font_ctx, viewport) {
        edges.left = v;
    }

    edges
}

/// Expand a 1-/2-/3-/4-value CSS shorthand into [`EdgeSizes`].
///
/// Tokenisation is whitespace-driven — we split on runs of whitespace and try
/// to parse each chunk as a [`Length`]. Unparseable tokens are skipped (treated
/// as `0`) so a stray `auto` or junk doesn't poison the whole rule.
fn parse_shorthand_lengths(
    value: Option<&str>,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> EdgeSizes {
    let Some(value) = value else {
        return EdgeSizes::default();
    };
    let tokens: Vec<&str> = value.split_whitespace().collect();
    let parsed: Vec<usize> = tokens
        .iter()
        .filter_map(|t| parse_length_token(Some(t), containing_width, font_ctx, viewport))
        .collect();
    match parsed.len() {
        0 => EdgeSizes::default(),
        1 => EdgeSizes::new(parsed[0], parsed[0], parsed[0], parsed[0]),
        2 => EdgeSizes::new(parsed[0], parsed[1], parsed[0], parsed[1]),
        3 => EdgeSizes::new(parsed[0], parsed[1], parsed[2], parsed[1]),
        _ => EdgeSizes::new(parsed[0], parsed[1], parsed[2], parsed[3]),
    }
}

fn resolve_length(length: Length, containing: usize, font: FontContext, viewport: Viewport) -> usize {
    length.resolve(LengthContext {
        containing,
        font,
        viewport,
    })
}
///
/// `auto` / missing / unsupported unit → `None` (caller decides what default
/// to apply).
fn parse_length_token(
    value: Option<&str>,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> Option<usize> {
    let raw = value?;
    match Length::parse(raw)? {
        Length::Auto => None,
        len => Some(resolve_length(len, containing_width, font_ctx, viewport)),
    }
}

/// Parse a `border-width` value to cells. Mirrors [`parse_length_token`]; any
/// value CSS accepts as a length works (including `thin` / `medium` / `thick`
/// once added to [`Length::parse`]).
pub(super) fn parse_border_width(
    value: Option<&str>,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> usize {
    parse_length_token(value, containing_width, font_ctx, viewport).unwrap_or(0)
}

/// Map a `border-style` keyword to a [`BorderStyle`]. Unknown / missing
/// defaults to [`BorderStyle::None`].
pub(super) fn parse_border_style(value: Option<&str>) -> BorderStyle {
    let Some(value) = value else {
        return BorderStyle::None;
    };
    BorderStyle::parse(value)
}

/// Build one [`Border`] edge from `border-{side}` (priority), falling back to
/// the universal `border` shorthand, finally to the resolved
/// `border-width`/`-style`/`-color` defaults.
///
/// The `fallback` argument already encodes the universal defaults resolved by
/// the caller — passing it down means we don't re-parse the same map entries
/// four times.
pub(super) fn parse_border_side(
    side_raw: Option<&String>,
    shorthand_raw: Option<&String>,
    fallback: Border,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> Border {
    if let Some(value) = side_raw {
        if let Some(parsed) = parse_border_shorthand(value, containing_width, font_ctx, viewport) {
            return parsed;
        }
        // `border-top` was present but unparseable — fall through to next
        // priority rather than dropping the side entirely.
    }
    if let Some(value) = shorthand_raw {
        if let Some(parsed) = parse_border_shorthand(value, containing_width, font_ctx, viewport) {
            return parsed;
        }
    }
    fallback
}

/// Parse a `border` shorthand declaration.
///
/// CSS allows any of width / style / color in any order, with some pieces
/// optional. We tokenise on whitespace and classify each token; tokens we
/// can't classify fall through (treated as missing) so a typo in one piece
/// doesn't drop the whole border.
fn parse_border_shorthand(
    value: &str,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> Option<Border> {
    let mut width: Option<usize> = None;
    let mut style: Option<BorderStyle> = None;
    let mut color: Option<Color> = None;

    for token in value.split_whitespace() {
        if let Some(w) = parse_length_token(Some(token), containing_width, font_ctx, viewport) {
            width = Some(w);
            continue;
        }
        let lower = token.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "solid"
                | "dashed"
                | "dotted"
                | "double"
                | "none"
                | "hidden"
                | "groove"
                | "ridge"
                | "inset"
                | "outset"
        ) {
            style = Some(BorderStyle::parse(token));
            continue;
        }
        if let Some(c) = parse_color_str(token) {
            color = Some(c);
            continue;
        }
        // Unrecognised token — ignore (don't fail the whole rule).
    }

    // At least width or style must be present; otherwise the value wasn't a
    // border shorthand (e.g. someone wrote `border: inherit;`).
    if width.is_none() && style.is_none() && color.is_none() {
        return None;
    }

    Some(Border {
        width: width.unwrap_or(0),
        style: style.unwrap_or(BorderStyle::None),
        color,
    })
}

/// Font-size context propagated through the layout tree.
///
/// `FontContext` carries the **current element's effective font-size** and
/// the **root font-size** so `em` and `rem` lengths can resolve against the
/// active font-size cascade. It is intentionally viewport-free: viewport
/// information lives in [`Viewport`] and the two are bundled together by
/// [`LengthContext`] / [`crate::layout::LayoutContext`] when handed to
/// [`Length::resolve`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontContext {
    pub font_size_cells: usize,
    pub root_font_size_cells: usize,
}

/// Terminal viewport used by viewport-relative CSS lengths.
///
/// `Viewport` is a peer of [`FontContext`], not a field inside it. `vw` /
/// `vh` always anchor to the terminal columns / rows respectively; carrying
/// the viewport separately makes that explicit at every call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub cols: usize,
    pub rows: usize,
}

/// Inputs required to resolve one CSS length into terminal cells.
///
/// `Length::resolve` consumes a `LengthContext`; the same `Length::resolve`
/// call therefore yields consistent `em` / `rem` / `vw` / `vh` results no
/// matter which CSS property it appears in. `containing` is the
/// containing-block width in cells (used by `Pct`); `font` supplies the
/// em / rem base sizes; `viewport` supplies the `vw` / `vh` anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LengthContext {
    pub containing: usize,
    pub font: FontContext,
    pub viewport: Viewport,
}

impl FontContext {
    pub fn root() -> Self {
        Self {
            font_size_cells: 1,
            root_font_size_cells: 1,
        }
    }

    pub fn child(self, child_cells: usize) -> Self {
        Self {
            font_size_cells: child_cells,
            root_font_size_cells: self.root_font_size_cells,
        }
    }
}

/// A typed CSS length value resolved against the terminal's monospace grid.
///
/// Terminals render in **cells**, not pixels or points. Every length we
/// accept is normalised to an integer column count by [`Length::resolve`].
///
/// Accepted forms:
/// - `auto`                         → [`Length::Auto`]   (caller decides what to fill)
/// - `40px`, bare `40`              → [`Length::Cells(n)`]
/// - `50%`                          → [`Length::Pct(p)`] (raw 0–100, divided at resolve)
/// - `2em`, `1.5em`                 → [`Length::Em(n)`]
/// - `2rem`, `1.5rem`               → [`Length::Rem(n)`]
/// - `50vw`                         → [`Length::Vw(n)`]  (1vw = viewport_cols / 100)
/// - `50vh`                         → [`Length::Vh(n)`]  (1vh = viewport_rows / 100)
///
/// Rejected (returns `None` from [`Length::parse`]):
/// - `vmin`, `vmax` — derive from `vw` / `vh` after `Layout` has viewport info;
///   not yet wired through.
/// - `pt`, `cm`, `mm`, `in`, `pc` — no DPI. Use `px` (= cells).
/// - Negative or signed (`+`, `-`) values — CSS forbids these for `width`.
/// - Trailing junk after the value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    /// `auto` — defer to the containing block.
    Auto,
    /// Absolute CSS pixel length. Converted to terminal cells during resolve.
    Px(f32),
    /// Absolute cell count for bare numeric lengths.
    Cells(f32),
    /// Percentage 0..=100 of the containing block (CSS `50%`).
    Pct(f32),
    /// Em multiplier (CSS `2em`). 1em == 1 cell on the terminal grid.
    Em(f32),
    /// Root-em multiplier (CSS `2rem`). Same semantics as `em` here — the
    /// terminal has no root font-size distinction.
    Rem(f32),
    /// Viewport-width multiplier (CSS `50vw`).
    ///
    /// Resolves to `round(n * viewport_cols / 100)` cells, min 1.
    /// `vw` always anchors to the **viewport columns** (the terminal
    /// width), regardless of which CSS property it appears in — so
    /// `height: 50vw` mixes axes deliberately and is the caller's
    /// choice; `Length::resolve` does not need to know the axis.
    Vw(f32),
    /// Viewport-height multiplier (CSS `50vh`).
    ///
    /// Resolves to `round(n * viewport_rows / 100)` cells, min 1.
    /// Mirrors [`Length::Vw`] on the row axis.
    Vh(f32),
}

/// Owned mirror of the cssparser [`Token`] variants we accept. Lets
/// [`Length::parse`] release its borrow on the parser before checking for
/// trailing junk with `is_exhausted()`.
#[derive(Debug, Clone, Copy)]
enum PreLength {
    Px(f32),
    Cells(f32),
    Pct(f32),
    Em(f32),
    Rem(f32),
    Vw(f32),
    Vh(f32),
}

impl Length {
    /// Parse a CSS length token into a typed [`Length`].
    ///
    /// Returns `None` for unsupported units (`vw`/`vh`/`pt`/`cm`/...), signed
    /// values, trailing garbage, or empty input. The literal `auto` (any case)
    /// always returns [`Length::Auto`].
    pub fn parse(s: &str) -> Option<Length> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("auto") {
            return Some(Length::Auto);
        }
        // cssparser's tokenizer handles `40`, `40.5`, `40px`, `1.5em`, `50%`,
        // escapes, and unicode digits uniformly. We pin the lifetime with a
        // named ParserInput binding — see the warning in cssparser's docs:
        // a temporary `ParserInput::new(...)` would be dropped before the
        // parser finished reading.
        let mut input_buf = ParserInput::new(s);
        let mut input = CssParser::new(&mut input_buf);
        // Convert the borrowed `&Token` into an owned `PreLength` so we can
        // later call `input.is_exhausted()` to check for trailing junk.
        // Keeping the `&Token` alive would borrow `input` mutably and conflict
        // with that second call.
        let pre = match input.next().ok()? {
            Token::Number {
                value,
                int_value,
                has_sign: false,
                ..
            } => PreLength::Cells(int_value.map(|i| i as f32).unwrap_or(*value)),
            Token::Dimension {
                value,
                int_value,
                has_sign: false,
                unit,
                ..
            } => {
                let n = int_value.map(|i| i as f32).unwrap_or(*value);
                match unit.to_ascii_lowercase().as_str() {
                    // CSS px: anchor to the cell grid via PX_PER_CELL. With the
                    // current 1.0 ratio this collapses to the raw value, but the
                    // explicit multiply makes the conversion searchable and gives
                    // a single edit-point should the per-cell px ratio ever change.
                    "px" => PreLength::Px(n),
                    "em" => PreLength::Em(n),
                    "rem" => PreLength::Rem(n),
                    // 1vw = viewport_cols / 100, 1vh = viewport_rows / 100.
                    // The actual viewport size is supplied at `Length::resolve`
                    // time — the parser only carries the raw multiplier.
                    "vw" => PreLength::Vw(n),
                    "vh" => PreLength::Vh(n),
                    // vmin / vmax / pt / cm / mm / in / pc / ...
                    _ => return None,
                }
            }
            Token::Percentage {
                unit_value,
                has_sign: false,
                ..
            } => {
                // cssparser divides by 100 for us (`50%` → `0.5`). Multiply back
                // so the stored value matches what the user wrote.
                let percent = unit_value * 100.0;
                let nearest_integer = percent.round();
                let percent = if (percent - nearest_integer).abs() < 0.0001 {
                    nearest_integer
                } else {
                    percent
                };
                PreLength::Pct(percent)
            }
            _ => return None,
        };
        // Reject any non-whitespace junk after the value token.
        if !input.is_exhausted() {
            return None;
        }
        Some(match pre {
            PreLength::Px(n) => Length::Px(n),
            PreLength::Cells(n) => Length::Cells(n),
            PreLength::Pct(p) => Length::Pct(p),
            PreLength::Em(n) => Length::Em(n),
            PreLength::Rem(n) => Length::Rem(n),
            PreLength::Vw(n) => Length::Vw(n),
            PreLength::Vh(n) => Length::Vh(n),
        })
    }

    /// Resolve the length into a concrete column count given the containing
    /// block's width in cells and the active length-resolution context.
    ///
    /// - `Auto`              → `containing` (fill the container)
    /// - `Cells(n)`          → `n`
    /// - `Pct(p)`            → round(containing * p / 100), min 1
    /// - `Em(n)`             → round(n * context.font.font_size_cells), clamped at 0
    /// - `Rem(n)`            → round(n * context.font.root_font_size_cells), clamped at 0
    /// - `Vw(n)`             → round(n * context.viewport.cols / 100), min 1
    /// - `Vh(n)`             → round(n * context.viewport.rows / 100), min 1
    ///
    /// `em` resolves against the **current element's** effective font-size
    /// (in cells); `rem` resolves against the **root's** font-size. On a
    /// terminal the root font-size is 1 cell by default — it grows when the
    /// document root sets an explicit `font-size` rule.
    ///
    /// `vw` / `vh` anchor to the terminal viewport (columns / rows), not to
    /// the containing block. They are axis-independent: `vw` always uses
    /// `viewport_cols`, `vh` always uses `viewport_rows`, regardless of
    /// which CSS property they appear in.
    ///
    /// Floats round **away from zero** (Rust's `f32::round`), so `0.5em` at
    /// font-size=2 yields 1 cell and `2.5em` at font-size=2 yields 5 cells —
    /// never zero, never half.
    pub fn resolve(&self, context: LengthContext) -> usize {
        match self {
            Length::Auto => context.containing,
            &Length::Px(px) => (px / PX_PER_CELL).round().max(0.0) as usize,
            &Length::Cells(cells) => cells.round().max(0.0) as usize,
            &Length::Pct(p) => {
                let raw = (context.containing as f32 * p / 100.0).round();
                (raw as usize).max(1)
            }
            &Length::Em(n) => (n * context.font.font_size_cells as f32).round().max(0.0) as usize,
            &Length::Rem(n) => (n * context.font.root_font_size_cells as f32).round().max(0.0) as usize,
            &Length::Vw(n) => {
                let raw = (n * context.viewport.cols as f32 / 100.0).round();
                (raw as usize).max(1)
            }
            &Length::Vh(n) => {
                let raw = (n * context.viewport.rows as f32 / 100.0).round();
                (raw as usize).max(1)
            }
        }
    }
}

/// Text alignment direction.
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

// ============================================================
// Flexbox property values
//
// Parsers and enums for the CSS flex properties supported by the
// renderer. Kept in `values.rs` alongside the other typed enums
// because they're plain value-types with no DOM / layout coupling.
// The flex layout itself lives in `layout::flex::flex`.
// ============================================================

/// Main axis direction (CSS `flex-direction`).
///
/// Only the two forward variants are exposed in v1; `*-reverse` is rejected
/// at parse time so callers can rely on `Row` / `Column` being the only
/// possibilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    /// Items laid out left-to-right along the main axis.
    Row,
    /// Items laid out top-to-bottom along the main axis.
    Column,
}

#[doc(hidden)]
pub fn parse_flex_direction(s: &str) -> Option<FlexDirection> {
    match s.trim().to_ascii_lowercase().as_str() {
        "row" => Some(FlexDirection::Row),
        "column" => Some(FlexDirection::Column),
        _ => None,
    }
}

/// Wrapping behaviour (CSS `flex-wrap`).
///
/// Only the forward variants are exposed in v1; `wrap-reverse` is rejected
/// at parse time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexWrap {
    /// All items on a single line; items may shrink to fit.
    NoWrap,
    /// Items wrap onto additional lines when the main axis overflows.
    Wrap,
}

#[doc(hidden)]
pub fn parse_flex_wrap(s: &str) -> Option<FlexWrap> {
    match s.trim().to_ascii_lowercase().as_str() {
        "nowrap" => Some(FlexWrap::NoWrap),
        "wrap" => Some(FlexWrap::Wrap),
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
/// `Baseline` is intentionally absent — terminal text has no real baseline
/// to align against. The default `Stretch` only takes effect when the
/// container has a known cross-axis size (e.g. an explicit `height` for
/// row direction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignItems {
    Stretch,
    FlexStart,
    FlexEnd,
    Center,
}

#[doc(hidden)]
pub fn parse_align_items(s: &str) -> Option<AlignItems> {
    match s.trim().to_ascii_lowercase().as_str() {
        "stretch" => Some(AlignItems::Stretch),
        "flex-start" | "start" => Some(AlignItems::FlexStart),
        "flex-end" | "end" => Some(AlignItems::FlexEnd),
        "center" => Some(AlignItems::Center),
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
    Length(Length),
}

#[doc(hidden)]
pub fn parse_flex_basis(s: &str) -> Option<FlexBasis> {
    let trimmed = s.trim();
    if trimmed.eq_ignore_ascii_case("auto") {
        return Some(FlexBasis::Auto);
    }
    Length::parse(trimmed).map(FlexBasis::Length)
}

/// Resolve a `flex-basis` against the containing main-axis size and the
/// active font-size context.
///
/// `Auto` returns `None` so callers can apply their own fallback chain
/// (item's `width` → content size → distribute equally).
pub fn resolve_flex_basis(
    basis: FlexBasis,
    containing_main: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> Option<usize> {
    match basis {
        FlexBasis::Auto => None,
        FlexBasis::Length(len) => Some(len.resolve(LengthContext { containing: containing_main, font: font_ctx, viewport })),
    }
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
            row: v.resolve(LengthContext { containing: usize::MAX, font: font_ctx, viewport }),
            column: v.resolve(LengthContext { containing: usize::MAX, font: font_ctx, viewport }),
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
        row: row.resolve(LengthContext { containing: usize::MAX, font: font_ctx, viewport }),
        column: column.resolve(LengthContext { containing: usize::MAX, font: font_ctx, viewport }),
    })
}
