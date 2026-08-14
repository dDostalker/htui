use crossterm::style::Color;
use cssparser::{Parser as CssParser, ParserInput, Token};
use indexmap::IndexMap;

use crate::html::box_edges::{Border, BorderStyle, EdgeAuto, EdgeSizes};

use super::color::parse_color_str;

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
// ============================================================
// Box-model shorthand parsing
// ============================================================

/// Expand `padding` / `padding-{side}` (or `margin` / `margin-{side}`) into an
/// [`EdgeSizes`].
///
/// Each value is parsed by [`Length::parse`]; unparseable tokens become `0`.
/// Per-side longhands are layered on top of the shorthand so `padding: 0;
/// padding-top: 5px;` resolves to top=5, others=0.
pub(crate) fn parse_edge_sizes(
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

/// Expand a `margin` shorthand / `margin-{side}` set into per-side `auto`
/// flags (CSS `margin: 0 auto`). Mirrors the [`parse_shorthand_lengths`]
/// expansion pattern for the token positions, then lets the per-side
/// longhands override each edge. `auto` on the vertical axes is recorded but
/// ignored by block-flow layout (CSS keeps vertical auto margins at 0); flex
/// containers may consult it later.
pub(crate) fn parse_edge_auto(decls: &IndexMap<String, String>, shorthand: &str) -> EdgeAuto {
    let mut out = EdgeAuto::default();
    if let Some(value) = decls.get(shorthand) {
        let autos: Vec<bool> = value
            .split_whitespace()
            .map(|t| t.eq_ignore_ascii_case("auto"))
            .collect();
        match autos.as_slice() {
            [a] => {
                if *a {
                    out = EdgeAuto::new(true, true, true, true);
                }
            }
            [a, b] => {
                if *a {
                    out.top = true;
                    out.bottom = true;
                }
                if *b {
                    out.right = true;
                    out.left = true;
                }
            }
            [a, b, c] => {
                if *a {
                    out.top = true;
                }
                if *b {
                    out.right = true;
                    out.left = true;
                }
                if *c {
                    out.bottom = true;
                }
            }
            [a, b, c, d, ..] => {
                out.top = *a;
                out.right = *b;
                out.bottom = *c;
                out.left = *d;
            }
            [] => {}
        }
    }
    // Per-side longhands win over the shorthand positions.
    for (side_name, flag) in [
        ("top", &mut out.top),
        ("right", &mut out.right),
        ("bottom", &mut out.bottom),
        ("left", &mut out.left),
    ] {
        if let Some(raw) = decls.get(&format!("{shorthand}-{side_name}")) {
            *flag = raw.eq_ignore_ascii_case("auto");
        }
    }
    out
}

fn resolve_length(
    length: Length,
    containing: usize,
    font: FontContext,
    viewport: Viewport,
) -> usize {
    length.resolve(LengthContext {
        containing,
        font,
        viewport,
    })
}
///
/// `auto` / missing / unsupported unit → `None` (caller decides what default
/// to apply).
pub(super) fn parse_length_token(
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
pub(crate) fn parse_border_width(
    value: Option<&str>,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> usize {
    parse_length_token(value, containing_width, font_ctx, viewport).unwrap_or(0)
}

/// Map a `border-style` keyword to a [`BorderStyle`]. Unknown / missing
/// defaults to [`BorderStyle::None`].
pub(crate) fn parse_border_style(value: Option<&str>) -> BorderStyle {
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
pub(crate) fn parse_border_side(
    side_raw: Option<&String>,
    shorthand_raw: Option<&String>,
    fallback: Border,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> Border {
    if let Some(value) = side_raw
        && let Some(parsed) = parse_border_shorthand(value, containing_width, font_ctx, viewport)
    {
        return parsed;

        // `border-top` was present but unparseable — fall through to next
        // priority rather than dropping the side entirely.
    }
    if let Some(value) = shorthand_raw
        && let Some(parsed) = parse_border_shorthand(value, containing_width, font_ctx, viewport)
    {
        return parsed;
    }
    fallback
}

/// Overlay the per-side `border-{side}-{prop}` longhands on top of an
/// already-resolved side, mirroring CSS priority: `border-top-width` beats
/// `border-top`, which beats `border-width` / `border`.
///
/// An unparseable longhand (or one absent from the declaration map) leaves
/// that property untouched, so a bad value never wipes a valid shorthand.
pub(crate) fn merge_border_longhand(
    side: Border,
    decls: &IndexMap<String, String>,
    side_name: &str,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> Border {
    let mut side = side;
    if let Some(raw) = decls.get(&format!("border-{side_name}-width"))
        && let Some(w) =
            parse_length_token(Some(raw.as_str()), containing_width, font_ctx, viewport)
    {
        side.width = w;
    }
    if let Some(raw) = decls.get(&format!("border-{side_name}-style")) {
        side.style = parse_border_style(Some(raw.as_str()));
    }
    if let Some(raw) = decls.get(&format!("border-{side_name}-color"))
        && let Some(c) = parse_color_str(raw)
    {
        side.color = Some(c);
    }
    side
}

/// Parse a `border` shorthand declaration.
///
/// CSS allows any of width / style / color in any order, with some pieces
/// optional. We tokenise on whitespace and classify each token; tokens we
/// can't classify fall through (treated as missing) so a typo in one piece
/// doesn't drop the whole border. Functional colors (`rgb(255, 0, 0)`,
/// `hsl(120 50% 50%)`, …) are kept intact as one token — splitting them on
/// internal spaces / commas would break the color.
fn parse_border_shorthand(
    value: &str,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> Option<Border> {
    let mut width: Option<usize> = None;
    let mut style: Option<BorderStyle> = None;
    let mut color: Option<Color> = None;

    for token in split_border_tokens(value) {
        if let Some(w) =
            parse_length_token(Some(token.as_str()), containing_width, font_ctx, viewport)
        {
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
            style = Some(BorderStyle::parse(&token));
            continue;
        }
        if let Some(c) = parse_color_str(&token) {
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

/// Split a border-shorthand value on whitespace, keeping any parenthesised
/// functional colors (`rgb(…)` / `hsl(…)` / …) as single tokens so their
/// internal spaces and commas survive.
fn split_border_tokens(value: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    for c in value.chars() {
        match c {
            '(' => {
                depth = depth.saturating_add(1);
                cur.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                cur.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
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
/// - `12ch`                         → [`Length::Ch(n)`] (1ch == one cell)
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
    /// Character-width multiplier (CSS `12ch`). In a browser `1ch` is the
    /// advance width of the `0` glyph — exactly one cell on the terminal's
    /// monospace grid, so `1ch == 1 cell`. Unlike `em`, this stays aligned
    /// with character counts regardless of font-size, which makes it the
    /// unit of choice for widths measured in characters (the default for
    /// terminal layout).
    Ch(f32),
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
    Ch(f32),
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
                    // `ch` is the CSS character-width unit: 1ch is the `0`
                    // glyph's advance, exactly one cell on the monospace
                    // grid. It resolves to the raw value (no font context).
                    "ch" => PreLength::Ch(n),
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
            PreLength::Ch(n) => Length::Ch(n),
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
            &Length::Rem(n) => (n * context.font.root_font_size_cells as f32)
                .round()
                .max(0.0) as usize,
            // `ch` is font-size-independent by definition: one cell per `0`
            // glyph, so the raw multiplier is the cell count.
            &Length::Ch(n) => n.round().max(0.0) as usize,
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
