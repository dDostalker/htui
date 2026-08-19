use std::{
    fmt::{self, Display},
    ops::{Deref, DerefMut},
};

use bitflags::bitflags;
use crossterm::style::{Color, StyledContent, Stylize};

bitflags! {
    /// Display flags applied to a [`Block`].
    ///
    /// Aggregates the independent CSS font/text properties we render
    /// (`font-weight: bold`, `font-style: italic`, `text-decoration:
    /// underline`, `text-decoration: line-through`) into one bitfield so a
    /// single character cell can carry any combination without growing the
    /// `Block` shape on every new style attribute.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct FontStyle: u8 {
        const BOLD        = 0b0000_0001;
        const ITALIC      = 0b0000_0010;
        const UNDERLINED  = 0b0000_0100;
        const CROSSED_OUT = 0b0000_1000;
    }
}

#[derive(Debug, Clone)]
pub struct Block {
    pub word: String,
    pub fcolor: Option<Color>,
    pub bcolor: Option<Color>,
    pub style: FontStyle,
}

impl Deref for Block {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.word
    }
}

impl DerefMut for Block {
    fn deref_mut(&mut self) -> &mut str {
        &mut self.word
    }
}

impl Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            styled_string(&self.word, self.style, self.fcolor, self.bcolor)
        )
    }
}

/// Apply a [`FontStyle`] bitfield and optional foreground/background colors to
/// `text` and return the [`StyledContent`] wrapper (borrowing `text`) ready
/// for display.
///
/// Used by both [`Block`]'s `Display` impl and the render path so the SGR
/// emission lives in one place. Returning the wrapper instead of a formatted
/// `String` lets callers write the ANSI bytes straight into their output
/// buffer (`Vec<u8>` via `io::Write`, or a `fmt::Formatter` via `fmt::Write`)
/// without an intermediate heap allocation per styled run.
pub(crate) fn styled_string(
    text: &str,
    style: FontStyle,
    fg: Option<Color>,
    bg: Option<Color>,
) -> StyledContent<&str> {
    let mut styled = text.stylize();
    if style.contains(FontStyle::BOLD) {
        styled = styled.bold();
    }
    if style.contains(FontStyle::ITALIC) {
        styled = styled.italic();
    }
    if style.contains(FontStyle::UNDERLINED) {
        styled = styled.underlined();
    }
    if style.contains(FontStyle::CROSSED_OUT) {
        styled = styled.crossed_out();
    }
    match (fg, bg) {
        (Some(f), Some(b)) => styled.with(f).on(b),
        (Some(f), None) => styled.with(f),
        (None, Some(b)) => styled.on(b),
        (None, None) => styled,
    }
}

impl Default for Block {
    /// A blank cell: a single space with no color, no style, and no
    /// foreground/background. Used as the starting state for the canvas
    /// (`vec![Block::default(); n]`) and as a reusable template by the
    /// painters so each per-cell fill no longer needs to spell out
    /// `" ".to_string()` at the call site.
    ///
    /// The `word` buffer is pre-sized to 32 bytes so painters can `clear()`
    /// and `push_str(g)` any single grapheme cluster without triggering a
    /// reallocation. Practically every cluster (including the longest
    /// ZWJ-flag sequences we expect, such as 👨‍👩‍👧‍👦 plus regional
    /// indicators) fits comfortably under 32 bytes; background fills keep
    /// the trailing capacity wasted (harmless: 16K × 32 ≈ 512 KB once).
    /// Border glyphs (`─`, `│`, `┌`, `┐`, `└`, `┘` are 3 bytes) and single
    /// emoji codepoints (≤ 4 bytes) also fit without reallocating.
    fn default() -> Self {
        let mut word = String::with_capacity(32);
        word.push(' ');
        Self {
            word,
            fcolor: None,
            bcolor: None,
            style: FontStyle::empty(),
        }
    }
}

impl Block {
    pub fn new(
        word: String,
        fcolor: Option<Color>,
        bcolor: Option<Color>,
        style: FontStyle,
    ) -> Self {
        Self {
            word,
            fcolor,
            bcolor,
            style,
        }
    }
}
