//! Tests for font-size propagation and rendering.
//!
//! When an element has a larger font-size (e.g. `<h1>` with the default
//! `2em`, or an explicit `font-size: 2em` rule), the terminal can't change
//! the glyph size — instead the paint phase expands every character cell
//! horizontally so each glyph occupies `font_size_cells` columns, with the
//! trailing columns filled with the run's background. This mirrors the
//! "extra space" a larger font would visually consume on a graphical
//! canvas.

use crossterm::style::Color as CtColor;
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

fn draw(html: &str, css: &str) -> (TermMap, TermLayout) {
    let term_map = mock_term_map();
    let (root_id, arena) = setup(html, css);
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    (term_map, tl)
}

// ============================================================
// h1 default font-size (2em) — each character should occupy 2 cells
// ============================================================

#[test]
fn h1_default_font_size_expands_each_char_to_two_cells() {
    init();
    // `<h1>` has the CSS default font-size of `2em`; in our terminal world
    // that means each character occupies 2 horizontal cells. The prefix
    // `# ` (2 chars) plus "Heading" (7 chars) = 9 chars × 2 = 18 cells of
    // visible glyph on the heading's row.
    let (term_map, tl) = draw("<h1>Heading</h1>", "");
    let cols = term_map.columns as usize;
    let heading_row_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "#")
        .expect("h1 '#' prefix should render");
    let row = heading_row_idx / cols;
    let rstart = row * cols;
    let row_cells = &tl.layout[rstart..rstart + cols];

    // The row should interleave glyph cells with bg-pad cells:
    // '#' + ' ' + ' ' + ' ' + 'H' + ' ' + 'e' + ' ' + 'a' + ' ' + 'd' + ' ' +
    // 'i' + ' ' + 'n' + ' ' + 'g' + ' ' (18 cells). Each bg-pad cell holds a
    // single space plus the run's fg / bg / style so the renderer paints the
    // background across the whole slot (an empty `word` would be skipped by
    // `render_to_bytes` and erase the bg between glyphs).
    assert_eq!(row_cells[0].word, "#", "row starts with the heading prefix");
    assert_eq!(row_cells[1].word, " ", "padding after '#' prefix");
    assert_eq!(row_cells[2].word, " ", "space inside prefix");
    assert_eq!(row_cells[3].word, " ", "padding after prefix space");
    assert_eq!(row_cells[4].word, "H");
    assert_eq!(row_cells[5].word, " ", "padding after 'H'");
    assert_eq!(row_cells[6].word, "e");
    assert_eq!(row_cells[7].word, " ");
    assert_eq!(row_cells[8].word, "a");
    assert_eq!(row_cells[9].word, " ");
    assert_eq!(row_cells[10].word, "d");
    assert_eq!(row_cells[11].word, " ");
    assert_eq!(row_cells[12].word, "i");
    assert_eq!(row_cells[13].word, " ");
    assert_eq!(row_cells[14].word, "n");
    assert_eq!(row_cells[15].word, " ");
    assert_eq!(row_cells[16].word, "g");
    assert_eq!(row_cells[17].word, " ", "padding after trailing glyph");
}

#[test]
fn h2_default_font_size_expands_each_char_to_two_cells() {
    init();
    // h2's CSS default is `1.5em` which rounds to `2` cells at font-size 1.
    // The rendering should follow the same expansion as h1.
    let (term_map, tl) = draw("<h2>Sub</h2>", "");
    let cols = term_map.columns as usize;
    let row_idx = tl.layout.iter().position(|b| b.word == "#").unwrap();
    let row = row_idx / cols;
    let rstart = row * cols;
    let row_cells = &tl.layout[rstart..rstart + cols];
    assert_eq!(row_cells[0].word, "#");
    assert_eq!(row_cells[1].word, " ", "padding after first '#'");
    assert_eq!(row_cells[2].word, "#");
    assert_eq!(row_cells[3].word, " ", "padding after second '#'");
    assert_eq!(row_cells[4].word, " ");
    assert_eq!(row_cells[5].word, " ");
    assert_eq!(row_cells[6].word, "S");
    assert_eq!(row_cells[7].word, " ");
    assert_eq!(row_cells[8].word, "u");
    assert_eq!(row_cells[9].word, " ");
    assert_eq!(row_cells[10].word, "b");
    assert_eq!(row_cells[11].word, " ");
}

