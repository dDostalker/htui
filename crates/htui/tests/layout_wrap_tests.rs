//! End-to-end tests for the inline-text line-breaker.
//!
//! Each test stands up a small HTML snippet, runs the full
//! `html -> tdom -> layout -> tui` pipeline, and inspects the rendered
//! canvas to verify the wrap rules. The intent is to lock down the
//! browser-faithful behaviour the wrap layer is meant to provide:
//! ASCII word boundaries, CJK character boundaries, `white-space`
//! variants, and `word-break` / `overflow-wrap` policies.

use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::tdom::node::{NodeArena, NodeId};
use htui::tui::graph_api::TermLayout;

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

fn setup(html: &str, css: &str) -> (NodeId, NodeArena) {
    let mut html_parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = html_parser.parse();
    let combined = if css.is_empty() {
        embedded
    } else if embedded.is_empty() {
        css.to_string()
    } else {
        format!("{embedded}\n{css}")
    };
    // Always cascade: an empty stylesheet must still apply inline `style`
    // attributes and inheritance.
    let mut css_parser = css_parse::Parser::new(&combined);
    let rules = css_parser.parse();
    css_parse::css_into_node(&mut arena, root_id, &rules);
    (root_id, arena)
}

const COLS: u16 = 80;
const ROWS: u16 = 24;

fn mock_term_map() -> TermMap {
    TermMap {
        columns: COLS,
        rows: ROWS,
    }
}

/// Build the canvas and return the per-row text string for inspection.
fn render_rows(html: &str, css: &str) -> (Vec<String>, TermLayout) {
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let cols = term_map.columns as usize;
    let mut rows: Vec<String> = Vec::new();
    for chunk in tl.layout.chunks(cols) {
        let s: String = chunk.iter().map(|b| b.word.as_str()).collect();
        rows.push(s);
    }
    (rows, tl)
}

// ============================================================
// 1. Long ASCII paragraph wraps at word boundaries.
// ============================================================

#[test]
fn long_paragraph_wraps_across_rows() {
    // 5-word sentence of 39 ASCII characters. At width 80 it still fits
    // on one row, but the box's own wrapping layer must observe the
    // content-box width — a paragraph in a narrow container must split.
    init();
    let html = r#"<p>The quick brown fox jumps over the lazy dog.</p>"#;
    let css = r#"p { width: 20ch; }"#;
    let (rows, _) = render_rows(html, css);
    let first_idx = rows
        .iter()
        .position(|r| r.contains("quick"))
        .expect("first wrap line should contain 'quick'");
    let second_idx = rows
        .iter()
        .position(|r| r.contains("lazy"))
        .expect("second wrap line should contain 'lazy'");
    assert!(
        second_idx > first_idx,
        "wrapped second half should land below the first, got rows {first_idx}/{second_idx}"
    );
}

#[test]
fn short_paragraph_stays_on_one_row() {
    init();
    let (rows, _) = render_rows("<p>hi</p>", "");
    let row = rows
        .iter()
        .position(|r| r.contains('h'))
        .expect("paragraph should render");
    // No second visual line for a 2-character paragraph.
    let next_row = &rows[row + 1];
    assert!(
        !next_row.contains('h') && !next_row.contains('i'),
        "no wrap should occur for a short paragraph, next row = {next_row:?}"
    );
}

// ============================================================
// 2. `white-space: nowrap` keeps the text on a single line.
// ============================================================

#[test]
fn nowrap_keeps_text_on_one_line() {
    init();
    let html = r#"<p style="white-space: nowrap;">A B C D E F G H I J K L M</p>"#;
    let (rows, _) = render_rows(html, "");
    // Find the row that holds the start of the text and confirm no
    // second line was emitted by the same paragraph.
    let first_idx = rows
        .iter()
        .position(|r| r.contains('A'))
        .expect("paragraph should render");
    for (i, row) in rows.iter().enumerate().skip(first_idx + 1).take(5) {
        assert!(
            !row.contains('A') && !row.contains('B'),
            "nowrap should not emit a second visual line at row {i}: {row:?}"
        );
    }
}

// ============================================================
// 3. CJK text wraps at character boundaries under the default
//    `word-break: normal`.
// ============================================================

