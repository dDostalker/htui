//! `<blockquote>` tests — block-level extended quotation.
//!
//! The element carries browser UA default margins (`margin-block: 1em`,
//! `margin-inline: 40px`) adapted to the terminal as 1 blank row above /
//! below and 4 columns on each side. Any side the author overrides with an
//! explicit `margin` declaration loses its UA default, mirroring the
//! cascade priority of an author stylesheet over the UA stylesheet.

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
    let boxes = layout_tree(
        &arena,
        root_id,
        containing,
        &mut cursor,
        COLS as usize,
        ROWS as usize,
    );
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
fn test_blockquote_is_own_block_with_ua_margins() {
    init();
    // UA margins: 1 row above/below, 4 columns each side. The box must own
    // its rect (not be flattened into the parent flow like an unsupported
    // tag would be).
    let (boxes, arena) = layout_boxes("<blockquote>Quote</blockquote>", "");
    let bq = find_box_by_type(&boxes, &NodeType::BlockQuote, &arena)
        .expect("blockquote should produce its own box");
    assert_eq!(bq.rect.x, 4, "left UA margin of 4 columns");
    assert_eq!(bq.rect.y, 1, "top UA margin of 1 row");
    assert_eq!(bq.rect.width, 72, "80 minus 4 + 4 side margins");
    assert_eq!(bq.rect.height, 1, "single line of text");
}

#[test]
fn test_blockquote_text_position_on_canvas() {
    init();
    let (term_map, tl) = draw("<blockquote>Quote</blockquote>", "");
    let cols = term_map.columns as usize;
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "Q")
        .expect("quote text should render");
    assert_eq!(idx / cols, 1, "text sits on the row after the top margin");
    assert_eq!(idx % cols, 4, "text starts after the left margin");
}

#[test]
fn test_blockquote_margin_creates_gap_between_siblings() {
    init();
    // Two blockquotes: bottom margin of the first (1) + top margin of the
    // second (1) keep them two rows apart (the engine does not collapse
    // adjacent margins, same as every other element here).
    let (term_map, tl) = draw(
        "<blockquote>One</blockquote><blockquote>Two</blockquote>",
        "",
    );
    let cols = term_map.columns as usize;
    let one = tl
        .layout
        .iter()
        .position(|b| b.word == "O")
        .expect("first quote text");
    let two = tl
        .layout
        .iter()
        .position(|b| b.word == "T")
        .expect("second quote text");
    assert_eq!(one / cols, 1);
    assert_eq!(two / cols, 4);
    assert_eq!(one % cols, 4);
    assert_eq!(two % cols, 4);
}

#[test]
fn test_blockquote_author_margin_zero_removes_ua_defaults() {
    init();
    let (_, tl) = draw(
        "<blockquote>Quote</blockquote>",
        "blockquote { margin: 0; }",
    );
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "Q")
        .expect("quote text should render");
    assert_eq!(idx, 0, "author margin: 0 removes the UA margins entirely");
}

#[test]
fn test_blockquote_author_margin_left_override_drops_only_that_side() {
    init();
    // `margin-left: 0` drops the left UA margin only: text moves to col 0
    // but still sits one row down (top UA margin intact) and the right UA
    // margin still shrinks the box width to 76.
    let (boxes, arena) = layout_boxes(
        "<blockquote>Quote</blockquote>",
        "blockquote { margin-left: 0; }",
    );
    let bq = find_box_by_type(&boxes, &NodeType::BlockQuote, &arena).expect("blockquote box");
    assert_eq!(bq.rect.x, 0, "left UA margin dropped");
    assert_eq!(bq.rect.width, 76, "only the right margin (4) remains");
    assert_eq!(bq.rect.y, 1, "top UA margin still applies");
}

