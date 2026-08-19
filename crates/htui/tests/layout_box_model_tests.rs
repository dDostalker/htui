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
    tl.draw(&arena, root_id, &term_map);
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
        "div { padding: 2ch 3ch; background-color: yellow; }",
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
    let (_term_map, tl) = draw("<div></div>", "div { padding: 4ch 0 2ch 0; }");

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
        "div { padding: 3ch; background-color: cyan; }",
    );
    let cols = term_map.columns as usize;

    // Row 0 is a padding row — must carry cyan bg.
    let row0 = &tl.layout[0..cols];
    let cyan_count = row0
        .iter()
        .filter(|b| {
            b.bcolor
                == Some(Color::Rgb {
                    r: 0,
                    g: 255,
                    b: 255,
                })
        })
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
    // content area (after padding). padding-top=2 plus the <p>'s 1-row UA
    // margin-top puts the text at row 3.
    let (term_map, tl) = draw(
        "<div><p>Inner</p></div>",
        "div { padding-top: 2ch; padding-left: 5ch; }",
    );
    let cols = term_map.columns as usize;
    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "I")
        .expect("'Inner' should render");
    assert_eq!(text_idx / cols, 3, "p starts after padding-top=2 + its UA margin-top=1");
    assert_eq!(text_idx % cols, 5, "p must start after padding-left=5");
}

// ============================================================
// margin
// ============================================================

#[test]
fn margin_left_shifts_box_horizontally() {
    init();
    // margin-left=4 should push the box 4 cols to the right.
    let (term_map, tl) = draw("<div>X</div>", "div { margin-left: 4ch; }");
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
    let (term_map, tl) = draw("<p>First</p><p>Second</p>", "p { margin-top: 3ch; }");
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
    // `margin: 2ch 8ch` → top/bottom=2, left/right=8. Both X and Y carry
    // margin-top=2, so Y sits 1 own-row + the collapsed 2-row margin below
    // X (adjacent vertical margins merge to the larger — CSS collapsing).
    let (term_map, tl) = draw("<p>X</p><p>Y</p>", "p { margin: 2ch 8ch; }");
    let cols = term_map.columns as usize;
    let x_idx = tl.layout.iter().position(|b| b.word == "X").unwrap();
    let y_idx = tl.layout.iter().position(|b| b.word == "Y").unwrap();
    assert_eq!(x_idx % cols, 8, "margin-left=8 shifts X right");
    assert_eq!(y_idx % cols, 8, "margin-left=8 shifts Y right");
    assert_eq!(
        y_idx / cols,
        x_idx / cols + 1 + 2,
        "Y sits 1 own-row + the collapsed 2-row margin below X"
    );
}

#[test]
fn margin_auto_centers_block_horizontally() {
    init();
    // `margin: 0 auto` + explicit width centers the box: both auto margins
    // share the free space. 80-col canvas, width 20 → left = (80-20)/2 = 30.
    let (term_map, tl) = draw(
        "<div>X</div>",
        "div { width: 20ch; margin: 0 auto; background-color: yellow; }",
    );
    let cols = term_map.columns as usize;
    let text_idx = tl.layout.iter().position(|b| b.word == "X").unwrap();
    assert_eq!(
        text_idx % cols,
        30,
        "both auto margins split the free space: (80-20)/2 = 30"
    );
}

#[test]
fn margin_right_auto_pushes_box_to_the_left() {
    init();
    // `margin-right: auto` absorbs the whole remainder: the box stays flush
    // left with the free space on its right.
    let (term_map, tl) = draw("<div>X</div>", "div { width: 20ch; margin-right: auto; }");
    let cols = term_map.columns as usize;
    let text_idx = tl.layout.iter().position(|b| b.word == "X").unwrap();
    assert_eq!(text_idx % cols, 0, "left margin stays 0, auto right absorbs");
}

#[test]
fn margin_left_auto_pushes_box_to_the_right() {
    init();
    // `margin-left: auto` absorbs the whole remainder: the box sits flush
    // against the right edge of the containing block.
    let (term_map, tl) = draw(
        "<div>X</div>",
        "div { width: 20ch; margin-left: auto; background-color: yellow; }",
    );
    let cols = term_map.columns as usize;
    let text_idx = tl.layout.iter().position(|b| b.word == "X").unwrap();
    assert_eq!(
        text_idx % cols,
        60,
        "auto left margin = 80 - 20 = 60"
    );
}

