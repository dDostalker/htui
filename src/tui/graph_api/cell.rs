use std::{
    fmt::{self, Display},
    ops::{Deref, DerefMut},
};

use bitflags::bitflags;
use crossterm::style::{Color, Stylize};

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
/// `text` and return the rendered ANSI sequence as a `String`.
///
/// Used by both [`Block`]'s `Display` impl and `TermLayout::render_to_bytes`
/// so the SGR emission lives in one place.
pub(super) fn styled_string(
    text: &str,
    style: FontStyle,
    fg: Option<Color>,
    bg: Option<Color>,
) -> String {
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
    styled = match (fg, bg) {
        (Some(f), Some(b)) => styled.with(f).on(b),
        (Some(f), None) => styled.with(f),
        (None, Some(b)) => styled.on(b),
        (None, None) => styled,
    };
    format!("{styled}")
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
