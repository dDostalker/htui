//! One inline segment of a block-level element.

use crossterm::style::Color;
use unicode_segmentation::UnicodeSegmentation;

use crate::graphics::FontStyle;
use crate::tdom::node::NodeId;

/// One inline segment of a block-level element.
///
/// A block can hold multiple `InlineRun`s when its inline tree contains
/// `<span>`s with distinct styles. Each run carries the resolved style, fg
/// and bg that should be applied to its text. The paint phase walks `runs`
/// in order and emits one styled cell per `char_width` column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineRun {
    pub text: String,
    pub style: FontStyle,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    /// `true` for a `<br>` marker run (text is a single `\n` of zero cell
    /// width): the line breaker treats it as an unconditional forced break
    /// in every `white-space` mode, where a plain newline would collapse.
    pub force_break: bool,
    /// The DOM element that produced this run — set for runs inside an
    /// inline element with its own hit semantics like `<label>` or `<a>`, so
    /// the hit map can give that element its own clickable region (its text
    /// spans a slice of a block box). `None` for plain text / synthesized
    /// runs.
    pub node_id: Option<NodeId>,
    /// Cells added after every grapheme (CSS `letter-spacing`, resolved).
    /// The spacing columns are transparent to the text data — they widen
    /// every width measurement and the painter emits them as background-
    /// only columns, so copy / hit geometry stay aligned with the glyphs.
    pub letter_spacing: usize,
    /// Signed delta applied to every space grapheme (CSS `word-spacing`).
    /// A negative value tightens words; each space floors at zero width.
    pub word_spacing: isize,
}

impl InlineRun {
    /// Cells one grapheme of this run advances at `font_size_cells`:
    /// `max(grapheme_width, font_size)` + letter spacing, with positive word
    /// spacing riding on spaces (a negative value clamps to zero — the
    /// painter's cells cannot overlap, so tightening below the natural
    /// space is not expressible).
    pub fn grapheme_advance(&self, g: &str, font_size_cells: usize) -> usize {
        use crate::graphics::grapheme_width;
        let base = grapheme_width(g).max(font_size_cells.max(1));
        let word = if g == " " {
            self.word_spacing.max(0) as usize
        } else {
            0
        };
        base + word + self.letter_spacing
    }

    /// Create an inline run with the given text, style, and colors.
    pub fn new(
        text: impl Into<String>,
        style: FontStyle,
        fg: Option<Color>,
        bg: Option<Color>,
    ) -> Self {
        Self {
            text: text.into(),
            style,
            fg,
            bg,
            force_break: false,
            node_id: None,
            letter_spacing: 0,
            word_spacing: 0,
        }
    }

    /// A run attributed to an inline element's node id (e.g. the text inside
    /// a `<label>`), so the hit map can resolve clicks on that element.
    pub fn owned(
        text: impl Into<String>,
        style: FontStyle,
        fg: Option<Color>,
        bg: Option<Color>,
        node_id: Option<NodeId>,
    ) -> Self {
        Self {
            text: text.into(),
            style,
            fg,
            bg,
            force_break: false,
            node_id,
            letter_spacing: 0,
            word_spacing: 0,
        }
    }

    /// [`Self::owned`] with resolved letter / word spacing attached — the
    /// collector's form once the block's `letter-spacing` / `word-spacing`
    /// have been resolved against the font context.
    pub fn owned_spaced(
        text: impl Into<String>,
        style: FontStyle,
        fg: Option<Color>,
        bg: Option<Color>,
        node_id: Option<NodeId>,
        letter_spacing: usize,
        word_spacing: isize,
    ) -> Self {
        Self {
            letter_spacing,
            word_spacing,
            ..Self::owned(text, style, fg, bg, node_id)
        }
    }

    /// A forced line break run (`<br>`): text is a single `\n` (zero cell
    /// width), `force_break` set so wrapping splits on it unconditionally.
    pub fn forced_break(style: FontStyle, fg: Option<Color>, bg: Option<Color>) -> Self {
        Self {
            text: "\n".to_string(),
            style,
            fg,
            bg,
            force_break: true,
            node_id: None,
            letter_spacing: 0,
            word_spacing: 0,
        }
    }

    /// Returns the cell width this run occupies at the default
    /// (un-expanded) font-size: `grapheme_width` per grapheme cluster, so a
    /// wide CJK glyph (or a 2-column emoji) at the end of one run does not
    /// collide with the first cell of the next. Layouts that need the width
    /// at a larger font-size should call [`Self::width_at`].
    ///
    /// Iterating graphemes rather than `char`s lets multi-codepoint emoji
    /// (`👨‍👩‍👧`, `👍🏽`, `❤️`) count as a single cluster — `char_width`
    /// would either over- or under-count them depending on how many
    /// combining / ZWJ codepoints the cluster contains.
    pub fn width(&self) -> usize {
        self.text
            .graphemes(true)
            .map(|g| self.grapheme_advance(g, 1))
            .sum()
    }

    /// Cell width this run occupies at the supplied `font_size_cells`. Each
    /// grapheme is treated as `max(grapheme_width(g), font_size_cells)`
    /// cells wide (matching the expansion the paint phase applies), so a
    /// heading with font-size=2 has its `text-align` measured against the
    /// expanded width — not the single-cell-per-char default. Letter / word
    /// spacing ride along via [`Self::grapheme_advance`].
    pub fn width_at(&self, font_size_cells: usize) -> usize {
        self.text
            .graphemes(true)
            .map(|g| self.grapheme_advance(g, font_size_cells))
            .sum()
    }

    /// Returns true when the run has no visible text.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}
