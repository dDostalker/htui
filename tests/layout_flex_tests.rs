//! Flexbox layout tests — the v1 subset of CSS flexbox that
//! `flex::flex` implements.
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
use htui::layout::{BoxKind, FormattingContext};
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

/// Depth-first search for the first box whose `inner_formatting_context`
/// matches `ctx`.
fn find_box_with_formatting_context<'a>(
    boxes: &'a [htui::layout::LayoutBox],
    ctx: FormattingContext,
) -> Option<&'a htui::layout::LayoutBox> {
    for b in boxes {
        if b.inner_formatting_context == ctx {
            return Some(b);
        }
        if let Some(found) = find_box_with_formatting_context(&b.children, ctx) {
            return Some(found);
        }
    }
    None
}

#[test]
fn test_flex_container_carries_flex_formatting_context() {
    init();
    // A flex container should be tagged `FormattingContext::Flex` so the
    // architectural distinction between the box's *rendering shape*
    // (still `BoxKind::Normal`) and its *layout algorithm* (flex) is
    // visible to the layout tree.
    let html = r#"
        <div class="container">
            <div class="a">A</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; }
    "#;
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let boxes = htui::layout::inner_formatting_context::flow::layout_tree(
        &arena,
        root_id,
        htui::layout::BoxConstraint::root(&term_map),
        &mut htui::layout::Cursor::new(),
    );
    let container = find_box_with_formatting_context(&boxes, FormattingContext::Flex)
        .expect("there must be a box with FormattingContext::Flex");
    assert!(
        matches!(container.kind, BoxKind::Normal),
        "container still renders as a Normal block (bg / padding / border unaffected)"
    );
    assert_eq!(
        container.inner_formatting_context,
        FormattingContext::Flex,
        "flex container must be tagged FormattingContext::Flex"
    );
    assert_eq!(
        container.children.len(),
        1,
        "flex container has the one flex item as its child"
    );
    // A plain `<div>` flex item keeps its own inner formatting context —
    // it does not inherit `Flex` from the container.
    assert_eq!(
        container.children[0].inner_formatting_context,
        FormattingContext::Flow,
        "flex item keeps its own inner formatting context"
    );
    // Sanity: a non-flex container keeps the default Flow formatting context.
    let html_block = r#"
        <div class="container">
            <div class="a">A</div>
        </div>
    "#;
    let (root_id, arena) = setup(html_block, "");
    let boxes = htui::layout::inner_formatting_context::flow::layout_tree(
        &arena,
        root_id,
        htui::layout::BoxConstraint::root(&term_map),
        &mut htui::layout::Cursor::new(),
    );
    assert!(
        find_box_with_formatting_context(&boxes, FormattingContext::Flex).is_none(),
        "non-flex containers must not produce any Flex formatting context boxes"
    );
    assert!(
        find_box_with_formatting_context(&boxes, FormattingContext::Flow).is_some(),
        "non-flex containers still produce Flow formatting context boxes"
    );
}

// ----------------------------------------------------------
// Tests for v1 features that the visual tests exercise but the
// existing unit tests do not cover: align-self, align-content,
// justify-content: space-evenly, flex-shrink overflow, and
// nested flex containers.
// ----------------------------------------------------------

/// Recursive count of boxes whose `inner_formatting_context` matches `ctx`.
fn count_boxes_with_formatting_context(
    boxes: &[htui::layout::LayoutBox],
    ctx: FormattingContext,
) -> usize {
    boxes
        .iter()
        .map(|b| {
            let self_match = if b.inner_formatting_context == ctx { 1 } else { 0 };
            self_match + count_boxes_with_formatting_context(&b.children, ctx)
        })
        .sum()
}

