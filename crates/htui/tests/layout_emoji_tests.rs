//! Tests for emoji & multi-codepoint grapheme cluster rendering.
//!
//! Pins down the contract for emoji width handling:
//!
//! - [`char_width`](htui::tui::graph_api::char_width) reports 2 for any single
//!   codepoint in an emoji-presenting block (Emoticons, Misc Symbols and
//!   Pictographs, Transport, Dingbats, …) that the old hand-rolled range
//!   table missed.
//! - [`grapheme_width`](htui::tui::graph_api::grapheme_width) honours the
//!   full emoji ligature rule set: variation-selector pairs (❤️), modifier
//!   sequences (👍🏽), and ZWJ sequences (👨‍👩‍👧) all measure as 2.
//! - The paint phase writes the whole grapheme cluster into a single cell and
//!   reserves the trailing column as an empty word, mirroring how wide CJK
//!   glyphs already work.

use htui::html::{css_parse, html_parse};
use htui::layout::InlineRun;
use htui::layout::layout::TermMap;
use htui::tui::graph_api::{FontStyle, TermLayout, char_width, grapheme_width};

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

fn setup(html: &str, css: &str) -> (htui::tdom::node::NodeId, htui::tdom::node::NodeArena) {
    let mut html_parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = html_parser.parse();
    let combined = if css.is_empty() {
        embedded
    } else if embedded.is_empty() {
        css.to_string()
    } else {
        format!("{embedded}\n{css}")
    };
    if !combined.trim().is_empty() {
        let mut css_parser = css_parse::Parser::new(&combined);
        let rules = css_parser.parse();
        css_parse::css_into_node(&mut arena, root_id, &rules);
    }
    (root_id, arena)
}

fn mock_term_map() -> TermMap {
    TermMap {
        columns: 80,
        rows: 24,
    }
}

fn canvas_string(tl: &TermLayout) -> String {
    tl.layout.iter().map(|b| b.word.as_str()).collect()
}

fn position_of(tl: &TermLayout, word: &str) -> Option<usize> {
    tl.layout.iter().position(|b| b.word == word)
}

// ============================================================
// char_width — single codepoint emoji
// ============================================================

#[test]
fn test_char_width_emoji_presents_as_two_columns() {
    // Emoticons block (U+1F600–U+1F64F): previously returned 1.
    assert_eq!(char_width('\u{1F600}'), 2, "😀 (grinning face)");
    assert_eq!(
        char_width('\u{1F603}'),
        2,
        "😃 (smiling face with open mouth)"
    );
    assert_eq!(
        char_width('\u{1F604}'),
        2,
        "😄 (smiling face with open mouth and smiling eyes)"
    );
}

#[test]
fn test_char_width_misc_symbols_and_pictographs_is_two() {
    // Misc Symbols and Pictographs (U+1F300–U+1F5FF): previously returned 1.
    assert_eq!(char_width('\u{1F680}'), 2, "🚀 (rocket)");
    assert_eq!(char_width('\u{2B50}'), 2, "⭐ (star)");
}

#[test]
fn test_char_width_dingbats_is_two() {
    // Dingbats (U+2700–U+27BF) that carry Emoji_Presentation=Yes are
    // 2 columns on their own.
    assert_eq!(char_width('\u{2705}'), 2, "✅ (white heavy check mark)");

    // ❤ (U+2764) does NOT have Emoji_Presentation by default — it is a
    // text-default codepoint and only becomes 2-column emoji when paired
    // with VS-16 (U+FE0F). `char_width` reports the bare codepoint width;
    // `grapheme_width` handles the presentation selector.
    assert_eq!(char_width('\u{2764}'), 1, "❤ alone (no VS-16) is 1 column");
    assert_eq!(
        grapheme_width("\u{2764}\u{FE0F}"),
        2,
        "❤ + VS-16 becomes the 2-column emoji ❤️"
    );
}

#[test]
fn test_char_width_ascii_is_one() {
    // Sanity: ASCII still occupies a single column.
    assert_eq!(char_width('a'), 1);
    assert_eq!(char_width('Z'), 1);
    assert_eq!(char_width(' '), 1);
}

#[test]
fn test_char_width_cjk_is_two() {
    // Sanity: CJK ideographs continue to occupy two columns.
    assert_eq!(char_width('你'), 2);
    assert_eq!(char_width('界'), 2);
    assert_eq!(char_width('ア'), 2);
}

