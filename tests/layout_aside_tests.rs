use crossterm::style::Color;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::layout::layout_fn::aside::text_display_width;
use htui::tdom::node::{NodeArena, NodeId};
use htui::tui::graph_api::TermLayout;

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
fn test_aside_basic() {
    init();
    let (root_id, arena) = setup("<aside>Side note</aside>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("Side note"), "aside should contain text");
}

#[test]
fn test_aside_with_color() {
    init();
    let html = "<aside>Colored aside</aside>";
    let css = "aside { color: blue; background-color: yellow; }";
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
fn test_aside_with_nested_div() {
    init();
    let html = "<aside><div>Inner div</div></aside>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(
        canvas.contains("Inner div"),
        "aside should render nested div"
    );
}

#[test]
fn test_aside_width_50pct() {
    init();
    let html = "<aside>Half</aside>";
    let css = "aside { width: 50%; background-color: blue; color: white; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let text_cells: Vec<_> = tl.layout.iter().filter(|b| b.bcolor.is_some()).collect();
    assert_eq!(
        text_cells.len(),
        4,
        "bg should only cover text 'Half' (4 chars)"
    );
    for cell in &text_cells {
        assert_eq!(cell.bcolor, Some(Color::Blue));
        assert_eq!(cell.fcolor, Some(Color::White));
        assert_ne!(cell.word, " ");
    }

    let first_bg = tl.layout.iter().position(|b| b.bcolor.is_some()).unwrap();
    let col_in_row = first_bg % cols;
    let expected_left = (cols - cols / 2) / 2;
    assert!(
        col_in_row >= expected_left,
        "text should be within block, not in margin"
    );
}

#[test]
fn test_aside_width_fixed_px() {
    init();
    let html = "<aside>Fix</aside>";
    let css = "aside { width: 20px; background-color: red; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();

    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let bg_cells: Vec<_> = tl.layout.iter().filter(|b| b.bcolor.is_some()).collect();
    assert_eq!(
        bg_cells.len(),
        3,
        "bg should only cover text 'Fix' (3 chars)"
    );
    for cell in &bg_cells {
        assert_eq!(cell.bcolor, Some(Color::Red));
    }
}

#[test]
fn test_aside_width_default_full() {
    init();
    let html = "<aside>Full width</aside>";
    let css = "aside { background-color: green; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();

    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let bg_cells: Vec<_> = tl.layout.iter().filter(|b| b.bcolor.is_some()).collect();
    assert_eq!(
        bg_cells.len(),
        "Full width".len(),
        "bg should only cover text, not fill row"
    );
    for cell in &bg_cells {
        assert_eq!(cell.bcolor, Some(Color::Green));
    }
    let text_with_bg = bg_cells.iter().filter(|c| c.word != " ").count();
    assert!(text_with_bg > 0, "text cells should have bg");
}

#[test]
fn test_aside_text_align_center() {
    init();
    let html = "<aside>Centered</aside>";
    let css = "aside { text-align: center; width: 40px; background-color: darkblue; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word != " ")
        .expect("should find text");
    let row = text_idx / cols;
    let rstart = row * cols;

    let left_margin = (cols - 40) / 2;
    let text_start = rstart + left_margin;
    let block = &tl.layout[text_start..text_start + 40];

    let first_non_space = block
        .iter()
        .position(|b| b.word != " ")
        .expect("should find text");
    let text_width = text_display_width("Centered");
    let expected_padding = (40 - text_width) / 2;
    assert_eq!(
        first_non_space, expected_padding,
        "text should be centered in block"
    );

    let bg_cells: Vec<_> = tl.layout.iter().filter(|b| b.bcolor.is_some()).collect();
    assert_eq!(bg_cells.len(), text_width, "bg only on text cells");
}

#[test]
fn test_aside_text_align_right() {
    init();
    let html = "<aside>Right</aside>";
    let css = "aside { text-align: right; width: 40px; background-color: darkblue; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word != " ")
        .expect("should find text");
    let row = text_idx / cols;
    let rstart = row * cols;

    let left_margin = (cols - 40) / 2;
    let text_start = rstart + left_margin;
    let block = &tl.layout[text_start..text_start + 40];

    let first_non_space = block
        .iter()
        .position(|b| b.word != " ")
        .expect("should find text");
    let text_width = text_display_width("Right");
    let expected_padding = 40 - text_width;
    assert_eq!(
        first_non_space, expected_padding,
        "text should be right-aligned"
    );

    let bg_cells: Vec<_> = tl.layout.iter().filter(|b| b.bcolor.is_some()).collect();
    assert_eq!(bg_cells.len(), text_width, "bg only on text cells");
}
