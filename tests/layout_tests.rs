use htui::html::html_parse::Parser;
use htui::layout::layout::TermMap;
use htui::tui::graph_api::TermLayout;

#[test]
fn test_term_map_mock() {
    let map = TermMap {
        columns: 80,
        rows: 24,
    };
    assert_eq!(map.columns, 80);
    assert_eq!(map.rows, 24);
}

#[test]
fn test_layout_p() {
    let mut p = Parser::new("<p>Hello</p><p>World</p>");
    let (root, arena, _) = p.parse().unwrap();
    let term_map = TermMap {
        columns: 80,
        rows: 24,
    };
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root, &term_map).unwrap();
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("Hello"));
    assert!(canvas.contains("World"));
}
