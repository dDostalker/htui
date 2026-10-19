//! `<data>` / `<time>` tests — inline elements with no UA visual default.
//!
//! Both are plain inline containers like `<span>`: the text content renders
//! as-is and the `value` / `datetime` attributes are machine-readable only
//! (addressed via CSS / DOM, never painted), matching browser behaviour.

use crossterm::style::Color;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::tdom::node::{NodeArena, NodeId, NodeType};
use htui::tui::graph_api::{FontStyle, TermLayout};

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

/// Render the document onto the terminal canvas.
fn draw(html: &str, css: &str) -> (TermMap, TermLayout) {
    let term_map = mock_term_map();
    let (root_id, arena) = setup(html, css);
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    (term_map, tl)
}

/// Index of the first canvas cell carrying `word`.
fn first_cell_with(tl: &TermLayout, word: &str) -> Option<usize> {
    tl.layout.iter().position(|b| b.word == word)
}

#[test]
fn test_parse_tag_map_data_time() {
    init();
    assert_eq!(
        html_parse::tag_name_to_node_type("data"),
        Some(NodeType::Data)
    );
    assert_eq!(
        html_parse::tag_name_to_node_type("time"),
        Some(NodeType::Time)
    );
}

#[test]
fn test_data_renders_content_inline() {
    init();
    // `<data value="...">` renders its text content as plain inline text;
    // the machine-readable `value` has no visual effect.
    let (term_map, tl) = draw("<p>Qty: <data value=\"42\">forty-two</data>.</p>", "");
    let cols = term_map.columns as usize;
    let idx = first_cell_with(&tl, "f").expect("data content should render");
    let period = first_cell_with(&tl, ".").expect("trailing text should render");
    assert_eq!(
        idx / cols,
        1,
        "content sits on the paragraph's text row (UA margin-top=1)"
    );
    assert_eq!(period, idx + 9, "sentence continues after the data element");
}

#[test]
fn test_time_renders_content_inline() {
    init();
    // `<time datetime="...">` renders its text content as plain inline text.
    let (term_map, tl) = draw("<p>On <time datetime=\"2024-01-01\">Jan 1</time>.</p>", "");
    let cols = term_map.columns as usize;
    let idx = first_cell_with(&tl, "J").expect("time content should render");
    let period = first_cell_with(&tl, ".").expect("trailing text should render");
    assert_eq!(
        idx / cols,
        1,
        "content sits on the paragraph's text row (UA margin-top=1)"
    );
    assert_eq!(period, idx + 5, "sentence continues after the time element");
}

#[test]
fn test_data_no_default_decoration() {
    init();
    let (_, tl) = draw("<p><data value=\"1\">One</data></p>", "");
    let idx = first_cell_with(&tl, "O").expect("data content should render");
    assert!(
        tl.layout[idx].style.is_empty(),
        "data must carry no UA style bits (got {:?})",
        tl.layout[idx].style
    );
}

#[test]
fn test_time_type_selector_styles_content() {
    init();
    let (_, tl) = draw(
        "<p><time datetime=\"2024-01-01\">Jan</time></p>",
        "time { font-weight: bold; }",
    );
    let idx = first_cell_with(&tl, "J").expect("time content should render");
    assert!(
        tl.layout[idx].style.contains(FontStyle::BOLD),
        "type selector must style time text"
    );
}

#[test]
fn test_data_type_selector_colors_content() {
    init();
    let (_, tl) = draw(
        "<p><data value=\"1\">One</data></p>",
        "data { color: blue; }",
    );
    let idx = first_cell_with(&tl, "O").expect("data content should render");
    assert_eq!(
        tl.layout[idx].fcolor,
        Some(Color::Rgb { r: 0, g: 0, b: 255 }),
        "type selector must style data text"
    );
}
