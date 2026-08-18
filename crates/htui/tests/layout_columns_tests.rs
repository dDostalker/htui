//! CSS Multi-column layout tests (`column-count` / `column-width` /
//! `column-gap` / `column-fill` / `column-rule`).
//!
//! Each test renders a small HTML snippet through the full pipeline (parse →
//! cascade → layout → paint) and asserts on the cells of the `TermLayout`
//! canvas: where each column's content landed, whether gaps separate the
//! columns, and whether column rules were painted.

use crossterm::style::Color;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::tui::graph_api::TermLayout;

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

fn setup(html: &str, css: &str) -> TermLayout {
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
    let term_map = TermMap {
        columns: 80,
        rows: 24,
    };
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    tl
}

fn row(tl: &TermLayout, y: usize) -> String {
    let cols = 80usize;
    let start = y.saturating_mul(cols);
    tl.layout[start..start + cols]
        .iter()
        .map(|b| b.word.as_str())
        .collect()
}

fn cell_word(tl: &TermLayout, x: usize, y: usize) -> String {
    tl.layout[y * 80 + x].word.clone()
}

fn cell_color(tl: &TermLayout, x: usize, y: usize) -> Option<Color> {
    tl.layout[y * 80 + x].fcolor
}

// ============================================================
// column-count: block children fragment into balanced columns
// ============================================================

#[test]
fn test_multicol_count_fragments_block_children() {
    init();
    // 60-col container, 2 columns, no gap → 30 cells per column. Four 1-row
    // children balance 2/2: the first two land in column 0 (x 0..29), the
    // last two in column 1 (x 30..59), top-aligned on the same rows.
    let html = r#"
        <div class="cols">
            <div>A1</div><div>A2</div><div>B1</div><div>B2</div>
        </div>
    "#;
    let css = ".cols { width: 60px; column-count: 2; column-gap: 0; }";
    let tl = setup(html, css);
    // Column 0 holds A1/A2 stacked at x 0; column 1 holds B1/B2 at x 30.
    assert_eq!(row(&tl, 0), format!("A1{}B1{}", " ".repeat(28), " ".repeat(48)));
    assert_eq!(row(&tl, 1), format!("A2{}B2{}", " ".repeat(28), " ".repeat(48)));
    assert_eq!(row(&tl, 2), " ".repeat(80), "content fits in two rows");
}

#[test]
fn test_multicol_balance_uneven_content() {
    init();
    // 5 one-row children across 3 columns → target 2 rows per column: 2/2/1.
    let html = r#"
        <div class="cols">
            <div>A</div><div>B</div><div>C</div><div>D</div><div>E</div>
        </div>
    "#;
    let css = ".cols { width: 60px; column-count: 3; column-gap: 0; }";
    let tl = setup(html, css);
    // Column 0: A, B at x 0. Column 1: C, D at x 20. Column 2: E at x 40.
    assert_eq!(cell_word(&tl, 0, 0), "A");
    assert_eq!(cell_word(&tl, 0, 1), "B");
    assert_eq!(cell_word(&tl, 20, 0), "C");
    assert_eq!(cell_word(&tl, 20, 1), "D");
    assert_eq!(cell_word(&tl, 40, 0), "E");
    assert_eq!(cell_word(&tl, 40, 1), " ");
    assert_eq!(cell_word(&tl, 40, 2), " ");
}

#[test]
fn test_multicol_balance_packs_uneven_heights_browser_style() {
    init();
    // Three 3-row children (padding) in 2 columns: the smallest column
    // height that fits both columns is 6, so the first two children pack
    // into column 0 (6/3) — not the naive ceil(9/2)=5 split (3/6).
    let html = r#"
        <div class="cols">
            <div class="t">A</div><div class="t">B</div><div class="t">C</div>
        </div>
    "#;
    let css = ".cols { width: 60px; column-count: 2; column-gap: 0; } .t { padding: 1px; }";
    let tl = setup(html, css);
    // Each `.t` box is 3 rows tall (padding 1 + content 1 + padding 1), so
    // the content glyph sits at x 1 (padding-left) on the box's middle row.
    assert_eq!(cell_word(&tl, 1, 1), "A", "first child in column 0");
    assert_eq!(
        cell_word(&tl, 1, 4),
        "B",
        "second child packs under A in column 0"
    );
    assert_eq!(cell_word(&tl, 31, 1), "C", "third child starts column 1");
    assert_eq!(cell_word(&tl, 31, 4), " ", "column 1 has no fourth row");
    assert_eq!(cell_word(&tl, 1, 6), " ", "container is 6 rows tall");
}

