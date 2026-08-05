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
/// - Hexadecimal: `#RRGGBB`, `#RGB`
/// - Functional: `rgb(255, 0, 0)`
/// - The 148 CSS named colors from Color Module Level 4 (e.g. `red`,
///   `rebeccapurple`), resolved to their exact sRGB value so they render
///   identically on any terminal theme.
///
/// Unknown formats return `None` so callers can treat the declaration as
/// invalid (CSS drops unsupported values) instead of silently substituting
/// a colour.
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

    // 3. CSS named colors (see [`NAMED_COLORS`]).
    parse_named_color(&lower)
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

/// CSS Color Module Level 4 named colours (148 total), each mapped to its
/// exact sRGB value. Returning truecolor [`Color::Rgb`] instead of the
/// terminal's 16/256-colour palette makes a named colour render identically
/// on every theme — `red` is always `#FF0000`, `darkblue` always `#00008B`.
///
/// Values follow the CSS spec hex codes verbatim; the two spellings of the
/// grey-scale names (`gray`/`grey`, `darkgray`/`darkgrey`, …) share one
/// entry each. Sorted alphabetically for readability.
const NAMED_COLORS: &[(&str, u8, u8, u8)] = &[
    ("aliceblue", 0xF0, 0xF8, 0xFF),
    ("antiquewhite", 0xFA, 0xEB, 0xD7),
    ("aqua", 0x00, 0xFF, 0xFF),
    ("aquamarine", 0x7F, 0xFF, 0xD4),
    ("azure", 0xF0, 0xFF, 0xFF),
    ("beige", 0xF5, 0xF5, 0xDC),
    ("bisque", 0xFF, 0xE4, 0xC4),
    ("black", 0x00, 0x00, 0x00),
    ("blanchedalmond", 0xFF, 0xEB, 0xCD),
    ("blue", 0x00, 0x00, 0xFF),
    ("blueviolet", 0x8A, 0x2B, 0xE2),
    ("brown", 0xA5, 0x2A, 0x2A),
    ("burlywood", 0xDE, 0xB8, 0x87),
    ("cadetblue", 0x5F, 0x9E, 0xA0),
    ("chartreuse", 0x7F, 0xFF, 0x00),
    ("chocolate", 0xD2, 0x69, 0x1E),
    ("coral", 0xFF, 0x7F, 0x50),
    ("cornflowerblue", 0x64, 0x95, 0xED),
    ("cornsilk", 0xFF, 0xF8, 0xDC),
    ("crimson", 0xDC, 0x14, 0x3C),
    ("cyan", 0x00, 0xFF, 0xFF),
    ("darkblue", 0x00, 0x00, 0x8B),
    ("darkcyan", 0x00, 0x8B, 0x8B),
    ("darkgoldenrod", 0xB8, 0x86, 0x0B),
    ("darkgray", 0xA9, 0xA9, 0xA9),
    ("darkgreen", 0x00, 0x64, 0x00),
    ("darkgrey", 0xA9, 0xA9, 0xA9),
    ("darkkhaki", 0xBD, 0xB7, 0x6B),
    ("darkmagenta", 0x8B, 0x00, 0x8B),
    ("darkolivegreen", 0x55, 0x6B, 0x2F),
    ("darkorange", 0xFF, 0x8C, 0x00),
    ("darkorchid", 0x99, 0x32, 0xCC),
    ("darkred", 0x8B, 0x00, 0x00),
    ("darksalmon", 0xE9, 0x96, 0x7A),
    ("darkseagreen", 0x8F, 0xBC, 0x8F),
    ("darkslateblue", 0x48, 0x3D, 0x8B),
    ("darkslategray", 0x2F, 0x4F, 0x4F),
    ("darkslategrey", 0x2F, 0x4F, 0x4F),
    ("darkturquoise", 0x00, 0xCE, 0xD1),
    ("darkviolet", 0x94, 0x00, 0xD3),
    ("deeppink", 0xFF, 0x14, 0x93),
    ("deepskyblue", 0x00, 0xBF, 0xFF),
    ("dimgray", 0x69, 0x69, 0x69),
    ("dimgrey", 0x69, 0x69, 0x69),
    ("dodgerblue", 0x1E, 0x90, 0xFF),
    ("firebrick", 0xB2, 0x22, 0x22),
    ("floralwhite", 0xFF, 0xFA, 0xF0),
    ("forestgreen", 0x22, 0x8B, 0x22),
    ("fuchsia", 0xFF, 0x00, 0xFF),
    ("gainsboro", 0xDC, 0xDC, 0xDC),
    ("ghostwhite", 0xF8, 0xF8, 0xFF),
    ("gold", 0xFF, 0xD7, 0x00),
    ("goldenrod", 0xDA, 0xA5, 0x20),
    ("gray", 0x80, 0x80, 0x80),
    ("green", 0x00, 0x80, 0x00),
    ("greenyellow", 0xAD, 0xFF, 0x2F),
    ("grey", 0x80, 0x80, 0x80),
    ("honeydew", 0xF0, 0xFF, 0xF0),
    ("hotpink", 0xFF, 0x69, 0xB4),
    ("indianred", 0xCD, 0x5C, 0x5C),
    ("indigo", 0x4B, 0x00, 0x82),
    ("ivory", 0xFF, 0xFF, 0xF0),
    ("khaki", 0xF0, 0xE6, 0x8C),
    ("lavender", 0xE6, 0xE6, 0xFA),
    ("lavenderblush", 0xFF, 0xF0, 0xF5),
    ("lawngreen", 0x7C, 0xFC, 0x00),
    ("lemonchiffon", 0xFF, 0xFA, 0xCD),
    ("lightblue", 0xAD, 0xD8, 0xE6),
    ("lightcoral", 0xF0, 0x80, 0x80),
    ("lightcyan", 0xE0, 0xFF, 0xFF),
    ("lightgoldenrodyellow", 0xFA, 0xFA, 0xD2),
    ("lightgray", 0xD3, 0xD3, 0xD3),
    ("lightgreen", 0x90, 0xEE, 0x90),
    ("lightgrey", 0xD3, 0xD3, 0xD3),
    ("lightpink", 0xFF, 0xB6, 0xC1),
    ("lightsalmon", 0xFF, 0xA0, 0x7A),
    ("lightseagreen", 0x20, 0xB2, 0xAA),
    ("lightskyblue", 0x87, 0xCE, 0xFA),
    ("lightslategray", 0x77, 0x88, 0x99),
    ("lightslategrey", 0x77, 0x88, 0x99),
    ("lightsteelblue", 0xB0, 0xC4, 0xDE),
    ("lightyellow", 0xFF, 0xFF, 0xE0),
    ("lime", 0x00, 0xFF, 0x00),
    ("limegreen", 0x32, 0xCD, 0x32),
    ("linen", 0xFA, 0xF0, 0xE6),
    ("magenta", 0xFF, 0x00, 0xFF),
    ("maroon", 0x80, 0x00, 0x00),
    ("mediumaquamarine", 0x66, 0xCD, 0xAA),
    ("mediumblue", 0x00, 0x00, 0xCD),
    ("mediumorchid", 0xBA, 0x55, 0xD3),
    ("mediumpurple", 0x93, 0x70, 0xDB),
    ("mediumseagreen", 0x3C, 0xB3, 0x71),
    ("mediumslateblue", 0x7B, 0x68, 0xEE),
    ("mediumspringgreen", 0x00, 0xFA, 0x9A),
    ("mediumturquoise", 0x48, 0xD1, 0xCC),
    ("mediumvioletred", 0xC7, 0x15, 0x85),
    ("midnightblue", 0x19, 0x19, 0x70),
    ("mintcream", 0xF5, 0xFF, 0xFA),
    ("mistyrose", 0xFF, 0xE4, 0xE1),
    ("moccasin", 0xFF, 0xE4, 0xB5),
    ("navajowhite", 0xFF, 0xDE, 0xAD),
    ("navy", 0x00, 0x00, 0x80),
    ("oldlace", 0xFD, 0xF5, 0xE6),
    ("olive", 0x80, 0x80, 0x00),
    ("olivedrab", 0x6B, 0x8E, 0x23),
    ("orange", 0xFF, 0xA5, 0x00),
    ("orangered", 0xFF, 0x45, 0x00),
    ("orchid", 0xDA, 0x70, 0xD6),
    ("palegoldenrod", 0xEE, 0xE8, 0xAA),
    ("palegreen", 0x98, 0xFB, 0x98),
    ("paleturquoise", 0xAF, 0xEE, 0xEE),
    ("palevioletred", 0xDB, 0x70, 0x93),
    ("papayawhip", 0xFF, 0xEF, 0xD5),
    ("peachpuff", 0xFF, 0xDA, 0xB9),
    ("peru", 0xCD, 0x85, 0x3F),
    ("pink", 0xFF, 0xC0, 0xCB),
    ("plum", 0xDD, 0xA0, 0xDD),
    ("powderblue", 0xB0, 0xE0, 0xE6),
    ("purple", 0x80, 0x00, 0x80),
    ("rebeccapurple", 0x66, 0x33, 0x99),
    ("red", 0xFF, 0x00, 0x00),
    ("rosybrown", 0xBC, 0x8F, 0x8F),
    ("royalblue", 0x41, 0x69, 0xE1),
    ("saddlebrown", 0x8B, 0x45, 0x13),
    ("salmon", 0xFA, 0x80, 0x72),
    ("sandybrown", 0xF4, 0xA4, 0x60),
    ("seagreen", 0x2E, 0x8B, 0x57),
    ("seashell", 0xFF, 0xF5, 0xEE),
    ("sienna", 0xA0, 0x52, 0x2D),
    ("silver", 0xC0, 0xC0, 0xC0),
    ("skyblue", 0x87, 0xCE, 0xEB),
    ("slateblue", 0x6A, 0x5A, 0xCD),
    ("slategray", 0x70, 0x80, 0x90),
    ("slategrey", 0x70, 0x80, 0x90),
    ("snow", 0xFF, 0xFA, 0xFA),
    ("springgreen", 0x00, 0xFF, 0x7F),
    ("steelblue", 0x46, 0x82, 0xB4),
    ("tan", 0xD2, 0xB4, 0x8C),
    ("teal", 0x00, 0x80, 0x80),
    ("thistle", 0xD8, 0xBF, 0xD8),
    ("tomato", 0xFF, 0x63, 0x47),
    ("turquoise", 0x40, 0xE0, 0xD0),
    ("violet", 0xEE, 0x82, 0xEE),
    ("wheat", 0xF5, 0xDE, 0xB3),
    ("white", 0xFF, 0xFF, 0xFF),
    ("whitesmoke", 0xF5, 0xF5, 0xF5),
    ("yellow", 0xFF, 0xFF, 0x00),
    ("yellowgreen", 0x9A, 0xCD, 0x32),
];