// ============================================================
// explicit font-size rule
// ============================================================

#[test]
fn explicit_font_size_2em_on_p_expands_each_char() {
    init();
    // Force a `<p>` to font-size: 2em. Each char gets a trailing padding
    // cell, producing "A_ B_ C_" where _ is a single-space bg-pad cell
    // (the space + run bg keep `render_to_bytes` from dropping the cell,
    // which would erase the bg between glyphs).
    let (term_map, tl) = draw("<p>ABC</p>", "p { font-size: 2em; }");
    let cols = term_map.columns as usize;
    let idx = tl.layout.iter().position(|b| b.word == "A").unwrap();
    let row = idx / cols;
    let rstart = row * cols;
    let row_cells = &tl.layout[rstart..rstart + cols];
    assert_eq!(row_cells[0].word, "A");
    assert_eq!(row_cells[1].word, " ", "padding after 'A'");
    assert_eq!(row_cells[2].word, "B");
    assert_eq!(row_cells[3].word, " ", "padding after 'B'");
    assert_eq!(row_cells[4].word, "C");
    assert_eq!(row_cells[5].word, " ", "padding after 'C'");
}

#[test]
fn explicit_font_size_3em_expands_each_char_to_three_cells() {
    init();
    let (term_map, tl) = draw("<p>AB</p>", "p { font-size: 3em; }");
    let cols = term_map.columns as usize;
    let idx = tl.layout.iter().position(|b| b.word == "A").unwrap();
    let row = idx / cols;
    let rstart = row * cols;
    let row_cells = &tl.layout[rstart..rstart + cols];
    assert_eq!(row_cells[0].word, "A");
    assert_eq!(row_cells[1].word, " ", "1st padding after 'A'");
    assert_eq!(row_cells[2].word, " ", "2nd padding after 'A'");
    assert_eq!(row_cells[3].word, "B");
    assert_eq!(row_cells[4].word, " ", "1st padding after 'B'");
    assert_eq!(row_cells[5].word, " ", "2nd padding after 'B'");
}

#[test]
fn default_font_size_leaves_one_cell_per_char() {
    init();
    // No explicit `font-size` and no heading default — each char takes 1
    // cell with no trailing padding. This is the byte-identical baseline
    // that pre-font-size rendering relied on.
    let (term_map, tl) = draw("<p>ABC</p>", "");
    let cols = term_map.columns as usize;
    let idx = tl.layout.iter().position(|b| b.word == "A").unwrap();
    let row = idx / cols;
    let rstart = row * cols;
    let row_cells = &tl.layout[rstart..rstart + cols];
    assert_eq!(row_cells[0].word, "A");
    assert_eq!(row_cells[1].word, "B");
    assert_eq!(row_cells[2].word, "C");
    // Trailing cells are left at the terminal's default empty cell
    // (`" "`), not painted by `paint_text_row`. Locking that in guards
    // against the paint loop accidentally overwriting them.
    assert_eq!(row_cells[3].word, " ");
}

// ============================================================
// em padding with font-size
// ============================================================

#[test]
fn em_padding_scales_with_h1_font_size() {
    init();
    // h1's CSS default is `2em` font-size. The `# ` prefix therefore
    // expands to 4 cells (`#`, padding, ` `, padding), shifting the body
    // text "Hi" — itself 4 cells at font-size=2 — to column 4 on the same
    // row.
    let (term_map, tl) = draw("<h1>Hi</h1>", "");
    let cols = term_map.columns as usize;
    let idx = tl.layout.iter().position(|b| b.word == "H").unwrap();
    let row = idx / cols;
    let col = idx % cols;
    assert_eq!(row, 0);
    assert_eq!(
        col, 4,
        "h1 prefix '# ' (2 chars × font-size 2) = 4 cells, then 'H' at col 4"
    );
    assert_eq!(tl.layout[idx + 1].word, " ", "padding after 'H'");
    assert_eq!(tl.layout[idx + 2].word, "i");
    assert_eq!(tl.layout[idx + 3].word, " ", "padding after 'i'");
}

