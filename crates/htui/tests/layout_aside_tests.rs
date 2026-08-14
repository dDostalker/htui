use crossterm::style::Color;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;

use htui::tdom::node::{NodeArena, NodeId, NodeType};
use htui::tui::graph_api::TermLayout;

/// Recursively walk the layout tree (depth-first) and return the first box
/// whose node type matches `target`. The top-level layout is rooted in the
/// transparent `<html>` wrapper, so tests that want to inspect a specific
/// element must descend through it instead of scanning only the root slice.
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

#[test]
fn test_aside_basic() {
    init();
    let (root_id, arena) = setup("<aside>Side note</aside>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    // Filter the inter-cell whitespace from the joined canvas so the
    // assertion compares against the rendered text content only —
    // adjacent text-node runs and the wrap algorithm both preserve the
    // original whitespace inside `Side note`, but the surrounding cells
    // in the empty canvas read as `' '` when concatenated.
    let joined: String = canvas.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        joined.contains("Sidenote"),
        "aside should contain 'Side note', got {:?}",
        joined
    );
}

#[test]
fn test_aside_with_color() {
    init();
    let html = "<aside>Colored aside</aside>";
    let css = "aside { color: blue; background-color: yellow; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let has_fg = tl
        .layout
        .iter()
        .any(|b| b.fcolor == Some(Color::Rgb { r: 0, g: 0, b: 255 }));
    let has_bg = tl.layout.iter().any(|b| {
        b.bcolor
            == Some(Color::Rgb {
                r: 255,
                g: 255,
                b: 0,
            })
    });
    assert!(has_fg, "should have blue foreground");
    assert!(has_bg, "should have yellow background");
}

#[test]
fn test_aside_with_nested_div() {
    init();
    let html = "<aside><div>Inner div</div></aside>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    // See `test_aside_basic` for why we filter whitespace: the canvas
    // is 80×24 of which most cells are default whitespace, and the
    // rendered content "Inner div" sits at the start of one row.
    let joined: String = canvas.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        joined.contains("Innerdiv"),
        "aside should render nested div, got {:?}",
        joined
    );
}

#[test]
fn test_aside_width_50pct() {
    // Browser-like geometry: a block with explicit `width` sits flush left
    // inside its containing block (CSS `margin-left: 0` default). The 50%-wide
    // aside occupies cols 0..cols/2; the right half is bg-free.
    init();
    let html = "<aside>Half</aside>";
    let css = "aside { width: 50%; background-color: blue; color: white; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let block_left = 0;
    let block_width = cols / 2;
    let row = tl
        .layout
        .iter()
        .position(|b| b.bcolor == Some(Color::Rgb { r: 0, g: 0, b: 255 }))
        .map(|index| index / cols)
        .unwrap();
    let row_start = row * cols;
    let block = &tl.layout[row_start..row_start + cols];
    assert!(
        block[block_left..block_left + block_width]
            .iter()
            .all(|b| b.bcolor == Some(Color::Rgb { r: 0, g: 0, b: 255 })),
        "cols 0..cols/2 should all carry blue bg (block is flush left)"
    );
    assert!(
        block[block_left + block_width..]
            .iter()
            .all(|b| b.bcolor.is_none()),
        "cols cols/2..cols should be bg-free (outside the block)"
    );
    for cell in &block[block_left..block_left + block_width] {
        if cell.word != " " {
            assert_eq!(
                cell.fcolor,
                Some(Color::Rgb {
                    r: 255,
                    g: 255,
                    b: 255
                })
            );
        }
    }
}

#[test]
fn test_aside_width_fixed_px() {
    // Browser-like geometry: a fixed-width block sits flush left. The 20-col
    // block occupies cols 0..20; no centering offset.
    init();
    let html = "<aside>Fix</aside>";
    let css = "aside { width: 20px; background-color: red; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let row = tl
        .layout
        .iter()
        .position(|b| b.bcolor == Some(Color::Rgb { r: 255, g: 0, b: 0 }))
        .map(|index| index / term_map.columns as usize)
        .unwrap();
    let row_start = row * term_map.columns as usize;
    // Block spans cols 0..20 (flush left, NOT cols 30..50).
    let block = &tl.layout[row_start..row_start + 20];
    assert!(
        block
            .iter()
            .all(|b| { b.bcolor == Some(Color::Rgb { r: 255, g: 0, b: 0 }) }),
        "cols 0..20 should all carry red bg (block is flush left, 20 wide)"
    );
    // Cols 20..30 must be bg-free.
    let after = &tl.layout[row_start + 20..row_start + 30];
    assert!(
        after.iter().all(|b| b.bcolor.is_none()),
        "cols 20..30 should be bg-free (outside the 20-col block)"
    );
}

