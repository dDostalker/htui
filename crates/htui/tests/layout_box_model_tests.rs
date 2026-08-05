//! Box-model tests — `padding` / `margin` / `border` resolution through the
//! two-phase layout / paint pipeline.
//!
//! Each test stands up an HTML snippet with a single block-level element and
//! asserts on the resulting `Block` cells in the `TermLayout` canvas:
//!
//! - Padding tests check the bg extends into the padding area and that
//!   inline content shifts into the content-box (rect shrunk by padding).
//! - Margin tests check the box's outer position / inter-sibling spacing.
//! - Border tests check the four Unicode box-drawing edges and corners
//!   appear around the rect.

use crossterm::style::Color;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::tdom::node::NodeArena;
use htui::tdom::node::NodeId;
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

/// Build a fresh canvas and draw the given DOM.
fn draw(html: &str, css: &str) -> (TermMap, TermLayout) {
    let term_map = mock_term_map();
    let (root_id, arena) = setup(html, css);
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    (term_map, tl)
}

// ============================================================
// padding
// ============================================================

#[test]
fn padding_pushes_inline_content_inward() {
    init();
    // `<div>` with padding-top=2, padding-left=3. The text "Hi" should land
    // at row 2 (below 2 padding rows) and col 3 (after 3 padding cols).
    let (term_map, tl) = draw(
        "<div>Hi</div>",
        "div { padding: 2px 3px; background-color: yellow; }",
    );
    let cols = term_map.columns as usize;

    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "H")
        .expect("'Hi' should render");
    let row = text_idx / cols;
    let col = text_idx % cols;
    assert_eq!(row, 2, "padding-top should push text 2 rows down");
    assert_eq!(col, 3, "padding-left should push text 3 cols right");
}

#[test]
fn padding_expands_rect_vertically() {
    init();
    // An empty `<div>` with padding-top=4, padding-bottom=2 should still
    // own a border-box of `1 (own row) + 6 (padding)` = 7 rows.
    let (_term_map, tl) = draw("<div></div>", "div { padding: 4px 0 2px 0; }");

    // No bg → no filled cells; we inspect `rendered_rows` which tracks the
    // deepest descendant's bottom row.
    assert!(
        tl.rendered_rows >= 7,
        "padding should grow the div's rect to at least 7 rows, got {}",
        tl.rendered_rows
    );
}

#[test]
fn padding_background_fills_padding_box() {
    init();
    // Padding area carries the bg, not just the content row.
    let (term_map, tl) = draw(
        "<div>X</div>",
        "div { padding: 3px; background-color: cyan; }",
    );
    let cols = term_map.columns as usize;

    // Row 0 is a padding row — must carry cyan bg.
    let row0 = &tl.layout[0..cols];
    let cyan_count = row0
        .iter()
        .filter(|b| b.bcolor == Some(Color::Rgb { r: 0, g: 255, b: 255 }))
        .count();
    assert_eq!(
        cyan_count, cols,
        "padding-top row must be filled with the bg (full width)"
    );
}

#[test]
fn padding_shrinks_inner_containing_block() {
    init();
    // A nested `<p>` inside `<div>` with padding should sit inside the
    // content area (after padding). padding-top=2 means <p> lands at row 2.
    let (term_map, tl) = draw(
        "<div><p>Inner</p></div>",
        "div { padding-top: 2px; padding-left: 5px; }",
    );
    let cols = term_map.columns as usize;
    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "I")
        .expect("'Inner' should render");
    assert_eq!(text_idx / cols, 2, "p must start after padding-top=2");
    assert_eq!(text_idx % cols, 5, "p must start after padding-left=5");
}

// ============================================================
// margin
// ============================================================

#[test]
fn margin_left_shifts_box_horizontally() {
    init();
    // margin-left=4 should push the box 4 cols to the right.
    let (term_map, tl) = draw("<div>X</div>", "div { margin-left: 4px; }");
    let cols = term_map.columns as usize;
    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "X")
        .expect("'X' should render");
    assert_eq!(text_idx % cols, 4, "margin-left should shift the box right");
}

#[test]
fn margin_top_reserves_rows_above() {
    init();
    // Two paragraphs: each `margin-top` row must land above its own box.
    // First sits at row 3 (margin-top=3); Second sits at row 7
    // (= 3 + 1 own + 3 margin-top).
    let (term_map, tl) = draw("<p>First</p><p>Second</p>", "p { margin-top: 3px; }");
    let cols = term_map.columns as usize;
    let first_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "F")
        .expect("'First' should render");
    let second_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "S")
        .expect("'Second' should render");
    let first_row = first_idx / cols;
    let second_row = second_idx / cols;
    assert_eq!(first_row, 3, "First sits after its own margin-top=3");
    assert_eq!(
        second_row,
        first_row + 1 + 3,
        "Second should sit 1 own-row + 3 margin-top rows below First"
    );
}

