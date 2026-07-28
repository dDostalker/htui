use std::str::FromStr;

use crossterm::style::Color;
use cssparser::{Parser as CssParser, ParserInput, Token};
use indexmap::IndexMap;

use crate::html::box_edges::{Border, BorderStyle, EdgeSizes};

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
    font_size_cells: usize,
    root_font_size_cells: usize,
) -> EdgeSizes {
    let shorthand_raw = decls.get(shorthand);
    let mut edges = parse_shorthand_lengths(
        shorthand_raw.map(|s| s.as_str()),
        containing_width,
        font_size_cells,
        root_font_size_cells,
    );

    // Layer per-side longhands on top (they take precedence).
    let top_raw = decls.get(&format!("{shorthand}-top")).map(|s| s.as_str());
    if let Some(v) = parse_length_token(
        top_raw,
        containing_width,
        font_size_cells,
        root_font_size_cells,
    ) {
        edges.top = v;
    }
    let right_raw = decls
        .get(&format!("{shorthand}-right"))
        .map(|s| s.as_str());
    if let Some(v) = parse_length_token(
        right_raw,
        containing_width,
        font_size_cells,
        root_font_size_cells,
    ) {
        edges.right = v;
    }
    let bottom_raw = decls
        .get(&format!("{shorthand}-bottom"))
        .map(|s| s.as_str());
    if let Some(v) = parse_length_token(
        bottom_raw,
        containing_width,
        font_size_cells,
        root_font_size_cells,
    ) {
        edges.bottom = v;
    }
    let left_raw = decls.get(&format!("{shorthand}-left")).map(|s| s.as_str());
    if let Some(v) = parse_length_token(
        left_raw,
        containing_width,
        font_size_cells,
        root_font_size_cells,
    ) {
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
    font_size_cells: usize,
    root_font_size_cells: usize,
) -> EdgeSizes {
    let Some(value) = value else {
        return EdgeSizes::default();
    };
    let tokens: Vec<&str> = value.split_whitespace().collect();
    let parsed: Vec<usize> = tokens
        .iter()
        .filter_map(|t| {
            parse_length_token(
                Some(t),
                containing_width,
                font_size_cells,
                root_font_size_cells,
            )
        })
        .collect();
    match parsed.len() {
        0 => EdgeSizes::default(),
        1 => EdgeSizes::new(parsed[0], parsed[0], parsed[0], parsed[0]),
        2 => EdgeSizes::new(parsed[0], parsed[1], parsed[0], parsed[1]),
        3 => EdgeSizes::new(parsed[0], parsed[1], parsed[2], parsed[1]),
        _ => EdgeSizes::new(parsed[0], parsed[1], parsed[2], parsed[3]),
    }
}

/// Parse one CSS length token to an absolute cell count.
///
/// `auto` / missing / unsupported unit → `None` (caller decides what default
/// to apply).
fn parse_length_token(
    value: Option<&str>,
    containing_width: usize,
    font_size_cells: usize,
    root_font_size_cells: usize,
) -> Option<usize> {
    let raw = value?;
    match Length::parse(raw)? {
        Length::Auto => None,
        len => Some(len.resolve(containing_width, font_size_cells, root_font_size_cells)),
    }
}

/// Parse a `border-width` value to cells. Mirrors [`parse_length_token`]; any
/// value CSS accepts as a length works (including `thin` / `medium` / `thick`
/// once added to [`Length::parse`]).
pub(super) fn parse_border_width(
    value: Option<&str>,
    containing_width: usize,
    font_size_cells: usize,
    root_font_size_cells: usize,
) -> usize {
    parse_length_token(
        value,
        containing_width,
        font_size_cells,
        root_font_size_cells,
    )
    .unwrap_or(0)
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
    font_size_cells: usize,
    root_font_size_cells: usize,
) -> Border {
    if let Some(value) = side_raw {
        if let Some(parsed) =
            parse_border_shorthand(value, containing_width, font_size_cells, root_font_size_cells)
        {
            return parsed;
        }
        // `border-top` was present but unparseable — fall through to next
        // priority rather than dropping the side entirely.
    }
    if let Some(value) = shorthand_raw {
        if let Some(parsed) =
            parse_border_shorthand(value, containing_width, font_size_cells, root_font_size_cells)
        {
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
    font_size_cells: usize,
    root_font_size_cells: usize,
) -> Option<Border> {
    let mut width: Option<usize> = None;
    let mut style: Option<BorderStyle> = None;
    let mut color: Option<Color> = None;

    for token in value.split_whitespace() {
        if let Some(w) = parse_length_token(
            Some(token),
            containing_width,
            font_size_cells,
            root_font_size_cells,
        ) {
            width = Some(w);
            continue;
        }
        let lower = token.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "solid" | "dashed" | "dotted" | "double" | "none" | "hidden" | "groove" | "ridge"
                | "inset" | "outset"
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
///
/// Rejected (returns `None` from [`Length::parse`]):
/// - `vw`, `vh`, `vmin`, `vmax` — the terminal has no viewport. Use `%`.
/// - `pt`, `cm`, `mm`, `in`, `pc` — no DPI. Use `px` (= cells).
/// - Negative or signed (`+`, `-`) values — CSS forbids these for `width`.
/// - Trailing junk after the value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    /// `auto` — defer to the containing block.
    Auto,
    /// Absolute cell count (CSS `40px` or bare `40`).
    Cells(usize),
    /// Percentage 0..=100 of the containing block (CSS `50%`).
    Pct(f32),
    /// Em multiplier (CSS `2em`). 1em == 1 cell on the terminal grid.
    Em(f32),
    /// Root-em multiplier (CSS `2rem`). Same semantics as `em` here — the
    /// terminal has no root font-size distinction.
    Rem(f32),
}

/// Owned mirror of the cssparser [`Token`] variants we accept. Lets
/// [`Length::parse`] release its borrow on the parser before checking for
/// trailing junk with `is_exhausted()`.
#[derive(Debug, Clone, Copy)]
enum PreLength {
    Cells(f32),
    Pct(f32),
    Em(f32),
    Rem(f32),
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
                    "px" => PreLength::Cells(n),
                    "em" => PreLength::Em(n),
                    "rem" => PreLength::Rem(n),
                    // vw / vh / vmin / vmax / pt / cm / mm / in / pc / ...
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
            PreLength::Cells(n) => Length::Cells(n.round() as usize),
            PreLength::Pct(p) => Length::Pct(p),
            PreLength::Em(n) => Length::Em(n),
            PreLength::Rem(n) => Length::Rem(n),
        })
    }

    /// Resolve the length into a concrete column count given the containing
    /// block's width in cells and the active font-size context.
    ///
    /// - `Auto`              → `containing` (fill the container)
    /// - `Cells(n)`          → `n`
    /// - `Pct(p)`            → round(containing * p / 100), min 1
    /// - `Em(n)`             → round(n * font_size_cells), clamped at 0
    /// - `Rem(n)`            → round(n * root_font_size_cells), clamped at 0
    ///
    /// `em` resolves against the **current element's** effective font-size
    /// (in cells); `rem` resolves against the **root's** font-size. On a
    /// terminal the root font-size is 1 cell by default — it grows when the
    /// document root sets an explicit `font-size` rule.
    ///
    /// Floats round **away from zero** (Rust's `f32::round`), so `0.5em` at
    /// font-size=2 yields 1 cell and `2.5em` at font-size=2 yields 5 cells —
    /// never zero, never half.
    pub fn resolve(
        &self,
        containing: usize,
        font_size_cells: usize,
        root_font_size_cells: usize,
    ) -> usize {
        match self {
            Length::Auto => containing,
            &Length::Cells(n) => n,
            &Length::Pct(p) => {
                let raw = (containing as f32 * p / 100.0).round();
                (raw as usize).max(1)
            }
            &Length::Em(n) => (n * font_size_cells as f32).round().max(0.0) as usize,
            &Length::Rem(n) => (n * root_font_size_cells as f32).round().max(0.0) as usize,
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
