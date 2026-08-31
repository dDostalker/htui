//! Geometry for replaced elements: `<input>` / `<textarea>` / `<button>` /
//! `<select>` / `<img>`, driven by the payload attached at generation time.

use crate::{
    graphics::{
        IMAGE_PIXELS_PER_CELL_COL, IMAGE_PIXELS_PER_CELL_ROW, char_width, image_intrinsic_size,
    },
    layout::{
        BoxConstraint, Cursor, LayoutBox, LayoutContext, PaintStyle, Rect, ReplacedContent,
        layoutbox::generation::{GeneratedBox, GeneratedPayload},
    },
    tdom::node::NodeArena,
};

use super::select_overlay::select_options;

/// Geometry for replaced elements driven solely by the payload attached at
/// generation time. Tag-specific data (the resource path and intrinsic
/// `width` / `height` attributes) is read from the DOM only when the
/// payload indicates an image; the dispatch never inspects `NodeType` to
/// decide which geometry algorithm to run.
pub(super) fn layout_replaced(
    arena: &NodeArena,
    generated: &GeneratedBox,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    match &generated.payload {
        GeneratedPayload::Input => {
            layout_input(arena, generated, containing, cursor, context, pre_allocated_rect)
        }
        GeneratedPayload::Textarea => {
            layout_textarea(arena, generated, containing, cursor, context, pre_allocated_rect)
        }
        GeneratedPayload::Button => {
            layout_button(arena, generated, containing, cursor, context, pre_allocated_rect)
        }
        GeneratedPayload::Select => {
            layout_select(arena, generated, containing, cursor, context, pre_allocated_rect)
        }
        GeneratedPayload::Image { src } if !src.is_empty() => layout_image(
            arena,
            generated,
            containing,
            cursor,
            pre_allocated_rect,
            src,
        ),
        _ => None,
    }
}

/// Geometry for `<input>`: a single-line field sized from the HTML `width`
/// attribute (cells) or a fixed default width when absent.
///
/// The width is deliberately *independent of the value* (mirroring a browser
/// `size` attribute): typing never changes the layout, so the edit path only
/// repaints the field's rectangle instead of re-running layout.
fn layout_input(
    arena: &NodeArena,
    generated: &GeneratedBox,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    let style = replaced_chrome(arena, generated, containing.width, context);
    let (x, y, block_width, height) = match pre_allocated_rect {
        Some(rect) => (rect.x, rect.y, rect.width.max(1), rect.height.max(1)),
        None => {
            let node = &arena[generated.node_id];
            // `<input type=checkbox|radio>` is a two-cell bare terminal toggle
            // (glyph + one spacing column so it never glues onto adjacent
            // text); a button input (`button` / `submit` / `reset` / `image`)
            // sizes itself from its label like `<button>` (label + padding);
            // every other kind is a text field sized from the HTML `width`
            // attribute (cells) or a fixed default width when absent. A CSS
            // `border:` opt-in frame adds its thickness on both axes — except
            // for toggles, which paint no chrome of their own so they reserve
            // no border cells.
            let is_toggle = node
                .get_type()
                .is_some_and(|t| matches!(t.to_ascii_lowercase().as_str(), "checkbox" | "radio"));
            let (border_h, border_v) = if is_toggle {
                (0, 0)
            } else {
                (style.1, style.2)
            };
            let content_w = if is_toggle {
                2
            } else if let Some(label) = node.input_button_label() {
                let text_w: usize = label.chars().map(|c| char_width(c).max(1)).sum();
                (text_w.saturating_add(4)).min(containing.width).max(4)
            } else {
                match node.get_width() {
                    Some(w) => (w as usize).min(containing.width).max(1),
                    None => 8usize.min(containing.width.max(1)),
                }
            };
            let block_width = content_w.saturating_add(border_h);
            let x = containing.x;
            let y = cursor.allocate(1usize.saturating_add(border_v));
            (x, y, block_width, 1usize.saturating_add(border_v))
        }
    };
    Some(LayoutBox::new_replaced(
        generated.node_id,
        Rect::new(x, y, block_width, height),
        ReplacedContent::Input,
        style.0,
    ))
}

