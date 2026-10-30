use std::sync::Arc;

use crossterm::style::Color;
use cssparser::{Parser as CssParser, ParserInput, Token};
use indexmap::IndexMap;

use crate::html::box_edges::{Border, BorderStyle, EdgeAuto, EdgeSizes, MarginSizes};

use super::color::parse_color_str;

/// Number of CSS px represented by one terminal cell on each axis.
/// CSS defines 1px as 1/96 inch; terminals cannot query physical DPI, so
/// px lengths are converted to cells at layout time via this ratio. A
/// monospace glyph is 0.5em wide at the 16px default font-size, so one
/// cell anchors at 8px small values like `border: 1px` resolve to 0
/// cells (sub-cell, invisible), mirroring a browser rendered on a
/// character grid. The constant is named so future changes (e.g. a 2x DPI
/// mode) have one edit point. This constant only governs CSS px, not the
/// numeric pixels on img HTML attributes, which describe source-image
/// pixels and are sampled at the ratios defined in
/// tui::graph_api::image (IMAGE_PIXELS_PER_CELL_COL /
/// IMAGE_PIXELS_PER_CELL_ROW).
pub const PX_PER_CELL: f32 = 8.0;
// ============================================================
// Box-model shorthand parsing
// ============================================================

fn compose_key<'k>(buf: &'k mut [u8; 32], parts: &[&str]) -> &'k str {
    let mut len = 0usize;
    for part in parts {
        buf[len..len + part.len()].copy_from_slice(part.as_bytes());
        len += part.len();
    }
    std::str::from_utf8(&buf[..len]).expect("concatenation of &str parts is valid UTF-8")
}