#[test]
fn test_aside_width_default_full() {
    init();
    let html = "<aside>Full width</aside>";
    let css = "aside { background-color: green; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let row = tl
        .layout
        .iter()
        .position(|b| b.bcolor == Some(Color::Rgb { r: 0, g: 128, b: 0 }))
        .map(|index| index / term_map.columns as usize)
        .unwrap();
    let row_start = row * term_map.columns as usize;
    assert!(
        tl.layout[row_start..row_start + COLS as usize]
            .iter()
            .all(|b| b.bcolor == Some(Color::Rgb { r: 0, g: 128, b: 0 }))
    );
    assert!(
        tl.layout[row_start..row_start + COLS as usize]
            .iter()
            .any(|b| b.word == " ")
    );
}

#[test]
fn test_aside_text_align_center() {
    // Browser-like geometry: the BLOCK is flush left (cols 0..40); the TEXT
    // inside is centered within the block (offset = (40 - len("Centered")) / 2).
    // The block must not be centered in the terminal.
    init();
    let html = "<aside>Centered</aside>";
    let css = "aside { text-align: center; width: 40px; background-color: darkblue; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    // The block spans cols 0..40; text "Centered" (8 chars) is centered, so it
    // starts at col (40 - 8) / 2 = 16.
    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == *"C")
        .expect("should find first text char");
    let col = text_idx % cols;
    assert_eq!(
        col, 16,
        "text must be centered within the (flush-left) 40-col block"
    );
    // Block bg occupies exactly 40 cells, all within cols 0..40.
    let bg_cells: Vec<_> = tl.layout.iter().filter(|b| b.bcolor.is_some()).collect();
    assert_eq!(
        bg_cells.len(),
        40,
        "background should cover the whole block"
    );
    let bg_cols: Vec<_> = bg_cells
        .iter()
        .map(|b| tl.layout.iter().position(|c| std::ptr::eq(c, *b)).unwrap() % cols)
        .collect();
    assert!(
        bg_cols.iter().all(|&c| c < 40),
        "block bg must be within cols 0..40 (flush left)"
    );
}

#[test]
fn test_aside_text_align_right() {
    // Browser-like geometry: the BLOCK is flush left (cols 0..40); the TEXT
    // inside is right-aligned within the block (offset = 40 - len("Right") = 35).
    init();
    let html = "<aside>Right</aside>";
    let css = "aside { text-align: right; width: 40px; background-color: darkblue; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    // The block spans cols 0..40; text "Right" (5 chars) is right-aligned, so
    // it starts at col 40 - 5 = 35.
    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == *"R")
        .expect("should find first text char");
    let col = text_idx % cols;
    assert_eq!(
        col, 35,
        "text must be right-aligned within the (flush-left) 40-col block"
    );
    let bg_cells: Vec<_> = tl.layout.iter().filter(|b| b.bcolor.is_some()).collect();
    assert_eq!(
        bg_cells.len(),
        40,
        "background should cover the whole block"
    );
    let bg_cols: Vec<_> = bg_cells
        .iter()
        .map(|b| tl.layout.iter().position(|c| std::ptr::eq(c, *b)).unwrap() % cols)
        .collect();
    assert!(
        bg_cols.iter().all(|&c| c < 40),
        "block bg must be within cols 0..40 (flush left)"
    );
}

// ============================================================
// Multi-row background — regression coverage for the bug where
// `<aside>` rendered its background only on the first row, leaving
// nested block children visually detached from the aside's bg.
// ============================================================

