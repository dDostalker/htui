//! Tests for the two-phase layout pipeline introduced by the
//! `LayoutBox` / `layout_tree` / `paint_layout_tree` refactor.
//!
//! These tests target invariants that the previous single-pass
//! `LayoutFn + TermLayout.point` implementation could not preserve:
//!
//! * Each block-level child of the document claims a deterministic row.
//! * Nested block elements inherit `x` / `width` from their parent's
//!   containing block, so `text-align` is computed against the inner width
//!   rather than the terminal width.
//! * `<aside>` provides a containing block to its children (previously it
//!   called `collect_all_text()` and could not nest anything meaningful).

use crossterm::style::Color;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::tdom::node::{NodeArena, NodeId};
use htui::tui::graph_api::TermLayout;

const COLS: u16 = 80;
const ROWS: u16 = 24;

fn setup(html: &str, css: &str) -> (NodeId, NodeArena) {
    let mut html_parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = html_parser.parse().unwrap();
    let combined = if css.is_empty() {
        embedded
    } else if embedded.is_empty() {
        css.to_string()
    } else {
        format!("{embedded}\n{css}")
    };
    if !combined.trim().is_empty() {
        let mut css_parser = css_parse::Parser::new(&combined);
        let rules = css_parser.parse().unwrap();
        css_parse::css_into_node(&mut arena, root_id, &rules).unwrap();
    }
    (root_id, arena)
}

fn mock_term_map() -> TermMap {
    TermMap {
        columns: COLS,
        rows: ROWS,
    }
}

#[test]
fn two_paragraphs_land_on_different_rows() {
    let (root_id, arena) = setup("<p>first</p><p>second</p>", "");
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let first_idx = tl
        .layout
        .iter()
        .position(|b| b.word == *"f")
        .expect("first paragraph should render");
    let second_idx = tl
        .layout
        .iter()
        .rposition(|b| b.word == *"s")
        .expect("second paragraph should render");

    let first_row = first_idx / cols;
    let second_row = second_idx / cols;
    assert!(
        second_row > first_row,
        "second <p> must start on a row strictly below the first, got rows {first_row}/{second_row}"
    );
}

#[test]
fn nested_paragraph_in_aside_is_separate_row() {
    // The bug we want to lock down: previously <aside> called
    // `collect_all_text()`, so a nested <p> was inlined into the aside row
    // and could not honour a containing block. After the refactor, the
    // nested <p> gets its own row and the aside's bg no longer overlaps it.
    let (root_id, arena) = setup(
        "<aside><p>The rough-skinned newt defends itself.</p></aside>",
        "aside { background-color: darkblue; } p { color: yellow; }",
    );
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    // Find the aside row (it has darkblue bg).
    let aside_row = tl
        .layout
        .iter()
        .position(|b| b.bcolor == Some(Color::DarkBlue))
        .map(|i| i / cols)
        .expect("aside should paint a darkblue row");
    // Find the paragraph row (it has yellow text but no bg).
    let paragraph_row = tl
        .layout
        .iter()
        .position(|b| b.fcolor == Some(Color::Yellow))
        .map(|i| i / cols)
        .expect("nested paragraph should still render yellow text");
    assert!(
        paragraph_row > aside_row,
        "nested <p> must live on its own row below the aside, got rows {aside_row}/{paragraph_row}"
    );
}

#[test]
fn nested_paragraph_uses_parent_containing_block_for_text_align() {
    // When <aside width: 40px> contains <p> with inherited `text-align: center`,
    // the paragraph's text must be centered within the *aside* (40 cols), not
    // within the terminal (80 cols). The aside is centered, so its inner x is
    // 20 and the paragraph text should sit at col 20 + (40 - text_w) / 2.
    let html = "<aside><p>Centered</p></aside>";
    let css = "aside { width: 40px; background-color: darkblue; text-align: center; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == *"C")
        .expect("should find paragraph text");
    let col = text_idx % cols;
    let aside_left = (cols - 40) / 2;
    let text_w = "Centered".len();
    let expected = aside_left + (40 - text_w) / 2;
    assert_eq!(
        col, expected,
        "nested <p> text must be centered within the aside, col={col} expected={expected}"
    );
}

