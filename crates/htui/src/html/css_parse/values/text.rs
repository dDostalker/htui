use super::length::{FontContext, Length, LengthContext, Viewport};

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
/// Parse a `text-transform` keyword.
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
/// Parse a `line-height` value (`normal`, unitless number, or length).
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
/// Parse a `white-space` keyword.
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
/// Parse a `word-break` keyword.
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
/// Parse an `overflow-wrap` keyword.
pub fn parse_overflow_wrap(s: &str) -> Option<OverflowWrap> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(OverflowWrap::Normal),
        "break-word" => Some(OverflowWrap::BreakWord),
        "anywhere" => Some(OverflowWrap::Anywhere),
        _ => None,
    }
}

/// Text-overflow behaviour for a clipped (`overflow: hidden`-family) box
/// whose content still exceeds the content width (CSS `text-overflow`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextOverflow {
    /// `clip` (the default) — the overflow simply stops at the clip edge.
    #[default]
    Clip,
    /// `ellipsis` — the clipped line ends in a `…` glyph.
    Ellipsis,
}

/// Parse a `text-overflow` keyword (`string` values are not supported).
pub fn parse_text_overflow(s: &str) -> Option<TextOverflow> {
    match s.trim().to_ascii_lowercase().as_str() {
        "clip" => Some(TextOverflow::Clip),
        "ellipsis" => Some(TextOverflow::Ellipsis),
        _ => None,
    }
}