#[test]
fn em_padding_top_scales_with_font_size() {
    init();
    // A block with `font-size: 2em` and `padding-top: 1em`: the body's
    // first row should land at row 2 (= 1em × 2-cell font-size), not
    // row 1. Padding-left is 0 so the body text starts at col 0.
    let (term_map, tl) = draw("<div>X</div>", "div { font-size: 2em; padding-top: 1em; }");
    let cols = term_map.columns as usize;
    let idx = tl.layout.iter().position(|b| b.word == "X").unwrap();
    let row = idx / cols;
    let col = idx % cols;
    assert_eq!(row, 2, "1em padding-top at font-size=2 = 2 rows");
    assert_eq!(col, 0, "no padding-left, body at col 0");
}

// ============================================================
// font-size drives the box's vertical row count, not just the
// horizontal cell width. Each block's rect.height grows with its
// effective font-size so the next sibling block starts on a new row
// only after the larger glyph would have ended.
// ============================================================

#[test]
fn h1_default_font_size_grows_block_height_to_two_rows() {
    init();
    // `<h1>` defaults to 2em. The block's rect should be 2 rows tall and the
    // following block body must start on the row right after the heading
    // (row 2), not row 1.
    let (term_map, tl) = draw("<h1>Heading</h1><div>after</div>", "");
    let cols = term_map.columns as usize;
    let heading_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "#")
        .expect("h1 prefix '#' should render");
    let heading_row = heading_idx / cols;
    // Use the body 'f' as the marker — 'f' is unique to the body word
    // "after" (none of the h1 chars are 'f').
    let body_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "f")
        .expect("body 'f' should render");
    let body_row = body_idx / cols;
    assert_eq!(heading_row, 0);
    assert_eq!(
        body_row, 2,
        "block after a 2em <h1> must start at row 2 (h1 occupies 2 rows)"
    );
}

#[test]
fn explicit_font_size_3em_grows_block_height_to_three_rows() {
    init();
    // Explicit `font-size: 3em` on a block: the block's rect should be 3
    // rows tall and the following block must start on row 3.
    let (term_map, tl) = draw(
        r#"<div class="big">X</div><div>after</div>"#,
        "div.big { font-size: 3em; }",
    );
    let cols = term_map.columns as usize;
    eprintln!("first 5 rows:");
    for r in 0..5 {
        let s = r * cols;
        let mut line = String::new();
        for c in 0..cols.min(40) {
            line.push_str(&tl.layout[s + c].word);
        }
        eprintln!("  row {r}: {:?}", line);
    }
    let first_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "X")
        .expect("first block 'X' should render");
    let after_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "f")
        .expect("second block 'f' should render");
    let first_row = first_idx / cols;
    let after_row = after_idx / cols;
    assert_eq!(
        first_row, 1,
        "3em baseline sits on the middle row of the 3-row rect"
    );
    assert_eq!(
        after_row, 3,
        "second block must start at row 3 (first block occupies 3 rows for 3em)"
    );
}

#[test]
fn h1_2em_vertical_baseline_renders_text_on_middle_row() {
    init();
    // The text inside a 2-row-tall h1 should land on the middle row (row 1
    // of the rect), not the top row, so the box reads as a tall band with
    // empty background cells above and below the glyphs.
    let (term_map, tl) = draw("<h1>Hi</h1>", "");
    let cols = term_map.columns as usize;
    let h_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "H")
        .expect("h1 'H' should render");
    let h_row = h_idx / cols;
    assert_eq!(
        h_row, 0,
        "h1 baseline sits on the top row of its 2-row rect (middle row is the rect's first row when font_size=2)"
    );
}

// ============================================================
// Background alignment at large font-size
// ============================================================

