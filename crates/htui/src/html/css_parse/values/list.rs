//! List-marker values (CSS `list-style-type` and the `list-style` shorthand).

/// The marker style of a list item (CSS `list-style-type`).
///
/// The terminal renders markers as text prefixes, so every style maps to a
/// string generator over the item's 1-based ordinal. `None` suppresses the
/// marker entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListStyleType {
    /// `disc` — `•` (the `<ul>` default).
    #[default]
    Disc,
    /// `circle` — `○`.
    Circle,
    /// `square` — `▪`.
    Square,
    /// `none` — no marker.
    None,
    /// `decimal` — `1.`, `2.`, … (the `<ol>` default).
    Decimal,
    /// `lower-alpha` / `lower-latin` — `a.`, `b.`, …
    LowerAlpha,
    /// `upper-alpha` / `upper-latin` — `A.`, `B.`, …
    UpperAlpha,
    /// `lower-roman` — `i.`, `ii.`, …
    LowerRoman,
    /// `upper-roman` — `I.`, `II.`, …
    UpperRoman,
}

impl ListStyleType {
    /// The marker prefix for the item with 1-based `ordinal` (already
    /// carrying its trailing space, like the built-in `• ` marker).
    pub fn marker(self, ordinal: usize) -> String {
        match self {
            ListStyleType::Disc => "• ".to_string(),
            ListStyleType::Circle => "○ ".to_string(),
            ListStyleType::Square => "▪ ".to_string(),
            ListStyleType::None => String::new(),
            ListStyleType::Decimal => format!("{ordinal}. "),
            ListStyleType::LowerAlpha => format!("{}. ", to_alpha(ordinal).to_ascii_lowercase()),
            ListStyleType::UpperAlpha => format!("{}. ", to_alpha(ordinal)),
            ListStyleType::LowerRoman => format!("{} ", to_roman(ordinal).to_ascii_lowercase()),
            ListStyleType::UpperRoman => format!("{} ", to_roman(ordinal)),
        }
    }
}

/// Bijective base-26 (spreadsheet columns): 1 → `A`, 26 → `Z`, 27 → `AA`.
fn to_alpha(mut n: usize) -> String {
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(b'A' + (n % 26) as u8);
        n /= 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Standard subtractive Roman numerals (1–3999 monotone; larger ordinals
/// fall back to the overline-less repetition the string allows).
fn to_roman(mut n: usize) -> String {
    const PAIRS: [(usize, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (value, glyph) in PAIRS {
        while n >= value {
            out.push_str(glyph);
            n -= value;
        }
    }
    out
}

/// Parse a `list-style-type` value (ASCII case-insensitive).
pub fn parse_list_style_type(s: &str) -> Option<ListStyleType> {
    match s.trim().to_ascii_lowercase().as_str() {
        "disc" => Some(ListStyleType::Disc),
        "circle" => Some(ListStyleType::Circle),
        "square" => Some(ListStyleType::Square),
        "none" => Some(ListStyleType::None),
        "decimal" => Some(ListStyleType::Decimal),
        "lower-alpha" | "lower-latin" => Some(ListStyleType::LowerAlpha),
        "upper-alpha" | "upper-latin" => Some(ListStyleType::UpperAlpha),
        "lower-roman" => Some(ListStyleType::LowerRoman),
        "upper-roman" => Some(ListStyleType::UpperRoman),
        _ => None,
    }
}

/// Whether a word is a `list-style-type` keyword (used by the `list-style`
/// shorthand to pick its type component out of the value list).
pub fn is_list_style_type_keyword(word: &str) -> bool {
    parse_list_style_type(word).is_some()
}