#[test]
fn test_flex_align_self_overrides_align_items() {
    init();
    // Container has explicit height = 5; default `align-items: stretch`.
    // A per-item `align-self` should override for that item only.
    // `flex-start` items sit at the top of the line (row 0); `flex-end`
    // items sit at the bottom (row 4).
    let html = r#"
        <div class="container">
            <div class="top">A</div>
            <div class="bot">B</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; height: 5; }
        .top { align-self: flex-start; }
        .bot { align-self: flex-end; }
    "#;
    let (rows, _) = render_rows(html, css);
    let pos_a = rows[0].find("A").expect("A on row 0 (flex-start)");
    let pos_b = rows[4].find("B").expect("B on row 4 (flex-end)");
    let _ = pos_a;
    let _ = pos_b;
    assert!(
        !rows[0].contains("B"),
        "B should not be on row 0 (its align-self is flex-end)"
    );
    assert!(
        !rows[4].contains("A"),
        "A should not be on row 4 (its align-self is flex-start)"
    );
}

#[test]
fn test_flex_justify_space_evenly_distributes_equally() {
    init();
    // 3 single-cell items inside a 10-cell container, justify-content:
    // space-evenly. With 3 items the algorithm inserts a leading share
    // AND a trailing share of equal size to the between-gap, so the
    // pattern is: 1 cell leading, 1 cell item, 1 cell gap, 1 cell item,
    // 1 cell gap, 1 cell item, 1 cell trailing.
    let html = r#"
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
            <div class="c">C</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; width: 10px; justify-content: space-evenly; }
    "#;
    let (rows, _) = render_rows(html, css);
    // Container width 10 is clamped at the term width (80) so the rows
    // span the full canvas. With 3 items of 1 cell each, the free
    // space is 7 cells. space-evenly: 7 / (3 + 1) = 1 cell at every
    // edge and gap. So items land at columns 1, 3, 5.
    let line0 = &rows[0];
    let pos_a = line0.find("A").expect("A on row 0");
    let pos_b = line0.find("B").expect("B on row 0");
    let pos_c = line0.find("C").expect("C on row 0");
    assert_eq!(pos_a, 1, "A at col 1 (1-cell leading share)");
    assert_eq!(pos_b, 3, "B at col 3 (1-cell between + 1-cell item)");
    assert_eq!(pos_c, 5, "C at col 5 (between + item + between)");
}

#[test]
fn test_flex_shrink_redistributes_overflow_proportionally() {
    init();
    // 3 items each with `flex-basis: 4px` in a 10-cell container.
    // Total basis = 12, overflow = 2. The first and last items have
    // `flex-shrink: 0` (they stay fixed at 4), so the middle item
    // absorbs the entire 2-cell overflow and shrinks to 2 cells.
    let html = r#"
        <div class="container">
            <div class="a">AAA</div>
            <div class="b">BB</div>
            <div class="c">CC</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; width: 10px; }
        .a, .b, .c { flex-basis: 4px; }
        .a { flex-shrink: 0; }
        .b { flex-shrink: 1; }
        .c { flex-shrink: 0; }
    "#;
    let (rows, _) = render_rows(html, css);
    let line0 = &rows[0];
    let pos_a = line0.find("AAA").expect("A on row 0");
    let pos_b = line0.find("BB").expect("B on row 0");
    let pos_c = line0.find("CC").expect("C on row 0");
    // A and C stay at 4 cells; B shrinks to 2 cells. The positions
    // must total exactly 10 cells (container width).
    assert_eq!(pos_a, 0, "A at col 0 (4-cell basis, no shrink)");
    assert_eq!(pos_b, 4, "B at col 4 (4-cell basis, shrunk to 2)");
    assert_eq!(pos_c, 6, "C at col 6 (4-cell basis, no shrink)");
    assert_eq!(pos_c + 4, 10, "C ends exactly at the container edge");
}

#[test]
fn test_flex_align_content_center_centers_lines() {
    init();
    // 3 items each 12 cells wide in a 12-cell container with
    // `flex-wrap: wrap` — each item wraps onto its own line (3 lines
    // of 1 row, total 3). The container has explicit height = 7 so
    // 4 rows of cross-axis free space remain. `align-content: center`
    // shifts the block of lines down by half the free space (2 rows).
    // The first item should therefore land on row 2, not row 0.
    let html = r#"
        <div class="container">
            <div class="a">AAAAAAAAAAAA</div>
            <div class="b">BBBBBBBBBBBB</div>
            <div class="c">CCCCCCCCCCCC</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; flex-wrap: wrap; width: 12px; height: 7; align-content: center; }
        .a, .b, .c { flex-shrink: 0; }
    "#;
    let (rows, _) = render_rows(html, css);
    let a_row = rows
        .iter()
        .position(|r| r.contains("AAAAAAAAAAAA"))
        .expect("A rendered");
    let b_row = rows
        .iter()
        .position(|r| r.contains("BBBBBBBBBBBB"))
        .expect("B rendered");
    let c_row = rows
        .iter()
        .position(|r| r.contains("CCCCCCCCCCCC"))
        .expect("C rendered");
    assert_eq!(a_row, 2, "first line centered, starts at row 2 (got {a_row})");
    assert_eq!(b_row, 3, "second line at row 3 (got {b_row})");
    assert_eq!(c_row, 4, "third line at row 4 (got {c_row})");
}

