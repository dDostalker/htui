//! Tests for the `<img>` layout pipeline.
//!
//! The bugs being guarded against:
//!   - `_create_img` previously interpreted `term_size` as pixel dimensions
//!     even though `paint_image_at` documents it as cell dimensions, so only
//!     `cols/2 × rows/4` cells were actually rasterized into the rect.
//!   - `layout_img` stored the unclamped cell width (`img_cols`) as
//!     `LayoutBox::cols`, while `rect.width` was clamped to the terminal
//!     width, so painting could overflow the rect into neighboring boxes.

use std::path::PathBuf;

use htui::html::html_parse;
use htui::layout::layout::TermMap;
use htui::layout::inner_formatting_context::flow::layout_img;
use htui::layout::{BoxConstraint, BoxKind, Cursor, FontContext, ReplacedContent};
use htui::tdom::node::{self, NodeType};
use htui::tui::graph_api::{
    IMAGE_PIXELS_PER_CELL_COL, IMAGE_PIXELS_PER_CELL_ROW, TermLayout, render_image_at,
};
use image::{ImageBuffer, Rgb};

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

const COLS: u16 = 80;
const ROWS: u16 = 24;

fn mock_term_map() -> TermMap {
    TermMap {
        columns: COLS,
        rows: ROWS,
    }
}

fn find_img_node(arena: &node::NodeArena, root_id: node::NodeId) -> node::NodeId {
    for (id, n) in node::Node::descendants(arena, root_id) {
        if matches!(n.node_type, NodeType::Img) {
            return id;
        }
    }
    panic!("no <img> node found in parsed arena");
}

// ============================================================
// Bug #2 — LayoutBox cols / rows must match rect.width / height
// ============================================================

#[test]
fn layout_img_clamps_cols_to_containing_width() {
    init();
    let mut p = html_parse::Parser::new(r#"<img src="x.png" width="200" height="100">"#);
    let (root_id, arena, _) = p.parse();
    let img_id = find_img_node(&arena, root_id);

    let containing = BoxConstraint { x: 0, width: 80 };
    let mut cursor = Cursor::new();
    let lb = layout_img(&arena, img_id, containing, &mut cursor, FontContext::root(80, 24))
        .expect("<img> should produce a LayoutBox");

    match &lb.kind {
        BoxKind::Replaced(ReplacedContent::Image { cols, rows, .. }) => {
            assert_eq!(
                *cols, lb.rect.width,
                "img cols must equal rect.width so paint does not overflow (Bug #2 fix)"
            );
            assert_eq!(*rows, lb.rect.height, "img rows must equal rect.height");
            assert!(
                lb.rect.width <= containing.width,
                "rect.width must be clamped to containing.width, got {}",
                lb.rect.width
            );
        }
        BoxKind::Normal | BoxKind::Anonymous => {
            panic!("expected BoxKind::Replaced(Image), got {:?}", lb.kind)
        }
    }
}

#[test]
fn layout_img_unclamped_when_img_fits() {
    init();
    let mut p = html_parse::Parser::new(r#"<img src="x.png" width="120" height="60">"#);
    let (root_id, arena, _) = p.parse();
    let img_id = find_img_node(&arena, root_id);

    let containing = BoxConstraint { x: 0, width: 80 };
    let mut cursor = Cursor::new();
    let lb = layout_img(&arena, img_id, containing, &mut cursor, FontContext::root(80, 24)).unwrap();

    // 120 source pixels / IMAGE_PIXELS_PER_CELL_COL (2) = 60 cells (no clamp),
    // 60 source pixels / IMAGE_PIXELS_PER_CELL_ROW (4) = 15 cells. These are
    // SOURCE-IMAGE pixels, not CSS px — see `doc/layout.md` section 10.4.
    let expected_cols = 120u32.div_ceil(IMAGE_PIXELS_PER_CELL_COL) as usize;
    let expected_rows = 60u32.div_ceil(IMAGE_PIXELS_PER_CELL_ROW) as usize;
    match &lb.kind {
        BoxKind::Replaced(ReplacedContent::Image { cols, rows, .. }) => {
            assert_eq!(*cols, expected_cols);
            assert_eq!(*rows, expected_rows);
            assert_eq!(*cols, lb.rect.width);
            assert_eq!(*rows, lb.rect.height);
        }
        _ => panic!("expected BoxKind::Replaced(Image), got {:?}", lb.kind),
    }
}

#[test]
fn image_sampling_constants_lock_in_2_by_4_ratio() {
    // Standardized source-pixel → terminal-cell sampling ratio. Each
    // terminal cell renders a 2-wide x 4-tall block of source pixels as one
    // braille glyph (8 dot positions). Changing these values without a
    // coordinated update to `_create_img`, `layout_img`, and the doc would
    // silently corrupt image rendering.
    assert_eq!(IMAGE_PIXELS_PER_CELL_COL, 2);
    assert_eq!(IMAGE_PIXELS_PER_CELL_ROW, 4);
    assert_eq!(IMAGE_PIXELS_PER_CELL_COL * IMAGE_PIXELS_PER_CELL_ROW, 8);
}

// ============================================================
// Bug #1 — `_create_img` cell→pixel conversion
// ============================================================

/// Build a small in-memory PNG so the test does not depend on disk fixtures.
fn write_test_png(path: &PathBuf) {
    let img = ImageBuffer::<Rgb<u8>, _>::from_fn(64, 64, |x, y| {
        // Alternating black/white pattern — every (2×4) cell will produce
        // a non-trivial braille/quarter-block glyph.
        if (x / 4 + y / 8) % 2 == 0 {
            Rgb([255, 255, 255])
        } else {
            Rgb([0, 0, 0])
        }
    });
    img.save(path).unwrap();
}

#[test]
fn paint_image_at_fills_requested_cell_area() {
    init();

    let img_path = std::env::temp_dir().join("htui_layout_img_test.png");
    write_test_png(&img_path);

    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);

    // Request 40 cells wide × 10 cells tall. After the fix, `_create_img`
    // multiplies by (2, 4) internally and uses `resize_exact` so the source
    // 64×64 PNG is squashed to 80×40 pixels and rasterized into 40×10 cells.
    let cell_w: u32 = 40;
    let cell_h: u32 = 10;
    render_image_at(
        &mut tl,
        img_path.to_str().unwrap(),
        (0, 0),
        (cell_w, cell_h),
    )
    .expect("paint_image_at should succeed");

    // Every cell the image requested must have been written — none should
    // still hold the default space. Before the fix only `cell_w/2 × cell_h/4`
    // cells got filled, leaving most of the rect blank.
    let cells_per_row = term_map.columns as usize;
    for row in 0..(cell_h as usize) {
        for col in 0..(cell_w as usize) {
            let idx = row * cells_per_row + col;
            let word = &tl.layout[idx].word;
            assert_ne!(
                word, " ",
                "image cell at row={row} col={col} still holds default space (idx={idx})"
            );
        }
    }

    // The image uses half-block / quarter-block glyphs (BMP > U+007F). A
    // blank canvas would only carry ASCII spaces.
    let has_unicode = tl
        .layout
        .iter()
        .any(|b| b.word.chars().any(|c| c as u32 > 0x7F));
    assert!(
        has_unicode,
        "canvas should contain braille/quarter-block glyphs"
    );
}
