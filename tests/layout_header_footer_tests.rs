use crossterm::style::Color;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::layout::layout_fn::header_footer::text_display_width;
use htui::tdom::node::{Node, NodeArena, NodeId, NodeType};
use htui::tui::graph_api::{TermLayout, char_width};

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

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

const COLS: u16 = 80;
const ROWS: u16 = 24;

fn mock_term_map() -> TermMap {
    TermMap {
        columns: COLS,
        rows: ROWS,
    }
}

#[test]
fn test_header_basic() {
    init();
    let (root_id, arena) = setup("<header>My Site</header>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("My Site"), "header should contain text");
}

#[test]
fn test_footer_basic() {
    init();
    let (root_id, arena) = setup("<footer>(c) 2024</footer>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("(c) 2024"), "footer should contain text");
}

#[test]
fn test_header_with_color() {
    init();
    let html = "<header>Colored</header>";
    let css = "header { color: blue; background-color: yellow; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();
    let has_fg = tl.layout.iter().any(|b| b.fcolor == Some(Color::Blue));
    let has_bg = tl.layout.iter().any(|b| b.bcolor == Some(Color::Yellow));
    assert!(has_fg, "should have blue foreground");
    assert!(has_bg, "should have yellow background");
}

#[test]
fn test_header_solid_bg() {
    init();
    let html = "<header>Solid</header>";
    let css = "header { background-color: green; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let content_idx = tl
        .layout
        .iter()
        .position(|b| b.word != " " && b.bcolor == Some(Color::Green))
        .expect("should find green text");
    let content_row = content_idx / cols;
    let row_start = content_row * cols;
    let row_end = row_start + cols;
    let bg_count = tl.layout[row_start..row_end]
        .iter()
        .filter(|b| b.bcolor == Some(Color::Green))
        .count();
    assert_eq!(bg_count, cols, "default 100% width should fill row");
}

#[test]
fn test_header_width_50pct() {
    init();
    let html = "<header>Half</header>";
    let css = "header { width: 50%; background-color: blue; color: white; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let content_idx = tl
        .layout
        .iter()
        .position(|b| b.bcolor.is_some() && b.word != " ")
        .expect("should find colored content");
    let content_row = content_idx / cols;
    let row = &tl.layout[content_row * cols..(content_row + 1) * cols];

    let left_margin = (cols - cols / 2) / 2;
    for i in 0..left_margin {
        assert!(
            row[i].bcolor.is_none(),
            "left margin should have no bg col={i}"
        );
    }
    let block_end = left_margin + cols / 2;
    let mut has_text = false;
    for i in left_margin..block_end {
        assert!(row[i].bcolor.is_some(), "block should have bg col={i}");
        if row[i].word != " " {
            has_text = true;
        }
    }
    assert!(has_text, "block should contain text");
    for i in block_end..cols {
        assert!(
            row[i].bcolor.is_none(),
            "right margin should have no bg col={i}"
        );
    }
}

#[test]
fn test_header_width_fixed() {
    init();
    let html = "<header>Fix</header>";
    let css = "header { width: 20px; background-color: red; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let content_idx = tl
        .layout
        .iter()
        .position(|b| b.bcolor.is_some() && b.word != " ")
        .expect("should find content");
    let row = content_idx / cols;
    let rstart = row * cols;
    let rend = rstart + cols;

    let bg_cols: Vec<usize> = (rstart..rend)
        .filter(|&i| tl.layout[i].bcolor.is_some())
        .collect();
    assert_eq!(bg_cols.len(), 20, "background should be exactly 20 columns");
}

#[test]
fn test_header_layout_structure() {
    init();
    let html = "<header>Site Title</header>";
    let css = "header { background-color: darkblue; text-align: center; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(!canvas.contains('\u{2550}'));
    assert!(!canvas.contains('\u{2500}'));

    let content_idx = tl
        .layout
        .iter()
        .position(|b| b.bcolor.is_some() && b.word != " ")
        .expect("should find content");
    let row = content_idx / cols;
    let rstart = row * cols;
    let rend = rstart + cols;
    for i in rstart..rend {
        assert!(
            tl.layout[i].bcolor.is_some(),
            "default width should fill entire row"
        );
    }
    let next = (row + 1) * cols;
    if next < tl.layout.len() {
        assert!(
            tl.layout[next].bcolor.is_none(),
            "next row should have no bg"
        );
    }
}

#[test]
fn test_footer_layout_structure() {
    init();
    let html = "<p>Above</p><footer>Copyright</footer>";
    let css = "footer { background-color: black; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(!canvas.contains('\u{2500}'));
    assert!(!canvas.contains('\u{2550}'));

    let content_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "C")
        .expect("should find footer");
    let row = content_idx / cols;
    let rstart = row * cols;
    let rend = rstart + cols;
    for i in rstart..rend {
        assert!(tl.layout[i].bcolor.is_some(), "footer row should have bg");
    }
}

#[test]
fn test_header_center_align() {
    init();
    let html = "<header>Centered</header>";
    let css = "header { text-align: center; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("Centered"));
}

#[test]
fn test_header_right_align() {
    init();
    let html = "<header>Right</header>";
    let css = "header { text-align: right; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("Right"));
}

#[test]
fn test_css_attr_text_align() {
    let (_, arena) = setup("<div></div>", "div { text-align: center; }");
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Div {
            assert_eq!(node.css_attr.get_text_align(), css_parse::TextAlign::Center);
            return;
        }
    }
    panic!("div not found");
}

#[test]
fn test_css_attr_bg_color() {
    let (_, arena) = setup("<div></div>", "div { background-color: red; }");
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Div {
            assert_eq!(node.css_attr.get_bg_color(), Some(Color::Red));
            return;
        }
    }
    panic!("div not found");
}

#[test]
fn test_css_attr_bg_color_fallback() {
    let (_, arena) = setup("<div></div>", "div { background: green; }");
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Div {
            assert_eq!(node.css_attr.get_bg_color(), Some(Color::Green));
            return;
        }
    }
    panic!("div not found");
}

#[test]
fn test_css_attr_font_weight() {
    let (_, arena) = setup("<div></div>", "div { font-weight: bold; }");
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Div {
            assert_eq!(node.css_attr.get_font_weight(), css_parse::FontWeight::Bold);
            return;
        }
    }
    panic!("div not found");
}

#[test]
fn test_text_display_width() {
    assert_eq!(text_display_width("abc"), 3);
    assert_eq!(text_display_width("\u{4f60}\u{597d}"), 4);
    assert_eq!(char_width('\u{2550}'), 1);
    assert_eq!(char_width('\u{2500}'), 1);
    assert_eq!(char_width('\u{4f60}'), 2);
}
