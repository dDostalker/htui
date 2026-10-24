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
    graphics::{FontStyle, grapheme_width},
    html::css_parse::{OverflowWrap, WhiteSpace, WordBreak},
    layout::InlineRun,
    tdom::node::NodeId,
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
    /// A forced break (`'\n'`) in the source text.
    Newline,
    /// An unconditional forced break (`<br>`). Unlike [`Token::Newline`],
    /// it splits the line even under `white-space: normal` (where a text
    /// newline collapses as a space boundary).
    Break,
    /// A maximal non-whitespace grapheme run lifted from one source
    /// `InlineRun`. The styling is carried alongside so the output
    /// lines preserve span boundaries; `node_id` names the DOM element
    /// the source run belonged to (e.g. a `<label>`) so inline hit
    /// regions survive the wrap.
    Word {
        text: String,
        width: usize,
        style: FontStyle,
        fg: Option<Color>,
        bg: Option<Color>,
        node_id: Option<NodeId>,
    },
}

/// Returns `true` when `g` is one of the horizontal whitespace
/// graphemes the HTML spec treats as collapsible when `white-space`
/// is `normal` / `nowrap` / `pre-line`.
fn is_horizontal_ws(g: &str) -> bool {
    matches!(g, " " | "\t")
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
                    node_id: run.node_id,
                });
            }
        }
        *count = 0;
        *width = 0;
    };

    for g in run.text.graphemes(true) {
        if g == "\n" {
            // Close whatever is pending, then push the forced break.
            // A `<br>` marker run (force_break) is unconditional — it
            // splits the line even under `white-space: normal`.
            flush(
                pending_kind,
                &mut pending_text,
                &mut pending_count,
                &mut pending_width,
                run,
                out,
            );
            pending_kind = None;
            out.push(if run.force_break {
                Token::Break
            } else {
                Token::Newline
            });
            continue;
        }
        let w = grapheme_width(g).max(font);
        if is_horizontal_ws(g) {
            if pending_kind != Some(0) {
                flush(
                    pending_kind,
                    &mut pending_text,
                    &mut pending_count,
                    &mut pending_width,
                    run,
                    out,
                );
                pending_kind = Some(0);
            }
            pending_text.push_str(g);
            pending_count += 1;
            pending_width += w;
        } else if word_break != WordBreak::KeepAll
            && g.chars().next().is_some_and(is_cjk_break_opportunity)
        {
            // CJK (or full-width) grapheme under `normal` / `break-all`:
            // flush whatever is pending so the boundary before this
            // character is recognised as a wrap opportunity, then emit
            // it as its own one-grapheme word.
            flush(
                pending_kind,
                &mut pending_text,
                &mut pending_count,
                &mut pending_width,
                run,
                out,
            );
            pending_kind = None;
            out.push(Token::Word {
                text: g.to_string(),
                width: w,
                style: run.style,
                fg: run.fg,
                bg: run.bg,
                node_id: run.node_id,
            });
        } else {
            // Non-CJK, or CJK under `keep-all`: accumulate as one word
            // so the wrap phase treats the run as unbreakable.
            if pending_kind != Some(1) {
                flush(
                    pending_kind,
                    &mut pending_text,
                    &mut pending_count,
                    &mut pending_width,
                    run,
                    out,
                );
                pending_kind = Some(1);
            }
            pending_text.push_str(g);
            pending_count += 1;
            pending_width += w;
        }
    }
    flush(
        pending_kind,
        &mut pending_text,
        &mut pending_count,
        &mut pending_width,
        run,
        out,
    );
}