#[test]
fn test_char_width_control_is_zero() {
    // Control characters should not consume a column — matches the legacy
    // behaviour callers rely on (and what the wider terminal ecosystem does).
    assert_eq!(char_width('\0'), 0);
    assert_eq!(char_width('\u{0007}'), 0);
}

// ============================================================
// grapheme_width — multi-codepoint emoji
// ============================================================

#[test]
fn test_grapheme_width_heart_with_vs16_is_two() {
    // ❤ + VS-16 is a presentation selector pair: unicode-width treats the
    // pair as a single 2-column emoji.
    assert_eq!(grapheme_width("\u{2764}\u{FE0F}"), 2);
}

#[test]
fn test_grapheme_width_zwj_family_is_two() {
    // 👨‍👩‍👧 = 👨 + ZWJ + 👩 + ZWJ + 👧 — a fully-qualified ZWJ sequence.
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    assert_eq!(grapheme_width(family), 2);
}

#[test]
fn test_grapheme_width_modifier_sequence_is_two() {
    // 👍🏽 = 👍 + 🏽 (Fitzpatrick modifier).
    let modifier = "\u{1F44D}\u{1F3FD}";
    assert_eq!(grapheme_width(modifier), 2);
}

#[test]
fn test_grapheme_width_flag_is_two() {
    // 🇨🇳 = regional indicator C + regional indicator N.
    let flag = "\u{1F1E8}\u{1F1F3}";
    assert_eq!(grapheme_width(flag), 2);
}

#[test]
fn test_grapheme_width_ascii_text_sums_columns() {
    assert_eq!(grapheme_width("Hello"), 5);
    assert_eq!(grapheme_width(""), 0);
}

#[test]
fn test_grapheme_width_mixed_cjk_emoji_ascii() {
    // Hello(5) + space(1) + 世界(4) + space(1) + 😀(2) = 13.
    let s = "Hello 世界 \u{1F600}";
    assert_eq!(grapheme_width(s), 13);
}

// ============================================================
// InlineRun::width — layout-side measurement
// ============================================================

#[test]
fn test_inline_run_width_counts_graphemes() {
    let run = InlineRun::new("Hi \u{1F600}", FontStyle::empty(), None, None);
    // H(1) + i(1) + space(1) + 😀(2) = 5
    assert_eq!(run.width(), 5);
}

#[test]
fn test_inline_run_width_handles_zwj_family() {
    // 👨‍👩‍👧 is one grapheme cluster, width 2.
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let run = InlineRun::new(family, FontStyle::empty(), None, None);
    assert_eq!(run.width(), 2);
}

#[test]
fn test_inline_run_width_at_respects_font_size_floor() {
    // font-size 2 still floor-clusters at 2 columns, even if the cluster is
    // already 2 wide.
    let run = InlineRun::new("\u{1F600}", FontStyle::empty(), None, None);
    assert_eq!(run.width_at(2), 2);

    // ASCII clusters expand up to the font-size floor.
    let run = InlineRun::new("Hi", FontStyle::empty(), None, None);
    assert_eq!(run.width_at(2), 4);
}

// ============================================================
// End-to-end: paint phase writes emoji into the canvas
// ============================================================

