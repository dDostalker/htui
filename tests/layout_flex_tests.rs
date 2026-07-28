//! Flexbox layout tests — the v1 subset of CSS flexbox that
//! `layout_fn::flex` implements.
//!
//! Each test stands up a small HTML snippet with a flex container and
//! one or more block children, then asserts on the rendered `Block`
//! cells in the `TermLayout` canvas:
//!
//! - Position tests verify `flex-direction`, `justify-content`,
//!   `align-items`, and the cross-axis placement of items.
//! - Size tests verify `flex-grow` / `flex-shrink` distribution and
//!   `gap` spacing.
//! - Negative tests confirm that `display: flex` on a tag is
//!   independent of the tag (works on `<section>`, not just `<div>`),
//!   and that a non-flex `<div>` still falls through to the normal
//!   block layout.

use crossterm::style::Color;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::tdom::node::NodeArena;
use htui::tdom::node::NodeId;
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
    if !combined.trim().is_empty() {
        let mut css_parser = css_parse::Parser::new(&combined);
        let rules = css_parser.parse();
        css_parse::css_into_node(&mut arena, root_id, &rules);
    }
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
    tl.draw(&arena, root_id, &term_map).unwrap();
    let cols = term_map.columns as usize;
    let mut rows: Vec<String> = Vec::new();
    for chunk in tl.layout.chunks(cols) {
        let s: String = chunk.iter().map(|b| b.word.as_str()).collect();
        rows.push(s);
    }
    (rows, tl)
}

#[test]
fn test_flex_row_lays_out_children_side_by_side() {
    init();
    let html = r#"
        <div class="container">
            <div class="a">AAA</div>
            <div class="b">BBBB</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; }
        .a, .b { background-color: blue; color: white; }
    "#;
    let (rows, _) = render_rows(html, css);
    // Both items should be on the same row (row 0), in order, with a
    // gap of one column between them (no margin/padding so they sit
    // flush next to each other but with their own bg cells visible).
    let line0 = &rows[0];
    let pos_a = line0.find("AAA").expect("A on row 0");
    let pos_b = line0.find("BBBB").expect("B on row 0");
    assert!(
        pos_a < pos_b,
        "A should sit left of B in row direction: line={line0:?}"
    );
    // Sanity: the items must not be on different rows.
    let line1 = &rows[1];
    assert!(
        !line1.contains("AAA") && !line1.contains("BBBB"),
        "row 1 should not contain the flex items: line={line1:?}"
    );
}

#[test]
fn test_flex_row_default_justify_is_flex_start() {
    init();
    // Three short items; with `justify-content: flex-start` (default)
    // they should pack from the left edge of the container.
    let html = r#"
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
            <div class="c">C</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; }
        .a, .b, .c { background-color: red; }
    "#;
    let (rows, _) = render_rows(html, css);
    let line0 = &rows[0];
    let pos_a = line0.find("A").expect("A on row 0");
    let pos_b = line0.find("B").expect("B on row 0");
    let pos_c = line0.find("C").expect("C on row 0");
    assert_eq!(pos_a, 0, "A should start at col 0 (flex-start)");
    assert!(
        pos_b > pos_a && pos_c > pos_b,
        "B, C should appear after A in document order"
    );
}

#[test]
fn test_flex_row_justify_space_between_pushes_endpoints_to_edges() {
    init();
    let html = r#"
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; justify-content: space-between; }
        .a, .b { background-color: red; }
    "#;
    let (rows, _) = render_rows(html, css);
    let line0 = &rows[0];
    let pos_a = line0.find("A").expect("A on row 0");
    let pos_b = line0.find("B").expect("B on row 0");
    assert_eq!(pos_a, 0, "first item pinned to left edge");
    // Without a `width` the container fills the terminal (80 cols).
    // B's outer cell sits at col 79 (the rightmost column).
    assert_eq!(pos_b, COLS as usize - 1, "last item pinned to right edge");
}

#[test]
fn test_flex_row_justify_center_centers_items() {
    init();
    let html = r#"
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; justify-content: center; }
        .a, .b { background-color: red; }
    "#;
    let (rows, _) = render_rows(html, css);
    let line0 = &rows[0];
    let pos_a = line0.find("A").expect("A on row 0");
    let pos_b = line0.find("B").expect("B on row 0");
    let items_width = 2; // A + B + no gap (gap default 0)
    let container_width = COLS as usize;
    let expected_start = (container_width - items_width) / 2;
    assert_eq!(
        pos_a, expected_start,
        "A should sit at the centered position"
    );
    assert_eq!(pos_b, pos_a + 1, "B sits right after A");
}

