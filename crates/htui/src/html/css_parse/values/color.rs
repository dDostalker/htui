use crossterm::style::Color;

/// Parse a CSS color string into a [`crossterm::style::Color`].
///
/// Supported formats:
/// - Hexadecimal: `#RRGGBB`, `#RGB`, `#RRGGBBAA`, `#RGBA` (alpha dropped)
/// - Functional: `rgb()` / `rgba()` / `hsl()` / `hsla()`, with comma or
///   space-separated syntax and an optional `/ alpha` component
///   (`rgb(255 0 0 / 50%)`). `rgb`/`rgba` components may be integers
///   (0–255, out-of-range values clamp) or percentages; `hsl` hue is in
///   degrees (negative values wrap) and saturation / lightness are
///   percentages.
/// - The 148 CSS named colors from Color Module Level 4 (e.g. `red`,
///   `rebeccapurple`), resolved to their exact sRGB value so they render
///   identically on any terminal theme.
///
/// Alpha is parsed and validated but **dropped**: the terminal has no
/// alpha compositing (the background is the theme's, unknown to the
/// renderer), so `rgba(…)` / `hsla(…)` / `#RRGGBBAA` paint fully opaque.
/// `transparent` returns `None` — callers treat it as "no paint", which is
/// exactly what backgrounds / borders want (`background-color: transparent`
/// lets the parent show through).
///
/// Unknown formats return `None` so callers can treat the declaration as
/// invalid (CSS drops unsupported values) instead of silently substituting
/// a colour.
#[doc(hidden)]
pub fn parse_color_str(s: &str) -> Option<Color> {
    let lower = s.trim().to_lowercase();

    // 1. Hexadecimal #RRGGBB / #RGB / #RRGGBBAA / #RGBA
    if let Some(hex) = lower.strip_prefix('#') {
        return parse_hex_color(hex);
    }

    // 2. Functional colors. Longer names first so `rgba` matches before
    //    `rgb` (and `hsla` before `hsl`).
    for (name, has_alpha) in [
        ("rgba", true),
        ("rgb", false),
        ("hsla", true),
        ("hsl", false),
    ] {
        if let Some(rest) = lower.strip_prefix(name)
            && let Some(inner) = rest.strip_prefix('(').and_then(|s| s.strip_suffix(')'))
        {
            return parse_functional_color(name, has_alpha, inner);
        }
    }

    // 3. CSS named colors (see [`NAMED_COLORS`]). `transparent` and
    //    `currentColor` fall through here and return `None`.
    parse_named_color(&lower)
}