#[test]
fn margin_shorthand_two_values_applies_to_top_bottom_and_left_right() {
    init();
    // `margin: 2px 8px` → top/bottom=2, left/right=8. Both X and Y carry
    // margin-top=2, so Y sits 1 own-row + 2 margin-top below X.
    let (term_map, tl) = draw("<p>X</p><p>Y</p>", "p { margin: 2px 8px; }");
    let cols = term_map.columns as usize;
    let x_idx = tl.layout.iter().position(|b| b.word == "X").unwrap();
    let y_idx = tl.layout.iter().position(|b| b.word == "Y").unwrap();
    assert_eq!(x_idx % cols, 8, "margin-left=8 shifts X right");
    assert_eq!(y_idx % cols, 8, "margin-left=8 shifts Y right");
    assert_eq!(
        y_idx / cols,
        x_idx / cols + 1 + 2 + 2,
        "Y sits 1 own-row + X's margin-bottom(2) + Y's margin-top(2) below X"
    );
}

// ============================================================
// border
// ============================================================

const BOX_H: &str = "─";
const BOX_V: &str = "│";
const BOX_TL: &str = "┌";
const BOX_TR: &str = "┐";
const BOX_BL: &str = "└";
const BOX_BR: &str = "┘";

#[test]
fn border_draws_four_edges_with_corners() {
    init();
    // A 5-col div with padding-top=1, border=1 on all sides, padding on
    // other axes = 0. Rect height = border-top(1) + padding-top(1) +
    // content(1) + padding-bottom(0) + border-bottom(1) = 4 rows.
    let (term_map, tl) = draw(
        "<div>X</div>",
        "div { width: 5px; padding-top: 1px; border: 1px solid red; background-color: yellow; }",
    );
    let cols = term_map.columns as usize;

    // Locate the div's top-left border corner at (0, 0).
    let top_left_idx = tl
        .layout
        .iter()
        .position(|b| b.word == BOX_TL)
        .expect("top-left border corner should render");
    assert_eq!(top_left_idx / cols, 0);
    assert_eq!(top_left_idx % cols, 0);

    // Top edge: horizontal bar across columns 1..4 of row 0.
    let row0 = &tl.layout[0..cols];
    let top_count = row0.iter().filter(|b| b.word == BOX_H).count();
    assert_eq!(
        top_count, 3,
        "top border should span 3 cells between the corners"
    );

    // Bottom edge: same width on row 3 (rect.height - 1).
    let row3 = &tl.layout[3 * cols..4 * cols];
    let bottom_count = row3.iter().filter(|b| b.word == BOX_H).count();
    assert_eq!(bottom_count, 3, "bottom border should span 3 cells");

    // Left + right edges on rows 1 and 2 (between top/bottom corners).
    for row_idx in [1usize, 2] {
        let row = &tl.layout[row_idx * cols..(row_idx + 1) * cols];
        let has_left = row[0].word == BOX_V;
        let has_right = row[4].word == BOX_V;
        assert!(has_left, "left border glyph should appear on row {row_idx}");
        assert!(
            has_right,
            "right border glyph should appear on row {row_idx}"
        );
    }
    // Make sure the four corners are present.
    assert!(
        tl.layout.iter().any(|b| b.word == BOX_TR),
        "top-right corner should render"
    );
    assert!(
        tl.layout.iter().any(|b| b.word == BOX_BL),
        "bottom-left corner should render"
    );
    assert!(
        tl.layout.iter().any(|b| b.word == BOX_BR),
        "bottom-right corner should render"
    );
}

#[test]
fn border_color_overrides_text_color() {
    init();
    // `border: 1px solid green;` paints the borders green even though the
    // text color (if any) is unset.
    let (_, tl) = draw(
        "<div></div>",
        "div { width: 5px; padding: 1px; border: 1px solid green; }",
    );
    let border_cells: Vec<_> = tl
        .layout
        .iter()
        .filter(|b| {
            b.word == BOX_H
                || b.word == BOX_V
                || matches!(b.word.as_str(), BOX_TL | BOX_TR | BOX_BL | BOX_BR)
        })
        .collect();
    assert!(!border_cells.is_empty(), "border glyphs should render");
    for b in &border_cells {
        assert_eq!(
            b.fcolor,
            Some(Color::Rgb {
                r: 0,
                g: 128,
                b: 0
            }),
            "border glyphs should pick up the border's color"
        );
    }
}