#[test]
fn cjk_wraps_between_characters() {
    init();
    let html = r#"<p>你好世界欢迎使用终端渲染器</p>"#;
    let css = r#"p { width: 4ch; }"#;
    let (rows, _) = render_rows(html, css);
    let first_idx = rows
        .iter()
        .position(|r| r.contains('好'))
        .expect("'好' should be in the first wrap line");
    let second_idx = rows
        .iter()
        .position(|r| r.contains('世'))
        .expect("'世' should be in the second wrap line");
    assert!(
        second_idx > first_idx,
        "second CJK character should wrap to a later row, got rows {first_idx}/{second_idx}"
    );
}

#[test]
fn keep_all_keeps_cjk_intact() {
    init();
    let html = r#"<p style="word-break: keep-all;">你好世界</p>"#;
    let css = r#"p { width: 2ch; }"#;
    let (rows, _) = render_rows(html, css);
    // Under `word-break: keep-all` the CJK sequence is treated as one
    // unbreakable word, so the paragraph should still occupy a single
    // visual line.
    let first_idx = rows
        .iter()
        .position(|r| r.contains('你'))
        .expect("paragraph should render");
    for (i, row) in rows.iter().enumerate().skip(first_idx + 1).take(3) {
        assert!(
            !row.contains('你') && !row.contains('好'),
            "keep-all should keep the CJK sequence on one line, row {i}: {row:?}"
        );
    }
}

// ============================================================
// 4. `white-space: pre` preserves newlines verbatim but disables wrap.
// ============================================================

#[test]
fn pre_preserves_newlines_no_wrap() {
    init();
    let html = "<pre>line one\nline two\nline three</pre>";
    let (rows, _) = render_rows(html, "");
    let one = rows
        .iter()
        .position(|r| r.contains("line one"))
        .expect("first line should render");
    eprintln!("pre rows:");
    for (i, r) in rows.iter().enumerate().take(15) {
        eprintln!("{i:3}: {:?}", r);
    }
    let two = rows
        .iter()
        .position(|r| r.contains("line two"))
        .expect("second line should render");
    let three = rows
        .iter()
        .position(|r| r.contains("line three"))
        .expect("third line should render");
    assert!(one < two && two < three, "newlines should stack rows");
}

#[test]
fn pre_preserves_consecutive_spaces() {
    // `<pre>` must not collapse runs of horizontal whitespace the way
    // `normal` does. The `tokenize_run` phase reduces consecutive
    // spaces to a single Space token for the wrap path, so a
    // dedicated `emit_preserving_newlines` step is required to keep
    // the original whitespace byte-for-byte.
    init();
    let html = "<pre>a    b</pre>";
    let (rows, _) = render_rows(html, "");
    assert!(
        rows.iter().any(|r| r.contains("a    b")),
        "four spaces must survive verbatim, rows = {:?}",
        rows
    );
}

#[test]
fn pre_element_uses_ua_default_white_space() {
    // Author CSS does not declare `white-space` on the `<pre>` — the
    // UA default of `pre` (preserves whitespace, treats `\n` as a
    // forced break, no soft wrapping) must take effect.
    init();
    let html = "<pre>x\ny</pre>";
    let (rows, _) = render_rows(html, "");
    let x = rows
        .iter()
        .position(|r| r.contains('x'))
        .expect("first newline row should render");
    let y = rows
        .iter()
        .position(|r| r.contains('y'))
        .expect("second newline row should render");
    assert!(y > x, "UA default should split the two lines, rows {x}/{y}");
}

// ============================================================
// 5. `overflow-wrap: break-word` splits an unbreakable long word.
// ============================================================

#[test]
fn break_word_splits_long_word() {
    init();
    let html = r#"<p style="overflow-wrap: break-word;">aaaaaaaaaaaaaaaaaaaa</p>"#;
    let css = r#"p { width: 10ch; }"#;
    let (rows, _) = render_rows(html, css);
    let first_idx = rows
        .iter()
        .position(|r| r.contains('a'))
        .expect("first split should render");
    let last_with_a = rows
        .iter()
        .rposition(|r| r.contains('a'))
        .expect("any 'a' should render");
    assert!(
        last_with_a > first_idx,
        "long word should wrap, rows {first_idx}/{last_with_a}"
    );
}