/// Apply the `white-space` collapsing rules to a token stream and
/// emit "logical lines": sequences of tokens that must appear on the
/// same line (unless wrapping kicks in). Forced breaks (`Newline`)
/// separate logical lines.
///
/// `font_size_cells` is the run's effective font expansion: a collapsed
/// space occupies one *expanded* slot (`max(grapheme_width, font)`), the
/// same width the paint phase's `width_at` accounts for the `" "` run —
/// the wrap accounting and the paint accounting must agree or
/// `text-align` offsets and the trailing-slot clip diverge once the font
/// size exceeds 1.
fn apply_white_space(tokens: Vec<Token>, ws: WhiteSpace, font_size_cells: usize) -> Vec<Vec<Token>> {
    let space_width = font_size_cells.max(1);
    if ws.preserves_whitespace() {
        // `pre` / `pre-wrap`: every Newline is a forced break; spaces
        // are preserved as-is. `<br>` breaks are unconditional.
        let mut lines: Vec<Vec<Token>> = vec![Vec::new()];
        for t in tokens {
            if matches!(t, Token::Newline | Token::Break) {
                lines.push(Vec::new());
            } else if let Some(line) = lines.last_mut() {
                line.push(t);
            }
        }
        return lines;
    }
    // `normal` / `nowrap` / `pre-line`: collapse runs of Space (and
    // any Newlines they contain, except `pre-line` where standalone
    // newlines are forced breaks). A `<br>` Break is forced in every mode.
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
            Token::Break => {
                // `<br>` forces a line break regardless of `white-space`.
                lines.push(Vec::new());
            }
            Token::Space { .. } => {
                if let Some(line) = lines.last_mut() {
                    let last_is_space = matches!(line.last(), Some(Token::Space { .. }));
                    if !last_is_space {
                        line.push(Token::Space {
                            width: space_width,
                            _count: 1,
                        });
                    }
                }
            }
            Token::Word { .. } => {
                if let Some(line) = lines.last_mut() {
                    line.push(t);
                }
            }
        }
    }
    lines
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
    // a row having consistent attributes. The owner (`node_id`) rides
    // along so a space inside an inline element's text still belongs
    // to that element's hit region.
    let mut last_run_style: (Option<Color>, Option<Color>, FontStyle) =
        (None, None, FontStyle::empty());
    let mut last_run_node_id: Option<NodeId> = None;

    // Try to place `word` on the current line. If it does not fit and
    // the line is non-empty, flush and retry on a fresh line. If the
    // word is wider than the line, split it according to the
    // `word_break` / `overflow_wrap` policy.
    for token in logical {
        match token {
            Token::Newline => unreachable!("Newline should have split logical lines"),
            Token::Break => unreachable!("Break should have split logical lines"),
            Token::Space { width, .. } => {
                if current_line.is_empty() {
                    continue; // skip leading spaces
                }
                if current_width + width <= max_width {
                    // Keep the space. Width bookkeeping is on the
                    // output InlineRun (single cell), so the input
                    // width already counts it.
                    let (fg, bg, style) = last_run_style;
                    current_line.push(InlineRun::owned(" ", style, fg, bg, last_run_node_id));
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
            Token::Word {
                text,
                width,
                style,
                fg,
                bg,
                node_id,
            } => {
                last_run_style = (fg, bg, style);
                last_run_node_id = node_id;
                if width + current_width <= max_width {
                    // Whole word fits on the current line.
                    current_line.push(InlineRun::owned(text, style, fg, bg, node_id));
                    current_width += width;
                } else if width <= max_width && !current_line.is_empty() {
                    // Word fits on a fresh line; flush current first.
                    out.push(std::mem::take(&mut current_line));
                    current_line.push(InlineRun::owned(text, style, fg, bg, node_id));
                    current_width = width;
                } else if width <= max_width {
                    // Word fits on a fresh line; current is empty.
                    current_line.push(InlineRun::owned(text, style, fg, bg, node_id));
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
                        current_line.push(InlineRun::owned(text, style, fg, bg, node_id));
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
                            current_line.push(InlineRun::owned(
                                std::mem::take(&mut buf),
                                style,
                                fg,
                                bg,
                                node_id,
                            ));
                            out.push(std::mem::take(&mut current_line));
                            buf_width = 0;
                        }
                        buf.push_str(g);
                        buf_width += w;
                    }
                    if !buf.is_empty() {
                        current_line.push(InlineRun::owned(buf, style, fg, bg, node_id));
                        current_width = buf_width;
                    }
                }
            }
        }
    }
    out.push(current_line);
}