#[test]
fn test_aside_bg_covers_nested_p_row() {
    // Browser-like geometry: `<aside>` does NOT reserve its own empty row.
    // The aside's rect wraps the children directly, so for
    // `<aside><p>Nested content</p></aside>`:
    //   - aside.rect = (x, y, w, 1)
    //   - nested <p> sits on the same row as the aside
    //   - the next row is bg-free
    // Row 0 carries the aside's bg on every non-text cell; the row after
    // the aside is bg-free.
    init();
    let html = "<aside><p>Nested content</p></aside>";
    let css = "aside { background-color: darkblue; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let row0 = &tl.layout[0..cols];
    let row1 = &tl.layout[cols..2 * cols];

    // Row 0 = the aside's row, which is also the nested <p>'s row. The
    // bg covers every non-text cell.
    let row0_bg = row0
        .iter()
        .filter(|b| b.bcolor == Some(Color::Rgb { r: 0, g: 0, b: 139 }))
        .count();
    assert!(
        row0_bg > 0,
        "row 0 must carry the aside's bg (got {row0_bg}/{cols} bg cells)"
    );
    assert!(
        row0_bg >= cols - "Nested content".len(),
        "row 0 bg should fill all non-text cells (got {row0_bg} bg cells, expected >= {})",
        cols - "Nested content".len()
    );
    // Row 1 is outside the aside — must be bg-free.
    assert!(
        row1.iter().all(|b| b.bcolor.is_none()),
        "row 1 must not carry the aside's bg (aside is exactly 1 row tall)"
    );
}

#[test]
fn test_aside_rect_height_includes_nested_children() {
    // Direct check on the layout tree: the aside wraps its children on the
    // same row, so rect.height equals the rows occupied by children (1 for
    // a single <p>). The nested <p> starts at the aside's own y, not one
    // row below.
    init();
    let html = "<aside><p>only child</p></aside>";
    let css = "aside { background-color: darkblue; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();

    use htui::layout::{BoxConstraint, Cursor, inner_formatting_context::flow::layout_tree};
    let containing = BoxConstraint::root(&term_map);
    let mut cursor = Cursor::new();
    let boxes = layout_tree(&arena, root_id, containing, &mut cursor, 80, 24);

    let aside_box = find_box_by_type(&boxes, &NodeType::Aside, &arena)
        .expect("aside box should be present in the layout tree");
    assert_eq!(
        aside_box.rect.height, 1,
        "aside rect.height should NOT reserve an empty own row; it wraps the children (1 row for 1 <p>)"
    );
    assert_eq!(aside_box.children.len(), 1, "aside should have 1 child box");
    assert_eq!(
        aside_box.children[0].rect.y, aside_box.rect.y,
        "child should share the aside's row (no offset)"
    );
}

#[test]
fn test_aside_bg_covers_two_nested_p_rows() {
    // Two nested <p> siblings share the aside's bg across rows 0 and 1.
    // The aside's rect is exactly 2 rows tall (rows 0..2), so row 2 must
    // be bg-free.
    init();
    let html = "<aside><p>first</p><p>second</p></aside>";
    let css = "aside { background-color: darkblue; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    // Rows 0..2 are the aside's rect. Each row carries bg on non-text
    // cells; rows 0 and 1 lose cells to "first"/"second" text.
    for row in 0..2 {
        let start = row * cols;
        let row_slice = &tl.layout[start..start + cols];
        let bg_count = row_slice
            .iter()
            .filter(|b| b.bcolor == Some(Color::Rgb { r: 0, g: 0, b: 139 }))
            .count();
        assert!(
            bg_count > 0,
            "row {row} must carry the aside's bg (got {bg_count}/{cols})"
        );
    }
    // Row 2 must be bg-free (rect is exactly 2 rows).
    let row2_start = 2 * cols;
    if row2_start + cols <= tl.layout.len() {
        let row2 = &tl.layout[row2_start..row2_start + cols];
        assert!(
            row2.iter().all(|b| b.bcolor.is_none()),
            "row 2 must not carry the aside's bg (rect should be exactly 2 rows)"
        );
    }
}

#[test]
fn test_header_has_no_trailing_spacing() {
    // `<header>` is a plain block now: exactly one row, no two trailing
    // blank rows baked in.
    init();
    let html = "<header>Pinned title</header><p>after</p>";
    let css = "header { background-color: green; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();

    use htui::layout::{BoxConstraint, Cursor, inner_formatting_context::flow::layout_tree};
    let containing = BoxConstraint::root(&term_map);
    let mut cursor = Cursor::new();
    let boxes = layout_tree(&arena, root_id, containing, &mut cursor, 80, 24);

    let header_box =
        find_box_by_type(&boxes, &NodeType::Header, &arena).expect("header box should be present");
    assert_eq!(
        header_box.rect.height, 1,
        "header must not reserve trailing spacing rows"
    );
}

