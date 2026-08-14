use crossterm::style::Color;
use htui::html::css_parse;
use htui::html::html_parse;
use htui::layout::layout::TermMap;
use htui::tdom::node::{NodeArena, NodeId, NodeType};
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

fn cell_containing(tl: &TermLayout, needle: &str) -> Option<(usize, usize, usize)> {
    // Returns (start_idx, row, col) of the first cell whose render text
    // equals `needle`. `char_width` does not apply here because we look up the
    // exact cell produced by the paint phase (no wide-char padding).
    for (idx, block) in tl.layout.iter().enumerate() {
        if block.word == needle {
            let cols = tl.columns as usize;
            return Some((idx, idx / cols, idx % cols));
        }
    }
    None
}

#[test]
fn test_parse_tag_map_ul_ol_li() {
    assert_eq!(html_parse::tag_name_to_node_type("ul"), Some(NodeType::Ul));
    assert_eq!(html_parse::tag_name_to_node_type("ol"), Some(NodeType::Ol));
    assert_eq!(html_parse::tag_name_to_node_type("li"), Some(NodeType::Li));
}

#[test]
fn test_parse_tag_map_uppercase_ul_ol_li() {
    assert_eq!(html_parse::tag_name_to_node_type("UL"), Some(NodeType::Ul));
    assert_eq!(html_parse::tag_name_to_node_type("OL"), Some(NodeType::Ol));
    assert_eq!(html_parse::tag_name_to_node_type("LI"), Some(NodeType::Li));
}

#[test]
fn test_ul_renders_bullet_markers() {
    init();
    let (root_id, arena) = setup("<ul><li>One</li><li>Two</li></ul>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(
        canvas.contains("•"),
        "unordered list must emit a bullet glyph"
    );
    assert!(canvas.contains("One"), "first item text missing");
    assert!(canvas.contains("Two"), "second item text missing");
}