/// Emit one visual line per logical line under [`WhiteSpace::Pre`].
///
/// `pre` preserves every whitespace grapheme verbatim AND forces a
/// break at each `\n`. Soft wrapping is off (no `max_width`
/// splitting), so this routine is much simpler than the greedy
/// wrap: walk graphemes, accumulate into a per-style buffer, and
/// flush the buffer at each newline. Style changes flush the
/// current buffer to a new [`InlineRun`] within the same visual
/// line so span boundaries stay intact.
///
/// Always returns at least one entry (an empty visual line for an
/// empty input). The returned visual lines preserve leading and
/// trailing spaces, tabs, and any number of consecutive spaces
/// within a line — the collapsing that the standard
/// `apply_white_space` path applies for `normal` / `nowrap` is
/// explicitly *not* in effect here.
fn emit_preserving_newlines(runs: &[InlineRun]) -> Vec<Vec<InlineRun>> {
    let mut lines: Vec<Vec<InlineRun>> = Vec::new();
    let mut line: Vec<InlineRun> = Vec::new();
    let mut buf = String::new();
    let mut cur_fg: Option<Color> = None;
    let mut cur_bg: Option<Color> = None;
    let mut cur_style = FontStyle::empty();
    let mut cur_node_id: Option<NodeId> = None;
    let mut buffering = false;

    for run in runs {
        if run.is_empty() {
            continue;
        }
        for g in run.text.graphemes(true) {
            if g == "\n" {
                if buffering {
                    line.push(InlineRun::owned(
                        std::mem::take(&mut buf),
                        cur_style,
                        cur_fg,
                        cur_bg,
                        cur_node_id,
                    ));
                    buffering = false;
                }
                lines.push(std::mem::take(&mut line));
                continue;
            }
            // Style change inside a single visual line flushes the
            // accumulated buffer so fg / bg / style never silently
            // bleed across span boundaries.
            if buffering
                && (cur_fg != run.fg
                    || cur_bg != run.bg
                    || cur_style != run.style
                    || cur_node_id != run.node_id)
            {
                line.push(InlineRun::owned(
                    std::mem::take(&mut buf),
                    cur_style,
                    cur_fg,
                    cur_bg,
                    cur_node_id,
                ));
                buffering = false;
            }
            if !buffering {
                cur_fg = run.fg;
                cur_bg = run.bg;
                cur_style = run.style;
                cur_node_id = run.node_id;
                buffering = true;
            }
            buf.push_str(g);
        }
    }
    if buffering {
        line.push(InlineRun::owned(
            buf,
            cur_style,
            cur_fg,
            cur_bg,
            cur_node_id,
        ));
    }
    // Always emit a trailing visual line — even an empty one when
    // the input ended on a newline. This keeps callers (which split
    // the canvas one line at a time) aligned with the source.
    lines.push(line);
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

/// Wrap inline runs into visual lines according to the white-space, word-break, and overflow-wrap policies.
pub fn wrap_runs(
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
        if matches!(white_space, WhiteSpace::Pre) {
            // `pre` keeps every whitespace grapheme (spaces, tabs) AND
            // every newline. Soft wrapping is forbidden, but `\n` is
            // a forced break: each newline starts a fresh visual line.
            // Falls through to a dedicated splitter that walks the
            // grapheme stream so it can keep multi-space runs intact
            // (which the `tokenize_run` collapse phase would otherwise
            // strip away).
            return emit_preserving_newlines(runs);
        }
        // `nowrap`: collapse whitespace (still happens in apply_white_space)
        // and never wrap. A single visual line; text newlines collapse —
        // except `<br>` breaks, which split the line unconditionally.
        let mut out: Vec<Vec<InlineRun>> = vec![Vec::new()];
        for r in runs {
            if r.force_break {
                out.push(Vec::new());
            } else if !r.is_empty()
                && let Some(line) = out.last_mut()
            {
                line.push(r.clone());
            }
        }
        return out;
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
    let logical_lines = apply_white_space(tokens, white_space, font_size_cells);
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
    if visual_lines.is_empty() || visual_lines.iter().all(|l| l.is_empty()) {
        // Whitespace-only input collapses away to no visible content: report
        // zero lines so callers don't reserve a phantom inline row for it.
        return Vec::new();
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
    let lines = wrap_runs(
        runs,
        max_width,
        font_size_cells,
        white_space,
        word_break,
        overflow_wrap,
    );
    let font = font_size_cells.max(1);
    let max_line_w = lines
        .iter()
        .map(|l| l.iter().map(|r| r.width_at(font)).sum::<usize>())
        .max()
        .unwrap_or(0);
    let total_rows = lines.len().saturating_mul(font);
    (max_line_w, total_rows)
}
