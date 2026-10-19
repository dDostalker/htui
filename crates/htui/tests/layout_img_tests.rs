//! Tests for the `<img>` layout pipeline.
//!
//! The bugs being guarded against:
//!   - `_create_img` previously interpreted `term_size` as pixel dimensions
//!     even though `paint_image_at` documents it as cell dimensions, so only
//!     `cols/2 × rows/4` cells were actually rasterized into the rect.
//!   - `layout_img` stored the unclamped cell width (`img_cols`) as
//!     `LayoutBox::cols`, while `rect.width` was clamped to the terminal
//!     width, so painting could overflow the rect into neighboring boxes.

use std::sync::Arc;

use crossterm::style::Color;
use htui::html::html_parse;
use htui::layout::inner_formatting_context::flow::layout_img;
use htui::layout::layout::TermMap;
use htui::layout::{
    BoxConstraint, BoxKind, Cursor, LayoutBox, LayoutContext, ReplacedContent, Viewport,
};
use htui::resources::{ResourceRegistry, with_registry};
use htui::tdom::node::{self, NodeType};
use htui::tui::graph_api::{
    ClipRect, IMAGE_PIXELS_PER_CELL_COL, IMAGE_PIXELS_PER_CELL_ROW, ImageCache, TermLayout,
    render_image_at, render_image_at_slice,
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
    let lb = layout_img(
        &arena,
        img_id,
        containing,
        &mut cursor,
        LayoutContext {
            font: htui::html::css_parse::FontContext::root(),
            viewport: Viewport { cols: 80, rows: 24 },
        },
    )
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
        BoxKind::Replaced(
            ReplacedContent::Input
            | ReplacedContent::Select
            | ReplacedContent::Textarea
            | ReplacedContent::Button,
        ) => {
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

    // A 120px-wide image now maps to 120 cells (1 source px per column in
    // half-block mode), so an 80-column container would clamp it. Use a
    // wide-enough containing block to exercise the unclamped path.
    let containing = BoxConstraint { x: 0, width: 200 };
    let mut cursor = Cursor::new();
    let lb = layout_img(
        &arena,
        img_id,
        containing,
        &mut cursor,
        LayoutContext {
            font: htui::html::css_parse::FontContext::root(),
            viewport: Viewport { cols: 80, rows: 24 },
        },
    )
    .unwrap();

    // 120 source pixels / IMAGE_PIXELS_PER_CELL_COL (1) = 120 cells (no clamp),
    // 60 source pixels / IMAGE_PIXELS_PER_CELL_ROW (2) = 30 cells. These are
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
fn image_sampling_constants_lock_in_1_by_2_ratio() {
    // Standardized source-pixel → terminal-cell sampling ratio. Each
    // terminal cell renders a 1-wide x 2-tall block of source pixels as one
    // half-block glyph (upper half = foreground color, lower half =
    // background color). Changing these values without a coordinated update
    // to `_create_img`, `layout_img`, and the doc would silently corrupt
    // image rendering.
    assert_eq!(IMAGE_PIXELS_PER_CELL_COL, 1);
    assert_eq!(IMAGE_PIXELS_PER_CELL_ROW, 2);
    assert_eq!(IMAGE_PIXELS_PER_CELL_COL * IMAGE_PIXELS_PER_CELL_ROW, 2);
}

// ============================================================
// Bug #1 — `_create_img` cell→pixel conversion
// ============================================================

/// Build a small 64×64 checkerboard PNG in memory and register it under a
/// unique `mem://` key, returning the key. Tests use these instead of disk
/// fixtures: the image pipeline resolves every source through the `mem://`
/// registry, so byte-fed tests exercise the exact same decode path.
fn register_test_png(reg: &ResourceRegistry, tag: &str) -> String {
    let key = format!("mem://img/{tag}_{}", std::process::id());
    let mut bytes: Vec<u8> = Vec::new();
    let img = ImageBuffer::<Rgb<u8>, _>::from_fn(64, 64, |x, y| {
        // Alternating black/white pattern — every (2×4) cell will produce
        // a non-trivial braille/quarter-block glyph.
        if (x / 4 + y / 8) % 2 == 0 {
            Rgb([255, 255, 255])
        } else {
            Rgb([0, 0, 0])
        }
    });
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .expect("encode test png");
    reg.register(&key, bytes).expect("register mem image");
    key
}

/// Build a 800×200 source PNG in memory and register it, returning the key.
/// Four times wider than tall, so a default (no attributes) `<img>` must NOT
/// lay out as a square.
fn register_wide_test_png(reg: &ResourceRegistry, tag: &str) -> String {
    let key = format!("mem://img/{tag}_wide_{}", std::process::id());
    let mut bytes: Vec<u8> = Vec::new();
    let img = ImageBuffer::<Rgb<u8>, _>::from_fn(800, 200, |_, _| Rgb([200, 100, 50]));
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .expect("encode wide test png");
    reg.register(&key, bytes).expect("register mem image");
    key
}

#[test]
fn paint_image_at_fills_requested_cell_area() {
    init();
    let reg = Arc::new(ResourceRegistry::new());
    with_registry(Arc::clone(&reg), || {
        let key = register_test_png(&reg, "paint_fill");

        let term_map = mock_term_map();
        let mut tl = TermLayout::new(&term_map, 1.0);

        // Request 40 cells wide × 10 cells tall. `_create_img` multiplies the
        // vertical axis by IMAGE_PIXELS_PER_CELL_ROW (2) internally and resizes
        // exactly, so the source 64×64 PNG is squashed to 40×20 pixels
        // and rasterized into 40×10 cells.
        let cell_w: u32 = 40;
        let cell_h: u32 = 10;
        render_image_at(&mut tl, &key, (0, 0), (cell_w, cell_h))
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

        // The image uses half-block glyphs (BMP > U+007F). A
        // blank canvas would only carry ASCII spaces.
        let has_unicode = tl
            .layout
            .iter()
            .any(|b| b.word.chars().any(|c| c as u32 > 0x7F));
        assert!(
            has_unicode,
            "canvas should contain braille/quarter-block glyphs"
        );
    });
}

// ============================================================
// Cell format: half-block glyph in `word`, colors in fcolor/bcolor
// ============================================================

#[test]
fn image_cells_store_half_block_glyph_and_colors_separately() {
    init();
    let reg = Arc::new(ResourceRegistry::new());
    with_registry(Arc::clone(&reg), || {
        let key = register_test_png(&reg, "cell_format");

        let term_map = mock_term_map();
        let mut tl = TermLayout::new(&term_map, 1.0);
        let cell_w: u32 = 40;
        let cell_h: u32 = 10;
        render_image_at(&mut tl, &key, (0, 0), (cell_w, cell_h))
            .expect("render_image_at should succeed");

        let cells_per_row = term_map.columns as usize;
        for row in 0..(cell_h as usize) {
            for col in 0..(cell_w as usize) {
                let cell = &tl.layout[row * cells_per_row + col];
                assert_eq!(
                    cell.word, "\u{2580}",
                    "cell word must be the upper-half-block glyph, got {:?}",
                    cell.word
                );
                assert!(
                    matches!(cell.fcolor, Some(Color::Rgb { .. })),
                    "top source-pixel color must live in fcolor, got {:?}",
                    cell.fcolor
                );
                assert!(
                    matches!(cell.bcolor, Some(Color::Rgb { .. })),
                    "bottom source-pixel color must live in bcolor, got {:?}",
                    cell.bcolor
                );
            }
        }
    });
}

// ============================================================
// ImageCache: one decode per (path, size)
// ============================================================

#[test]
fn image_cache_reuses_decoded_raster() {
    init();
    let reg = Arc::new(ResourceRegistry::new());
    with_registry(Arc::clone(&reg), || {
        let key = register_test_png(&reg, "cache_reuse");
        let mut cache = ImageCache::new();

        let first = cache
            .get_or_decode(&key, (40, 10))
            .expect("first decode should succeed");
        let (cols, rows, size) = (first.cloumns, first.rows, first.term_size);

        // The raster is cached: unregistering the source must not break a second
        // request, which would otherwise re-resolve the bytes and fail.
        assert!(reg.unregister(&key), "unregister mem image");

        let second = cache
            .get_or_decode(&key, (40, 10))
            .expect("cached entry must serve the second request without re-reading");
        assert_eq!(cols, second.cloumns);
        assert_eq!(rows, second.rows);
        assert_eq!(size, second.term_size);
    });
}

#[test]
fn image_cache_keys_by_size() {
    init();
    let reg = Arc::new(ResourceRegistry::new());
    with_registry(Arc::clone(&reg), || {
        let key = register_test_png(&reg, "cache_sizes");
        let mut cache = ImageCache::new();

        let (small_cols, small_rows) = {
            let small = cache
                .get_or_decode(&key, (20, 5))
                .expect("decode small size");
            (small.cloumns, small.rows)
        };
        let (large_cols, large_rows) = {
            let large = cache
                .get_or_decode(&key, (40, 10))
                .expect("decode large size");
            (large.cloumns, large.rows)
        };
        assert_eq!((small_cols, small_rows), (20, 5));
        assert_eq!((large_cols, large_rows), (40, 10));
    });
}

// ============================================================
// Intrinsic aspect ratio (default sizing without width/height attrs)
// ============================================================

#[test]
fn layout_img_default_size_preserves_intrinsic_aspect_ratio() {
    init();
    let reg = Arc::new(ResourceRegistry::new());
    with_registry(Arc::clone(&reg), || {
        let key = register_wide_test_png(&reg, "aspect");

        let mut p = html_parse::Parser::new(&format!(r#"<img src="{key}">"#));
        let (root_id, arena, _) = p.parse();
        let img_id = find_img_node(&arena, root_id);

        let containing = BoxConstraint { x: 0, width: 80 };
        let mut cursor = Cursor::new();
        let lb = layout_img(
            &arena,
            img_id,
            containing,
            &mut cursor,
            LayoutContext {
                font: htui::html::css_parse::FontContext::root(),
                viewport: Viewport { cols: 80, rows: 24 },
            },
        )
        .expect("<img> should produce a LayoutBox");

        // 800×200 source capped to an 80-col container: 80 cols wide, height
        // follows the 4:1 aspect -> 80 * 200 / 800 = 20 source px = 10 rows.
        assert_eq!(lb.rect.width, 80, "default width = containing width");
        assert_eq!(
            lb.rect.height, 10,
            "default height must preserve the source aspect ratio, not be a square"
        );
    });
}

// ============================================================
// Paint clipping: crop at native scale instead of squeezing
// ============================================================

#[test]
fn render_image_at_slice_crops_to_clip_region() {
    init();
    let reg = Arc::new(ResourceRegistry::new());
    with_registry(Arc::clone(&reg), || {
        // 64×64 source: the top 16 pixel rows are black, the rest white. At full
        // scale (64×32 cells, 2 source px per cell row) the top 8 cell rows are
        // all-black half-blocks.
        let key = format!("mem://img/clip_{}", std::process::id());
        let mut bytes: Vec<u8> = Vec::new();
        let img = ImageBuffer::<Rgb<u8>, _>::from_fn(64, 64, |_, y| {
            if y < 16 {
                Rgb([0, 0, 0])
            } else {
                Rgb([255, 255, 255])
            }
        });
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode clip png");
        reg.register(&key, bytes).expect("register clip image");

        let term_map = mock_term_map();
        // The canvas now starts at one viewport (24 rows) but this unit test
        // stamps a raw 64×32-cell grid directly through `render_image_at_slice`
        // (bypassing the paint pipeline's on-demand grow), so give it enough
        // rows to hold the full image.
        let term_map = TermMap {
            columns: term_map.columns,
            rows: 48,
        };
        let mut tl = TermLayout::new(&term_map, 1.0);
        let mut cache = ImageCache::new();
        let cells_per_row = term_map.columns as usize;

        // Decode at the FULL 64×32-cell grid but stamp only rows 0..8.
        render_image_at_slice(
            &mut cache,
            &mut tl.layout,
            term_map.columns,
            &key,
            (0, 0),
            (64, 32),
            Some(ClipRect {
                x: 0,
                y: 0,
                width: 64,
                height: 8,
            }),
        )
        .expect("render_image_at_slice should succeed");

        // Inside the clip: the top of the image at native scale — every cell is
        // a black half-block. (Squeezing the whole image into 8 rows would mix
        // in the white below the black band, so all-black rows prove the crop.)
        for row in 0..8usize {
            for col in 0..64usize {
                let cell = &tl.layout[row * cells_per_row + col];
                assert_eq!(cell.word, "\u{2580}", "clipped row must be painted");
                assert_eq!(
                    cell.fcolor,
                    Some(Color::Rgb { r: 0, g: 0, b: 0 }),
                    "clipped row must show the image's top band at native scale"
                );
            }
        }
        // Outside the clip: rows 8..32 must remain untouched blank cells.
        for row in 8..32usize {
            for col in 0..64usize {
                let cell = &tl.layout[row * cells_per_row + col];
                assert_eq!(cell.word, " ", "rows outside the clip must stay blank");
            }
        }
    });
}

// ============================================================
// SVG sources (rasterized through resvg)
// ============================================================

/// Register a small 200×100 SVG (solid orange fill) under a unique `mem://`
/// key and return the key. Byte-fed tests exercise the exact same decode path
/// as a page `<img src="mem://…svg">`.
fn register_test_svg(reg: &ResourceRegistry, tag: &str) -> String {
    let key = format!("mem://img/{tag}_svg_{}", std::process::id());
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
      <rect x="0" y="0" width="200" height="100" fill="#ff8800"/>
    </svg>"##;
    reg.register(&key, svg.as_bytes().to_vec())
        .expect("register mem svg");
    key
}

#[test]
fn svg_image_decodes_through_resvg() {
    init();
    let reg = Arc::new(ResourceRegistry::new());
    with_registry(Arc::clone(&reg), || {
        let key = register_test_svg(&reg, "decode");

        let term_map = mock_term_map();
        let mut tl = TermLayout::new(&term_map, 1.0);
        // 200×100 source rendered into a 40×20-cell area (80×40 source pixels
        // after the half-block vertical doubling).
        render_image_at(&mut tl, &key, (0, 0), (40, 20)).expect("svg should decode and render");

        // The solid fill survives the resize: every cell carries the half-block
        // glyph with the source color in both fcolor and bcolor.
        let cells_per_row = term_map.columns as usize;
        for row in 0..20usize {
            for col in 0..40usize {
                let cell = &tl.layout[row * cells_per_row + col];
                assert_eq!(
                    cell.word, "\u{2580}",
                    "svg cell at row={row} col={col} must hold the half-block glyph"
                );
                assert_eq!(
                    cell.fcolor,
                    Some(Color::Rgb {
                        r: 0xff,
                        g: 0x88,
                        b: 0x00
                    }),
                    "svg solid fill must land in fcolor"
                );
                assert_eq!(
                    cell.bcolor,
                    Some(Color::Rgb {
                        r: 0xff,
                        g: 0x88,
                        b: 0x00
                    }),
                    "svg solid fill must land in bcolor"
                );
            }
        }
    });
}

#[test]
fn svg_intrinsic_aspect_ratio_drives_default_layout() {
    init();
    let reg = Arc::new(ResourceRegistry::new());
    with_registry(Arc::clone(&reg), || {
        let key = register_test_svg(&reg, "aspect");

        let mut p = html_parse::Parser::new(&format!(r#"<img src="{key}">"#));
        let (root_id, arena, _) = p.parse();
        let img_id = find_img_node(&arena, root_id);

        let containing = BoxConstraint { x: 0, width: 80 };
        let mut cursor = Cursor::new();
        let lb = layout_img(
            &arena,
            img_id,
            containing,
            &mut cursor,
            LayoutContext {
                font: htui::html::css_parse::FontContext::root(),
                viewport: Viewport { cols: 80, rows: 24 },
            },
        )
        .expect("<img> should produce a LayoutBox");

        // 200×100 SVG capped to an 80-col container: 80 cols wide, height follows
        // the 2:1 aspect -> 80 * 100 / 200 = 40 source px = 20 cells.
        assert_eq!(lb.rect.width, 80, "default width = containing width");
        assert_eq!(
            lb.rect.height, 20,
            "default height must follow the SVG intrinsic aspect ratio"
        );
    });
}

// ============================================================
// <base href> — relative `src` resolution
// ============================================================

#[test]
fn layout_img_resolves_src_against_document_base() {
    // A relative `<img src>` must resolve against `<base href>` at
    // generation time, so the geometry/paint phases only ever see the
    // absolute `mem://` key.
    init();
    let reg = Arc::new(ResourceRegistry::new());
    with_registry(Arc::clone(&reg), || {
        let pid = std::process::id();
        let key = format!("mem://dir/pic_{pid}.png");
        reg.register(&key, vec![0u8; 16])
            .expect("register mem image");

        let html = format!(
            "<base href=\"mem://dir/\"><img src=\"pic_{pid}.png\" width=\"20\" height=\"10\">"
        );
        let (root_id, arena, _) = html_parse::Parser::new(&html).parse();
        let term_map = mock_term_map();
        let mut tl = TermLayout::new(&term_map, 1.0);
        let boxes = tl.draw(&arena, root_id, &term_map);

        // The img box sits under the implicit <html>/<body> wrappers, so search
        // the whole box tree.
        let img_box = find_img_box(&boxes).expect("img must produce a replaced layout box");
        match &img_box.kind {
            BoxKind::Replaced(ReplacedContent::Image { path, .. }) => {
                assert_eq!(
                    path.as_str(),
                    key.as_str(),
                    "relative img src must resolve against <base href>"
                );
            }
            _ => unreachable!(),
        }
    });
}

fn find_img_box(boxes: &[LayoutBox]) -> Option<&LayoutBox> {
    boxes.iter().find_map(|b| match &b.kind {
        BoxKind::Replaced(ReplacedContent::Image { .. }) => Some(b),
        _ => find_img_box(&b.children),
    })
}