#[test]
fn border_side_component_longhand_renders() {
    init();
    // `border-left-width: 3ch; border-left-style: solid; border-left-color:
    // green;` — the longhands alone must paint a left border edge. The
    // padding-top gives the box a second row so the edge isn't overwritten
    // by the corner glyph on row 0.
    let (_, tl) = draw(
        "<div></div>",
        "div { width: 5ch; padding-top: 1ch; border-left-width: 3ch; border-left-style: solid; border-left-color: green; }",
    );
    // The left edge glyph renders as a vertical bar somewhere in the box.
    let edge = tl
        .layout
        .iter()
        .find(|b| b.word == BOX_V)
        .expect("left edge vertical bar")
        .clone();
    assert_eq!(edge.word, BOX_V, "left edge renders a vertical bar");
    assert_eq!(
        edge.fcolor,
        Some(Color::Rgb {
            r: 0,
            g: 128,
            b: 0
        }),
        "longhand color applies"
    );
}

#[test]
fn border_component_longhand_overrides_side_shorthand_in_layout() {
    init();
    // `border: 1ch solid red; border-left-color: blue;` — the component
    // longhand overrides just the left color, others stay red.
    let (_, tl) = draw(
        "<div></div>",
        "div { width: 5ch; border: 1ch solid red; border-left-color: blue; }",
    );
    let left_edge = tl
        .layout
        .iter()
        .find(|b| b.word == BOX_V)
        .expect("left edge")
        .clone();
    let top_edge = tl
        .layout
        .iter()
        .find(|b| b.word == BOX_H)
        .expect("top edge")
        .clone();
    assert_eq!(left_edge.word, BOX_V);
    assert_eq!(
        left_edge.fcolor,
        Some(Color::Rgb {
            r: 0,
            g: 0,
            b: 255
        }),
        "component longhand wins"
    );
    assert_eq!(
        top_edge.fcolor,
        Some(Color::Rgb {
            r: 255,
            g: 0,
            b: 0
        }),
        "other sides keep shorthand"
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
        "div { width: 5ch; padding-top: 1ch; border: 1ch solid red; background-color: yellow; }",
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
    // `border: 1ch solid green;` paints the borders green even though the
    // text color (if any) is unset.
    let (_, tl) = draw(
        "<div></div>",
        "div { width: 5ch; padding: 1ch; border: 1ch solid green; }",
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
            Some(Color::Rgb { r: 0, g: 128, b: 0 }),
            "border glyphs should pick up the border's color"
        );
    }
}

#[test]
fn border_per_side_longhand_overrides_universal() {
    init();
    // `border: 1ch solid red; border-top: 2ch solid blue;` — top edge blue,
    // others red. (Width > 1 still renders as 1 cell.)
    let (_, tl) = draw(
        "<div></div>",
        "div { width: 5ch; padding: 1ch; border: 1ch solid red; border-top: 2ch solid blue; }",
    );
    let cols = 80;
    let top_color = tl.layout[0].fcolor;
    let left_color = tl.layout[cols].fcolor;
    assert_eq!(
        top_color,
        Some(Color::Rgb { r: 0, g: 0, b: 255 }),
        "top border should be blue"
    );
    assert_eq!(
        left_color,
        Some(Color::Rgb { r: 255, g: 0, b: 0 }),
        "left border should be red"
    );
}