fn parse_hex_color(hex: &str) -> Option<Color> {
    let hex = hex.trim();
    let (r, g, b) = if hex.len() == 8 {
        // #RRGGBBAA — the alpha channel is validated then dropped.
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        u8::from_str_radix(&hex[6..8], 16).ok()?;
        (r, g, b)
    } else if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        (r, g, b)
    } else if hex.len() == 4 {
        // #RGBA — alpha validated then dropped.
        let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
        let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
        let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
        u8::from_str_radix(&hex[3..4], 16).ok()?;
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

/// Parse the body of a functional color (`rgb` / `rgba` / `hsl` / `hsla`)
/// — the text between the parentheses.
///
/// Accepts the legacy comma syntax (`rgb(255, 0, 0)`, `rgba(255, 0, 0, 0.5)`)
/// and the CSS Color 4 space syntax (`rgb(255 0 0)`, `rgb(255 0 0 / 50%)`).
fn parse_functional_color(kind: &str, has_alpha: bool, inner: &str) -> Option<Color> {
    // Split off an optional `/ alpha` component.
    let (body, alpha) = match inner.find('/') {
        Some(i) => (&inner[..i], Some(inner[i + 1..].trim())),
        None => (inner, None),
    };
    if let Some(a) = alpha {
        // Validated, then dropped — the terminal cannot composite.
        parse_alpha(a)?;
    }

    let parts: Vec<&str> = if body.contains(',') {
        body.split(',').map(str::trim).collect()
    } else {
        body.split_whitespace().collect()
    };

    match kind {
        "rgb" | "rgba" => {
            // 3 components, or 4 when legacy comma syntax carries the alpha.
            if parts.len() == 3 {
                let r = parse_rgb_component(parts[0])?;
                let g = parse_rgb_component(parts[1])?;
                let b = parse_rgb_component(parts[2])?;
                Some(Color::Rgb { r, g, b })
            } else if parts.len() == 4 && has_alpha {
                let r = parse_rgb_component(parts[0])?;
                let g = parse_rgb_component(parts[1])?;
                let b = parse_rgb_component(parts[2])?;
                parse_alpha(parts[3])?;
                Some(Color::Rgb { r, g, b })
            } else {
                None
            }
        }
        "hsl" | "hsla" => {
            if parts.len() == 3 {
                let h = parse_hue(parts[0])?;
                let s = parse_percent(parts[1])?;
                let l = parse_percent(parts[2])?;
                let (r, g, b) = hsl_to_rgb(h, s, l);
                Some(Color::Rgb { r, g, b })
            } else if parts.len() == 4 && has_alpha {
                let h = parse_hue(parts[0])?;
                let s = parse_percent(parts[1])?;
                let l = parse_percent(parts[2])?;
                parse_alpha(parts[3])?;
                let (r, g, b) = hsl_to_rgb(h, s, l);
                Some(Color::Rgb { r, g, b })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// An `rgb()` / `rgba()` component: an integer 0–255 (out-of-range values
/// clamp, per CSS Color 4) or a percentage.
fn parse_rgb_component(tok: &str) -> Option<u8> {
    let tok = tok.trim();
    if let Some(pct) = tok.strip_suffix('%') {
        let v = pct.trim().parse::<f64>().ok()?;
        return Some((v.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u8);
    }
    let v = tok.parse::<f64>().ok()?;
    Some(v.clamp(0.0, 255.0).round() as u8)
}

/// An `hsl()` hue: degrees (`120`, `120deg`), negative values wrap around
/// the colour wheel.
fn parse_hue(tok: &str) -> Option<f64> {
    let tok = tok.trim();
    if let Some(v) = tok.strip_suffix("deg") {
        v.trim().parse::<f64>().ok()
    } else if let Some(v) = tok.strip_suffix("turn") {
        v.trim().parse::<f64>().ok().map(|t| t * 360.0)
    } else {
        tok.parse::<f64>().ok()
    }
}

/// A percentage component (saturation / lightness). Accepts `50%` and, for
/// leniency, a bare number treated as a percentage.
fn parse_percent(tok: &str) -> Option<f64> {
    let tok = tok.trim();
    let v = tok.strip_suffix('%').unwrap_or(tok).parse::<f64>().ok()?;
    Some(v / 100.0)
}

/// Validate an alpha value: `0`–`1` or `0%`–`100%`. The value itself is
/// discarded by the caller (no terminal compositing).
fn parse_alpha(tok: &str) -> Option<()> {
    let tok = tok.trim();
    let v = if let Some(pct) = tok.strip_suffix('%') {
        pct.trim().parse::<f64>().ok()? / 100.0
    } else {
        tok.parse::<f64>().ok()?
    };
    if (0.0..=1.0).contains(&v) {
        Some(())
    } else {
        None
    }
}

/// Convert HSL to sRGB (the standard CSS algorithm).
///
/// `h` is in degrees (any sign — wrapped), `s` / `l` are fractions of 1.
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let h = ((h % 360.0) + 360.0) % 360.0 / 360.0;
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);
    if s == 0.0 {
        let v = (l * 255.0).round() as u8;
        return (v, v, v);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hue = |mut t: f64| -> f64 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let r = (hue(h + 1.0 / 3.0) * 255.0).round() as u8;
    let g = (hue(h) * 255.0).round() as u8;
    let b = (hue(h - 1.0 / 3.0) * 255.0).round() as u8;
    (r, g, b)
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
