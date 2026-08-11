//! `<menu>` / `<dl>` / `<dt>` / `<dd>` / `<search>` tests.
//!
//! - `<menu>` renders like `<ul>`: each `<li>` child gets the `• ` marker
//!   and nesting indents per level.
//! - `<dl>` keeps the browser UA `margin-block: 1em` default (one blank row
//!   above/below); `<dt>` is a plain block; `<dd>` keeps the UA
//!   `margin-inline-start: 40px` default (4 columns).
//! - `<search>` is a plain block container.

use crossterm::style::Color;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::tdom::node::{NodeArena, NodeId, NodeType};
use htui::tui::graph_api::TermLayout;

/// Recursively walk the layout tree (depth-first) and return the first box
/// whose node type matches `target`.
fn find_box_by_type<'a>(
    boxes: &'a [htui::layout::LayoutBox],
    target: &NodeType,
    arena: &NodeArena,
) -> Option<&'a htui::layout::LayoutBox> {
    for b in boxes {
        if &arena[b.node_id].node_type == target {
            return Some(b);
        }
        if let Some(found) = find_box_by_type(&b.children, target, arena) {
            return Some(found);
        }
    }
    None
}

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

/// Layout the document and return the box tree plus the arena needed to
/// map boxes back to node types.
fn layout_boxes(html: &str, css: &str) -> (Vec<htui::layout::LayoutBox>, NodeArena) {
    use htui::layout::{BoxConstraint, Cursor, inner_formatting_context::flow::layout_tree};
    let (root_id, arena) = setup(html, css);
    let containing = BoxConstraint::root(&mock_term_map());
    let mut cursor = Cursor::new();
    let boxes =
        layout_tree(&arena, root_id, containing, &mut cursor, COLS as usize, ROWS as usize);
    (boxes, arena)
}

/// Render the document onto the terminal canvas.
fn draw(html: &str, css: &str) -> (TermMap, TermLayout) {
    let term_map = mock_term_map();
    let (root_id, arena) = setup(html, css);
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    (term_map, tl)
}

#[test]
fn test_parse_tag_map_menu_dl_dt_dd_search() {
    init();
    assert_eq!(html_parse::tag_name_to_node_type("menu"), Some(NodeType::Menu));
    assert_eq!(html_parse::tag_name_to_node_type("dl"), Some(NodeType::Dl));
    assert_eq!(html_parse::tag_name_to_node_type("dt"), Some(NodeType::Dt));
    assert_eq!(html_parse::tag_name_to_node_type("dd"), Some(NodeType::Dd));
    assert_eq!(
        html_parse::tag_name_to_node_type("search"),
        Some(NodeType::Search)
    );
}

#[test]
fn test_menu_li_renders_bullet_marker() {
    init();
    // `<menu>` behaves like `<ul>`: the `<li>` carries the `• ` marker, so
    // the item text starts two columns in.
    let (term_map, tl) = draw("<menu><li>Item</li></menu>", "");
    let cols = term_map.columns as usize;
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "I")
        .expect("menu item text should render");
    assert_eq!(idx / cols, 0);
    assert_eq!(idx % cols, 2, "text starts after the `• ` marker");
}

#[test]
fn test_nested_menu_indents_per_level() {
    init();
    // A `<menu>` inside `<menu>` adds the list indent (2 cols per level),
    // like nested `<ul>`/`<ol>`.
    let (term_map, tl) = draw("<menu><menu><li>Deep</li></menu></menu>", "");
    let cols = term_map.columns as usize;
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "D")
        .expect("nested menu item text should render");
    assert_eq!(idx / cols, 0);
    assert_eq!(idx % cols, 4, "2-col menu indent + 2-col marker");
}

#[test]
fn test_menu_produces_own_box() {
    init();
    let (boxes, arena) = layout_boxes("<menu><li>Item</li></menu>", "");
    let menu = find_box_by_type(&boxes, &NodeType::Menu, &arena)
        .expect("menu should produce its own box");
    assert_eq!(menu.rect.x, 0);
    assert_eq!(menu.rect.y, 0);
    assert_eq!(menu.rect.width, 80);
}