#[test]
fn overflow_wrap_normal_does_not_split() {
    // Without `overflow-wrap: break-word` an unbreakable word that
    // exceeds the line width is left to overflow — the box still
    // reports a single visual line because there is no break
    // opportunity to use.
    init();
    let html = "<p>aaaaaaaaaaaaaaaaaaaa</p>";
    let css = "p { width: 10ch; }";
    let (rows, _) = render_rows(html, css);
    let first_idx = rows
        .iter()
        .position(|r| r.contains('a'))
        .expect("paragraph should render");
    // The next row should not contain any 'a' that came from the
    // paragraph (the canvas is initialised empty so no 'a' from
    // elsewhere could be present).
    let next_row = &rows[first_idx + 1];
    assert!(
        !next_row.contains('a'),
        "overflow-wrap: normal should not split the long word, next row = {next_row:?}"
    );
}

// ============================================================
// 6. List marker stays on the first line of its item.
// ============================================================

#[test]
fn li_marker_stays_on_first_line() {
    init();
    let html = "<ul><li>alpha beta gamma delta epsilon zeta eta theta iota kappa</li></ul>";
    let css = "ul { width: 12ch; }";
    let (rows, _) = render_rows(html, css);
    // The '•' marker should be on the same row as 'alpha' (the first
    // word of the body) — never on a later wrap line.
    let marker_idx = rows
        .iter()
        .position(|r| r.contains('•'))
        .expect("li marker should render");
    let alpha_idx = rows
        .iter()
        .position(|r| r.contains("alpha"))
        .expect("first body word should render");
    assert_eq!(
        marker_idx, alpha_idx,
        "li marker and first body word must share a row"
    );
}

// ============================================================
// 7. Heading prefix stays on the first line of the heading.
// ============================================================

#[test]
fn h2_prefix_stays_on_first_line() {
    // The prefix is part of the first visual line's run list, so the
    // prefix glyphs always land on row 0 of the heading. With a wide
    // container, the first body word also lives on row 0.
    //
    // `font-size: 1em` disables h2's 1.5em default so each character
    // paints into a single terminal cell. The headless test harness
    // collects per-cell words one at a time and concatenates them
    // into a single row string — font-size expansion would insert a
    // padding cell between each character, which would scatter
    // contiguous substrings like "alpha" across multiple cells even
    // though the visual layout is identical.
    init();
    let html = "<h2>alpha beta gamma</h2>";
    let css = "h2 { width: 30ch; font-size: 1em; }";
    let (rows, _) = render_rows(html, css);
    let prefix_idx = rows
        .iter()
        .position(|r| r.contains("#"))
        .expect("h2 prefix should render");
    let body_idx = rows
        .iter()
        .position(|r| r.contains("alpha"))
        .expect("h2 body should render");
    assert_eq!(
        prefix_idx, body_idx,
        "heading prefix and first body word must share a row"
    );
}

// ============================================================
// 8. Collapsible vs non-collapsible whitespace
// ============================================================

#[test]
fn whitespace_only_span_collapses_to_single_space() {
    // A whitespace-only inline element between two words must collapse to
    // a single space, not disappear entirely.
    init();
    let html = "<p>hello<span>   </span>world</p>";
    let (rows, _) = render_rows(html, "");
    assert!(
        rows.iter().any(|r| r.contains("hello world")),
        "whitespace-only span should collapse to a single space, rows = {:?}",
        rows
    );
}

#[test]
fn nbsp_entity_is_preserved() {
    // `&nbsp;` decodes to U+00A0, a non-collapsible space: three of them
    // must survive verbatim instead of collapsing to a single space.
    init();
    let html = "<p>a&nbsp;&nbsp;&nbsp;b</p>";
    let (rows, _) = render_rows(html, "");
    assert!(
        rows.iter()
            .any(|r| r.contains("a\u{00A0}\u{00A0}\u{00A0}b")),
        "&nbsp; must survive as non-collapsible spaces, rows = {:?}",
        rows
    );
}

// ============================================================
// letter-spacing / word-spacing / text-indent / text-overflow
// ============================================================

/// Row string with trailing blanks trimmed, for exact-prefix assertions.
fn row_trimmed(rows: &[String], idx: usize) -> String {
    rows.get(idx)
        .map(|r| r.trim_end().to_string())
        .unwrap_or_default()
}

/// First non-blank row's trimmed text (skips UA top margins like `<p>`'s).
fn first_content_row(rows: &[String]) -> String {
    rows.iter()
        .map(|r| r.trim_end())
        .find(|r| !r.is_empty())
        .unwrap_or_default()
        .to_string()
}