#[test]
fn border_per_side_longhand_overrides_universal() {
    init();
    // `border: 1px solid red; border-top: 2px solid blue;` — top edge blue,
    // others red. (Width > 1 still renders as 1 cell.)
    let (_, tl) = draw(
        "<div></div>",
        "div { width: 5px; padding: 1px; border: 1px solid red; border-top: 2px solid blue; }",
    );
    let cols = 80;
    let top_color = tl.layout[0].fcolor;
    let left_color = tl.layout[cols].fcolor;
    assert_eq!(
        top_color,
        Some(Color::Rgb {
            r: 0,
            g: 0,
            b: 255
        }),
        "top border should be blue"
    );
    assert_eq!(
        left_color,
        Some(Color::Rgb {
            r: 255,
            g: 0,
            b: 0
        }),
        "left border should be red"
    );
}

#[test]
fn border_style_none_hides_the_border() {
    init();
    let (_, tl) = draw(
        "<div></div>",
        "div { width: 5px; padding: 1px; border-style: none; border-color: red; }",
    );
    let has_border = tl.layout.iter().any(|b| {
        matches!(
            b.word.as_str(),
            BOX_H | BOX_V | BOX_TL | BOX_TR | BOX_BL | BOX_BR
        )
    });
    assert!(
        !has_border,
        "border-style: none should suppress all border glyphs"
    );
}

// ============================================================
// combined padding + border + margin
// ============================================================

#[test]
fn combined_padding_border_margin_size_correctly() {
    init();
    // padding=2 + border=1 on all sides; content sits in the inner box.
    let (term_map, tl) = draw(
        "<div>Hi</div>",
        "div { width: 10px; padding: 2px; border: 1px solid red; margin: 3px; background-color: yellow; }",
    );
    let cols = term_map.columns as usize;

    // 'H' should sit at row 3 (margin-top) + 1 (border-top) + 2 (padding-top)
    // = 6, and col 3 (margin-left) + 1 (border-left) + 2 (padding-left) = 6.
    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "H")
        .expect("'Hi' should render");
    assert_eq!(
        text_idx / cols,
        6,
        "row should account for margin+border+padding"
    );
    assert_eq!(
        text_idx % cols,
        6,
        "col should account for margin+border+padding"
    );

    // The border-box rect's total rows should fit margin+border+padding+content
    // + margin-bottom.
    let bg_rows = tl
        .layout
        .iter()
        .enumerate()
        .filter(|(_, b)| b.bcolor == Some(Color::Rgb { r: 255, g: 255, b: 0 }))
        .map(|(i, _)| i / cols)
        .collect::<std::collections::HashSet<_>>();
    // Expected rows: 3 (margin-top) + 1 (border-top) + 2 (padding-top)
    //               + 1 (content) + 2 (padding-bottom) + 1 (border-bottom)
    //             = 10 rows for the border-box.
    for row in 3..=9 {
        assert!(
            bg_rows.contains(&row),
            "row {row} should be yellow (part of border-box bg)"
        );
    }
}

#[test]
fn margin_top_creates_visible_gap_between_siblings() {
    init();
    // Two `<div>`s; the second should land `margin-top` rows below the first.
    let (term_map, tl) = draw(
        "<div>A</div><div>B</div>",
        "div { margin-top: 5px; background-color: yellow; }",
    );
    let cols = term_map.columns as usize;

    let a_idx = tl.layout.iter().position(|b| b.word == "A").unwrap();
    let a_row = a_idx / cols;
    let b_idx = tl.layout.iter().position(|b| b.word == "B").unwrap();
    let b_row = b_idx / cols;
    assert!(
        b_row >= a_row + 5,
        "B should sit at least margin-top=5 rows below A: a={a_row}, b={b_row}"
    );
}

// ============================================================
// box-sizing
// ============================================================

#[test]
fn box_sizing_content_box_adds_padding_and_border_to_width() {
    init();
    let yellow = Color::Rgb {
        r: 255,
        g: 255,
        b: 0,
    };
    let css = "div { width: 10px; padding: 2px; border: 1px solid red; background-color: yellow; }";

    // Default (border-box): `width` is the full border-box, so the bg
    // (padding-box) covers cols 1..9 (10 minus the 1-col borders). Count the
    // pure bg-fill cells (word == " ") on row 1 — the padding-top row that
    // carries no border glyphs.
    let (term_map, tl) = draw("<div>Hi</div>", css);
    let cols = term_map.columns as usize;
    let bg_count_row1 = tl.layout[cols..2 * cols]
        .iter()
        .filter(|b| b.bcolor == Some(yellow) && b.word == " ")
        .count();
    assert_eq!(bg_count_row1, 8, "border-box width=10 -> 8 bg cells");

    // `box-sizing: content-box`: the 10px width is the content box, so the
    // border-box grows to 10 + 4 (padding) + 2 (border) = 16 -> 14 bg cells.
    let (_, tl) = draw(
        "<div>Hi</div>",
        "div { width: 10px; padding: 2px; border: 1px solid red; background-color: yellow; box-sizing: content-box; }",
    );
    let bg_count_row1 = tl.layout[cols..2 * cols]
        .iter()
        .filter(|b| b.bcolor == Some(yellow) && b.word == " ")
        .count();
    assert_eq!(bg_count_row1, 14, "content-box width=10 -> 14 bg cells");
}