// ============================================================
// column-width: the count derives from how many fit
// ============================================================

#[test]
fn test_multicol_column_width_derives_count() {
    init();
    // `column-width: 20px` in the default 80-col viewport → 4 columns of
    // 20 cells, no gap. Four one-row children fill one column each.
    let html = r#"
        <div class="cols">
            <div>A</div><div>B</div><div>C</div><div>D</div>
        </div>
    "#;
    let css = ".cols { column-width: 20px; column-gap: 0; }";
    let tl = setup(html, css);
    // `column-width: 20px` in 80 cols → 4 columns of 20 cells, no gap. Four
    // one-row children fill one column each, all on the top row.
    assert_eq!(cell_word(&tl, 0, 0), "A");
    assert_eq!(cell_word(&tl, 20, 0), "B");
    assert_eq!(cell_word(&tl, 40, 0), "C");
    assert_eq!(cell_word(&tl, 60, 0), "D");
    assert_eq!(cell_word(&tl, 20, 1), " ", "each column holds exactly one item");
}

#[test]
fn test_multicol_columns_shorthand() {
    init();
    // `columns: 30px 2` sets both width and count; the smaller wins and the
    // columns widen to fill the 60-col container (30 each).
    let html = r#"
        <div class="cols">
            <div>A</div><div>B</div><div>C</div><div>D</div>
        </div>
    "#;
    let css = ".cols { width: 60px; columns: 30px 2; column-gap: 0; }";
    let tl = setup(html, css);
    assert_eq!(cell_word(&tl, 0, 0), "A");
    assert_eq!(cell_word(&tl, 0, 1), "B");
    assert_eq!(cell_word(&tl, 30, 0), "C");
    assert_eq!(cell_word(&tl, 30, 1), "D");
}

// ============================================================
// column-gap
// ============================================================

#[test]
fn test_multicol_gap_spaces_columns() {
    init();
    // gap 4 in the 80-col container → each column is (80-4)/2 = 38 wide and
    // column 1 starts at x 42.
    let html = r#"
        <div class="cols">
            <div>A</div><div>B</div>
        </div>
    "#;
    let css = ".cols { column-count: 2; column-gap: 4px; }";
    let tl = setup(html, css);
    assert_eq!(cell_word(&tl, 0, 0), "A");
    assert_eq!(cell_word(&tl, 42, 0), "B");
    assert_eq!(cell_word(&tl, 38, 0), " ", "gap is blank");
    assert_eq!(cell_word(&tl, 41, 0), " ");
}

#[test]
fn test_multicol_percentage_gap_resolves_against_container() {
    init();
    // `column-gap: 50%` of a 60-col container is 30 cells (unlike the flex
    // `gap`, the multicol container width is in scope). Two columns then get
    // (60-30)/2 = 15 cells each; column 1 starts at x 45.
    let html = r#"
        <div class="cols">
            <div>A</div><div>B</div>
        </div>
    "#;
    let css = ".cols { width: 60px; column-count: 2; column-gap: 50%; }";
    let tl = setup(html, css);
    assert_eq!(cell_word(&tl, 0, 0), "A");
    assert_eq!(cell_word(&tl, 45, 0), "B");
    assert_eq!(cell_word(&tl, 15, 0), " ", "half-width gap");
}

// ============================================================
// column-fill: auto fills the first column up to the container
// height before moving on
// ============================================================

#[test]
fn test_multicol_fill_auto_stacks_first_column() {
    init();
    // `column-fill: auto` with an explicit 4-row height packs all four
    // 1-row children into column 0 (4 ≤ 4); balance would have split 2/2.
    let html = r#"
        <div class="cols">
            <div>A</div><div>B</div><div>C</div><div>D</div>
        </div>
    "#;
    let css = ".cols { column-count: 2; column-gap: 0; column-fill: auto; height: 4px; }";
    let tl = setup(html, css);
    for y in 0..4 {
        assert_eq!(cell_word(&tl, 0, y), "ABCD".chars().nth(y).unwrap().to_string());
        assert_eq!(
            cell_word(&tl, 40, y),
            " ",
            "column 1 stays empty when column 0 fills the whole height"
        );
    }
}