#[test]
fn letter_spacing_widens_text() {
    init();
    let (plain, _) = render_rows("<p>ab</p>", "");
    let (spaced, _) = render_rows("<p style=\"letter-spacing: 2ch\">ab</p>", "");
    let plain_row = first_content_row(&plain);
    let spaced_row = first_content_row(&spaced);
    assert!(
        plain_row.contains("ab"),
        "baseline renders, got {plain_row:?}"
    );
    // Two graphemes with +2-cell spacing each → "a  b" vs "ab".
    assert!(
        spaced_row.contains("a  b"),
        "letter-spacing inserts blank columns, got {spaced_row:?}"
    );
}

#[test]
fn word_spacing_widens_word_gaps() {
    init();
    let (spaced, _) = render_rows("<p style=\"word-spacing: 3ch\">a b</p>", "");
    let row = first_content_row(&spaced);
    assert!(
        row.contains("a    b"),
        "word-spacing widens the space by 3 columns, got {row:?}"
    );
}

#[test]
fn text_indent_shifts_first_line() {
    init();
    // Two forced lines via <br>: the first shifts by 4 columns, the second
    // stays flush. Content starts after `<p>`'s UA top margin.
    let (rows, _) = render_rows("<p style=\"text-indent: 4ch\">aa<br>bb</p>", "");
    let content_start = rows
        .iter()
        .position(|r| !r.trim_end().is_empty())
        .expect("content rendered");
    let first = row_trimmed(&rows, content_start);
    let second = row_trimmed(&rows, content_start + 1);
    assert!(
        first.starts_with("    aa"),
        "first line indented by 4 columns, got {first:?}"
    );
    assert!(
        second.starts_with("bb"),
        "second line stays flush, got {second:?}"
    );
}

#[test]
fn text_overflow_ellipsis_clips_nowrap() {
    init();
    let (rows, _) = render_rows(
        "<div style=\"width: 6ch; overflow: hidden; white-space: nowrap; text-overflow: ellipsis\">abcdefgh</div>",
        "",
    );
    let row = first_content_row(&rows);
    assert!(
        row.contains("abcde…") && !row.contains("abcdef"),
        "clipped nowrap line ends with ellipsis, got {row:?}"
    );
}

#[test]
fn word_spacing_wrap_keeps_every_word_and_fits() {
    // Regression (right-overflow / silent content loss): the collapsed
    // space used to be re-counted at bare font width (1 cell) while paint
    // advanced by the real 5-cell space, so wrap packed far more words per
    // line than the row could hold — the paint pass then truncated the
    // line tail (whole letters vanished) and spaced lines ran past the box
    // edge. 26 letters × 5-cell gaps = 151 cells force wrapping at 80.
    init();
    let (rows, _) = render_rows(
        "<p style=\"word-spacing: 4ch\">a b c d e f g h i j k l m n o p q r s t u v w x y z</p>",
        "",
    );
    let canvas: String = rows.concat();
    for ch in "abcdefghijklmnopqrstuvwxyz".chars() {
        assert!(canvas.contains(ch), "letter {ch} lost, rows={rows:?}");
    }
    // Every content row must respect the 80-cell budget: at 5-cell gaps
    // that means at most 16 letters per row.
    for row in rows.iter() {
        let letters = row.chars().filter(|c| c.is_ascii_lowercase()).count();
        assert!(
            letters <= 16,
            "row overflows the width ({letters} letters): {row:?}"
        );
    }
}

#[test]
fn letter_spacing_wrap_keeps_every_word() {
    // Same class of regression on the letter axis: the wrap budget must
    // include the per-grapheme letter spacing so wrapped lines keep all
    // their words within the width.
    init();
    let (rows, _) = render_rows(
        "<p style=\"letter-spacing: 1ch\">aaa bbb ccc ddd eee fff ggg hhh iii jjj kkk lll mmm nnn ooo ppp</p>",
        "",
    );
    // Spacing columns paint as blank cells, so compare with blanks removed.
    let canvas: String = rows.concat().replace(' ', "");
    for word in [
        "aaa", "bbb", "ccc", "ddd", "eee", "fff", "ggg", "hhh", "iii", "jjj", "kkk", "lll", "mmm",
        "nnn", "ooo", "ppp",
    ] {
        assert!(canvas.contains(word), "word {word} lost, rows={rows:?}");
    }
    // No content row may exceed the terminal width.
    for row in rows.iter() {
        assert!(
            row.trim_end().chars().count() <= 80,
            "row exceeds the width: {row:?}"
        );
    }
}
