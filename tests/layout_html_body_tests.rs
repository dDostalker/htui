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

fn canvas_string(tl: &TermLayout) -> String {
    tl.layout.iter().map(|b| b.word.as_str()).collect()
}

#[test]
fn test_html_body_renders_body_content() {
    init();
    let html = "<html><body><p>hello</p></body></html>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas = canvas_string(&tl);
    assert!(
        canvas.contains("hello"),
        "<p> inside <body> should be rendered"
    );
}

#[test]
fn test_html_body_with_head_title_not_rendered() {
    // <title> lives inside <head>. <title> itself is a sink in the layout
    // registry, so its Text child is never visited by the dispatcher and the
    // title text must not appear in the output.
    init();
    let html = "<html><head><title>SecretTitle</title></head><body><p>visible</p></body></html>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas = canvas_string(&tl);
    assert!(canvas.contains("visible"), "body content must be rendered");
    assert!(
        !canvas.contains("SecretTitle"),
        "<title> text inside <head> must not appear in the output"
    );
}

#[test]
fn test_title_outside_head_not_rendered() {
    // Even when <title> appears outside <head> (which is invalid HTML but
    // still occurs in user input), its text must never reach the canvas.
    init();
    let html = "<html><body><title>Leak</title><p>ok</p></body></html>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas = canvas_string(&tl);
    assert!(canvas.contains("ok"), "body content must be rendered");
    assert!(
        !canvas.contains("Leak"),
        "<title> text outside <head> must not be rendered either"
    );
}

#[test]
fn test_meta_in_head_does_not_render() {
    // <meta> is a void metadata element. It must never contribute content
    // to the canvas, regardless of the attributes it carries.
    init();
    let html = "<html><head><meta charset=\"utf-8\"><meta name=\"description\" content=\"d\"></head><body><p>shown</p></body></html>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas = canvas_string(&tl);
    assert!(canvas.contains("shown"), "body content must be rendered");
    assert!(
        !canvas.contains("utf-8"),
        "<meta charset> attribute must not appear in the output"
    );
    assert!(
        !canvas.contains("description"),
        "<meta name> attribute must not appear in the output"
    );
}

#[test]
fn test_body_css_color_applies_to_descendants() {
    init();
    let html = "<html><body><p>colored</p></body></html>";
    let css = "body { color: magenta; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let has_color = tl.layout.iter().any(|b| b.fcolor == Some(Color::Magenta));
    assert!(
        has_color,
        "body-level color should cascade to <p> descendant"
    );
}

#[test]
fn test_body_css_background_color_fills_terminal() {
    init();
    let html = "<html><body><p>bg</p></body></html>";
    let css = "body { background-color: yellow; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let content_idx = tl
        .layout
        .iter()
        .position(|b| b.word != " " && b.bcolor == Some(Color::Yellow))
        .expect("should find colored content");
    let row = content_idx / cols;
    let rstart = row * cols;
    let rend = rstart + cols;
    let bg_count = tl.layout[rstart..rend]
        .iter()
        .filter(|b| b.bcolor == Some(Color::Yellow))
        .count();
    assert_eq!(
        bg_count, cols,
        "body background-color should fill the entire row by default"
    );
}

#[test]
fn test_html_css_color_applies_to_descendants() {
    init();
    let html = "<html><body><p>html-color</p></body></html>";
    let css = "html { color: cyan; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let has_color = tl.layout.iter().any(|b| b.fcolor == Some(Color::Cyan));
    assert!(
        has_color,
        "html-level color should cascade through body to <p>"
    );
}

#[test]
fn test_html_body_nested_div_renders() {
    init();
    let html = "<html><body><div><p>nested</p></div></body></html>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas = canvas_string(&tl);
    assert!(
        canvas.contains("nested"),
        "nested <p> inside <div> inside <body> should render"
    );
}