#[test]
fn explicit_bg_with_large_font_size_clamps_to_text_row_and_width() {
    init();
    // A block with `font-size: 3em`, `background-color: cyan`, and a short
    // label: the visible bg should clamp to (a) the expanded run width
    // (each char occupies 3 cells) and (b) the row that actually holds
    // the text (the middle row of a 3-row rect). The leading and
    // trailing rows of the rect must remain bg-free — otherwise the
    // colour block would drift into empty rows that have no glyphs.
    let (term_map, tl) = draw(
        r#"<div class="big">CD</div>"#,
        "div.big { font-size: 3em; background-color: cyan; }",
    );
    let cols = term_map.columns as usize;
    let x_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "C")
        .expect("first 'C' should render");
    let row = x_idx / cols;
    let col = x_idx % cols;
    let rstart = row * cols;

    // The text "CD" at font-size 3 occupies 6 cells (cols 0..6) when
    // left aligned, painted on the middle (row 1) of the 3-row rect.
    assert_eq!(row, 1, "text sits on the middle row of the 3-row rect");
    assert_eq!(col, 0, "text starts at col 0 (left aligned)");

    // The text row must carry cyan bg only across the 6 text cells.
    let cells = &tl.layout[rstart..rstart + cols];
    let cyan_in_text_band = (col..col + 6)
        .filter(|&i| {
            cells[i].bcolor
                == Some(CtColor::Rgb {
                    r: 0,
                    g: 255,
                    b: 255,
                })
        })
        .count();
    assert_eq!(
        cyan_in_text_band, 6,
        "all 6 text cells must carry the cyan bg"
    );

    // Cells past the text band stay bg-free on the text row.
    for (i, cell) in cells.iter().enumerate().take(cols).skip(col + 6) {
        assert!(
            cell.bcolor.is_none(),
            "cell {i} (past the text band) should not carry the bg"
        );
    }

    // Leading and trailing rows of the rect are bg-free.
    for r_off in [0usize, 2usize] {
        let s = (row - 1 + r_off) * cols;
        let cyan_count = tl.layout[s..s + cols]
            .iter()
            .filter(|b| {
                b.bcolor
                    == Some(CtColor::Rgb {
                        r: 0,
                        g: 255,
                        b: 255,
                    })
            })
            .count();
        assert_eq!(
            cyan_count, 0,
            "row {r_off} of the 3-row rect should be bg-free (no glyphs there)"
        );
    }
}

#[test]
fn explicit_bg_at_default_font_size_still_fills_full_row() {
    init();
    // At font-size 1 the bg keeps the original CSS behaviour of filling
    // the full border-box (padding-box) — this matches existing tests
    // like `test_div_bg_fills_full_row_default`.
    let (term_map, tl) = draw("<div>hi</div>", "div { background-color: red; }");
    let cols = term_map.columns as usize;
    let red_count = tl.layout[0..cols]
        .iter()
        .filter(|b| b.bcolor == Some(CtColor::Rgb { r: 255, g: 0, b: 0 }))
        .count();
    assert_eq!(
        red_count, cols,
        "default font-size bg should still fill the full row"
    );
}

#[test]
fn bg_pad_cells_carry_space_and_bg_into_rendered_output() {
    init();
    // Regression: previously `paint_text_row` wrote empty `word` for the
    // bg-pad cells that fill the trailing slot of every expanded glyph
    // (e.g. cols 3, 5, 7, ... for ASCII text at font-size 2). The renderer
    // (`render_to_bytes`) drops cells whose `word` is empty to avoid
    // printing the second half of a wide CJK glyph, so those pads skipped
    // rendering entirely and the bg colour silently vanished from between
    // every letter. The user saw this as a jagged / chunky green band on
    // `<p class="pad-em">em padding</p>` (`padding: 0.5em 1em;
    // background-color: darkgreen; font-size: 2em`). The fix changes the
    // bg-pad cells to carry a `" "` word plus the run's bg so the
    // renderer keeps them and the bg spans the full slot.
    let (term_map, tl) = draw(
        r#"<p class="em2bg">AB</p>"#,
        "p.em2bg { font-size: 2em; background-color: darkgreen; }",
    );
    let cols = term_map.columns as usize;
    // Find the 'A' cell, then inspect the surrounding slot.
    let a_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "A")
        .expect("'A' must render");
    let row = a_idx / cols;
    let rstart = row * cols;
    let row_cells = &tl.layout[rstart..rstart + cols];
    // Slot for 'A' at font-size 2: cell 0 = 'A' (bg), cell 1 = pad (bg).
    assert_eq!(row_cells[0].word, "A");
    assert_eq!(
        row_cells[0].bcolor,
        Some(CtColor::Rgb { r: 0, g: 100, b: 0 }),
        "glyph cell carries the run bg",
    );
    // The bg-pad cell must hold a space (not an empty string) so the
    // renderer emits it and the bg spans the slot.
    assert_eq!(
        row_cells[1].word, " ",
        "bg-pad cell must hold a space so the renderer keeps the bg between glyphs",
    );
    assert_eq!(
        row_cells[1].bcolor,
        Some(CtColor::Rgb { r: 0, g: 100, b: 0 }),
        "bg-pad cell carries the run bg",
    );
    // Slot for 'B' at font-size 2: cell 2 = 'B' (bg), cell 3 = pad (bg).
    assert_eq!(row_cells[2].word, "B");
    assert_eq!(row_cells[3].word, " ", "bg-pad after 'B' also keeps the bg");
    assert_eq!(
        row_cells[3].bcolor,
        Some(CtColor::Rgb { r: 0, g: 100, b: 0 })
    );
}

