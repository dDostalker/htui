//! CSS Text Level 3 line-breaker for terminal inline runs.
//!
//! Walks a flat list of [`InlineRun`]s and emits a vector of visual
//! lines, each no wider than `max_width` terminal cells. The break
//! rules are driven by `white_space`, `word_break`, and
//! `overflow_wrap` per the CSS Text Module Level 3 specification
//! (terminal simplification: greedy best-fit, not minimum-raggedness).
//!
//! Visible from the layout phase (block + flex) via
//! [`crate::layout::layoutbox::wrap_runs`] /
//! [`crate::layout::layoutbox::wrapped_size`].

use unicode_segmentation::UnicodeSegmentation;

use crossterm::style::Color;

use crate::{
    html::css_parse::{OverflowWrap, WhiteSpace, WordBreak},
    layout::InlineRun,
    tui::graph_api::{FontStyle, grapheme_width},
};

/// One token extracted from the input run list.
///
/// `Word` owns its text so the line-breaker can copy it into the
/// per-line `InlineRun` outputs without lifetime gymnastics.
#[derive(Debug, Clone)]
enum Token {
    /// A run of horizontal whitespace graphemes (`' '`, `'\t'`, NBSP).
    /// `_count` is the number of graphemes; `width` is the total cell
    /// width after font-size expansion. `_count` is currently unused
    /// after the `apply_white_space` collapsing phase but is kept on
    /// the variant for symmetry with `width` and future use.
    Space { width: usize, _count: usize },
    /// A forced break (`'\n'`).
    Newline,
    /// A maximal non-whitespace grapheme run lifted from one source
    /// `InlineRun`. The styling is carried alongside so the output
    /// lines preserve span boundaries.
    Word {
        text: String,
        width: usize,
        style: FontStyle,
        fg: Option<Color>,
        bg: Option<Color>,
    },
}

/// Returns `true` when `g` is one of the horizontal whitespace
/// graphemes the HTML spec treats as collapsible when `white-space`
/// is `normal` / `nowrap` / `pre-line`.
fn is_horizontal_ws(g: &str) -> bool {
    matches!(g, " " | "\t" | "\u{00A0}")
}

/// Returns `true` when `c` is in any of the major CJK ranges for
/// which browsers introduce break opportunities under
/// `word-break: normal`.
///
/// Covers CJK Unified Ideographs (basic + extensions), compatibility
/// forms, Hiragana / Katakana, Hangul Syllables, full-width ASCII
/// forms, CJK punctuation, Kangxi Radicals, and Tangut. The
/// table-based detection mirrors the character classes browsers
/// use for the CJK line-break opportunity rule.
pub fn is_cjk_break_opportunity(c: char) -> bool {
    let cp = c as u32;
    matches!(cp,
        0x4E00..=0x9FFF       // CJK Unified Ideographs
        | 0x3400..=0x4DBF     // CJK Unified Ideographs Extension A
        | 0x20000..=0x2A6DF   // Extension B
        | 0x2A700..=0x2B73F   // Extension C
        | 0x2B740..=0x2B81F   // Extension D
        | 0x2B820..=0x2CEAF   // Extension E
        | 0x2EBF0..=0x2EE5D   // Extension I
        | 0xF900..=0xFAFF     // CJK Compatibility Ideographs
        | 0x2F800..=0x2FA1F   // CJK Compatibility Supplement
        | 0x3040..=0x309F     // Hiragana
        | 0x30A0..=0x30FF     // Katakana
        | 0xAC00..=0xD7AF     // Hangul Syllables
        | 0x3000..=0x303F     // CJK Symbols & Punctuation
        | 0xFF00..=0xFFEF     // Half-width / Full-width Forms
        | 0x2E80..=0x2EFF     // CJK Radicals Supplement
        | 0x31C0..=0x31EF     // CJK Strokes
        | 0x2F00..=0x2FDF     // Kangxi Radicals
        | 0x17000..=0x187FF   // Tangut
        | 0x1B170..=0x1B2FF   // Tangut Components
        | 0x18B00..=0x18CFF   // Khitan Small Script
    )
}

