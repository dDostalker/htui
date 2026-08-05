use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Display cell width of a single `char`.
///
/// Returns `0` for control characters (matching the legacy behaviour — null,
/// BEL, etc. don't consume a column). Width values follow
/// [Unicode Standard Annex #11] (East Asian Width) and pick up emoji codepoints
/// that the previous hand-rolled range table missed (Emoticons,
/// Misc Symbols and Pictographs, Transport, Dingbats, etc.).
///
/// Use [`grapheme_width`] for multi-codepoint constructs (ZWJ sequences such
/// as 👨‍👩‍👧, modifier sequences such as 👍🏽, presentation selectors such
/// as ❤️) — `char_width` only sees the leading codepoint and reports a
/// smaller width than the cluster actually occupies.
///
/// [Unicode Standard Annex #11]: https://www.unicode.org/reports/tr11/
pub fn char_width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Display cell width of an entire grapheme cluster.
///
/// Built on [`UnicodeWidthStr::width`], which honours the full set of
/// emoji-specific ligature rules (fully-qualified ZWJ sequences, modifier
/// sequences, emoji presentation sequences, script-specific ligatures) that
/// cannot be expressed at the `char` level. Falls back to summing per-char
/// widths for ordinary text.
pub fn grapheme_width(g: &str) -> usize {
    UnicodeWidthStr::width(g)
}