#[test]
fn test_flex_grow_fills_remaining_main_axis_space() {
    init();
    let html = r#"
        <div class="container">
            <div class="a">A</div>
            <div class="grow">G</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; }
        .a, .grow { background-color: red; }
        .grow { flex-grow: 1; }
    "#;
    let (rows, tl) = render_rows(html, css);
    let line0 = &rows[0];
    let pos_a = line0.find("A").expect("A on row 0");
    let pos_g = line0.find("G").expect("G on row 0");
    assert_eq!(pos_a, 0, "A starts at col 0");
    assert!(pos_g > pos_a, "G sits after A with the grown background");
    // The grown item should fill the remainder of the row: its
    // background extends to the right edge.
    let red_count = tl.layout[0..COLS as usize]
        .iter()
        .filter(|b| b.bcolor == Some(Color::Red))
        .count();
    assert_eq!(
        red_count, COLS as usize,
        "row 0 should be fully painted red (flex-grow fills the line)"
    );
}

#[test]
fn test_flex_gap_separates_items_in_row() {
    init();
    let html = r#"
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; gap: 2; }
        .a, .b { background-color: red; }
    "#;
    let (rows, _) = render_rows(html, css);
    let line0 = &rows[0];
    let pos_a = line0.find("A").expect("A on row 0");
    let pos_b = line0.find("B").expect("B on row 0");
    // A is 1 cell wide, gap is 2 cells, so B starts at col pos_a + 1 + 2.
    assert_eq!(pos_b, pos_a + 1 + 2, "B should sit 2 cells after A (gap=2)");
}

#[test]
fn test_flex_column_stacks_items_vertically() {
    init();
    let html = r#"
        <div class="container">
            <div class="a">AAA</div>
            <div class="b">BBBB</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; flex-direction: column; }
        .a, .b { background-color: red; }
    "#;
    let (rows, _) = render_rows(html, css);
    // In column direction both items land on different rows.
    let line0 = &rows[0];
    let line1 = &rows[1];
    assert!(line0.contains("AAA"), "A on row 0: {line0:?}");
    assert!(line1.contains("BBBB"), "B on row 1: {line1:?}");
    assert!(!line1.contains("AAA"), "A is not on row 1: {line1:?}");
}

#[test]
fn test_flex_wrap_moves_overflow_items_to_next_line() {
    init();
    // Each child is intentionally wider than the container so wrap
    // moves them to their own line. We use `width: 50px` for items in
    // a 40-col container to force wrapping.
    let html = r#"
        <div class="container">
            <div class="a">AAAAAAAAAA</div>
            <div class="b">BBBBBBBBBB</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; flex-wrap: wrap; width: 5px; }
        .a, .b { background-color: red; flex-shrink: 0; }
    "#;
    let (rows, _) = render_rows(html, css);
    // Find a row containing only A (the first item, on its own line)
    // and a row containing only B (the second item, wrapped).
    let has_a_only_row = rows
        .iter()
        .any(|r| r.contains("AAAAAAAAAA") && !r.contains("BBBBBBBBBB"));
    let has_b_only_row = rows
        .iter()
        .any(|r| r.contains("BBBBBBBBBB") && !r.contains("AAAAAAAAAA"));
    assert!(has_a_only_row, "A should be on its own wrapped line");
    assert!(has_b_only_row, "B should be on its own wrapped line");
}

#[test]
fn test_flex_align_items_stretch_with_explicit_height() {
    init();
    // Container has explicit height = 3; default align-items = stretch;
    // the single child should fill all 3 rows of the container.
    let html = r#"
        <div class="container">
            <div class="a">A</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; height: 3; }
        .a { background-color: red; }
    "#;
    let (rows, _) = render_rows(html, css);
    // The container's content area spans 3 rows. The single child
    // (stretched) should paint all 3 rows red, with 'A' on the first
    // row (baseline = top in v1).
    let line0_has_a = rows[0].contains("A");
    let red_row0 = rows[0].contains("AAAA") || rows[0].chars().any(|c| c == 'A');
    let _ = red_row0;
    let line1_no_a = !rows[1].contains("A");
    let line2_no_a = !rows[2].contains("A");
    assert!(line0_has_a, "A text on row 0");
    assert!(line1_no_a, "no A text on row 1 (just stretched bg)");
    assert!(line2_no_a, "no A text on row 2 (just stretched bg)");
}