#[test]
fn test_footer_has_no_trailing_spacing() {
    // `<footer>` is a plain block now: exactly one row, no two trailing
    // blank rows baked in.
    init();
    let html = "<p>above</p><footer>tail</footer>";
    let css = "footer { background-color: red; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();

    use htui::layout::{BoxConstraint, Cursor, inner_formatting_context::flow::layout_tree};
    let containing = BoxConstraint::root(&term_map);
    let mut cursor = Cursor::new();
    let boxes = layout_tree(&arena, root_id, containing, &mut cursor, 80, 24);

    let footer_box =
        find_box_by_type(&boxes, &NodeType::Footer, &arena).expect("footer box should be present");
    assert_eq!(
        footer_box.rect.height, 1,
        "footer must not reserve trailing spacing rows"
    );
}

#[test]
fn test_semantic_blocks_have_no_implicit_background() {
    // `<aside>` / `<header>` / `<footer>` are plain blocks like `<div>`:
    // no default background unless CSS sets one.
    init();
    for tag in ["aside", "header", "footer"] {
        let html = format!("<{tag}>text</{tag}>");
        let (root_id, arena) = setup(&html, "");
        let term_map = mock_term_map();
        let mut tl = TermLayout::new(&term_map, 1.0);
        tl.draw(&arena, root_id, &term_map);
        assert!(
            tl.layout.iter().all(|b| b.bcolor.is_none()),
            "<{tag}> must not paint an implicit background"
        );
    }
}

#[test]
fn test_aside_with_nested_div_extends_bg_to_div_row() {
    // Block-level child of any kind (not just <p>) shares the aside's row.
    // For `<aside><div>block child</div></aside>` the aside rect is 1 row
    // tall, so the bg (and the nested <div>) live on row 0; row 1 is
    // bg-free.
    init();
    let html = "<aside><div>block child</div></aside>";
    let css = "aside { background-color: darkblue; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let row0 = &tl.layout[0..cols];
    let row0_bg = row0
        .iter()
        .filter(|b| b.bcolor == Some(Color::Rgb { r: 0, g: 0, b: 139 }))
        .count();
    assert!(
        row0_bg > 0,
        "row 0 should carry the aside's bg around the nested <div> (got {row0_bg} bg cells)"
    );
    assert!(
        row0_bg >= cols - "block child".len(),
        "row 0 should be mostly bg except for text cells"
    );
    // Row 1 is outside the aside — must be bg-free.
    let row1 = &tl.layout[cols..2 * cols];
    assert!(
        row1.iter().all(|b| b.bcolor.is_none()),
        "row 1 must not carry the aside's bg (aside is exactly 1 row tall)"
    );
}

#[test]
fn test_aside_without_children_still_has_height_one() {
    // Guard against an over-eager fix: an aside with no nested blocks must
    // still report height=1 so single-row tests (e.g. test_aside_text_align_center)
    // stay green.
    init();
    let html = "<aside>just text</aside>";
    let css = "aside { background-color: darkblue; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();

    use htui::layout::{BoxConstraint, Cursor, inner_formatting_context::flow::layout_tree};
    let containing = BoxConstraint::root(&term_map);
    let mut cursor = Cursor::new();
    let boxes = layout_tree(&arena, root_id, containing, &mut cursor, 80, 24);

    let aside_box = find_box_by_type(&boxes, &NodeType::Aside, &arena)
        .expect("aside box should be present in the layout tree");
    assert_eq!(
        aside_box.rect.height, 1,
        "aside with no nested blocks must remain height=1"
    );
    assert!(
        aside_box.children.is_empty(),
        "aside should have no children"
    );
}

// ============================================================
// Background inheritance — text inside a child box without its own
// background-color must adopt the enclosing rect's effective bg, so the
// rendering matches a web browser (no "镂空" gaps on the parent's bg).
// ============================================================