fn parse_named_color(name: &str) -> Option<Color> {
    NAMED_COLORS
        .iter()
        .find(|(n, ..)| *n == name)
        .map(|&(_, r, g, b)| Color::Rgb { r, g, b })
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
        self.top_left > 0
            || self.top_right > 0
            || self.bottom_right > 0
            || self.bottom_left > 0
    }
}

/// Expand the `border-radius` shorthand (1–4 values) into a [`CornerRadii`].
///
/// Follows the standard CSS 1-/2-/3-/4-value pattern: one value applies to
/// every corner, two set `(top-left & bottom-right) / (top-right &
/// bottom-left)`, three set top-left / (top-right & bottom-left) /
/// bottom-right, four set each corner in clockwise order. Tokens that cannot
/// be parsed as a [`Length`] are treated as `0`.
pub(super) fn parse_border_radius(
    decls: &IndexMap<String, String>,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> CornerRadii {
    let Some(value) = decls.get("border-radius") else {
        return CornerRadii::default();
    };
    let radii: Vec<usize> = value
        .split_whitespace()
        .filter_map(|t| parse_length_token(Some(t), containing_width, font_ctx, viewport))
        .collect();
    match radii.len() {
        0 => CornerRadii::default(),
        1 => CornerRadii {
            top_left: radii[0],
            top_right: radii[0],
            bottom_right: radii[0],
            bottom_left: radii[0],
        },
        2 => CornerRadii {
            top_left: radii[0],
            top_right: radii[1],
            bottom_right: radii[0],
            bottom_left: radii[1],
        },
        3 => CornerRadii {
            top_left: radii[0],
            top_right: radii[1],
            bottom_right: radii[2],
            bottom_left: radii[1],
        },
        _ => CornerRadii {
            top_left: radii[0],
            top_right: radii[1],
            bottom_right: radii[2],
            bottom_left: radii[3],
        },
    }
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