/// Tokenize one `InlineRun`'s text into a flat sequence of `Token`s
/// appended to `out`. Words preserve the source run's styling; the
/// `Space` token only stores total cell width and grapheme count.
///
/// CJK Unified Ideographs (and other East-Asian wide characters) get
/// their own one-grapheme `Word` token each: under the browser-default
/// `word-break: normal` the line-breaker must allow a break between any
/// two adjacent CJK characters, so we cannot merge them into a single
/// word the way we merge runs of ASCII letters. CJK runs of consecutive
/// characters therefore become a sequence of single-grapheme `Word`
/// tokens; the wrap phase then fits them greedily into the available
/// width and naturally breaks at character boundaries.
///
/// `word-break: keep-all` flips this for CJK text: the entire CJK run
/// is accumulated into a single `Word` so the wrap phase treats the
/// sequence as unbreakable. The text is then emitted as one wide word
/// (overflowing the line if it cannot fit) — the same behaviour
/// browsers ship for `keep-all`.
fn tokenize_run(
    run: &InlineRun,
    font_size_cells: usize,
    word_break: WordBreak,
    out: &mut Vec<Token>,
) {
    let font = font_size_cells.max(1);

    // 0 = "currently accumulating spaces", 1 = "currently accumulating a word"
    let mut pending_kind: Option<u8> = None;
    let mut pending_text = String::new();
    let mut pending_count: usize = 0;
    let mut pending_width: usize = 0;

    let flush = |kind: Option<u8>,
                     text: &mut String,
                     count: &mut usize,
                     width: &mut usize,
                     run: &InlineRun,
                     out: &mut Vec<Token>| {
        if let Some(k) = kind {
            if *count == 0 || text.is_empty() {
                return;
            }
            if k == 0 {
                out.push(Token::Space {
                    width: *width,
                    _count: *count,
                });
                // Drop the space graphemes from the accumulator so the
                // next pending kind starts with a clean string. (Without
                // this, the next `Word` token inherits the spaces as
                // leading characters — the "row" example becomes "
                // row", which paints an extra leading space.)
                text.clear();
            } else {
                out.push(Token::Word {
                    text: std::mem::take(text),
                    width: *width,
                    style: run.style,
                    fg: run.fg,
                    bg: run.bg,
                });
            }
        }
        *count = 0;
        *width = 0;
    };

    for g in run.text.graphemes(true) {
        if g == "\n" {
            // Close whatever is pending, then push the forced break.
            flush(pending_kind, &mut pending_text, &mut pending_count, &mut pending_width, run, out);
            pending_kind = None;
            out.push(Token::Newline);
            continue;
        }
        let w = grapheme_width(g).max(font);
        if is_horizontal_ws(g) {
            if pending_kind != Some(0) {
                flush(pending_kind, &mut pending_text, &mut pending_count, &mut pending_width, run, out);
                pending_kind = Some(0);
            }
            pending_text.push_str(g);
            pending_count += 1;
            pending_width += w;
        } else if word_break != WordBreak::KeepAll
            && g.chars().next().map_or(false, is_cjk_break_opportunity)
        {
            // CJK (or full-width) grapheme under `normal` / `break-all`:
            // flush whatever is pending so the boundary before this
            // character is recognised as a wrap opportunity, then emit
            // it as its own one-grapheme word.
            flush(pending_kind, &mut pending_text, &mut pending_count, &mut pending_width, run, out);
            pending_kind = None;
            out.push(Token::Word {
                text: g.to_string(),
                width: w,
                style: run.style,
                fg: run.fg,
                bg: run.bg,
            });
        } else {
            // Non-CJK, or CJK under `keep-all`: accumulate as one word
            // so the wrap phase treats the run as unbreakable.
            if pending_kind != Some(1) {
                flush(pending_kind, &mut pending_text, &mut pending_count, &mut pending_width, run, out);
                pending_kind = Some(1);
            }
            pending_text.push_str(g);
            pending_count += 1;
            pending_width += w;
        }
    }
    flush(pending_kind, &mut pending_text, &mut pending_count, &mut pending_width, run, out);
}

/// Apply the `white-space` collapsing rules to a token stream and
/// emit "logical lines": sequences of tokens that must appear on the
/// same line (unless wrapping kicks in). Forced breaks (`Newline`)
/// separate logical lines.
fn apply_white_space(tokens: Vec<Token>, ws: WhiteSpace) -> Vec<Vec<Token>> {
    if ws.preserves_whitespace() {
        // `pre` / `pre-wrap`: every Newline is a forced break; spaces
        // are preserved as-is.
        let mut lines: Vec<Vec<Token>> = vec![Vec::new()];
        for t in tokens {
            if matches!(t, Token::Newline) {
                lines.push(Vec::new());
            } else {
                lines.last_mut().unwrap().push(t);
            }
        }
        return lines;
    }
    // `normal` / `nowrap` / `pre-line`: collapse runs of Space (and
    // any Newlines they contain, except `pre-line` where standalone
    // newlines are forced breaks).
    let mut lines: Vec<Vec<Token>> = vec![Vec::new()];
    for t in tokens {
        match t {
            Token::Newline => {
                if matches!(ws, WhiteSpace::PreLine) {
                    lines.push(Vec::new());
                }
                // `normal` / `nowrap` swallow the newline as a
                // collapsible-space boundary.
            }
            Token::Space { .. } => {
                let last = lines.last().unwrap().last();
                if !matches!(last, Some(Token::Space { .. })) {
                    lines.last_mut().unwrap().push(Token::Space { width: 1, _count: 1 });
                }
            }
            Token::Word { .. } => lines.last_mut().unwrap().push(t),
        }
    }
    lines
}

