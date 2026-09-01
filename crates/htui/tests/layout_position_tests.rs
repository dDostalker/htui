//! CSS Positioned Layout tests — `position` (relative / absolute / fixed) and
//! `z-index` stacking.
//!
//! Each test renders a small document into a `TermLayout` canvas and asserts
//! on the `Block` cells:
//!
//! - `relative` offsets a box without moving its flow siblings.
//! - `absolute` / `fixed` boxes are removed from flow (no row claims) and
//!   anchored to their containing block (nearest positioned ancestor, the
//!   initial containing block, or the viewport).
//! - `z-index` orders overlapping positioned boxes; positioned boxes paint
//!   above static content at the same level.
//! - `absolute` children of a flex container detach from the flex layout.

// Grid indexing is written uniformly as `row * cols + col` (including
// row 0 / row 1) so every assertion reads the same way.
#![allow(clippy::erasing_op, clippy::identity_op)]

use crossterm::style::Color;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::layout::layoutbox::pin_sticky_boxes;
use htui::layout::Rect;
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

/// Render and return the per-row text strings for inspection.
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

/// Character at `(x, y)` (0-based cell), or `None` when out of bounds.
fn cell(rows: &[String], x: usize, y: usize) -> Option<char> {
    rows.get(y).and_then(|r| r.chars().nth(x))
}

/// True when the cell is blank (space or out of bounds).
fn is_blank(rows: &[String], x: usize, y: usize) -> bool {
    matches!(cell(rows, x, y), None | Some(' '))
}

#[test]
fn absolute_positions_within_relative_parent() {
    init();
    let html = r#"
        <div class="wrap">
          <div class="abs">A</div>
        </div>
    "#;
    let css = r#"
        .wrap { position: relative; width: 10ch; }
        .abs { position: absolute; top: 1ch; left: 2ch; }
    "#;
    let (rows, _) = render_rows(html, css);
    assert_eq!(
        cell(&rows, 2, 1),
        Some('A'),
        "A must sit at (2, 1) inside the parent"
    );
    assert!(is_blank(&rows, 2, 0), "nothing above A");
}

#[test]
fn absolute_does_not_consume_flow_space() {
    init();
    let html = r#"
        <div class="wrap">
          <div class="abs">A</div>
        </div>
        <div id="sib">B</div>
    "#;
    let css = r#"
        .wrap { position: relative; }
        .abs { position: absolute; top: 3ch; left: 0ch; }
    "#;
    let (rows, _) = render_rows(html, css);
    // The absolute box claims no flow rows: the (min-height 1) wrapper is
    // followed by the sibling on the next row.
    assert_eq!(
        cell(&rows, 0, 1),
        Some('B'),
        "sibling must start right after the wrapper"
    );
    assert_eq!(
        cell(&rows, 0, 3),
        Some('A'),
        "absolute box must sit at row 3"
    );
}

#[test]
fn absolute_anchors_to_initial_containing_block() {
    init();
    let html = r#"
        <div>
          <div id="inner">I</div>
        </div>
        <div class="abs">A</div>
    "#;
    let css = r#"
        .abs { position: absolute; top: 0ch; left: 0ch; }
    "#;
    let (rows, _) = render_rows(html, css);
    // No positioned ancestor: the absolute box anchors to the initial
    // containing block (viewport origin), painting over the flow content
    // at the same level (positioned paints above static).
    assert_eq!(cell(&rows, 0, 0), Some('A'));
}

#[test]
fn absolute_uses_right_and_bottom_to_anchor() {
    init();
    let html = r#"
        <div class="wrap">
          <div>line1</div>
          <div>line2</div>
          <div class="abs">X</div>
        </div>
    "#;
    let css = r#"
        .wrap { position: relative; width: 10ch; }
        .abs { position: absolute; right: 1ch; bottom: 1ch; width: 3ch; }
    "#;
    let (rows, _) = render_rows(html, css);
    // Parent content: width 10, two lines (height 2). The absolute box is
    // 3 wide, 1 tall; right:1 → x = 10 - 3 - 1 = 6; bottom:1 → y = 2 - 1 - 1 = 0.
    assert_eq!(cell(&rows, 6, 0), Some('X'), "X must anchor bottom-right");
    assert!(is_blank(&rows, 6, 1), "nothing below X");
}