// ============================================================
// min-width / max-width
// ============================================================

#[test]
fn min_width_overrides_declared_width() {
    init();
    // `min-width` floors the border-box width: 10px declared, 30px minimum
    // -> the box spans 30 columns even though its content is narrow.
    let (term_map, tl) = draw(
        "<div>Hi</div>",
        "div { width: 10px; min-width: 30px; background-color: yellow; }",
    );
    let cols = term_map.columns as usize;
    let yellow = Color::Rgb {
        r: 255,
        g: 255,
        b: 0,
    };
    let max_bg_col = tl.layout[0..cols]
        .iter()
        .enumerate()
        .filter(|(_, b)| b.bcolor == Some(yellow))
        .map(|(i, _)| i)
        .max()
        .expect("bg should render");
    assert_eq!(max_bg_col, 29, "min-width=30 -> bg spans cols 0..30");
}

#[test]
fn max_width_caps_auto_and_declared_width() {
    init();
    let yellow = Color::Rgb {
        r: 255,
        g: 255,
        b: 0,
    };
    // A block normally fills the whole viewport (80 cols); `max-width` caps
    // it to 30 columns.
    let (term_map, tl) = draw(
        "<div>Hi</div>",
        "div { max-width: 30px; background-color: yellow; }",
    );
    let cols = term_map.columns as usize;
    let max_bg_col = tl.layout[0..cols]
        .iter()
        .enumerate()
        .filter(|(_, b)| b.bcolor == Some(yellow))
        .map(|(i, _)| i)
        .max()
        .expect("bg should render");
    assert_eq!(max_bg_col, 29, "max-width=30 -> auto width capped to 30");

    // `max-width` also caps an explicit `width`.
    let (_, tl) = draw(
        "<div>Hi</div>",
        "div { width: 60px; max-width: 30px; background-color: yellow; }",
    );
    let max_bg_col = tl.layout[0..cols]
        .iter()
        .enumerate()
        .filter(|(_, b)| b.bcolor == Some(yellow))
        .map(|(i, _)| i)
        .max()
        .expect("bg should render");
    assert_eq!(max_bg_col, 29, "width=60 with max-width=30 -> 30");
}

// ============================================================
// min-height
// ============================================================

#[test]
fn min_height_expands_short_box() {
    init();
    // An empty-ish block is 1 row tall by default; `min-height: 4` grows
    // the border-box to 4 rows.
    let (term_map, tl) = draw(
        "<div>X</div>",
        "div { min-height: 4px; background-color: yellow; }",
    );
    let cols = term_map.columns as usize;
    let yellow = Color::Rgb {
        r: 255,
        g: 255,
        b: 0,
    };
    let bg_rows: std::collections::HashSet<usize> = tl
        .layout
        .iter()
        .enumerate()
        .filter(|(_, b)| b.bcolor == Some(yellow))
        .map(|(i, _)| i / cols)
        .collect();
    for row in 0..4 {
        assert!(bg_rows.contains(&row), "row {row} should carry the bg");
    }
}

// ============================================================
// border-radius
// ============================================================

#[test]
fn border_radius_renders_rounded_corner_glyphs() {
    init();
    // `border-radius` swaps the square corner glyphs (┌ ┐ └ ┘) for the
    // rounded box-drawing set (╭ ╮ ╰ ╯). Rect: width 6, height 3 (content
    // row + top/bottom border).
    let (term_map, tl) = draw(
        "<div>X</div>",
        "div { border: 1px solid red; border-radius: 2px; width: 6px; }",
    );
    let cols = term_map.columns as usize;
    let word_at = |tl: &TermLayout, x: usize, y: usize| tl.layout[y * cols + x].word.clone();
    assert_eq!(word_at(&tl, 0, 0), "╭", "top-left corner");
    assert_eq!(word_at(&tl, 5, 0), "╮", "top-right corner");
    assert_eq!(word_at(&tl, 0, 2), "╰", "bottom-left corner");
    assert_eq!(word_at(&tl, 5, 2), "╯", "bottom-right corner");

    // Control: without `border-radius` the corners stay square.
    let (_, tl) = draw(
        "<div>X</div>",
        "div { border: 1px solid red; width: 6px; }",
    );
    assert_eq!(word_at(&tl, 0, 0), "┌", "square corner without radius");
}