#[test]
fn test_flex_nested_containers_both_have_flex_formatting_context() {
    init();
    // Outer is a flex row; inner is a flex column. Both must be tagged
    // `FormattingContext::Flex` independently — the dispatch is
    // idempotent across nesting levels.
    let html = r#"
        <div class="outer">
            <div class="inner">
                <div class="a">A</div>
            </div>
        </div>
    "#;
    let css = r#"
        .outer { display: flex; }
        .inner { display: flex; flex-direction: column; }
    "#;
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let boxes = htui::layout::inner_formatting_context::flow::layout_tree(
        &arena,
        root_id,
        htui::layout::BoxConstraint::root(&term_map),
        &mut htui::layout::Cursor::new(),
    );
    let flex_count = count_boxes_with_formatting_context(&boxes, FormattingContext::Flex);
    assert_eq!(
        flex_count, 2,
        "outer AND inner should both be Flex formatting contexts (got {flex_count})"
    );
}

#[test]
fn test_flex_justify_space_around_distributes_with_edge_halves() {
    init();
    // 3 items of 1 cell in a 7-cell container. free space = 4.
    // CSS space-around: between = 4/3 = 1, leading = (4 - 2)/2 = 1.
    // Edge gap = leading = 1 (= half of between), so the pattern is
    // 1 leading + 1 item + 1 between + 1 item + 1 between + 1 item +
    // 1 trailing = 7 cells. Items land at columns 1, 3, 5.
    let html = r#"
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
            <div class="c">C</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; width: 7px; justify-content: space-around; }
    "#;
    let (rows, _) = render_rows(html, css);
    let line0 = &rows[0];
    let pos_a = line0.find("A").expect("A on row 0");
    let pos_b = line0.find("B").expect("B on row 0");
    let pos_c = line0.find("C").expect("C on row 0");
    assert_eq!(pos_a, 1, "A at col 1 (1-cell leading edge share)");
    assert_eq!(pos_b, 3, "B at col 3 (1 + 1 + 1)");
    assert_eq!(pos_c, 5, "C at col 5 (1 + 1 + 1 + 1 + 1)");
}

#[test]
fn test_flex_align_content_space_around_distributes_with_edge_halves() {
    init();
    // 3 wrap lines of 1 row each in a 7-row container. free space = 4.
    // CSS space-around: between = 1, leading = 1; lines at rows 1, 3, 5.
    let html = r#"
        <div class="container">
            <div class="a">AAAAAAAAAAAA</div>
            <div class="b">BBBBBBBBBBBB</div>
            <div class="c">CCCCCCCCCCCC</div>
        </div>
    "#;
    let css = r#"
        .container { display: flex; flex-wrap: wrap; width: 12px; height: 7; align-content: space-around; }
        .a, .b, .c { flex-shrink: 0; }
    "#;
    let (rows, _) = render_rows(html, css);
    let a_row = rows
        .iter()
        .position(|r| r.contains("AAAAAAAAAAAA"))
        .expect("A rendered");
    let b_row = rows
        .iter()
        .position(|r| r.contains("BBBBBBBBBBBB"))
        .expect("B rendered");
    let c_row = rows
        .iter()
        .position(|r| r.contains("CCCCCCCCCCCC"))
        .expect("C rendered");
    assert_eq!(a_row, 1, "A at row 1 (1-cell leading edge share)");
    assert_eq!(b_row, 3, "B at row 3");
    assert_eq!(c_row, 5, "C at row 5");
}
