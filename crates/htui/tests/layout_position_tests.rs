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
        .wrap { position: relative; width: 10; }
        .abs { position: absolute; top: 1; left: 2; }
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
        .abs { position: absolute; top: 3; left: 0; }
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
        .abs { position: absolute; top: 0; left: 0; }
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
        .wrap { position: relative; width: 10; }
        .abs { position: absolute; right: 1; bottom: 1; width: 3; }
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
        .abs { position: fixed; top: 1; left: 5; }
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
fn relative_offsets_without_moving_siblings() {
    init();
    let html = r#"
        <div class="rel">A</div>
        <div>B</div>
    "#;
    let css = r#"
        .rel { position: relative; top: 2; left: 1; }
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
        .a { position: absolute; top: 0; left: 0; }
        .b { position: absolute; top: 0; left: 0; z-index: 1; }
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
        .n { position: absolute; top: 0; left: 0; z-index: -1; }
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
        .a { position: absolute; top: 0; left: 0; }
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
        .abs { position: absolute; top: 0; left: 0; }
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
        .frame { position: relative; width: 20; }
        .row { margin: 1; }
        .s { background-color: #24283b; }
        .rel { position: relative; top: 2; left: 4; }
    "#;
    let (rows, _) = render_rows(html, css);
    // .row margin:1 → one(y=1, x=1), shifted flow y=4 offset → (5, 6), three(y=7).
    assert_eq!(cell(&rows, 1, 1), Some('o'), "first row at its flow slot");
    assert_eq!(
        cell(&rows, 5, 6),
        Some('s'),
        "shifted text must render at (5, 6)"
    );
    assert_eq!(
        cell(&rows, 1, 7),
        Some('t'),
        "third row at its flow slot y=7"
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
        .wrap { position: relative; width: 10; }
        .abs { position: absolute; top: 0; left: 0; background-color: red; }
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
