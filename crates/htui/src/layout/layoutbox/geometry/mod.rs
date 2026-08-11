//! Geometry assignment for generated layout boxes.
//!
//! Dispatch is based on box classification alone:
//!
//! - `BoxKind × OuterDisplay × FormattingContext` selects which algorithm
//!   owns the geometry for a box.
//! - Tag-specific data (e.g. `<img src>`) is read from the
//!   [`GeneratedPayload`](crate::layout::layoutbox::generation::GeneratedPayload)
//!   attached at generation time, never from the DOM.
//! - Replaced, flex, and table boxes are reached here before the normal-flow
//!   registry, so per-element `LayoutFn`s never re-claim them.

use crate::{
    html::css_parse::Position,
    layout::{
        BoxConstraint, Cursor, FormattingContext, LayoutBox, LayoutContext, PaintStyle, Rect,
        ReplacedContent,
        layoutbox::{
            apply_relative_offset,
            generation::{GeneratedBox, GeneratedBoxKind, GeneratedPayload},
        },
    },
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::{
        IMAGE_PIXELS_PER_CELL_COL, IMAGE_PIXELS_PER_CELL_ROW, char_width, image_intrinsic_size,
    },
};

/// Assign geometry to a generated box forest.
pub fn layout_generated_tree(
    arena: &NodeArena,
    boxes: &[GeneratedBox],
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
) -> Vec<LayoutBox> {
    boxes
        .iter()
        .filter_map(|generated| {
            layout_generated_box(arena, generated, containing, cursor, context, None)
        })
        .collect()
}

/// Assign geometry to one generated box using its box classifications.
pub fn layout_generated_box(
    arena: &NodeArena,
    generated: &GeneratedBox,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    // Positioned boxes are detached from the surrounding flow *before* the
    // classification dispatch: absolute / fixed lay out against a local
    // cursor (occupying no flow rows) and are repositioned by the
    // post-layout pass; relative keeps its flow slot and is offset in place.
    let positioning = arena[generated.node_id]
        .css_attr
        .get_positioning(containing.width, context.font, context.viewport);
    let detached = matches!(positioning.position, Position::Absolute | Position::Fixed);
    let mut local_cursor = Cursor { y: 0 };
    let working_cursor: &mut Cursor = if detached { &mut local_cursor } else { cursor };

    let mut lb = match (
        generated.kind,
        generated.outer_display,
        generated.inner_formatting_context,
    ) {
        (GeneratedBoxKind::Replaced, _, _) => layout_replaced(
            arena,
            generated,
            containing,
            working_cursor,
            pre_allocated_rect,
        )?,
        (GeneratedBoxKind::Normal, _, FormattingContext::Flex) => {
            crate::layout::inner_formatting_context::flex::layout_flex_dispatch(
                arena,
                generated.node_id,
                containing,
                working_cursor,
                context,
                pre_allocated_rect,
            )?
        }
        (GeneratedBoxKind::Normal, _, FormattingContext::Table) => {
            crate::layout::inner_formatting_context::flow::layout_table(
                arena,
                generated.node_id,
                containing,
                working_cursor,
                context,
                pre_allocated_rect,
            )?
        }
        (GeneratedBoxKind::Normal, _, FormattingContext::Flow) => {
            crate::layout::inner_formatting_context::flow::dispatch_flow_geometry(
                arena,
                generated.node_id,
                containing,
                working_cursor,
                context,
                pre_allocated_rect,
            )?
        }
        (GeneratedBoxKind::Normal, _, FormattingContext::Grid) => {
            log::debug!(
                "Grid geometry is not implemented for node {}",
                generated.node_id
            );
            return None;
        }
        (GeneratedBoxKind::Text | GeneratedBoxKind::Anonymous, _, _) => return None,
    };
    if positioning.position == Position::Relative {
        apply_relative_offset(&mut lb, positioning.inset);
    }
    lb.positioning = positioning;
    Some(lb)
}

/// Layout a single DOM node by generating its box and dispatching its
/// geometry. Used by callers that already have a `NodeId` and want the same
/// classification-driven behaviour as the box-tree walk.
pub fn layout_generated_box_via_node(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    let generated = crate::layout::layoutbox::generation::generate_node(arena, node_id)?;
    layout_generated_box(
        arena,
        &generated,
        containing,
        cursor,
        context,
        pre_allocated_rect,
    )
}