/// Resolve a replaced control's chrome in a single pass: the [`PaintStyle`]
/// (CSS border edges over the default replaced-element style) and the border
/// thickness on both axes. Layout reserves the border cells up front so a
/// declared `border:` renders as a real frame around the content; the control
/// painters then draw it from the same style.
fn replaced_chrome(
    arena: &NodeArena,
    generated: &GeneratedBox,
    containing_width: usize,
    context: LayoutContext,
) -> (PaintStyle, usize, usize) {
    let node = &arena[generated.node_id];
    let edges = node
        .css_attr
        .get_box_edges(containing_width, context.font, context.viewport);
    (
        PaintStyle {
            box_edges: edges,
            ..PaintStyle::default()
        },
        edges.border.horizontal_thickness(),
        edges.border.vertical_thickness(),
    )
}

/// Geometry for `<textarea>`: a fixed multi-line editable field.
///
/// Width is `cols` (browser default 20) and height is `rows` (browser
/// default 2), plus any CSS `border` thickness. The size is independent of
/// the value, so editing only repaints the field's rectangle — same as
/// `<input>`.
fn layout_textarea(
    arena: &NodeArena,
    generated: &GeneratedBox,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    let style = replaced_chrome(arena, generated, containing.width, context);
    let (x, y, block_width, height) = match pre_allocated_rect {
        Some(rect) => (rect.x, rect.y, rect.width.max(1), rect.height.max(1)),
        None => {
            let node = &arena[generated.node_id];
            let cols: usize = node
                .attributes
                .get("cols")
                .and_then(|v| v.parse().ok())
                .unwrap_or(20);
            let rows: usize = node
                .attributes
                .get("rows")
                .and_then(|v| v.parse().ok())
                .unwrap_or(2);
            let block_width = cols
                .max(1)
                .saturating_add(style.1)
                .min(containing.width)
                .max(1);
            let content_rows = rows.max(1);
            let x = containing.x;
            let y = cursor.allocate(content_rows.saturating_add(style.2));
            (x, y, block_width, content_rows.saturating_add(style.2))
        }
    };
    Some(LayoutBox::new_replaced(
        generated.node_id,
        Rect::new(x, y, block_width, height),
        ReplacedContent::Textarea,
        style.0,
    ))
}

/// Geometry for `<button>`: a single-row push-button sized from its text
/// content (label + padding). A `disabled` button still takes the same space
/// (matching browsers).
fn layout_button(
    arena: &NodeArena,
    generated: &GeneratedBox,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    let style = replaced_chrome(arena, generated, containing.width, context);
    let (x, y, block_width, height) = match pre_allocated_rect {
        Some(rect) => (rect.x, rect.y, rect.width.max(1), rect.height.max(1)),
        None => {
            let node = &arena[generated.node_id];
            let text_w: usize = node
                .text_content(arena)
                .chars()
                .map(|c| char_width(c).max(1))
                .sum();
            let block_width = (text_w.saturating_add(4).saturating_add(style.1))
                .min(containing.width)
                .max(4);
            let x = containing.x;
            let y = cursor.allocate(1usize.saturating_add(style.2));
            (x, y, block_width, 1usize.saturating_add(style.2))
        }
    };
    Some(LayoutBox::new_replaced(
        generated.node_id,
        Rect::new(x, y, block_width, height),
        ReplacedContent::Button,
        style.0,
    ))
}

