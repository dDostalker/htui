use htui::html::html_parse::Parser;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::tdom::node::{NodeArena, NodeId};
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

#[test]
fn test_layout_p() {
    let mut p = Parser::new("<p>Hello</p><p>World</p>");
    let (root, arena, _) = p.parse();
    let term_map = TermMap {
        columns: 80,
        rows: 24,
    };
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root, &term_map).unwrap();
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("Hello"));
    assert!(canvas.contains("World"));
}

// ============================================================
// text-align — exercised through `<p>` and `<h1>`-`<h5>` so the
// inheritance chain `div { text-align: ... } → <p>` is covered.
// ============================================================

#[test]
fn test_layout_p_text_align_center() {
    init();
    let html = "<p>Centered</p>";
    let css = "p { text-align: center; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word != " ")
        .expect("should find text");
    let row = text_idx / cols;
    let rstart = row * cols;
    let leading_spaces = tl.layout[rstart..rstart + cols]
        .iter()
        .take_while(|b| b.word == " ")
        .count();
    let text_str_width = "Centered".len();
    let expected_padding = (cols - text_str_width) / 2;
    assert_eq!(
        leading_spaces, expected_padding,
        "<p> text should be centered within row"
    );
}

#[test]
fn test_layout_p_text_align_right() {
    init();
    let html = "<p>Right</p>";
    let css = "p { text-align: right; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word != " ")
        .expect("should find text");
    let row = text_idx / cols;
    let rstart = row * cols;
    let leading_spaces = tl.layout[rstart..rstart + cols]
        .iter()
        .take_while(|b| b.word == " ")
        .count();
    let text_str_width = "Right".len();
    let expected_padding = cols - text_str_width;
    assert_eq!(
        leading_spaces, expected_padding,
        "<p> text should be right-aligned within row"
    );
}

#[test]
fn test_layout_p_text_align_inherited_from_div() {
    init();
    let html = "<div><p>centered</p></div>";
    let css = "div { text-align: center; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    // "centered" must not start at column 0 — it should be indented by the
    // center-padding because <p> inherited text-align from <div>.
    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "c")
        .expect("should find 'centered'");
    let row = text_idx / cols;
    let col_in_row = text_idx % cols;
    let text_str_width = "centered".len();
    let expected_padding = (cols - text_str_width) / 2;
    assert_eq!(
        col_in_row, expected_padding,
        "<p> should inherit text-align from <div>, row={row}"
    );
}

#[test]
fn test_layout_h1_text_align_center() {
    init();
    let html = "<h1>Title</h1>";
    let css = "h1 { text-align: center; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    // Rendered text is "# Title" (prefix included in alignment width).
    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "#")
        .expect("should find heading '#' prefix");
    let col_in_row = text_idx % cols;
    let rendered_width = "# Title".chars().count() * 2;
    let expected_padding = (cols - rendered_width) / 2;
    assert_eq!(
        col_in_row, expected_padding,
        "<h1> should be centered including '# ' prefix"
    );
}

#[test]
fn test_layout_h2_text_align_right() {
    init();
    let html = "<h2>Sub</h2>";
    let css = "h2 { text-align: right; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "#")
        .expect("should find heading '#' prefix");
    let col_in_row = text_idx % cols;
    let rendered_width = "## Sub".chars().count() * 2;
    let expected_padding = cols - rendered_width;
    assert_eq!(
        col_in_row, expected_padding,
        "<h2> should be right-aligned including '## ' prefix"
    );
}

#[test]
fn test_layout_heading_text_align_inherited_from_div() {
    init();
    let html = "<div><h1>Heading</h1></div>";
    let css = "div { text-align: center; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "#")
        .expect("should find heading prefix");
    let col_in_row = text_idx % cols;
    let rendered_width = "# Heading".chars().count() * 2;
    let expected_padding = (cols - rendered_width) / 2;
    assert_eq!(
        col_in_row, expected_padding,
        "<h1> should inherit text-align from <div>"
    );
}

#[test]
fn test_layout_p_text_align_default_left() {
    init();
    let html = "<p>Left</p>";
    let css = "";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word != " ")
        .expect("should find text");
    let row = text_idx / cols;
    let col_in_row = text_idx % cols;
    assert_eq!(
        col_in_row, 0,
        "default text-align should leave text at column 0, row={row}"
    );
}