#[test]
fn test_blockquote_nested_p_child_shares_row() {
    init();
    let (boxes, arena) = layout_boxes("<blockquote><p>Inner</p></blockquote>", "");
    let bq = find_box_by_type(&boxes, &NodeType::BlockQuote, &arena).expect("blockquote box");
    let p = find_box_by_type(&boxes, &NodeType::P, &arena).expect("nested p box");
    assert_eq!(bq.rect.x, 4);
    assert_eq!(bq.rect.y, 1);
    assert_eq!(bq.rect.width, 72);
    assert_eq!(p.rect.x, 4, "p inherits the blockquote content box");
    assert_eq!(p.rect.y, 1, "p shares the blockquote row");
}

#[test]
fn test_blockquote_background_fills_box_only() {
    init();
    let (term_map, tl) = draw(
        "<blockquote>Quote</blockquote>",
        "blockquote { background-color: yellow; }",
    );
    let cols = term_map.columns as usize;
    let bg_in_row = |row: usize| {
        let start = row * cols;
        tl.layout[start..start + cols]
            .iter()
            .filter(|b| {
                b.bcolor
                    == Some(Color::Rgb {
                        r: 255,
                        g: 255,
                        b: 0,
                    })
            })
            .count()
    };
    assert!(
        bg_in_row(1) > 0,
        "row 1 is the blockquote rect and carries the bg"
    );
    assert_eq!(
        bg_in_row(0),
        0,
        "row 0 is the top margin and must be bg-free"
    );
    assert_eq!(
        bg_in_row(2),
        0,
        "row 2 is the bottom margin and must be bg-free"
    );
}

#[test]
fn test_blockquote_border_insets_content() {
    init();
    let (term_map, tl) = draw(
        "<blockquote>Quote</blockquote>",
        "blockquote { border: 1px solid red; }",
    );
    let cols = term_map.columns as usize;
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "Q")
        .expect("quote text should render");
    assert_eq!(idx / cols, 2, "content sits below the 1-col top border");
    assert_eq!(idx % cols, 5, "content shifts past the 1-col border");
    let (boxes, arena) = layout_boxes(
        "<blockquote>Quote</blockquote>",
        "blockquote { border: 1px solid red; }",
    );
    let bq = find_box_by_type(&boxes, &NodeType::BlockQuote, &arena).expect("blockquote box");
    assert_eq!(bq.rect.height, 3, "content row + top/bottom borders");
}

#[test]
fn test_nested_blockquote_indents_further() {
    init();
    // Each nesting level re-applies the UA margins, so the inner quote is
    // shifted 4 more columns right and one row down.
    let (boxes, arena) = layout_boxes("<blockquote><blockquote>Deep</blockquote></blockquote>", "");
    let outer =
        find_box_by_type(&boxes, &NodeType::BlockQuote, &arena).expect("outer blockquote box");
    assert_eq!(outer.rect.x, 4);
    assert_eq!(outer.rect.y, 1);
    assert_eq!(outer.rect.width, 72);
    let inner = find_box_by_type(&outer.children, &NodeType::BlockQuote, &arena)
        .expect("inner blockquote box");
    assert_eq!(inner.rect.x, 8, "outer left margin + inner left margin");
    assert_eq!(inner.rect.y, 2, "outer top margin + inner top margin");
    assert_eq!(inner.rect.width, 64, "72 minus 4 + 4 side margins");
}

#[test]
fn test_blockquote_css_type_selector_applies_color() {
    init();
    let (_, tl) = draw(
        "<blockquote>Quote</blockquote>",
        "blockquote { color: blue; }",
    );
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "Q")
        .expect("quote text should render");
    let blue = Color::Rgb { r: 0, g: 0, b: 255 };
    assert_eq!(
        tl.layout[idx].fcolor,
        Some(blue),
        "type selector must style blockquote text"
    );
}

#[test]
fn test_paragraph_after_blockquote_starts_below_margin() {
    init();
    let (term_map, tl) = draw("<blockquote>Q</blockquote><p>after</p>", "");
    let cols = term_map.columns as usize;
    let p_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "a")
        .expect("paragraph text should render");
    assert_eq!(
        p_idx / cols,
        3,
        "paragraph starts one row below the blockquote bottom margin"
    );
    assert_eq!(p_idx % cols, 0, "paragraph is not indented");
}