fn cell_containing(tl: &TermLayout, ch: char) -> &htui::tui::graph_api::Block {
    let needle = ch.to_string();
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == needle)
        .unwrap_or_else(|| panic!("should find character {ch:?} in canvas"));
    &tl.layout[idx]
}

#[test]
fn test_nested_p_text_inherits_aside_bg() {
    // `<aside bg=cyan><p>30col</p></aside>` must render "30col" on cyan,
    // matching how a browser would render it.
    init();
    let html = "<aside><p>30col</p></aside>";
    let css = "aside { background-color: cyan; color: black; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    for ch in ['3', '0', 'c', 'o', 'l'] {
        let cell = cell_containing(&tl, ch);
        assert_eq!(
            cell.bcolor,
            Some(Color::Rgb {
                r: 0,
                g: 255,
                b: 255
            }),
            "text cell for {ch:?} must inherit the aside's cyan bg"
        );
        assert_eq!(
            cell.fcolor,
            Some(Color::Rgb { r: 0, g: 0, b: 0 }),
            "text cell for {ch:?} must keep the black fg"
        );
    }
}

#[test]
fn test_nested_p_text_bg_wins_over_parent() {
    // If both parent and child have a bg, the child wins.
    init();
    let html = "<aside><p>kid</p></aside>";
    let css = "aside { background-color: cyan; } p { background-color: yellow; color: black; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    for ch in ['k', 'i', 'd'] {
        let cell = cell_containing(&tl, ch);
        assert_eq!(
            cell.bcolor,
            Some(Color::Rgb {
                r: 255,
                g: 255,
                b: 0
            }),
            "text cell for {ch:?} should use <p>'s own yellow bg, not aside's cyan"
        );
    }
}

#[test]
fn test_nested_p_inherits_through_intermediate_div() {
    // `<aside bg=blue><div bg=red><p>X</p></div></aside>`: the <p> text must
    // show red bg, not blue, because the inner <div> sits between aside and p.
    init();
    let html = "<aside><div><p>X</p></div></aside>";
    let css =
        "aside { background-color: blue; } div { background-color: red; } p { color: white; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let cell = cell_containing(&tl, 'X');
    assert_eq!(
        cell.bcolor,
        Some(Color::Rgb { r: 255, g: 0, b: 0 }),
        "<p> text should inherit <div>'s red bg, not aside's blue"
    );
    assert_eq!(
        cell.fcolor,
        Some(Color::Rgb {
            r: 255,
            g: 255,
            b: 255
        })
    );
}

#[test]
fn test_nested_p_inherits_when_parent_has_no_bg() {
    // `<aside><p>foo</p></aside>` with no bg anywhere: <p> text should have
    // no bg (terminal default), so behaviour matches the previous pipeline.
    init();
    let html = "<aside><p>foo</p></aside>";
    let css = "";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    for ch in ['f', 'o'] {
        let cell = cell_containing(&tl, ch);
        assert!(
            cell.bcolor.is_none(),
            "with no parent bg, text cell {ch:?} should carry no bg"
        );
    }
}

#[test]
fn test_nested_p_in_aside_with_fixed_width_centered() {
    // Mirror the visual_test scenario exactly: `<aside width=30px bg=cyan>`
    // containing `<p>30col</p>`. The aside sits flush left at col 0; the
    // text "30col" (5 chars) is centered within the 30-col block, so it
    // starts at col (30 - 5) / 2 = 12.
    init();
    let html = "<aside><p>30col</p></aside>";
    let css = "aside { width: 30px; text-align: center; background-color: cyan; color: black; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let aside_left = 0;
    // Find the first text cell of "30col".
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == *"3")
        .expect("should find '3'");
    let col = idx % cols;
    assert_eq!(
        col,
        aside_left + (30 - 5) / 2,
        "30col must be centered inside the (flush-left) 30-col aside block"
    );
    // Every text cell must carry the parent's cyan bg.
    for ch in ['3', '0', 'c', 'o', 'l'] {
        let cell = cell_containing(&tl, ch);
        assert!(
            cell.bcolor
                == Some(Color::Rgb {
                    r: 0,
                    g: 255,
                    b: 255
                }),
            "text cell {ch:?} must inherit cyan bg from the aside"
        );
    }
}