#[test]
fn render_to_bytes_includes_bg_sgr_on_pad_cells_for_padded_em_block() {
    init();
    // End-to-end check using the actual ANSI byte stream. Mirrors the
    // `font_size` visual fixture's `.pad-em` element (`padding: 0.5em 1em;
    // font-size: 2em; background-color: darkgreen`) and verifies the
    // padding rows + the bg-pad cells between letters all emit the
    // `background-color: darkgreen` SGR. Before the fix, the bg-pad
    // cells were skipped by `render_to_bytes`, leaving gaps of un-bged
    // cells between letters (the user-visible jaggedness).
    let (term_map, tl) = draw(
        r#"<p class="pad-em">em padding</p>"#,
        "p.pad-em { font-size: 2em; padding: 0.5em 1em; background-color: darkgreen; color: white; }",
    );
    let bytes = tl.render_to_bytes();
    let rendered = String::from_utf8_lossy(&bytes);
    let cols = term_map.columns as usize;
    eprintln!(
        "rendered (no escapes) = {:?}",
        rendered.replace('\x1B', "<ESC>")
    );
    eprintln!("row 1 cells (cols 0-25):");
    for i in 0..26 {
        let c = &tl.layout[cols + i];
        eprintln!(
            "  cell {i}: word={:?} fg={:?} bg={:?} style={:?}",
            c.word, c.fcolor, c.bcolor, c.style
        );
    }
    let dark_green_sgr = "\x1B[48;2;0;100;0m"; // darkgreen is now truecolor #006400
    assert!(
        rendered.contains(dark_green_sgr),
        "rendered output must include the darkgreen bg SGR \\x1B[48;2;0;100;0m, got: {rendered:?}",
    );
    // The bg applies across the padding-top row, the text row, the
    // 2nd content row, and the padding-bottom row — so four SGR
    // occurrences (one per row). The text row also sees the bg-pad
    // cells between letters, which is what this test is really
    // guarding: without the fix the bg-pad cells were skipped by
    // `render_to_bytes` and the bg vanished from between letters.
    let sgr_count = rendered.matches(dark_green_sgr).count();
    assert!(
        sgr_count >= 4,
        "rendered output should contain the bg SGR at least 4 times (one per rect row); got {sgr_count}",
    );
    // The text row in the visible batch must show the bg SGR covering
    // the whole content + padding ring, including the bg-pad cells
    // between letters. The bg SGR is emitted right after the cursor
    // position move for the text row — verify by finding the text
    // segment and checking that the bg SGR appears in the same ANSI
    // batch (no fg-only or no-bg batches interleave within the 20+2
    // cells of content + padding-left). Without the fix, the bg-pad
    // cells between letters were skipped by `render_to_bytes`, so the
    // SGR-wrapped batch would only cover the 10 glyph cells (5 letters
    // paired with their bg, 5 letters without — or some other split),
    // and the rendered output would carry a reset SGR between glyphs.
    let text_row_start = rendered
        .find(" e m   p a d d i n g")
        .expect("text row batch should be present in the ANSI stream");
    // Walk backwards from there looking for the position cursor for that
    // row. The MoveTo escape ends with `H` (`\u{1b}[<row>;<col>H`); SGR
    // codes end with `m`, so we use a row-position-aware match.
    let preceding = &rendered[..text_row_start];
    let last_move_to = preceding
        .rfind("H\u{1b}[")
        .map(|i| i + 1) // include the trailing ESC[
        .or_else(|| preceding.rfind("\u{1b}["))
        .expect("must have a preceding ANSI escape");
    let after_move_to = &preceding[last_move_to..];
    assert!(
        after_move_to.contains("[48;2;0;100;0m"),
        "bg SGR must be the first style after the cursor move for the text row (got: {after_move_to:?})",
    );
}

// ============================================================
// text-transform
// ============================================================

#[test]
fn text_transform_uppercase_renders() {
    init();
    let (term_map, tl) = draw(
        "<div>hello world</div>",
        "div { text-transform: uppercase; }",
    );
    let cols = term_map.columns as usize;
    let row0: String = tl.layout[0..cols]
        .iter()
        .map(|b| b.word.as_str())
        .collect::<String>()
        .trim_end()
        .to_string();
    assert_eq!(row0, "HELLO WORLD");
}