/// Expand `padding` / `padding-{side}` (or `margin` / `margin-{side}`) into an
/// [`EdgeSizes`].
///
/// Each value is parsed by [`Length::parse`]; unparseable tokens become `0`.
/// Per-side longhands are layered on top of the shorthand so `padding: 1ch;
/// padding-top: 2ch;` resolves to top=2, others=1.
pub(crate) fn parse_edge_sizes(
    decls: &IndexMap<Arc<str>, Arc<str>>,
    shorthand: &str,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> EdgeSizes {
    let shorthand_raw = decls.get(shorthand);
    let mut edges = parse_shorthand_lengths(
        shorthand_raw.map(|s| s.as_ref()),
        containing_width,
        font_ctx,
        viewport,
    );

    let mut key = [0u8; 32];
    let top_raw = decls
        .get(compose_key(&mut key, &[shorthand, "-top"]))
        .map(|s| s.as_ref());
    if let Some(v) = parse_length_token(top_raw, containing_width, font_ctx, viewport) {
        edges.top = v;
    }
    let right_raw = decls
        .get(compose_key(&mut key, &[shorthand, "-right"]))
        .map(|s| s.as_ref());
    if let Some(v) = parse_length_token(right_raw, containing_width, font_ctx, viewport) {
        edges.right = v;
    }
    let bottom_raw = decls
        .get(compose_key(&mut key, &[shorthand, "-bottom"]))
        .map(|s| s.as_ref());
    if let Some(v) = parse_length_token(bottom_raw, containing_width, font_ctx, viewport) {
        edges.bottom = v;
    }
    let left_raw = decls
        .get(compose_key(&mut key, &[shorthand, "-left"]))
        .map(|s| s.as_ref());
    if let Some(v) = parse_length_token(left_raw, containing_width, font_ctx, viewport) {
        edges.left = v;
    }

    edges
}

/// Expand a 1-/2-/3-/4-value CSS shorthand into [`EdgeSizes`].
///
/// Tokenisation is whitespace-driven we split on runs of whitespace and try
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
pub(crate) fn parse_edge_auto(decls: &IndexMap<Arc<str>, Arc<str>>, shorthand: &str) -> EdgeAuto {
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
    let mut key = [0u8; 32];
    for (side_name, flag) in [
        ("top", &mut out.top),
        ("right", &mut out.right),
        ("bottom", &mut out.bottom),
        ("left", &mut out.left),
    ] {
        if let Some(raw) = decls.get(compose_key(&mut key, &[shorthand, "-", side_name])) {
            *flag = raw.eq_ignore_ascii_case("auto");
        }
    }
    out
}

/// Resolve one signed length token to a (possibly negative) cell count.
///
/// Returns `None` for `auto` / unparseable input mirrors
/// [`parse_length_token`] but keeps the sign so negative margins and insets
/// survive resolution.
fn parse_signed_length_token(
    value: Option<&str>,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> Option<isize> {
    let raw = value?;
    match Length::parse_signed(raw)? {
        Length::Auto => None,
        len => Some(len.resolve_signed(LengthContext {
            containing: containing_width,
            font: font_ctx,
            viewport,
        })),
    }
}

/// Expand a `margin` shorthand / `margin-{side}` into a signed [`MarginSizes`].
///
/// Mirrors [`parse_edge_sizes`] except the left / right edges are resolved
/// **signed** (negative margins are legal and widen / shift the box), while
/// the top / bottom edges are clamped to `0` when negative the cursor model
/// has no overlapping-row painting, so a negative vertical margin can only
/// reduce spacing to nothing. Per-side longhands (`margin-left`, ) win over
/// the shorthand; `auto` tokens collapse to `0` (the auto flags are tracked
/// separately by [`parse_edge_auto`]).
pub(crate) fn parse_margin_sizes(
    decls: &IndexMap<Arc<str>, Arc<str>>,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> MarginSizes {
    let shorthand_raw = decls.get("margin");
    let mut edges = parse_shorthand_margins(
        shorthand_raw.map(|s| s.as_ref()),
        containing_width,
        font_ctx,
        viewport,
    );

    let mut key = [0u8; 32];
    // Per-side longhands override the shorthand. `margin-top` / `margin-bottom`
    // clamp to 0 when negative (no overlapping rows); `margin-left` /
    // `margin-right` keep their sign.
    if let Some(raw) = decls.get(compose_key(&mut key, &["margin", "-top"]))
        && let Some(v) =
            parse_signed_length_token(Some(raw.as_ref()), containing_width, font_ctx, viewport)
    {
        edges.top = v.max(0) as usize;
    }
    if let Some(raw) = decls.get(compose_key(&mut key, &["margin", "-right"]))
        && let Some(v) =
            parse_signed_length_token(Some(raw.as_ref()), containing_width, font_ctx, viewport)
    {
        edges.right = v;
    }
    if let Some(raw) = decls.get(compose_key(&mut key, &["margin", "-bottom"]))
        && let Some(v) =
            parse_signed_length_token(Some(raw.as_ref()), containing_width, font_ctx, viewport)
    {
        edges.bottom = v.max(0) as usize;
    }
    if let Some(raw) = decls.get(compose_key(&mut key, &["margin", "-left"]))
        && let Some(v) =
            parse_signed_length_token(Some(raw.as_ref()), containing_width, font_ctx, viewport)
    {
        edges.left = v;
    }
    edges
}

/// Expand a 1-/2-/3-/4-value `margin` shorthand into a signed [`MarginSizes`],
/// clamping the vertical (top / bottom) edges to `0` when negative.
fn parse_shorthand_margins(
    value: Option<&str>,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> MarginSizes {
    let Some(value) = value else {
        return MarginSizes::default();
    };
    let parsed: Vec<isize> = value
        .split_whitespace()
        .filter_map(|t| parse_signed_length_token(Some(t), containing_width, font_ctx, viewport))
        .collect();
    let v = |x: isize| x.max(0) as usize;
    match parsed.len() {
        0 => MarginSizes::default(),
        1 => MarginSizes::new(v(parsed[0]), parsed[0], v(parsed[0]), parsed[0]),
        2 => MarginSizes::new(v(parsed[0]), parsed[1], v(parsed[0]), parsed[1]),
        3 => MarginSizes::new(v(parsed[0]), parsed[1], v(parsed[2]), parsed[1]),
        _ => MarginSizes::new(v(parsed[0]), parsed[1], v(parsed[2]), parsed[3]),
    }
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
/// Parse one CSS length property into resolved cells.
///
/// `auto` / missing / unsupported unit `None` (caller decides what default
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
/// the caller passing it down means we don't re-parse the same map entries
/// four times.
pub(crate) fn parse_border_side(
    side_raw: Option<&Arc<str>>,
    shorthand_raw: Option<&Arc<str>>,
    fallback: Border,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> Border {
    if let Some(value) = side_raw
        && let Some(parsed) = parse_border_shorthand(value, containing_width, font_ctx, viewport)
    {
        return parsed;

        // `border-top` was present but unparseable fall through to next
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
    decls: &IndexMap<Arc<str>, Arc<str>>,
    side_name: &str,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> Border {
    let mut side = side;
    let mut key = [0u8; 32];
    if let Some(raw) = decls.get(compose_key(&mut key, &["border-", side_name, "-width"]))
        && let Some(w) =
            parse_length_token(Some(raw.as_ref()), containing_width, font_ctx, viewport)
    {
        side.width = w;
    }
    if let Some(raw) = decls.get(compose_key(&mut key, &["border-", side_name, "-style"])) {
        side.style = parse_border_style(Some(raw.as_ref()));
    }
    if let Some(raw) = decls.get(compose_key(&mut key, &["border-", side_name, "-color"]))
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
/// `hsl(120 50% 50%)`, are kept intact as one token splitting them on
/// internal spaces / commas would break the color.
pub(crate) fn parse_border_shorthand(
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
        // Unrecognised token ignore (don't fail the whole rule).
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
/// functional colors (`rgb( ` / `hsl( ` / as single tokens so their
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
///
/// The default (`0x0`) stands for "no viewport known" — size conditions never
/// match it, so unconditional styling is unaffected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
    /// Create the root font context (1-cell font, 1-cell root font).
    pub fn root() -> Self {
        Self {
            font_size_cells: 1,
            root_font_size_cells: 1,
        }
    }

    /// Derive a child context with `child_cells` as its font size, keeping the root size.
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
/// accept is normalised to an integer column count by [`Length::resolve`]
/// through a single conversion layer: absolute lengths (`px`, bare
/// numbers, `em`, `rem`) first become CSS px, then divide by
/// [`PX_PER_CELL`] (8px per cell); `ch` stays cell-aligned by definition
/// (one cell per `0` glyph); percentages and viewport units keep their
/// relative semantics. The whole pipeline is anchored on this one
/// function, so every length-bearing property (width / height / padding /
/// border / margin / inset / flex-basis / ...) shares the same scale.
///
/// Accepted forms:
/// - `auto` [`Length::Auto`] (caller decides what to fill)
/// - `40px`, bare `40` [`Length::Px`] ( `PX_PER_CELL` at resolve)
/// - `50%` [`Length::Pct`] (raw 0 00, divided at resolve)
/// - `2em`, `1.5em` [`Length::Em`]
/// - `2rem`, `1.5rem` [`Length::Rem`]
/// - `12ch` [`Length::Ch`] (1ch == one cell)
/// - `50vw` [`Length::Vw`] (1vw = viewport_cols / 100)
/// - `50vh` [`Length::Vh`] (1vh = viewport_rows / 100)
///
/// Rejected (returns `None` from [`Length::parse`]):
/// - `vmin`, `vmax` derive from `vw` / `vh` after `Layout` has viewport info;
/// not yet wired through.
/// - `pt`, `cm`, `mm`, `in`, `pc` no DPI. Use `px` (converted via
/// [`PX_PER_CELL`]).
/// - Negative or signed (`+`, `-`) values CSS forbids these for `width` /
/// `height`; the signed properties (`margin-*`, the positioning insets) use
/// [`Length::parse_signed`], which accepts them.
/// - Trailing junk after the value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    /// `auto` defer to the containing block.
    Auto,
    /// Absolute CSS pixel length (including bare numbers, which CSS treats
    /// as px). Converted to terminal cells at resolve time by dividing by
    /// [`PX_PER_CELL`]. (The engine's own tests write `Nch` when they mean
    /// "N cells".)
    Px(f32),
    /// Percentage 0..=100 of the containing block (CSS `50%`).
    Pct(f32),
    /// Em multiplier (CSS `2em`). 1em == 1 cell on the terminal grid.
    Em(f32),
    /// Root-em multiplier (CSS `2rem`). Resolves against the root's font-size
    /// (`root_font_size_cells`), which is 1 cell by default and grows when the
    /// document root sets an explicit `font-size` rule unlike `em`, which
    /// resolves against the element's own font-size.
    Rem(f32),
    /// Character-width multiplier (CSS `12ch`). In a browser `1ch` is the
    /// advance width of the `0` glyph exactly one cell on the terminal's
    /// monospace grid, so `1ch == 1 cell`. Unlike `em`, this stays aligned
    /// with character counts regardless of font-size, which makes it the
    /// unit of choice for widths measured in characters (the default for
    /// terminal layout).
    Ch(f32),
    /// Viewport-width multiplier (CSS `50vw`).
    ///
    /// Resolves to `round(n * viewport_cols / 100)` cells, min 1.
    /// `vw` always anchors to the **viewport columns** (the terminal
    /// width), regardless of which CSS property it appears in so
    /// `height: 50vw` mixes axes deliberately and is the caller's
    /// choice; `Length::resolve` does not need to know the axis.
    Vw(f32),
    /// Viewport-height multiplier (CSS `50vh`).
    ///
    /// Resolves to `round(n * viewport_rows / 100)` cells, min 1.
    /// Mirrors [`Length::Vw`] on the row axis.
    Vh(f32),
    /// A `calc()` / `min()` / `max()` / `clamp()` expression (the supported
    /// subset — see `MathFn`). Kept unevaluated at parse time (the units
    /// need a [`LengthContext`]) and resolved cell-exact at
    /// [`resolve`](Self::resolve) time.
    Math(MathFn),
}

/// The math-function subset supported inside [`Length::Math`] — everything
/// stays `Copy` by keeping the expression flat:
///
/// - `calc(<a> ± <b>)` — a linear sum of two terms (each a plain length
///   value; nested math functions are not accepted).
/// - `min(<a>, <b>)` / `max(<a>, <b>)` — the smaller / larger of two terms.
/// - `clamp(<min>, <val>, <max>)` — `min ≤ val ≤ max`.
///
/// Multiplication / division by bare numbers inside `calc` is folded into
/// the term coefficient (`calc(2 * 10ch)` parses as the term `20ch`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MathFn {
    Calc(Term, Term),
    Min(Term, Term),
    Max(Term, Term),
    Clamp(Term, Term, Term),
}

/// One operand of a math expression: a coefficient and a unit the
/// coefficient multiplies at resolve time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Term {
    /// Signed coefficient.
    pub n: f32,
    /// The unit the coefficient scales (`%` carries `containing`).
    pub unit: MathUnit,
}

/// Units a [`Term`] can carry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MathUnit {
    Px,
    Pct,
    Em,
    Rem,
    Ch,
    Vw,
    Vh,
}

impl Term {
    /// Resolve this term to a signed cell count under `context`.
    fn cells(self, context: LengthContext) -> f32 {
        match self.unit {
            MathUnit::Px => self.n / PX_PER_CELL,
            MathUnit::Pct => self.n * context.containing as f32 / 100.0,
            MathUnit::Em => self.n * context.font.font_size_cells as f32,
            MathUnit::Rem => self.n * context.font.root_font_size_cells as f32,
            MathUnit::Ch => self.n,
            MathUnit::Vw => self.n * context.viewport.cols as f32 / 100.0,
            MathUnit::Vh => self.n * context.viewport.rows as f32 / 100.0,
        }
    }
}

impl MathFn {
    /// Evaluate the expression to a signed cell count under `context`.
    fn eval(self, context: LengthContext) -> f32 {
        match self {
            MathFn::Calc(a, b) => a.cells(context) + b.cells(context),
            MathFn::Min(a, b) => a.cells(context).min(b.cells(context)),
            MathFn::Max(a, b) => a.cells(context).max(b.cells(context)),
            MathFn::Clamp(min, val, max) => val
                .cells(context)
                .clamp(min.cells(context), max.cells(context)),
        }
    }
}

/// Owned mirror of the cssparser [`Token`] variants we accept. Lets
/// [`Length::parse`] release its borrow on the parser before checking for
/// trailing junk with `is_exhausted()`.
#[derive(Debug, Clone, Copy)]
enum PreLength {
    Px(f32),
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
    /// always returns [`Length::Auto`]. Use [`parse_signed`](Self::parse_signed)
    /// for the properties where a sign is legal (`margin-*`, the positioning
    /// insets `top`/`right`/`bottom`/`left`).
    pub fn parse(s: &str) -> Option<Length> {
        Self::parse_length_shared(s, false)
    }

    /// Parse a CSS length token, allowing a leading sign (`-` / `+`).
    ///
    /// CSS forbids signed values for `width` / `height` (hence [`Length::parse`]
    /// rejects them) but permits them for `margin-*` and the positioning
    /// insets `top` / `right` / `bottom` / `left`. The sign is carried by the
    /// stored magnitude (cssparser's `Token::Number` `value` is already
    /// signed); resolve it with [`resolve_signed`](Self::resolve_signed) so a
    /// negative length yields a negative cell count instead of clamping to 0.
    pub fn parse_signed(s: &str) -> Option<Length> {
        Self::parse_length_shared(s, true)
    }

    /// Resolve the length into an integer cell count that may be negative.
    ///
    /// Mirrors [`resolve`](Self::resolve) but returns `isize` (no non-negative
    /// clamp) so negative margins / insets keep their sign: `-2ch`, `-10px`,
    /// `-50%` and negative `em` / `rem` all resolve negative. `Auto` maps to
    /// the containing size (callers filter it out before calling). Positive
    /// values resolve to exactly the same cell count as [`resolve`](Self::resolve).
    pub fn resolve_signed(&self, context: LengthContext) -> isize {
        match self {
            Length::Auto => context.containing as isize,
            &Length::Px(px) => (px / PX_PER_CELL).round() as isize,
            &Length::Pct(p) => (context.containing as f32 * p / 100.0).round() as isize,
            &Length::Em(n) => (n * context.font.font_size_cells as f32).round() as isize,
            &Length::Rem(n) => (n * context.font.root_font_size_cells as f32).round() as isize,
            // `ch` is font-size-independent by definition: one cell per `0`
            // glyph, so the raw multiplier is the cell count.
            &Length::Ch(n) => n.round() as isize,
            &Length::Vw(n) => (n * context.viewport.cols as f32 / 100.0).round() as isize,
            &Length::Vh(n) => (n * context.viewport.rows as f32 / 100.0).round() as isize,
            Length::Math(func) => func.eval(context).round() as isize,
        }
    }

    /// Shared parser behind [`parse`](Self::parse) /
    /// [`parse_signed`](Self::parse_signed); `allow_signed` flips whether a
    /// leading `-` / `+` on the value token is accepted.
    fn parse_length_shared(s: &str, allow_signed: bool) -> Option<Length> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("auto") {
            return Some(Length::Auto);
        }
        // Math functions (`calc(…)` / `min(…)` / `max(…)` / `clamp(…)`)
        // parse through their own reader; the cssparser tokenizer below
        // would otherwise reject the function token.
        if let Some(math) = parse_math_function(s) {
            let _ = allow_signed;
            return Some(Length::Math(math));
        }
        // cssparser's tokenizer handles `40`, `40.5`, `40px`, `1.5em`, `50%`,
        // escapes, and unicode digits uniformly. We pin the lifetime with a
        // named ParserInput binding see the warning in cssparser's docs:
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
                has_sign,
                ..
            } => {
                if *has_sign && !allow_signed {
                    return None;
                }
                PreLength::Px(int_value.map(|i| i as f32).unwrap_or(*value))
            }
            Token::Dimension {
                value,
                int_value,
                has_sign,
                unit,
                ..
            } => {
                if *has_sign && !allow_signed {
                    return None;
                }
                let n = int_value.map(|i| i as f32).unwrap_or(*value);
                match unit.to_ascii_lowercase().as_str() {
                    // CSS px: kept as a distinct `Px` variant and converted to
                    // cells at resolve time by dividing by PX_PER_CELL (8px
                    // per cell). parse only records the raw multiplier; the
                    // ratio lives in one edit point.
                    "px" => PreLength::Px(n),
                    "em" => PreLength::Em(n),
                    "rem" => PreLength::Rem(n),
                    // `ch` is the CSS character-width unit: 1ch is the `0`
                    // glyph's advance, exactly one cell on the monospace
                    // grid. It resolves to the raw value (no font context).
                    "ch" => PreLength::Ch(n),
                    // 1vw = viewport_cols / 100, 1vh = viewport_rows / 100.
                    // The actual viewport size is supplied at `Length::resolve`
                    // time the parser only carries the raw multiplier.
                    "vw" => PreLength::Vw(n),
                    "vh" => PreLength::Vh(n),
                    // vmin / vmax / pt / cm / mm / in / pc / ...
                    _ => return None,
                }
            }
            Token::Percentage {
                unit_value,
                has_sign,
                ..
            } => {
                if *has_sign && !allow_signed {
                    return None;
                }
                // cssparser divides by 100 for us (`50%` `0.5`). Multiply back
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
    /// This is the single conversion layer every CSS length flows through:
    /// absolute units (`px`, bare numbers, `em`, `rem`) convert to px and
    /// divide by [`PX_PER_CELL`]; `ch` maps one-to-one to cells; relative
    /// units keep their ratio. Layout code never re-implements this math, so
    /// a change to the per-cell px ratio needs exactly one edit point.
    ///
    /// - `Auto` `containing` (fill the container)
    /// - `Px(n)` round(n / PX_PER_CELL)
    /// - `Pct(p)` round(containing * p / 100), min 1
    /// - `Em(n)` round(n * context.font.font_size_cells), clamped at 0
    /// - `Rem(n)` round(n * context.font.root_font_size_cells), clamped at 0
    /// - `Vw(n)` round(n * context.viewport.cols / 100), min 1
    /// - `Vh(n)` round(n * context.viewport.rows / 100), min 1
    ///
    /// `em` resolves against the **current element's** effective font-size
    /// (in cells, itself derived from the same conversion layer so
    /// `font-size: 16px` yields a 2-cell font and `1em` == 2 cells, exactly
    /// like a browser at 8px/cell); `rem` resolves against the **root's**
    /// font-size. On a terminal the root font-size is 1 cell by default it
    /// grows when the document root sets an explicit `font-size` rule.
    ///
    /// `vw` / `vh` anchor to the terminal viewport (columns / rows), not to
    /// the containing block. They are axis-independent: `vw` always uses
    /// `viewport_cols`, `vh` always uses `viewport_rows`, regardless of
    /// which CSS property they appear in.
    ///
    /// Floats round **away from zero** (Rust's `f32::round`), so `0.5em` at
    /// font-size=2 yields 1 cell and `2.5em` at font-size=2 yields 5 cells /// never zero, never half.
    pub fn resolve(&self, context: LengthContext) -> usize {
        match self {
            Length::Auto => context.containing,
            &Length::Px(px) => (px / PX_PER_CELL).round().max(0.0) as usize,
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
            Length::Math(func) => {
                let cells = func.eval(context).round();
                (cells.max(0.0) as usize).max(1)
            }
        }
    }
}

// ============================================================
// Math functions: calc() / min() / max() / clamp()
// ============================================================

/// Recognize and parse a leading `calc(` / `min(` / `max(` / `clamp(`
/// function (ASCII case-insensitive). Returns `None` for anything else so
/// the plain-token path stays untouched.
fn parse_math_function(s: &str) -> Option<MathFn> {
    let lower = s.to_ascii_lowercase();
    for name in ["calc", "min", "max", "clamp"] {
        let prefix = format!("{name}(");
        if let Some(rest) = lower.strip_prefix(&prefix)
            && rest.ends_with(')')
        {
            // Re-slice the original string so escapes / case survive.
            let body = &s[name.len() + 1..s.len() - 1];
            return parse_math_body(name, body);
        }
    }
    None
}

/// Parse the body of one math function into its expression form.
///
/// - `calc`: `term ± term` (a lone term is also accepted, folded as
///   `term + 0ch`); `number * term` and `term / number` fold into the
///   coefficient.
/// - `min` / `max`: exactly two comma-separated terms.
/// - `clamp`: exactly three (min, val, max).
fn parse_math_body(name: &str, body: &str) -> Option<MathFn> {
    match name {
        "calc" => {
            let (a, b) = split_calc_addends(body)?;
            Some(MathFn::Calc(a, b))
        }
        "min" | "max" => {
            let parts = split_commas(body);
            if parts.len() != 2 {
                return None;
            }
            let a = parse_term(parts[0])?;
            let b = parse_term(parts[1])?;
            Some(if name == "min" {
                MathFn::Min(a, b)
            } else {
                MathFn::Max(a, b)
            })
        }
        _ => {
            let parts = split_commas(body);
            if parts.len() != 3 {
                return None;
            }
            Some(MathFn::Clamp(
                parse_term(parts[0])?,
                parse_term(parts[1])?,
                parse_term(parts[2])?,
            ))
        }
    }
}

/// Split `calc`'s body at the top-level `+` / `-` into two addends (the
/// sign rides on the second term). A body with no operator is a single
/// term plus zero.
fn split_calc_addends(body: &str) -> Option<(Term, Term)> {
    let mut depth = 0usize;
    for (i, ch) in body.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '+' | '-' if depth == 0 && i > 0 => {
                // Ignore the sign directly following `*` or `/` (not part of
                // the supported grammar anyway — reject there).
                let a = parse_term(&body[..i])?;
                let b = parse_term(&body[i + 1..])?;
                let b_signed = Term {
                    n: if ch == '-' { -b.n } else { b.n },
                    unit: b.unit,
                };
                return Some((a, b_signed));
            }
            _ => {}
        }
    }
    let a = parse_term(body)?;
    Some((
        a,
        Term {
            n: 0.0,
            unit: MathUnit::Ch,
        },
    ))
}

/// Split on top-level commas (outside nested parens), trimming each part
/// and dropping empties.
fn split_commas(body: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, ch) in body.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(body[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(body[start..].trim());
    parts.into_iter().filter(|p| !p.is_empty()).collect()
}

/// Parse one term: `[number *] value`, `value [*|/ number]`, or a bare
/// value. The value itself is a plain length token (unit or `%`).
fn parse_term(s: &str) -> Option<Term> {
    let s = s.trim();
    // `N * value`
    if let Some((lhs, rhs)) = s.split_once('*') {
        let k: f32 = lhs.trim().parse().ok()?;
        let mut t = parse_value_term(rhs)?;
        t.n *= k;
        return Some(t);
    }
    // `value / N`
    if let Some((lhs, rhs)) = s.split_once('/') {
        let d: f32 = rhs.trim().parse().ok()?;
        let mut t = parse_value_term(lhs)?;
        if d == 0.0 {
            return None;
        }
        t.n /= d;
        return Some(t);
    }
    parse_value_term(s)
}

/// Parse a plain `<number><unit>` / `<percent>` token into a term.
fn parse_value_term(s: &str) -> Option<Term> {
    let s = s.trim();
    let (num_end, unit) = s
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+'))
        .map(|(i, _)| (i, &s[i..]))
        .unwrap_or((s.len(), ""));
    let n: f32 = s[..num_end].trim().parse().ok()?;
    let unit = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "px" => MathUnit::Px,
        "%" => MathUnit::Pct,
        "em" => MathUnit::Em,
        "rem" => MathUnit::Rem,
        "ch" => MathUnit::Ch,
        "vw" => MathUnit::Vw,
        "vh" => MathUnit::Vh,
        _ => return None,
    };
    Some(Term { n, unit })
}