#[test]
fn nested_div_in_div_stacks_below_outer_div() {
    // Two block children of the outer div should each claim a row.
    let html = "<div><p>outer text</p><div><p>inner text</p></div></div>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let outer_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "o")
        .expect("outer <p> should render");
    let inner_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "i")
        .expect("inner <p> should render");

    assert!(
        inner_idx / cols > outer_idx / cols,
        "inner <p> must sit on a later row than outer <p>"
    );
}

#[test]
fn layout_box_rect_is_deterministic() {
    // Direct unit test on the layout tree: building it twice with the same
    // DOM must produce identical rectangles. This guards against the cursor
    // accidentally depending on HashMap iteration order or similar.
    use htui::layout::{BoxConstraint, Cursor, tree::layout_tree};

    let (root_id, arena) = setup(
        "<div>A</div><div>B</div><div>C</div>",
        "div { width: 20px; background-color: red; }",
    );
    let containing = BoxConstraint::root(&mock_term_map());
    let mut c1 = Cursor::new();
    let boxes1 = layout_tree(&arena, root_id, containing, &mut c1).unwrap();
    let mut c2 = Cursor::new();
    let boxes2 = layout_tree(&arena, root_id, containing, &mut c2).unwrap();
    assert_eq!(boxes1.len(), boxes2.len());
    for (a, b) in boxes1.iter().zip(boxes2.iter()) {
        assert_eq!(a.rect, b.rect, "boxes must agree across runs");
        assert_eq!(a.text, b.text);
    }
}

#[test]
fn rendered_rows_walks_full_layout_tree() {
    // Locks down the scroll-fix: `rendered_rows` (which drives `max_scroll`)
    // must reflect the deepest descendant's bottom row, not just the
    // top-level boxes. Regression test for the bug where nested `<p>`
    // children of `<div>` were ignored when computing `max_y`.
    use htui::layout::tree::layout_tree;

    // Layout sketch (each `<p>` and `<div>` claims one row):
    //   row 0: <div>            (empty)
    //   row 1: <p>A</p>
    //   row 2: <div>            (empty)
    //     row 3: <p>B</p>       <- deepest leaf
    let (root_id, arena) = setup("<div><p>A</p><div><p>B</p></div></div>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    // Sanity-check via the layout tree itself.
    use htui::layout::{BoxConstraint, Cursor};
    let mut cursor = Cursor::new();
    let boxes = layout_tree(&arena, root_id, BoxConstraint::root(&term_map), &mut cursor).unwrap();
    let max_leaf = boxes
        .iter()
        .flat_map(|b| std::iter::once(b).chain(b.children.iter()))
        .flat_map(|b| std::iter::once(b).chain(b.children.iter()))
        .map(|b| b.rect.y + b.rect.height.max(1))
        .max()
        .unwrap();
    assert_eq!(max_leaf, 4, "deepest leaf should land on row 4");

    assert_eq!(
        tl.rendered_rows as usize, max_leaf,
        "rendered_rows must match the deepest leaf in the layout tree, \
         otherwise max_scroll() collapses to 0 and scrolling is disabled"
    );
}

#[test]
fn scroll_limits_match_layout_depth() {
    // End-to-end: a DOM taller than the viewport must allow scrolling.
    let html = "<p>0</p><p>1</p><p>2</p><p>3</p><p>4</p>\
                <p>5</p><p>6</p><p>7</p><p>8</p><p>9</p>\
                <p>10</p><p>11</p><p>12</p><p>13</p><p>14</p>\
                <p>15</p><p>16</p><p>17</p><p>18</p><p>19</p>\
                <p>20</p><p>21</p><p>22</p><p>23</p><p>24</p>\
                <p>25</p><p>26</p><p>27</p>";
    let (root_id, arena) = setup(html, "");
    let term_map = TermMap {
        columns: 80,
        rows: 10,
    };
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();
    assert!(
        tl.rendered_rows > term_map.rows as u32,
        "DOM is taller than the viewport"
    );
    assert!(
        tl.max_scroll() > 0,
        "scroll must be enabled when content overflows"
    );
}