/// Width of `text` in cells after font-size expansion, computed from
/// the grapheme stream.
fn grapheme_width_sum(text: &str, font_size_cells: usize) -> usize {
    let font = font_size_cells.max(1);
    text.graphemes(true).map(|g| grapheme_width(g).max(font)).sum()
}

/// Wrap a single logical line onto one or more visual lines. The
/// caller has already applied the `white_space` collapsing rules so
/// each `Space` token here represents a single-cell break opportunity
/// (or a run of spaces that the previous phase kept together).
fn wrap_logical_line(
    logical: Vec<Token>,
    max_width: usize,
    font_size_cells: usize,
    word_break: WordBreak,
    overflow_wrap: OverflowWrap,
    out: &mut Vec<Vec<InlineRun>>,
) {
    let mut current_line: Vec<InlineRun> = Vec::new();
    let mut current_width: usize = 0;
    // Carry the previous non-space run's fg / bg / style so a space
    // run inserted between two word runs paints with the same look —
    // the original text node carried the same fg / bg across the
    // space, and the renderer's SGR batcher relies on every cell in
    // a row having consistent attributes.
    let mut last_run_style: (Option<Color>, Option<Color>, FontStyle) =
        (None, None, FontStyle::empty());

    // Try to place `word` on the current line. If it does not fit and
    // the line is non-empty, flush and retry on a fresh line. If the
    // word is wider than the line, split it according to the
    // `word_break` / `overflow_wrap` policy.
    for token in logical {
        match token {
            Token::Newline => unreachable!("Newline should have split logical lines"),
            Token::Space { width, .. } => {
                if current_line.is_empty() {
                    continue; // skip leading spaces
                }
                if current_width + width <= max_width {
                    // Keep the space. Width bookkeeping is on the
                    // output InlineRun (single cell), so the input
                    // width already counts it.
                    let (fg, bg, style) = last_run_style;
                    current_line.push(InlineRun::new(" ", style, fg, bg));
                    current_width += width;
                } else {
                    // The space run does not fit on the current line. Flush
                    // the line we have so far and discard the space (the
                    // browser "hanging whitespace" rule: trailing whitespace
                    // at a line break is dropped even under `pre-wrap`).
                    out.push(std::mem::take(&mut current_line));
                    current_width = 0;
                }
            }
            Token::Word { text, width, style, fg, bg } => {
                last_run_style = (fg, bg, style);
                if width + current_width <= max_width {
                    // Whole word fits on the current line.
                    current_line.push(InlineRun::new(text, style, fg, bg));
                    current_width += width;
                } else if width <= max_width && !current_line.is_empty() {
                    // Word fits on a fresh line; flush current first.
                    out.push(std::mem::take(&mut current_line));
                    current_line.push(InlineRun::new(text, style, fg, bg));
                    current_width = width;
                } else if width <= max_width {
                    // Word fits on a fresh line; current is empty.
                    current_line.push(InlineRun::new(text, style, fg, bg));
                    current_width = width;
                } else {
                    // Word is wider than a full line. Split it.
                    let is_purely_cjk = text.chars().all(is_cjk_break_opportunity);
                    let should_split = match (word_break, overflow_wrap) {
                        (WordBreak::BreakAll, _) => true,
                        (WordBreak::KeepAll, _) if is_purely_cjk => false,
                        (WordBreak::KeepAll, _) => true,
                        (WordBreak::Normal, OverflowWrap::Anywhere) => true,
                        (WordBreak::Normal, OverflowWrap::BreakWord) => true,
                        (WordBreak::Normal, OverflowWrap::Normal) => false,
                    };
                    if !should_split {
                        // Word overflows; emit the whole thing.
                        current_line.push(InlineRun::new(text, style, fg, bg));
                        current_width += width;
                        continue;
                    }
                    // Flush anything already on the current line — the long
                    // word starts a fresh visual line.
                    if !current_line.is_empty() {
                        out.push(std::mem::take(&mut current_line));
                        current_width = 0;
                    }
                    // Greedy split by grapheme. `buf_width` tracks the
                    // accumulated width of the current buffer; when adding
                    // the next grapheme would overflow the line, flush the
                    // buffer as its own visual line and start a new one.
                    let font = font_size_cells.max(1);
                    let graphemes: Vec<&str> = text.graphemes(true).collect();
                    let mut buf = String::new();
                    let mut buf_width: usize = 0;
                    for g in graphemes {
                        let w = grapheme_width(g).max(font);
                        if !buf.is_empty() && buf_width + w > max_width {
                            current_line.push(InlineRun::new(
                                std::mem::take(&mut buf),
                                style,
                                fg,
                                bg,
                            ));
                            out.push(std::mem::take(&mut current_line));
                            buf_width = 0;
                        }
                        buf.push_str(g);
                        buf_width += w;
                    }
                    if !buf.is_empty() {
                        current_line.push(InlineRun::new(buf, style, fg, bg));
                        current_width = buf_width;
                    }
                }
            }
        }
    }
    out.push(current_line);
}