/// Geometry for `<select>`: a single-line dropdown field.
///
/// Width is the longest option label plus the chevron / padding, or the HTML
/// `width` attribute when set. The dropdown overlay is painted by the render
/// thread, so the box itself is always one row.
fn layout_select(
    arena: &NodeArena,
    generated: &GeneratedBox,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    let style = replaced_chrome(arena, generated, containing.width, context);
    let (x, y, block_width, height) = match pre_allocated_rect {
        Some(rect) => (rect.x, rect.y, rect.width.max(1), rect.height.max(1)),
        None => {
            let node = &arena[generated.node_id];
            let text_w: usize = select_options(arena, generated.node_id)
                .iter()
                .map(|opt| opt.label.chars().map(char_width).sum::<usize>())
                .max()
                .unwrap_or(0);
            let content_w = match node.get_width() {
                Some(w) => (w as usize).min(containing.width).max(1),
                None => (text_w.saturating_add(4)).min(containing.width).max(4),
            };
            let block_width = content_w.saturating_add(style.1);
            let x = containing.x;
            let y = cursor.allocate(1usize.saturating_add(style.2));
            (x, y, block_width, 1usize.saturating_add(style.2))
        }
    };
    Some(LayoutBox::new_replaced(
        generated.node_id,
        Rect::new(x, y, block_width, height),
        ReplacedContent::Select,
        style.0,
    ))
}

/// Geometry for `<img>`, driven solely by the payload attached at generation
/// time. Tag-specific data (the resource path and intrinsic `width` /
/// `height` attributes) is read from the DOM only when the payload indicates
/// an image; the dispatch never inspects `NodeType` to decide which geometry
/// algorithm to run.
fn layout_image(
    arena: &NodeArena,
    generated: &GeneratedBox,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    pre_allocated_rect: Option<Rect>,
    src: &str,
) -> Option<LayoutBox> {
    let src = src.to_string();
    let (x, y, block_width, img_rows) = match pre_allocated_rect {
        Some(rect) => {
            let width = rect.width.max(1);
            let height = rect.height.max(1);
            (rect.x, rect.y, width, height)
        }
        None => {
            let node = &arena[generated.node_id];
            let (img_cols, img_rows) = resolve_image_cell_size(
                node.get_width(),
                node.get_height(),
                image_intrinsic_size(&src),
                containing.width,
            );
            let block_width = img_cols.min(containing.width).max(1);
            let x = centered_left(containing, block_width);
            let y = cursor.allocate(img_rows);
            (x, y, block_width, img_rows)
        }
    };

    Some(LayoutBox::new_replaced(
        generated.node_id,
        Rect::new(x, y, block_width, img_rows),
        ReplacedContent::Image {
            path: src,
            cols: block_width,
            rows: img_rows,
        },
        PaintStyle::default(),
    ))
}

fn centered_left(containing: BoxConstraint, block_width: usize) -> usize {
    containing.x + containing.width.saturating_sub(block_width) / 2
}

/// Resolve the natural `<img>` display size in cells from the HTML `width` /
/// `height` attributes (source pixels) and the file's intrinsic size.
///
/// Mirrors browser sizing:
/// - Both attributes: the image is stretched to exactly `w × h` source
///   pixels (width still capped to the containing block).
/// - One attribute: the other axis follows the intrinsic aspect ratio.
/// - Neither: the intrinsic size, capped to the containing width; the height
///   follows the aspect ratio.
/// - No readable intrinsic size: `containing_width` square (historical
///   fallback).
fn resolve_image_cell_size(
    attr_w: Option<u32>,
    attr_h: Option<u32>,
    intrinsic: Option<(u32, u32)>,
    containing_width: usize,
) -> (usize, usize) {
    let containing_w = containing_width.max(1) as u32;
    let (intrinsic_w, intrinsic_h) = intrinsic.unwrap_or((containing_w, containing_w));
    // Width: an explicit attribute wins, otherwise the intrinsic width; both
    // are capped to the containing block so the box never overflows it.
    let pixel_w = attr_w.unwrap_or(intrinsic_w).min(containing_w).max(1);
    // Height: an explicit attribute wins, otherwise derived from the resolved
    // width preserving the source aspect ratio.
    let pixel_h = match attr_h {
        Some(h) => h.max(1),
        None => {
            let h = pixel_w as u64 * intrinsic_h as u64 / intrinsic_w.max(1) as u64;
            (h as u32).max(1)
        }
    };
    let img_cols = pixel_w.div_ceil(IMAGE_PIXELS_PER_CELL_COL).max(1) as usize;
    let img_rows = pixel_h.div_ceil(IMAGE_PIXELS_PER_CELL_ROW).max(1) as usize;
    (img_cols, img_rows)
}