#[test]
fn fixed_anchors_to_viewport() {
    init();
    let html = r#"
        <div class="abs">F</div>
        <div>G</div>
    "#;
    let css = r#"
        .abs { position: fixed; top: 1ch; left: 5ch; }
    "#;
    let (rows, _) = render_rows(html, css);
    // Fixed anchors to the viewport, independent of flow: F at (5, 1) while
    // the static sibling occupies row 0.
    assert_eq!(
        cell(&rows, 5, 1),
        Some('F'),
        "F must anchor to the viewport"
    );
    assert_eq!(
        cell(&rows, 0, 0),
        Some('G'),
        "static content must be on row 0"
    );
}

#[test]
fn fixed_stays_pinned_to_the_screen_when_scrolled() {
    init();
    // `position: fixed` is scroll-aware: the layout re-anchors the fixed box
    // to the current scroll offset, so the viewport slice keeps it at its
    // constant screen position instead of scrolling away with the document.
    let html = r#"
        <div class="abs">F</div>
        <div>G</div>
    "#;
    let css = r#"
        .abs { position: fixed; top: 1ch; left: 5ch; }
    "#;
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    // At scroll 0 the fixed box sits at viewport row 1.
    assert_eq!(tl.layout[1 * cols + 5].word, "F");

    // Scroll down 10 rows: the re-layout moves the fixed box to document row
    // 11 (= scroll + top), so the slice still shows it at screen row 1.
    tl.scroll_y = 10;
    tl.draw(&arena, root_id, &term_map);
    assert_eq!(
        tl.layout[11 * cols + 5].word, "F",
        "fixed box must sit at document row scroll_y + top"
    );
}

#[test]
fn fixed_right_bottom_anchors_follow_the_scrolled_viewport() {
    init();
    // right / bottom insets keep anchoring against the viewport edge after a
    // scroll: at scroll_y = 10 the box's document x / y are computed from the
    // scrolled viewport's right / bottom.
    let html = r#"
        <div class="abs">F</div>
        <div>G</div>
    "#;
    let css = r#"
        .abs { position: fixed; right: 1ch; bottom: 1ch; width: 3ch; }
    "#;
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let rows = term_map.rows as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.scroll_y = 10;
    tl.draw(&arena, root_id, &term_map);
    // x = viewport_width - width - right = cols - 3 - 1;
    // y = scroll_y + viewport_rows - height - bottom = 10 + rows - 1 - 1.
    let x = cols - 4;
    let y = 10 + rows - 2;
    assert_eq!(
        tl.layout[y * cols + x].word, "F",
        "fixed right/bottom anchors must follow the scrolled viewport"
    );
}

#[test]
fn relative_offsets_without_moving_siblings() {
    init();
    let html = r#"
        <div class="rel">A</div>
        <div>B</div>
    "#;
    let css = r#"
        .rel { position: relative; top: 2ch; left: 1ch; }
    "#;
    let (rows, _) = render_rows(html, css);
    // A keeps its flow slot (row 0) and shifts down 2 / right 1.
    assert_eq!(cell(&rows, 1, 2), Some('A'), "A must shift to (1, 2)");
    // B stays at its flow position (row 1) — the relative offset moved A's
    // painted position only.
    assert_eq!(cell(&rows, 0, 1), Some('B'), "B must keep its flow row");
}

#[test]
fn z_index_paints_high_value_on_top() {
    init();
    let html = r#"
        <div class="wrap">
          <div class="a">A</div>
          <div class="b">B</div>
        </div>
    "#;
    let css = r#"
        .wrap { position: relative; }
        .a { position: absolute; top: 0ch; left: 0ch; }
        .b { position: absolute; top: 0ch; left: 0ch; z-index: 1; }
    "#;
    let (rows, _) = render_rows(html, css);
    assert_eq!(
        cell(&rows, 0, 0),
        Some('B'),
        "higher z-index must paint on top"
    );
}