/// Wrap `runs` into a list of visual lines, each no wider than
/// `max_width` cells (after font-size expansion). The returned
/// `Vec<Vec<InlineRun>>` always has at least one entry; an empty
/// input returns `vec![Vec::new()]` (a single empty visual line).
pub fn wrap_runs(
    runs: &[InlineRun],
    max_width: usize,
    font_size_cells: usize,
    white_space: WhiteSpace,
    word_break: WordBreak,
    overflow_wrap: OverflowWrap,
) -> Vec<Vec<InlineRun>> {
    let result = wrap_runs_inner(runs, max_width, font_size_cells, white_space, word_break, overflow_wrap);
    result
}

fn wrap_runs_inner(
    runs: &[InlineRun],
    max_width: usize,
    font_size_cells: usize,
    white_space: WhiteSpace,
    word_break: WordBreak,
    overflow_wrap: OverflowWrap,
) -> Vec<Vec<InlineRun>> {
    if runs.is_empty() {
        return vec![Vec::new()];
    }
    if !white_space.allows_wrap() {
        // `pre` / `nowrap`: preserve order, no wrapping. Single line.
        let mut out: Vec<InlineRun> = Vec::with_capacity(runs.len());
        for r in runs {
            if !r.is_empty() {
                out.push(r.clone());
            }
        }
        return vec![out];
    }
    if max_width == 0 {
        // Defensive: avoid infinite loops in the splitter. Render
        // each non-empty run on its own line.
        let mut out: Vec<Vec<InlineRun>> = Vec::new();
        for r in runs {
            if !r.is_empty() {
                out.push(vec![r.clone()]);
            }
        }
        if out.is_empty() {
            out.push(Vec::new());
        }
        return out;
    }

    // Phase 1: tokenize.
    let mut tokens: Vec<Token> = Vec::new();
    for r in runs {
        tokenize_run(r, font_size_cells, word_break, &mut tokens);
    }
    // Phase 2: apply white-space collapsing / forced breaks.
    let logical_lines = apply_white_space(tokens, white_space);
    // Phase 3: greedy wrap each logical line.
    let mut visual_lines: Vec<Vec<InlineRun>> = Vec::new();
    for logical in logical_lines {
        wrap_logical_line(
            logical,
            max_width,
            font_size_cells,
            word_break,
            overflow_wrap,
            &mut visual_lines,
        );
    }
    if visual_lines.is_empty() {
        visual_lines.push(Vec::new());
    }
    visual_lines
}

/// Return `(max_line_width, line_count * font_size_cells)` of
/// wrapping `runs` to `max_width` under the same rules as
/// [`wrap_runs`]. Useful for intrinsic measurements where the
/// caller only needs the dimensions, not the per-line run lists.
pub fn wrapped_size(
    runs: &[InlineRun],
    max_width: usize,
    font_size_cells: usize,
    white_space: WhiteSpace,
    word_break: WordBreak,
    overflow_wrap: OverflowWrap,
) -> (usize, usize) {
    let lines = wrap_runs(runs, max_width, font_size_cells, white_space, word_break, overflow_wrap);
    let font = font_size_cells.max(1);
    let max_line_w = lines
        .iter()
        .map(|l| l.iter().map(|r| r.width_at(font)).sum::<usize>())
        .max()
        .unwrap_or(0);
    let total_rows = lines.len().saturating_mul(font);
    (max_line_w, total_rows)
}

