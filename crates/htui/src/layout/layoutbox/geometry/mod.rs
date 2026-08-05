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
    tdom::node::{NodeArena, NodeId},
    tui::graph_api::{
        IMAGE_PIXELS_PER_CELL_COL, IMAGE_PIXELS_PER_CELL_ROW, image_intrinsic_size,
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
    let src = match &generated.payload {
        GeneratedPayload::Image { src } if !src.is_empty() => src.clone(),
        GeneratedPayload::Transparent => return None,
        _ => return None,
    };

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