#[test]
fn test_dl_has_vertical_margins() {
    init();
    // Browser UA `margin-block: 1em` → one blank row above/below.
    let (term_map, tl) = draw("<dl>Text</dl>", "");
    let cols = term_map.columns as usize;
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "T")
        .expect("dl text should render");
    assert_eq!(idx / cols, 1, "one blank row above the dl");
    assert_eq!(idx % cols, 0, "no horizontal indent");
}

#[test]
fn test_dl_author_margin_zero_removes_ua_defaults() {
    init();
    let (_, tl) = draw("<dl>Text</dl>", "dl { margin: 0; }");
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "T")
        .expect("dl text should render");
    assert_eq!(idx, 0, "author margin: 0 removes the UA margins entirely");
}

#[test]
fn test_dt_dd_layout_inside_dl() {
    init();
    // `dt` sits flush at the dl content edge; `dd` carries the UA
    // `margin-inline-start: 40px` (4 columns) so it reads as the
    // description under its term.
    let (term_map, tl) = draw("<dl><dt>Term</dt><dd>Desc</dd></dl>", "");
    let cols = term_map.columns as usize;
    let t_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "T")
        .expect("dt text should render");
    let d_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "D")
        .expect("dd text should render");
    assert_eq!(t_idx / cols, 1, "dt starts after the dl top margin");
    assert_eq!(t_idx % cols, 0, "dt is flush left");
    assert_eq!(d_idx / cols, 2, "dd follows one row below the dt");
    assert_eq!(d_idx % cols, 4, "dd carries the UA left margin");

    let (boxes, arena) = layout_boxes("<dl><dt>Term</dt><dd>Desc</dd></dl>", "");
    let dl = find_box_by_type(&boxes, &NodeType::Dl, &arena).expect("dl box");
    let dt = find_box_by_type(&boxes, &NodeType::Dt, &arena).expect("dt box");
    let dd = find_box_by_type(&boxes, &NodeType::Dd, &arena).expect("dd box");
    assert_eq!(dl.rect.x, 0);
    assert_eq!(dl.rect.y, 1);
    assert_eq!(dl.rect.height, 2, "dl wraps both children");
    assert_eq!(dt.rect.x, 0);
    assert_eq!(dt.rect.y, 1);
    assert_eq!(dd.rect.x, 4, "dd box starts at the UA left margin");
    assert_eq!(dd.rect.width, 76, "80 minus the 4-col left margin");
}

#[test]
fn test_dd_author_margin_left_override() {
    init();
    // `dd { margin-left: 0 }` drops the UA indent only on that side.
    let (term_map, tl) = draw("<dl><dd>X</dd></dl>", "dd { margin-left: 0; }");
    let cols = term_map.columns as usize;
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "X")
        .expect("dd text should render");
    assert_eq!(idx / cols, 1);
    assert_eq!(idx % cols, 0, "author margin-left: 0 removes the dd indent");
}

#[test]
fn test_dt_type_selector_applies_color() {
    init();
    let (_, tl) = draw("<dl><dt>Term</dt></dl>", "dt { color: blue; }");
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "T")
        .expect("dt text should render");
    assert_eq!(
        tl.layout[idx].fcolor,
        Some(Color::Rgb { r: 0, g: 0, b: 255 }),
        "type selector must style dt text"
    );
}

#[test]
fn test_search_renders_as_plain_block() {
    init();
    let (boxes, arena) = layout_boxes("<search>Results</search>", "");
    let search = find_box_by_type(&boxes, &NodeType::Search, &arena)
        .expect("search should produce its own box");
    assert_eq!(search.rect.x, 0);
    assert_eq!(search.rect.y, 0);
    assert_eq!(search.rect.width, 80);
    assert_eq!(search.rect.height, 1);
}