// ============================================================
// Inline text fragments into columns too
// ============================================================

#[test]
fn test_multicol_inline_text_fragments() {
    init();
    // Six forced `<br>` lines of "AAAA".."FFFF" in a 60-col 3-column
    // container → 2 lines per column, top-aligned.
    let html = r#"
        <div class="cols">
            AAAA<br>BBBB<br>CCCC<br>DDDD<br>EEEE<br>FFFF
        </div>
    "#;
    let css = ".cols { width: 60px; column-count: 3; column-gap: 0; }";
    let tl = setup(html, css);
    assert_eq!(cell_word(&tl, 0, 0), "A");
    assert_eq!(cell_word(&tl, 1, 0), "A");
    assert_eq!(cell_word(&tl, 0, 1), "B");
    assert_eq!(cell_word(&tl, 20, 0), "C");
    assert_eq!(cell_word(&tl, 20, 1), "D");
    assert_eq!(cell_word(&tl, 40, 0), "E");
    assert_eq!(cell_word(&tl, 40, 1), "F");
    assert_eq!(cell_word(&tl, 40, 2), " ", "6 lines fill 2 rows per column");
}

// ============================================================
// column-rule: vertical rule painted between the columns
// ============================================================

#[test]
fn test_multicol_column_rule_painted() {
    init();
    // gap 4 → the rule sits centered in the gap at x 40 (boundary 38 + 2),
    // painted with the declared colour across the content rows.
    let html = r#"
        <div class="cols">
            <div>A</div><div>B</div><div>C</div><div>D</div>
        </div>
    "#;
    let css = ".cols { column-count: 2; column-gap: 4px; column-rule: 1px solid red; }";
    let tl = setup(html, css);
    assert_eq!(cell_word(&tl, 40, 0), "│", "rule glyph in the gap");
    assert_eq!(
        cell_color(&tl, 40, 0),
        Some(Color::Rgb { r: 255, g: 0, b: 0 }),
        "rule uses its colour"
    );
    assert_eq!(cell_word(&tl, 40, 1), "│", "rule spans the content height");
    assert_eq!(cell_word(&tl, 39, 0), " ", "gap blank left of the rule");
    assert_eq!(cell_word(&tl, 41, 0), " ");
}

#[test]
fn test_multicol_no_rule_without_style() {
    init();
    // A `column-rule-color` without a style paints nothing.
    let html = r#"
        <div class="cols">
            <div>A</div><div>B</div>
        </div>
    "#;
    let css = ".cols { column-count: 2; column-gap: 4px; column-rule-color: blue; }";
    let tl = setup(html, css);
    assert_eq!(cell_word(&tl, 40, 0), " ", "no rule without a style");
}

// ============================================================
// Degenerate cases fall back to single-column block layout
// ============================================================

#[test]
fn test_multicol_count_one_falls_back_to_single_column() {
    init();
    // `column-count: 1` (or a column-width that only fits one column) must
    // not fragment: children stack in one column exactly like normal block
    // flow.
    let html = r#"
        <div class="cols">
            <div>A</div><div>B</div>
        </div>
    "#;
    let css = ".cols { column-count: 1; }";
    let tl = setup(html, css);
    assert_eq!(cell_word(&tl, 0, 0), "A");
    assert_eq!(cell_word(&tl, 0, 1), "B");
    assert_eq!(cell_word(&tl, 40, 0), " ");
}

#[test]
fn test_multicol_narrow_container_reduces_count() {
    init();
    // `column-count: 3` on a 3-col container with a 1-col gap budget leaves
    // no room for 3 columns (3*1 + 2*gap would overflow) → count shrinks to
    // what fits.
    let html = r#"
        <div class="cols">
            <div>A</div><div>B</div>
        </div>
    "#;
    let css = ".cols { width: 4px; column-count: 3; column-gap: 0; }";
    let tl = setup(html, css);
    assert_eq!(cell_word(&tl, 0, 0), "A");
    assert_eq!(cell_word(&tl, 1, 0), "B", "4 cells fit 4 one-cell columns");
}
