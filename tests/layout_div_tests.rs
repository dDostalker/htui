use crossterm::style::Color;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
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
fn test_div_basic() {
    init();
    let (root_id, arena) = setup("<div>hello</div>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("hello"), "div should render text");
}

#[test]
fn test_div_with_color() {
    init();
    let html = "<div>colored</div>";
    let css = "div { color: blue; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();
    let has_fg = tl.layout.iter().any(|b| b.fcolor == Some(Color::Blue));
    assert!(has_fg, "div color should be applied to text cells");
}

#[test]
fn test_div_background_color() {
    init();
    let html = "<div>bg</div>";
    let css = "div { background-color: yellow; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();
    let has_bg = tl.layout.iter().any(|b| b.bcolor == Some(Color::Yellow));
    assert!(has_bg, "div background-color should be applied");
}

#[test]
fn test_div_bg_fills_full_row_default() {
    init();
    let html = "<div>short</div>";
    let css = "div { background-color: red; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let content_idx = tl
        .layout
        .iter()
        .position(|b| b.word != " " && b.bcolor == Some(Color::Red))
        .expect("should find colored content");
    let row = content_idx / cols;
    let rstart = row * cols;
    let rend = rstart + cols;
    let bg_count = tl.layout[rstart..rend]
        .iter()
        .filter(|b| b.bcolor == Some(Color::Red))
        .count();
    assert_eq!(bg_count, cols, "default width should fill entire row");
}

#[test]
fn test_div_width_50pct_centered() {
    init();
    let html = "<div>half</div>";
    let css = "div { width: 50%; background-color: blue; color: white; }";
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
            assert_eq!(row[i].fcolor, Some(Color::White), "text should be white");
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
fn test_div_width_fixed() {
    init();
    let html = "<div>Fix</div>";
    let css = "div { width: 20px; background-color: red; }";
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
    let bg_count = (rstart..rend)
        .filter(|&i| tl.layout[i].bcolor.is_some())
        .count();
    assert_eq!(bg_count, 20, "background should be exactly 20 columns");
}

#[test]
fn test_div_text_align_center() {
    init();
    let html = "<div>Centered</div>";
    let css = "div { text-align: center; width: 40px; }";
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

    let leading_spaces = block.iter().take_while(|b| b.word == " ").count();
    let text_str_width = "Centered".len();
    let expected_padding = (40 - text_str_width) / 2;
    assert_eq!(
        leading_spaces, expected_padding,
        "text should be centered within block"
    );
}

#[test]
fn test_div_text_align_right() {
    init();
    let html = "<div>Right</div>";
    let css = "div { text-align: right; width: 40px; }";
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

    let leading_spaces = block.iter().take_while(|b| b.word == " ").count();
    let text_str_width = "Right".len();
    let expected_padding = 40 - text_str_width;
    assert_eq!(
        leading_spaces, expected_padding,
        "text should be right-aligned within block"
    );
}

#[test]
fn test_div_nested_preserves_text() {
    init();
    let html = "<div><p>A</p><div><p>B</p></div></div>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("A"), "outer div should contain A");
    assert!(canvas.contains("B"), "inner div should contain B");
}

#[test]
fn test_div_no_bg_does_not_fill_row() {
    // Without background-color, div must not paint trailing spaces into the row,
    // otherwise nested scenarios lose outer block's bg.
    init();
    let html = "<div>hi</div>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let text_idx = tl.layout.iter().position(|b| b.word != " ").unwrap();
    let row = text_idx / cols;
    let rstart = row * cols;
    let rend = rstart + cols;
    let has_bg = tl.layout[rstart..rend].iter().any(|b| b.bcolor.is_some());
    assert!(
        !has_bg,
        "row should not have any bg cells when div has no background-color"
    );
}

#[test]
fn test_div_color_inherited_by_p() {
    // CSS `color` cascades from div to `<p>` descendants. `<p>` reads its own
    // css_attr (populated by `inherit_css`) when rendering, so a div-level
    // color should reach the paragraph without an explicit rule on `<p>`.
    init();
    let html = "<div><p>colored</p></div>";
    let css = "div { color: magenta; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();
    let has_color = tl.layout.iter().any(|b| b.fcolor == Some(Color::Magenta));
    assert!(has_color, "div color should inherit to <p> descendant");
}