/// Geometry for replaced elements driven solely by the payload attached at
/// generation time. Tag-specific data (the resource path and intrinsic
/// `width` / `height` attributes) is read from the DOM only when the
/// payload indicates an image; the dispatch never inspects `NodeType` to
/// decide which geometry algorithm to run.
fn layout_replaced(
    arena: &NodeArena,
    generated: &GeneratedBox,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    match &generated.payload {
        GeneratedPayload::Input => {
            layout_input(arena, generated, containing, cursor, pre_allocated_rect)
        }
        GeneratedPayload::Textarea => {
            layout_textarea(arena, generated, containing, cursor, pre_allocated_rect)
        }
        GeneratedPayload::Select => {
            layout_select(arena, generated, containing, cursor, pre_allocated_rect)
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

/// Read a `<select>`'s options as `(value, label)` pairs from its
/// `<option>` children. The `value` falls back to the option's text when the
/// `value` attribute is absent. Empty `value` attributes are treated as the
/// empty string (a browser option with `value=""`).
pub(crate) fn select_options(arena: &NodeArena, node_id: NodeId) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(node) = arena.get(node_id) else {
        return out;
    };
    for &child_id in &node.children {
        let child = &arena[child_id];
        if child.node_type != NodeType::HtmlOption {
            continue;
        }
        let label: String = child
            .children
            .iter()
            .filter_map(|&c| match &arena[c].node_type {
                NodeType::Text => arena[c].data.as_deref(),
                _ => None,
            })
            .collect();
        let value = child
            .attributes
            .get("value")
            .cloned()
            .unwrap_or_else(|| label.clone());
        out.push((value, label));
    }
    out
}

/// Dropdown overlay rectangle for an open `<select>`: directly below the
/// field, wide enough for the longest option label (plus the selection
/// marker), at least as wide as the field itself.
pub(crate) fn select_overlay_rect(options: &[(String, String)], select_rect: Rect) -> Rect {
    let text_w: usize = options
        .iter()
        .map(|(_, label)| label.chars().map(char_width).sum::<usize>())
        .max()
        .unwrap_or(0);
    let width = select_rect.width.max(text_w.saturating_add(2)).max(1);
    Rect::new(
        select_rect.x,
        select_rect.y.saturating_add(1),
        width,
        options.len().max(1),
    )
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
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    let (x, y, block_width, height) = match pre_allocated_rect {
        Some(rect) => (rect.x, rect.y, rect.width.max(1), rect.height.max(1)),
        None => {
            let node = &arena[generated.node_id];
            // `<input type=checkbox|radio>` is a two-cell toggle (glyph +
            // one spacing column so it never glues onto adjacent text); every
            // other kind is a text field sized from the HTML `width`
            // attribute (cells) or a fixed default width when absent.
            let is_toggle = node.get_type().is_some_and(|t| {
                matches!(t.to_ascii_lowercase().as_str(), "checkbox" | "radio")
            });
            let block_width = if is_toggle {
                2
            } else {
                match node.get_width() {
                    Some(w) => (w as usize).min(containing.width).max(1),
                    None => 8usize.min(containing.width.max(1)),
                }
            };
            let x = containing.x;
            let y = cursor.allocate(1);
            (x, y, block_width, 1)
        }
    };
    Some(LayoutBox::new_replaced(
        generated.node_id,
        Rect::new(x, y, block_width, height),
        ReplacedContent::Input,
        PaintStyle::default(),
    ))
}

/// Geometry for `<textarea>`: a fixed multi-line field with a box-drawing
/// border.
///
/// Width is `cols` (browser default 20) plus the two frame columns, height
/// is `rows` (browser default 2) plus the top/bottom border rows. The size is
/// independent of the value, so editing only repaints the field's rectangle
/// — same as `<input>`.
fn layout_textarea(
    arena: &NodeArena,
    generated: &GeneratedBox,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
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
            let block_width = (cols.saturating_add(2)).min(containing.width).max(1);
            let content_rows = rows.max(1);
            let x = containing.x;
            let y = cursor.allocate(content_rows.saturating_add(2));
            (x, y, block_width, content_rows.saturating_add(2))
        }
    };
    Some(LayoutBox::new_replaced(
        generated.node_id,
        Rect::new(x, y, block_width, height),
        ReplacedContent::Textarea,
        PaintStyle::default(),
    ))
}

/// Geometry for `<select>`: a single-line dropdown field.
///
/// Width is the longest option label plus the frame / chevron, or the HTML
/// `width` attribute when set. The dropdown overlay is painted by the render
/// thread, so the box itself is always one row.
fn layout_select(
    arena: &NodeArena,
    generated: &GeneratedBox,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    let (x, y, block_width, height) = match pre_allocated_rect {
        Some(rect) => (rect.x, rect.y, rect.width.max(1), rect.height.max(1)),
        None => {
            let node = &arena[generated.node_id];
            let text_w: usize = select_options(arena, generated.node_id)
                .iter()
                .map(|(_, label)| label.chars().map(char_width).sum::<usize>())
                .max()
                .unwrap_or(0);
            let block_width = match node.get_width() {
                Some(w) => (w as usize).min(containing.width).max(1),
                None => (text_w.saturating_add(4)).min(containing.width).max(4),
            };
            let x = containing.x;
            let y = cursor.allocate(1);
            (x, y, block_width, 1)
        }
    };
    Some(LayoutBox::new_replaced(
        generated.node_id,
        Rect::new(x, y, block_width, height),
        ReplacedContent::Select,
        PaintStyle::default(),
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