#[test]
fn z_index_negative_paints_below_static_content() {
    init();
    let html = r#"
        <div class="wrap">
          <div class="n">N</div>
          <div id="s">S</div>
        </div>
    "#;
    let css = r#"
        .wrap { position: relative; }
        .n { position: absolute; top: 0ch; left: 0ch; z-index: -1; }
    "#;
    let (rows, _) = render_rows(html, css);
    // Negative z-index paints below static content.
    assert_eq!(cell(&rows, 0, 0), Some('S'));
}

#[test]
fn positioned_paints_above_static_sibling() {
    init();
    let html = r#"
        <div class="wrap">
          <div id="static">S</div>
          <div class="a">A</div>
        </div>
    "#;
    let css = r#"
        .wrap { position: relative; }
        .a { position: absolute; top: 0ch; left: 0ch; }
    "#;
    let (rows, _) = render_rows(html, css);
    // Positioned (z-index auto) paints above static content regardless of DOM
    // order.
    assert_eq!(cell(&rows, 0, 0), Some('A'));
}

#[test]
fn absolute_detaches_from_flex_container() {
    init();
    let html = r#"
        <div class="flex">
          <div class="abs">A</div>
          <div id="item">B</div>
        </div>
    "#;
    let css = r#"
        .flex { display: flex; }
        .abs { position: absolute; top: 0ch; left: 0ch; }
    "#;
    let (rows, _) = render_rows(html, css);
    // The absolute child detaches from the flex layout: B is the only flex
    // item and starts at the container origin; A is anchored to the initial
    // containing block and paints above it.
    assert_eq!(
        cell(&rows, 0, 0),
        Some('A'),
        "positioned A must paint on top"
    );
}