#[test]
fn test_ol_renders_sequential_markers() {
    init();
    let (root_id, arena) = setup("<ol><li>First</li><li>Second</li><li>Third</li></ol>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("1."), "ol must show ordinal '1.'");
    assert!(canvas.contains("2."), "ol must show ordinal '2.'");
    assert!(canvas.contains("3."), "ol must show ordinal '3.'");
    assert!(canvas.contains("First"));
    assert!(canvas.contains("Second"));
    assert!(canvas.contains("Third"));
}

#[test]
fn test_ol_ordinal_skips_non_li_siblings() {
    // Whitespace text nodes and inline text must not advance the counter.
    init();
    let (root_id, arena) = setup("<ol><li>A</li>garbage<li>B</li></ol>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("1."), "first <li> should still be '1.'");
    assert!(
        canvas.contains("2."),
        "second <li> should be '2.', not '3.'"
    );
    assert!(!canvas.contains("3."), "counter must not skip ahead");
}

#[test]
fn test_empty_li_still_renders_marker() {
    init();
    let (root_id, arena) = setup("<ul><li></li><li>Two</li></ul>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    let bullet_count = canvas.matches('•').count();
    assert!(
        bullet_count >= 2,
        "expected at least 2 bullet markers (incl. empty <li>), got {bullet_count}"
    );
}

#[test]
fn test_li_items_each_occupy_their_own_row() {
    init();
    let (root_id, arena) = setup("<ul><li>A</li><li>B</li></ul>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let (a_idx, a_row, _) = cell_containing(&tl, "A").expect("missing 'A'");
    let (b_idx, b_row, _) = cell_containing(&tl, "B").expect("missing 'B'");
    assert_ne!(a_idx, b_idx, "A and B must be in different cells");
    assert_ne!(a_row, b_row, "A and B must be on different rows");
}

#[test]
fn test_ul_marker_sits_left_of_body() {
    // Bullet is at column 0; body text starts after the marker width.
    init();
    let (root_id, arena) = setup("<ul><li>Hello</li></ul>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let (_bullet_idx, bullet_row, bullet_col) = cell_containing(&tl, "•").expect("bullet missing");
    let (h_idx, h_row, h_col) = cell_containing(&tl, "H").expect("missing 'H'");
    assert_eq!(bullet_row, h_row, "bullet and body share the marker row");
    assert_eq!(bullet_col, 0, "bullet sits at column 0 for a top-level ul");
    assert!(
        h_col > bullet_col,
        "body 'H' (col {h_col}) must sit to the right of bullet (col {bullet_col}); h_idx={h_idx}"
    );
}

#[test]
fn test_nested_ul_indents_inner_items() {
    // Top-level ul: items at col 0; nested ul shifts its items by LIST_INDENT_PER_LEVEL.
    init();
    let html = "<ul><li>Outer A<ul><li>Inner A</li></ul></li><li>Outer B</li></ul>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    // Find all bullet markers. The first one (row 0) belongs to the outer
    // 'Outer A' <li>, the third (or later) one to 'Inner A'.
    let bullet_cells: Vec<(usize, usize)> = tl
        .layout
        .iter()
        .enumerate()
        .filter_map(|(idx, b)| {
            if b.word == "•" {
                let row = idx / tl.columns as usize;
                let col = idx % tl.columns as usize;
                Some((row, col))
            } else {
                None
            }
        })
        .collect();
    assert!(
        bullet_cells.len() >= 3,
        "need at least 3 bullets (Outer A, Inner A, Outer B); got {}",
        bullet_cells.len()
    );

    let outer_a_col = bullet_cells[0].1;
    let inner_a_col = bullet_cells[1].1;
    let outer_b_col = bullet_cells[2].1;

    assert_eq!(
        outer_a_col, 0,
        "outer items' bullet must be flush at col 0 (row={}, idx={})",
        bullet_cells[0].0, bullet_cells[0].1
    );
    assert_eq!(
        outer_b_col, 0,
        "second outer item's bullet must also be at col 0 (row={})",
        bullet_cells[2].0
    );
    assert!(
        inner_a_col > outer_a_col,
        "inner item bullet (col {inner_a_col}) must sit to the right of outer (col {outer_a_col})"
    );
}

#[test]
fn test_nested_ol_in_ul_renders_mixed_markers() {
    init();
    let html = "<ul><li>Outer<ol><li>Inner One</li><li>Inner Two</li></ol></li></ul>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("•"), "outer ul must emit bullet");
    assert!(canvas.contains("1."), "inner ol must emit ordinal 1");
    assert!(canvas.contains("2."), "inner ol must emit ordinal 2");
}

#[test]
fn test_li_with_span_styling() {
    init();
    let html = "<ul><li>plain <span class=\"bold\">bold</span></li></ul>";
    let css = "ul li .bold { font-weight: bold; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("plain"), "plain body text missing");
    assert!(canvas.contains("bold"), "span text missing");

    // The selector `ul li .bold` should set BOLD on the <span>'s letters.
    let bold_letters: String = tl
        .layout
        .iter()
        .filter(|b| b.style.contains(htui::tui::graph_api::FontStyle::BOLD))
        .map(|b| b.word.as_str())
        .collect();
    assert!(
        bold_letters.contains('b'),
        "expected at least one bold letter; got {bold_letters:?}"
    );
}

#[test]
fn test_li_with_css_color() {
    init();
    let html = "<ul><li>Red bullet item</li></ul>";
    let css = "ul li { color: red; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let has_red = tl
        .layout
        .iter()
        .any(|b| b.fcolor == Some(Color::Rgb { r: 255, g: 0, b: 0 }));
    assert!(
        has_red,
        "ul li should pick up `color: red` via selector match"
    );
}

#[test]
fn test_ul_with_bg_color_fills_list_rect() {
    init();
    let html = "<ul><li>first</li><li>second</li></ul>";
    let css = "ul { background-color: yellow; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let yellow_count = tl
        .layout
        .iter()
        .filter(|b| {
            b.bcolor
                == Some(Color::Rgb {
                    r: 255,
                    g: 255,
                    b: 0,
                })
        })
        .count();
    assert!(
        yellow_count > 0,
        "ul background-color should paint at least one cell"
    );
}

#[test]
fn test_orphan_li_uses_dash_marker() {
    init();
    let (root_id, arena) = setup("<li>orphan</li>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    // Orphan <li> falls back to a dash marker so it's not blank.
    assert!(
        canvas.contains('-'),
        "orphan <li> should render a dash marker; canvas={:?}",
        canvas
    );
    assert!(canvas.contains("orphan"), "orphan <li> body should render");
}

#[test]
fn test_ol_double_digit_ordinals() {
    init();
    // Item 10 -> marker "10. " (4 cells), still ahead of items 1..9.
    let html = "<ol><li>a</li><li>b</li><li>c</li><li>d</li><li>e</li><li>f</li><li>g</li><li>h</li><li>i</li><li>j</li></ol>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("10."), "double-digit ordinal must render");
    // All ten items should be present.
    for letter in ['a', 'b', 'c', 'd', 'e', 'j'] {
        assert!(canvas.contains(letter), "missing item body '{letter}'");
    }
}

#[test]
fn test_nested_ul_inside_li_stacks_below_parent_text() {
    init();
    // Regression: the parent <li>'s own text and its nested <ul> children must
    // stack on separate rows. Previously the nested list started on the same
    // row as the parent's text, so "Nested list" was partially overwritten by
    // the first nested item ("• Inner A").
    let html = "<ul><li>Nested list<ul><li>Inner A</li><li>Inner B</li></ul></li><li>Sibling of nested</li></ul>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let cols = term_map.columns as usize;
    let rows: Vec<String> = tl
        .layout
        .chunks(cols)
        .map(|chunk| chunk.iter().map(|b| b.word.as_str()).collect())
        .collect();
    let li_row = rows
        .iter()
        .position(|r| r.contains("Nested list"))
        .expect("parent li text should render intact");
    let inner_a_row = rows
        .iter()
        .position(|r| r.contains("Inner A"))
        .expect("first nested item should render");
    let inner_b_row = rows
        .iter()
        .position(|r| r.contains("Inner B"))
        .expect("second nested item should render");
    let sibling_row = rows
        .iter()
        .position(|r| r.contains("Sibling"))
        .expect("sibling li should render");
    assert!(
        li_row < inner_a_row && inner_a_row < inner_b_row && inner_b_row < sibling_row,
        "rows must stack li/{inner_a_row}/{inner_b_row}/{sibling_row}: {:?}",
        rows[li_row]
    );
}