/// Suppress unused-import warning for `grapheme_width_sum` — kept
/// for future use by callers that want to measure without wrapping.
#[allow(dead_code)]
fn _unused() {
    let _ = grapheme_width_sum;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &str, width: usize) -> Vec<Vec<InlineRun>> {
        wrap_runs(
            &[InlineRun::new(s, FontStyle::empty(), None, None)],
            width,
            1,
            WhiteSpace::Normal,
            WordBreak::Normal,
            OverflowWrap::Normal,
        )
    }

    fn run_ws(s: &str, width: usize, ws: WhiteSpace) -> Vec<Vec<InlineRun>> {
        wrap_runs(
            &[InlineRun::new(s, FontStyle::empty(), None, None)],
            width,
            1,
            ws,
            WordBreak::Normal,
            OverflowWrap::Normal,
        )
    }

    #[test]
    fn ascii_breaks_at_spaces() {
        // "hello" (5) + " " + "world" (5) + " " + "foo" (3) + " " + "bar" (3)
        // = 20 cells. Greedy wrap at width 6:
        //   line 1: "hello "  (6 cells)
        //   line 2: "world "  (6 cells)
        //   line 3: "foo"     (3 cells, the trailing space is dropped at the
        //                      wrap point, so "bar" does NOT fit next to it)
        //   line 4: "bar"
        let lines = run("hello world foo bar", 6);
        assert_eq!(lines.len(), 4, "got {} lines", lines.len());
    }

    #[test]
    fn cjk_breaks_between_characters() {
        // Each CJK character is 2 cells wide. Width 3 → one character
        // per line, so the 4-character string wraps to 4 visual lines.
        let lines = run("你好世界", 3);
        assert_eq!(lines.len(), 4, "got {} lines", lines.len());
    }

    #[test]
    fn nowrap_does_not_wrap() {
        let lines = run_ws("hello world", 5, WhiteSpace::NoWrap);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn pre_preserves_spaces_no_wrap() {
        let lines = run_ws("a   b", 10, WhiteSpace::Pre);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn pre_wrap_preserves_and_wraps() {
        // "a" (1) + "   " (3 cells) + "b" (1) = 5 cells total at width 3.
        // The space run does not fit next to "a" on line 1 (1+3=4>3), so
        // the line breaks after "a" and the trailing space is dropped;
        // "b" then occupies its own line.
        let lines = run_ws("a   b", 3, WhiteSpace::PreWrap);
        assert!(lines.len() >= 2, "got {} lines", lines.len());
    }

    #[test]
    fn pre_line_collapses_spaces_keeps_newlines() {
        let lines = run_ws("a   b\nc d", 10, WhiteSpace::PreLine);
        assert_eq!(lines.len(), 2, "got {} lines", lines.len());
    }

    #[test]
    fn break_all_splits_long_word() {
        let lines = wrap_runs(
            &[InlineRun::new("abcdef", FontStyle::empty(), None, None)],
            3,
            1,
            WhiteSpace::Normal,
            WordBreak::BreakAll,
            OverflowWrap::Normal,
        );
        assert!(lines.len() >= 2, "got {} lines", lines.len());
    }

    #[test]
    fn keep_all_keeps_cjk_intact() {
        let lines = wrap_runs(
            &[InlineRun::new("你好世界", FontStyle::empty(), None, None)],
            3,
            1,
            WhiteSpace::Normal,
            WordBreak::KeepAll,
            OverflowWrap::Normal,
        );
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn break_word_splits_unbreakable() {
        let lines = wrap_runs(
            &[InlineRun::new("abcdef", FontStyle::empty(), None, None)],
            3,
            1,
            WhiteSpace::Normal,
            WordBreak::Normal,
            OverflowWrap::BreakWord,
        );
        assert!(lines.len() >= 2, "got {} lines", lines.len());
    }

    #[test]
    fn overflow_normal_does_not_split() {
        let lines = wrap_runs(
            &[InlineRun::new("abcdef", FontStyle::empty(), None, None)],
            3,
            1,
            WhiteSpace::Normal,
            WordBreak::Normal,
            OverflowWrap::Normal,
        );
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn empty_input_yields_one_empty_line() {
        let lines = wrap_runs(&[], 80, 1, WhiteSpace::Normal, WordBreak::Normal, OverflowWrap::Normal);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].is_empty());
    }

    #[test]
    fn newlines_create_separate_lines() {
        // `pre-wrap` preserves `\n` as a forced break *and* allows
        // wrapping, so "a\nb\nc" becomes three visual lines regardless
        // of width. (`pre` is `white-space: pre`, which also preserves
        // `\n` but disables wrapping entirely; that case is covered by
        // `pre_preserves_spaces_no_wrap` above.)
        let lines = run_ws("a\nb\nc", 80, WhiteSpace::PreWrap);
        assert_eq!(lines.len(), 3, "got {} lines", lines.len());
    }
}