#[test]
fn relative_box_inside_relative_frame_renders_offset_text() {
    init();
    let html = r#"
        <div class="frame">
          <div class="row s">one</div>
          <div class="row rel">shifted</div>
          <div class="row s">three</div>
        </div>
    "#;
    let css = r#"
        .frame { position: relative; width: 20ch; }
        .row { margin: 1ch; }
        .s { background-color: #24283b; }
        .rel { position: relative; top: 2ch; left: 4ch; }
    "#;
    let (rows, _) = render_rows(html, css);
    // .row margin:1 → one(y=1, x=1); the adjacent 1-row margins collapse, so
    // shifted's flow slot is y=3 and its `top: 2` offset lands at (5, 5);
    // three follows at y=5.
    assert_eq!(cell(&rows, 1, 1), Some('o'), "first row at its flow slot");
    assert_eq!(
        cell(&rows, 5, 5),
        Some('s'),
        "shifted text must render at (5, 5)"
    );
    assert_eq!(
        cell(&rows, 1, 5),
        Some('t'),
        "third row at its flow slot y=5"
    );
}

#[test]
fn absolute_box_keeps_its_background() {
    init();
    let html = r#"
        <div class="wrap">
          <div class="abs">A</div>
        </div>
    "#;
    let css = r#"
        .wrap { position: relative; width: 10ch; }
        .abs { position: absolute; top: 0ch; left: 0ch; background-color: red; }
    "#;
    let (rows, tl) = render_rows(html, css);
    assert_eq!(cell(&rows, 0, 0), Some('A'));
    // The absolute box's background must fill its own rect.
    let cell_bg = tl.layout[0].bcolor;
    assert_eq!(
        cell_bg,
        Some(Color::Rgb { r: 255, g: 0, b: 0 }),
        "absolute box must paint its background"
    );
}

#[test]
fn sticky_keeps_its_flow_slot() {
    init();
    // `position: sticky` stays in normal flow: the sibling flows below the
    // sticky box, and at scroll 0 (top: 0) the box sits at its flow row.
    let html = r#"
        <div class="stick">S</div>
        <div class="body">B</div>
    "#;
    let css = r#"
        .stick { position: sticky; top: 0ch; height: 2ch; }
    "#;
    let (rows, _) = render_rows(html, css);
    assert_eq!(cell(&rows, 0, 0), Some('S'), "sticky box at its flow row");
    assert_eq!(
        cell(&rows, 0, 2),
        Some('B'),
        "sibling must flow below the sticky box"
    );
}

#[test]
fn sticky_clamps_to_viewport_top_when_scrolled() {
    init();
    // Scrolling past the sticky box clamps it to the viewport top (document
    // row scroll_y + top), so it stays visible at a constant screen row.
    let html = r#"
        <div class="stick">S</div>
        <div class="body">B</div>
    "#;
    let css = r#"
        .stick { position: sticky; top: 0ch; }
    "#;
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    assert_eq!(
        tl.layout[0 * cols].word, "S",
        "at scroll 0 the sticky box stays at its flow row"
    );

    tl.scroll_y = 10;
    tl.draw(&arena, root_id, &term_map);
    assert_eq!(
        tl.layout[10 * cols].word, "S",
        "sticky box must sit at document row scroll_y + top"
    );
}

#[test]
fn sticky_bottom_clamps_deep_box_to_viewport_bottom() {
    init();
    // A sticky box deep in the document (flow row 23) is clamped to the
    // viewport bottom edge by `bottom: 1` → row rows - 2.
    let html = r#"
        <div class="pad"></div>
        <div class="stick">S</div>
    "#;
    let css = r#"
        .pad { height: 23ch; }
        .stick { position: sticky; bottom: 1ch; }
    "#;
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let rows = term_map.rows as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    assert_eq!(
        tl.layout[(rows - 2) * cols].word, "S",
        "sticky bottom clamps the box to the viewport bottom"
    );
}

#[test]
fn sticky_repin_uses_recorded_flow_row() {
    init();
    // The render thread re-pins the *cached* boxes (already moved by the
    // previous scroll) on every scroll change. The clamp must be computed
    // from the recorded normal-flow row, not the moved rect — otherwise a
    // deep sticky box sticks at a stale offset instead of returning to its
    // document position once the viewport scrolls to it.
    let html = r#"
        <div style="height: 30ch;">pad</div>
        <div class="stick">S</div>
    "#;
    let css = r#"
        .stick { position: sticky; top: 0ch; }
    "#;
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let rows = term_map.rows as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);

    // First layout pins at scroll 0: the flow row (30) is recorded, and the
    // box is clamped to the viewport bottom (row rows - 1).
    let mut boxes = tl.layout_only(&arena, root_id, &term_map);
    let s_y = |b: &[htui::layout::LayoutBox]| -> usize {
        fn find(boxes: &[htui::layout::LayoutBox]) -> Option<usize> {
            for b in boxes {
                if matches!(b.positioning.position, htui::html::css_parse::Position::Sticky) {
                    return Some(b.rect.y);
                }
                if let Some(y) = find(&b.children) {
                    return Some(y);
                }
            }
            None
        }
        find(b).unwrap()
    };
    assert_eq!(s_y(&boxes), rows - 1, "clamped to the viewport bottom");

    // Scroll down 10 rows: S returns to its document row (30) — inside the
    // new viewport [10, 10 + rows), so no clamp applies.
    let viewport = Rect::new(0, 10, cols, rows);
    pin_sticky_boxes(&mut boxes, viewport);
    assert_eq!(s_y(&boxes), 30, "S returns to its flow row once visible");

    // Scroll down further (40): the flow row (30) is now above the viewport
    // top, so S clamps to the viewport top again.
    let viewport = Rect::new(0, 40, cols, rows);
    pin_sticky_boxes(&mut boxes, viewport);
    assert_eq!(s_y(&boxes), 40, "S clamps to the new viewport top");
}

#[test]
fn absolute_child_with_negative_left_anchors_to_parent_offset() {
    init();
    // A negative inset on an absolutely-positioned child anchors to its
    // containing block (the relative parent): the parent sits at x=8 via
    // `margin-left`, so `left: -2ch` pulls the child to x=6 instead of
    // clamming the whole subtree to the screen edge at x=0.
    let html = r#"
        <div class="parent">
            <div class="child">X</div>
        </div>
    "#;
    let css = r#"
        .parent { position: relative; margin-left: 8ch; width: 10ch; }
        .child { position: absolute; left: -2ch; top: 1ch; }
    "#;
    let (rows, _) = render_rows(html, css);
    assert_eq!(
        cell(&rows, 6, 1),
        Some('X'),
        "absolute child must sit at parent_x + (-2) = 6"
    );
}

