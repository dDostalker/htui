//! Unit tests for the pure line-breaker [`wrap_runs`].
//!
//! These exercise the CSS Text Level 3 wrapper directly (not through the
//! HTML → layout → paint pipeline, which is covered by `layout_wrap_tests`),
//! so each break policy can be asserted against exact run lists.

use htui::html::css_parse::{OverflowWrap, WhiteSpace, WordBreak};
use htui::layout::layoutbox::{InlineRun, wrap_runs};
use htui::tui::graph_api::FontStyle;

fn run(s: &str, width: usize) -> Vec<Vec<InlineRun>> {
    wrap_runs(
        &[InlineRun::new(s, FontStyle::empty(), None, None)],
        width,
        1,
        WhiteSpace::Normal,
        WordBreak::Normal,
        OverflowWrap::Normal,
        0,
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
        0,
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
        0,
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
        0,
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
        0,
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
        0,
    );
    assert_eq!(lines.len(), 1);
}

#[test]
fn empty_input_yields_one_empty_line() {
    let lines = wrap_runs(
        &[],
        80,
        1,
        WhiteSpace::Normal,
        WordBreak::Normal,
        OverflowWrap::Normal,
        0,
    );
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

#[test]
fn consecutive_spaces_collapse_to_single_space() {
    // `white-space: normal` collapses a run of collapsible spaces to a
    // single space (the browser-default HTML behaviour).
    let lines = run("a    b", 80);
    assert_eq!(lines.len(), 1);
    let joined: String = lines[0].iter().map(|r| r.text.as_str()).collect();
    assert_eq!(joined, "a b");
}

#[test]
fn nbsp_is_not_collapsible() {
    // U+00A0 is a non-breaking space: unlike `' '` / `\t`, it is NOT
    // collapsible under `white-space: normal` and must survive as three
    // cells.
    let lines = run("a\u{00A0}\u{00A0}\u{00A0}b", 80);
    assert_eq!(lines.len(), 1);
    let joined: String = lines[0].iter().map(|r| r.text.as_str()).collect();
    assert_eq!(joined, "a\u{00A0}\u{00A0}\u{00A0}b");
}

#[test]
fn cjk_closing_punctuation_never_starts_a_line() {
    // Simple kinsoku: a closing punctuation mark cannot detach from the word
    // before it. `甲乙丙丁。` at 4 cells (two 2-cell CJK glyphs per line): the
    // tokenizer used to emit `。` as its own breakable word, producing
    // 甲乙 / 丙丁 / 。 — a line opening with the full stop. The mark now
    // merges into `丁`, so the last line is `丁。` and no line opens with
    // the mark.
    let lines = run("甲乙丙丁。", 4);
    let l2: String = lines[2].iter().map(|r| r.text.as_str()).collect();
    assert_eq!(l2, "丁。", "the closing mark rides with 丁");
    assert!(
        lines
            .iter()
            .all(|l| l.first().map(|r| !r.text.starts_with('。')).unwrap_or(true)),
        "no visual line may open with the closing mark: {lines:?}"
    );
}

#[test]
fn cjk_kinsoku_applies_under_break_all() {
    // `word-break: break-all` allows breaks between any two CJK glyphs but
    // still honours the line-start prohibition — the closing mark stays
    // glued to its predecessor.
    let lines = wrap_runs(
        &[InlineRun::new("甲乙丙丁，", FontStyle::empty(), None, None)],
        4,
        1,
        WhiteSpace::Normal,
        WordBreak::BreakAll,
        OverflowWrap::Normal,
        0,
    );
    let l2: String = lines[2].iter().map(|r| r.text.as_str()).collect();
    assert_eq!(l2, "丁，", "the comma rides with the preceding glyph");
    assert!(
        lines
            .iter()
            .all(|l| l.first().map(|r| !r.text.starts_with('，')).unwrap_or(true)),
        "no visual line may open with the comma: {lines:?}"
    );
}