#[test]
fn test_emoji_reaches_canvas() {
    init();
    let (root_id, arena) = setup("<p>Hi \u{1F600}!</p>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let canvas = canvas_string(&tl);
    assert!(canvas.contains("Hi"), "ASCII prefix must render");
    assert!(canvas.contains('\u{1F600}'), "😀 must reach the canvas");
    assert!(canvas.contains("!"), "ASCII suffix must render");
}

#[test]
fn test_emoji_occupies_two_columns() {
    init();
    // 😃 has width 2 — there must be a trailing empty cell after it so the
    // renderer doesn't double-print.
    let (root_id, arena) = setup("<p>a\u{1F603}b</p>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let idx = position_of(&tl, "a").expect("'a' cell exists");
    // Layout order:
    //   idx    = "a"      (1 column)
    //   idx+1  = "😃"      (the emoji's main cell)
    //   idx+2  = ""       (trailing cell — sentinel `render_to_bytes` skips)
    //   idx+3  = "b"      (next char starts after the 2-column glyph)
    assert_eq!(tl.layout[idx + 1].word, "\u{1F603}", "emoji main cell");
    assert!(
        tl.layout[idx + 2].word.is_empty(),
        "cell immediately after emoji must be the empty trailing cell \
         so the renderer skips it (got {:?})",
        tl.layout[idx + 2].word
    );
    assert_eq!(tl.layout[idx + 3].word, "b", "post-emoji cell");
}

#[test]
fn test_heart_with_variation_selector_lands_in_one_cell() {
    init();
    // ❤ + VS-16 is one grapheme cluster (width 2). It should land in a
    // single cell whose `word` contains the full pair — the painter used
    // to split ❤ and VS-16 across two cells because it iterated `char`s.
    let (root_id, arena) = setup("<p>\u{2764}\u{FE0F}</p>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let pair = "\u{2764}\u{FE0F}";
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == pair)
        .unwrap_or_else(|| panic!("heart + VS-16 must occupy one cell with word {pair:?}"));

    // The cluster is 2 columns wide, so the next cell should be the
    // padding cell (empty word) — and crucially it must NOT contain VS-16
    // alone (the old code would split them up).
    assert!(
        tl.layout[idx + 1].word.is_empty(),
        "VS-16 must not be split into its own cell"
    );
}

#[test]
fn test_zwj_family_lands_in_one_cell() {
    init();
    // 👨‍👩‍👧 = 👨 + ZWJ + 👩 + ZWJ + 👧 — 5 codepoints, 1 grapheme.
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let (root_id, arena) = setup(&format!("<p>{family}</p>"), "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == family)
        .unwrap_or_else(|| panic!("family ZWJ sequence must occupy one cell with word {family:?}"));
    // Width 2 → trailing padding cell.
    assert!(
        tl.layout[idx + 1].word.is_empty(),
        "ZWJ family must reserve a trailing empty cell"
    );
}

#[test]
fn test_mixed_cjk_emoji_layout_alignment() {
    init();
    // Mixed: ASCII + CJK + emoji + ASCII must not collide. Concretely, the
    // cell that holds the suffix 'Z' must be exactly 4 columns to the right
    // of the prefix 'A'.
    let (root_id, arena) = setup("<p>A\u{4F60}\u{1F600}Z</p>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let a_idx = position_of(&tl, "A").expect("A renders");
    let z_idx = position_of(&tl, "Z").expect("Z renders");
    // A(1) + 你(2) + 😀(2) + Z(1) = 6 columns between A and Z.
    assert_eq!(
        z_idx - a_idx,
        5,
        "CJK (你, 2 cols) and emoji (😀, 2 cols) must each reserve their own column"
    );
}

// ============================================================
// CompactString heap fallback — clusters longer than 24 bytes
// ============================================================

#[test]
fn test_cluster_over_inline_capacity_lands_intact_in_one_cell() {
    init();
    // 👨‍👩‍👧‍👦 = 👨 + ZWJ + 👩 + ZWJ + 👧 + ZWJ + 👦 — 4 people + 3 ZWJ =
    // 25 bytes, one byte past the 24-byte inline capacity. The cell must
    // take the transparent heap fallback and keep the full cluster.
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    assert_eq!(family.len(), 25, "guards the inline-capacity boundary");
    let (root_id, arena) = setup(&format!("<p>{family}</p>"), "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == family)
        .unwrap_or_else(|| panic!("25-byte cluster must land intact in one cell (heap fallback)"));
    assert!(
        tl.layout[idx + 1].word.is_empty(),
        "over-capacity cluster must still reserve its trailing empty cell"
    );
}

#[test]
fn test_skin_tone_family_cluster_lands_intact_in_one_cell() {
    init();
    // 👨🏻‍👩🏻‍👧🏻‍👦🏻 — four family members each with a skin-tone
    // modifier: 4×(4+4) + 3×3 = 41 bytes, well past inline capacity. This
    // is the longest cluster class we expect in real documents; the cell
    // must render it byte-for-byte (no truncation).
    let tones = [
        '\u{1F3FB}',
        '\u{1F3FC}',
        '\u{1F3FD}',
        '\u{1F3FE}',
        '\u{1F3FF}',
    ];
    let family: String = [
        '\u{1F468}',
        tones[1],
        '\u{200D}',
        '\u{1F469}',
        tones[4],
        '\u{200D}',
        '\u{1F467}',
        tones[0],
        '\u{200D}',
        '\u{1F466}',
        tones[2],
    ]
    .iter()
    .collect();
    assert_eq!(family.len(), 41, "guards the heap-fallback path");
    let (root_id, arena) = setup(&format!("<p>{family}</p>"), "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    assert!(
        tl.layout.iter().any(|b| b.word == family),
        "skin-tone pair cluster must land intact in one cell"
    );
}