#[test]
fn border_style_none_hides_the_border() {
    init();
    let (_, tl) = draw(
        "<div></div>",
        "div { width: 5ch; padding: 1ch; border-style: none; border-color: red; }",
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

#[test]
fn border_wins_over_overflowing_content() {
    init();
    // `max-height` caps the box smaller than its text: the wrapped content
    // would land on the bottom border row. The frame must win the contended
    // cells — the border glyphs stay and the overflow is clipped instead of
    // painting over the border (the "出框" symptom in general form).
    let (term_map, tl) = draw(
        "<div>ABC</div>",
        "div { width: 5ch; border: 1ch solid red; max-height: 2ch; }",
    );
    let cols = term_map.columns as usize;
    let rows: Vec<String> = tl
        .layout
        .chunks(cols)
        .map(|chunk| chunk.iter().map(|b| b.word.as_str()).collect())
        .collect();
    // The 2-row box paints both borders on rows 0 (top) and 1 (bottom); the
    // squeezed "ABC" text must not appear on the border row.
    assert!(
        !rows[1].contains("ABC"),
        "overflowing content must not overwrite the border: {:?}",
        rows[1]
    );
    // The bottom border row keeps a continuous frame (corners + dashes).
    let bottom = &rows[1];
    assert!(
        bottom.contains(BOX_BL) && bottom.contains(BOX_BR) && bottom.contains(BOX_H),
        "bottom border must stay intact: {bottom:?}"
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
        "div { width: 10ch; padding: 2ch; border: 1ch solid red; margin: 3ch; background-color: yellow; }",
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
        .filter(|(_, b)| {
            b.bcolor
                == Some(Color::Rgb {
                    r: 255,
                    g: 255,
                    b: 0,
                })
        })
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
        "div { margin-top: 5ch; background-color: yellow; }",
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
    let css = "div { width: 10ch; padding: 2ch; border: 1ch solid red; background-color: yellow; }";

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

    // `box-sizing: content-box`: the 10ch width is the content box, so the
    // border-box grows to 10 + 4 (padding) + 2 (border) = 16 -> 14 bg cells.
    let (_, tl) = draw(
        "<div>Hi</div>",
        "div { width: 10ch; padding: 2ch; border: 1ch solid red; background-color: yellow; box-sizing: content-box; }",
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
    // `min-width` floors the border-box width: 10ch declared, 30ch minimum
    // -> the box spans 30 columns even though its content is narrow.
    let (term_map, tl) = draw(
        "<div>Hi</div>",
        "div { width: 10ch; min-width: 30ch; background-color: yellow; }",
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
        "div { max-width: 30ch; background-color: yellow; }",
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
        "div { width: 60ch; max-width: 30ch; background-color: yellow; }",
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
        "div { min-height: 4ch; background-color: yellow; }",
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
        "div { border: 1ch solid red; border-radius: 2ch; width: 6ch; }",
    );
    let cols = term_map.columns as usize;
    let word_at = |tl: &TermLayout, x: usize, y: usize| tl.layout[y * cols + x].word.clone();
    assert_eq!(word_at(&tl, 0, 0), "╭", "top-left corner");
    assert_eq!(word_at(&tl, 5, 0), "╮", "top-right corner");
    assert_eq!(word_at(&tl, 0, 2), "╰", "bottom-left corner");
    assert_eq!(word_at(&tl, 5, 2), "╯", "bottom-right corner");

    // Control: without `border-radius` the corners stay square.
    let (_, tl) = draw("<div>X</div>", "div { border: 1ch solid red; width: 6ch; }");
    assert_eq!(word_at(&tl, 0, 0), "┌", "square corner without radius");
}

// ============================================================
// CSS `height` — fixed-height boxes clip their text
// ============================================================

#[test]
fn css_height_fixes_the_frame_and_clips_text() {
    init();
    // `height` fixes the border-box height (htui's default box-sizing is
    // `border-box`): a 3-row card with six lines of content keeps its frame
    // and clips the overflow instead of growing — the box frame wins over
    // the text when the two conflict on a small terminal.
    let (term_map, tl) = draw(
        r#"<div class="card">line one<br>line two<br>line three<br>line four<br>line five<br>line six</div>"#,
        "body { margin: 0ch; } .card { height: 3ch; border: 1ch solid red; }",
    );
    let cols = term_map.columns as usize;
    let row_text = |y: usize| {
        tl.layout[y * cols..(y + 1) * cols]
            .iter()
            .map(|b| b.word.as_str())
            .collect::<String>()
    };
    let top = row_text(0);
    assert_eq!(top.chars().next(), Some('┌'), "top-left corner");
    assert_eq!(top.chars().last(), Some('┐'), "top-right corner");
    let content = row_text(1);
    assert!(
        content.contains("line one"),
        "the first line fits inside the fixed height"
    );
    for clipped in ["line two", "line three", "line six"] {
        assert!(
            !content.contains(clipped),
            "{clipped} must be clipped by the fixed height"
        );
    }
    let bottom = row_text(2);
    assert_eq!(bottom.chars().next(), Some('└'), "bottom-left corner");
    assert_eq!(bottom.chars().last(), Some('┘'), "bottom-right corner");
    // Rows beyond the fixed-height frame stay blank (no spilled text).
    assert!(
        tl.layout[3 * cols..]
            .iter()
            .all(|b| b.word.trim().is_empty()),
        "nothing may paint below the fixed-height frame"
    );
}