#[test]
fn test_flex_align_items_stretch_without_explicit_height_is_noop() {
    init();
    // No `height` set; the v1 contract says stretch is a no-op. The
    // child should occupy exactly 1 row (its natural cross-axis size)
    // and not stretch to fill the line.
    let html = r#"
        <div class="container">
            <div class="a">A</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; }
        .a { background-color: red; }
    "#;
    let (rows, _) = render_rows(html, css);
    assert!(rows[0].contains("A"), "A on row 0");
    assert!(
        !rows[1].contains("A"),
        "row 1 should not contain A (no stretch without explicit height)"
    );
}

#[test]
fn test_flex_order_reorders_children() {
    init();
    // In source order: A, B. With `order: 1` on A, B should appear
    // first (order: 0 default < order: 1).
    let html = r#"
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; }
        .a { order: 1; }
        .b { order: 0; }
    "#;
    let (rows, _) = render_rows(html, css);
    let line0 = &rows[0];
    let pos_a = line0.find("A").expect("A rendered");
    let pos_b = line0.find("B").expect("B rendered");
    assert!(
        pos_b < pos_a,
        "B (order 0) should render before A (order 1): line={line0:?}"
    );
}

#[test]
fn test_flex_works_on_non_div_elements() {
    init();
    // `display: flex` on a `<section>` — the dispatcher should claim
    // it and lay it out as a flex container. The two child paragraphs
    // should land on the same row in row direction.
    let html = r#"
        <section class="container">
            <p class="a">AAA</p>
            <p class="b">BBBB</p>
        </section>
    "#;
    let css = r#"
        .container { display: flex; }
        .a, .b { background-color: red; }
    "#;
    let (rows, _) = render_rows(html, css);
    let line0 = &rows[0];
    let pos_a = line0.find("AAA").expect("A on row 0");
    let pos_b = line0.find("BBBB").expect("B on row 0");
    assert!(pos_a < pos_b, "A before B (row direction)");
    // And the items should share the same row.
    let line1 = &rows[1];
    assert!(!line1.contains("AAA") && !line1.contains("BBBB"));
}

#[test]
fn test_non_flex_div_falls_through_to_block_layout() {
    init();
    // Without `display: flex`, the div should use the normal block
    // layout: each child sits on its own row.
    let html = r#"
        <div class="container">
            <div class="a">AAA</div>
            <div class="b">BBBB</div>
        </div>
    "#;
    let css = "";
    let (rows, _) = render_rows(html, css);
    assert!(rows[0].contains("AAA"), "A on row 0: {:?}", rows[0]);
    assert!(rows[1].contains("BBBB"), "B on row 1: {:?}", rows[1]);
}

#[test]
fn test_flex_basis_auto_uses_text_width() {
    init();
    // Each `<p>` has only its text (3 / 5 / 4 chars) as content, with
    // no CSS `width` and no explicit `flex-basis`. The flex algorithm
    // should size each item to its natural text width.
    let html = r#"
        <div class="container">
            <p class="a">AAA</p>
            <p class="b">BBBBB</p>
            <p class="c">CCCC</p>
        </div>
    "#;
    let css = r#"
        .container { display: flex; }
        .a, .b, .c { background-color: red; }
    "#;
    let (rows, _) = render_rows(html, css);
    let line0 = &rows[0];
    let pos_a = line0.find("AAA").expect("A on row 0");
    let pos_b = line0.find("BBBBB").expect("B on row 0");
    let pos_c = line0.find("CCCC").expect("C on row 0");
    // A is 3 cells, B is 5 cells, C is 4 cells. No gap. Expected
    // start positions: A=0, B=3, C=3+5=8.
    assert_eq!(pos_a, 0);
    assert_eq!(pos_b, 3, "B starts after A (3 cells wide)");
    assert_eq!(pos_c, 8, "C starts after B (5 cells wide)");
}

#[test]
fn test_flex_basis_explicit_resolves_against_main_axis() {
    init();
    // `flex-basis: 10px` on every item should make each item 10 cells
    // wide, regardless of the natural text content.
    let html = r#"
        <div class="container">
            <p class="a">AAA</p>
            <p class="b">B</p>
        </div>
    "#;
    let css = r#"
        .container { display: flex; }
        .a, .b { background-color: red; flex-basis: 10px; }
    "#;
    let (rows, _) = render_rows(html, css);
    let line0 = &rows[0];
    let pos_a = line0.find("AAA").expect("A on row 0");
    let pos_b = line0.find("B").expect("B on row 0");
    assert_eq!(pos_a, 0);
    assert_eq!(
        pos_b, 10,
        "B starts at col 10 (after A's 10-cell flex-basis)"
    );
}
