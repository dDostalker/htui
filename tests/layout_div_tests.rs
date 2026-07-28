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

#[test]
fn test_div_basic() {
    init();
    let (root_id, arena) = setup("<div>hello</div>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
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
    let mut tl = TermLayout::new(&term_map, 1.0);
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
    let mut tl = TermLayout::new(&term_map, 1.0);
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
    let mut tl = TermLayout::new(&term_map, 1.0);
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
fn test_div_width_50pct_left() {
    // Browser-like geometry: a block with explicit `width` sits flush left
    // inside its containing block (CSS `margin-left: 0` default). The
    // 50%-wide div occupies cols 0..cols/2; the right half is bg-free.
    init();
    let html = "<div>half</div>";
    let css = "div { width: 50%; background-color: blue; color: white; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let content_idx = tl
        .layout
        .iter()
        .position(|b| b.bcolor.is_some() && b.word != " ")
        .expect("should find colored content");
    let content_row = content_idx / cols;
    let row = &tl.layout[content_row * cols..(content_row + 1) * cols];

    let block_start = 0;
    let block_end = cols / 2;
    let mut has_text = false;
    for i in block_start..block_end {
        assert!(
            row[i].bcolor.is_some(),
            "block should have bg col={i} (flush left)"
        );
        if row[i].word != " " {
            assert_eq!(row[i].fcolor, Some(Color::White), "text should be white");
            has_text = true;
        }
    }
    assert!(has_text, "block should contain text");
    for i in block_end..cols {
        assert!(
            row[i].bcolor.is_none(),
            "cols cols/2..cols should be bg-free (outside the block)"
        );
    }
}

#[test]
fn test_div_width_fixed() {
    // Browser-like geometry: a fixed-width block sits flush left. The 20-col
    // block occupies cols 0..20.
    init();
    let html = "<div>Fix</div>";
    let css = "div { width: 20px; background-color: red; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
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
    assert_eq!(
        bg_count, 20,
        "background should be exactly 20 columns (cols 0..20, flush left)"
    );
    // Block bg starts at col 0 (flush left), not centered.
    let first_bg_col = (rstart..rend)
        .find(|&i| tl.layout[i].bcolor.is_some())
        .map(|i| i - rstart)
        .expect("block should have at least one bg cell");
    assert_eq!(
        first_bg_col, 0,
        "fixed-width block must sit flush left at col 0"
    );
}

#[test]
fn test_div_text_align_center() {
    // Browser-like geometry: the BLOCK is flush left (cols 0..40); the TEXT
    // inside is centered within the block.
    init();
    let html = "<div>Centered</div>";
    let css = "div { text-align: center; width: 40px; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    // Find the first text cell; text "Centered" (8 chars) sits at col
    // (40 - 8) / 2 = 16 within the flush-left block.
    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == *"C")
        .expect("should find text");
    let col = text_idx % cols;
    let expected_padding = (40 - "Centered".len()) / 2;
    assert_eq!(
        col, expected_padding,
        "text should be centered within the (flush-left) 40-col block"
    );
}

#[test]
fn test_div_text_align_right() {
    // Browser-like geometry: the BLOCK is flush left (cols 0..40); the TEXT
    // inside is right-aligned within the block.
    init();
    let html = "<div>Right</div>";
    let css = "div { text-align: right; width: 40px; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    // Find the first text cell; text "Right" (5 chars) sits at col
    // 40 - 5 = 35 within the flush-left block.
    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == *"R")
        .expect("should find text");
    let col = text_idx % cols;
    let expected_padding = 40 - "Right".len();
    assert_eq!(
        col, expected_padding,
        "text should be right-aligned within the (flush-left) 40-col block"
    );
}

#[test]
fn test_div_nested_preserves_text() {
    init();
    let html = "<div><p>A</p><div><p>B</p></div></div>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
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
    let mut tl = TermLayout::new(&term_map, 1.0);
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
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let has_color = tl.layout.iter().any(|b| b.fcolor == Some(Color::Magenta));
    assert!(has_color, "div color should inherit to <p> descendant");
}

#[test]
fn test_long_p_text_does_not_overflow_into_next_row() {
    // Regression guard: `paint_text_row` previously wrote cells linearly
    // with only a canvas-end check, so a long inline run would spill its
    // overflow into the rows below and overwrite the cells of subsequent
    // boxes (e.g. an h2 after the long p would show trailing "dth." in the
    // p's fg color). The fix clips each row's text to the content-box
    // right edge, so a 1-row `<p>` must NOT write past col 79.
    init();
    let long_text = "A longer paragraph that contains multiple sentences. This demonstrates how text wrapping and layout work in the terminal renderer. The text should flow naturally within the available terminal width.";
    let html = format!("<p>{}</p><h2>Next</h2>", long_text);
    let (root_id, arena) = setup(&html, "");
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    // The <p> occupies row 0 (1 row tall). Cells row 1+ belong to the <h2>.
    let p_row = 0;
    let p_row_end = (p_row + 1) * cols;
    for cell in &tl.layout[p_row_end..] {
        // None of the <p>'s colour / chars should appear past row 0. The
        // fix means the paint simply stops at col 79; cells beyond stay as
        // either the empty default (no fg, no bg) or get overwritten by
        // the next paint pass for the <h2>. We assert no p-coloured
        // character remains in those cells.
        let has_p_text =
            !cell.word.is_empty() && cell.word != " " && cell.fcolor == Some(Color::Grey);
        assert!(
            !has_p_text,
            "p text should not bleed past row 0 — found {:?} at cell index {}",
            cell.word, p_row_end
        );
    }
}

#[test]
fn test_long_p_text_stops_at_content_box_right_edge() {
    // Companion to the overflow guard above: the last char of a long
    // single-row <p> sits at col `cols - 1`, not at col `cols` (which would
    // be on the next row).
    init();
    let long_text = "x".repeat(200);
    let html = format!("<p>{}</p>", long_text);
    let (root_id, arena) = setup(&html, "");
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    // The <p> is 1 row tall. Row 1 should be untouched by the p (no 'x'
    // cells in grey).
    let row1_start = cols;
    let row1_end = 2 * cols;
    let p_chars_on_row1 = tl.layout[row1_start..row1_end]
        .iter()
        .filter(|c| c.fcolor == Some(Color::Grey) && c.word == "x")
        .count();
    assert_eq!(
        p_chars_on_row1, 0,
        "no 'x' characters should spill from the p onto row 1"
    );
}