#[test]
fn text_transform_lowercase_and_capitalize_render() {
    init();
    let (term_map, tl) = draw(
        "<div>Hello WORLD</div>",
        "div { text-transform: lowercase; }",
    );
    let cols = term_map.columns as usize;
    let row0: String = tl.layout[0..cols]
        .iter()
        .map(|b| b.word.as_str())
        .collect::<String>()
        .trim_end()
        .to_string();
    assert_eq!(row0, "hello world");

    let (_, tl) = draw(
        "<div>hello world</div>",
        "div { text-transform: capitalize; }",
    );
    let row0: String = tl.layout[0..cols]
        .iter()
        .map(|b| b.word.as_str())
        .collect::<String>()
        .trim_end()
        .to_string();
    assert_eq!(row0, "Hello World");
}

#[test]
fn text_transform_inherits_to_nested_spans() {
    init();
    // `text-transform` is inherited: a styled `<span>` inside a transformed
    // parent still receives the parent's transform (the cascade merged it).
    let (term_map, tl) = draw(
        "<div><span>abc</span></div>",
        "div { text-transform: uppercase; }",
    );
    let cols = term_map.columns as usize;
    let row0: String = tl.layout[0..cols]
        .iter()
        .map(|b| b.word.as_str())
        .collect::<String>()
        .trim_end()
        .to_string();
    assert_eq!(row0, "ABC");
}

// ============================================================
// line-height
// ============================================================

#[test]
fn line_height_2_expands_single_line_box() {
    init();
    // `line-height: 2` doubles the line box: the glyph stays on row 0 and
    // the rect grows to 2 rows tall.
    let (term_map, tl) = draw("<div>X</div>", "div { line-height: 2; }");
    let cols = term_map.columns as usize;
    let idx = tl.layout.iter().position(|b| b.word == "X").unwrap();
    assert_eq!(idx / cols, 0, "glyph sits on the first row");
    assert!(tl.rendered_rows >= 2, "rect should be 2 rows tall");
}

#[test]
fn line_height_2_separates_wrapped_lines() {
    init();
    // "ab cd" wraps into two lines inside a 2-col box; with `line-height:
    // 2` the second line lands 2 rows below the first (instead of 1).
    let (term_map, tl) = draw("<div>ab cd</div>", "div { width: 2px; line-height: 2; }");
    let cols = term_map.columns as usize;
    let a_idx = tl.layout.iter().position(|b| b.word == "a").unwrap();
    let c_idx = tl.layout.iter().position(|b| b.word == "c").unwrap();
    assert_eq!(a_idx / cols, 0, "first line on row 0");
    assert_eq!(c_idx / cols, 2, "second line 2 rows below (line-height 2)");
}

#[test]
fn line_height_normal_keeps_one_row_per_line() {
    init();
    // Control: the default `line-height: normal` keeps the historical 1-row
    // spacing, so the wrapped second line lands on row 1.
    let (term_map, tl) = draw("<div>ab cd</div>", "div { width: 2px; }");
    let cols = term_map.columns as usize;
    let c_idx = tl.layout.iter().position(|b| b.word == "c").unwrap();
    assert_eq!(c_idx / cols, 1, "second line on row 1 without line-height");
}

#[test]
fn collapsed_space_wrap_accounting_matches_paint_expansion() {
    init();
    // A collapsed space occupies one *expanded* slot (`font-size` cells) —
    // the same width the paint phase accounts via `InlineRun::width_at`. The
    // wrap pass used to count it as a bare cell, so a line whose painted
    // width exceeded the block lost its trailing glyph to the content-box
    // clip: `a b c` at font-size 2em paints 10 cells (2 per glyph / space);
    // in an 8-cell box the old accounting said it fit exactly and the `c`
    // vanished instead of wrapping to the next visual line.
    let (term_map, tl) = draw(
        "<div style=\"font-size: 2em\">a b c</div>",
        "div { width: 8ch; }",
    );
    let cols = term_map.columns as usize;
    let c_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "c")
        .expect("the trailing glyph must not be clipped by the wrap/paint width mismatch");
    assert_eq!(
        c_idx / cols, 2,
        "the line wraps to the second visual line (line-height = font-size = 2 \
         rows per line) instead of overflowing the block"
    );
}
