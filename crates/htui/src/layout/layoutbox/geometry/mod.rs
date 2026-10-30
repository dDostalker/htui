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
        BoxConstraint, Cursor, FormattingContext, LayoutBox, LayoutContext, Rect,
        layoutbox::{
            apply_relative_offset,
            generation::{GeneratedBox, GeneratedBoxKind},
        },
    },
    tdom::node::{NodeArena, NodeId},
};

mod replaced;
mod select_overlay;

pub use select_overlay::{SelectOption, SelectRow};
pub(crate) use select_overlay::{
    overlay_options, select_item_indices, select_options, select_overlay_rect, select_rows,
};

use replaced::layout_replaced;

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
    let positioning = arena[generated.node_id].css_attr.get_positioning(
        containing.width,
        context.font,
        context.viewport,
    );
    let detached = matches!(positioning.position, Position::Absolute | Position::Fixed);
    let mut local_cursor = Cursor {
        y: 0,
        prev_bottom_margin: 0,
    };
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
            context,
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
            crate::layout::inner_formatting_context::grid::layout_grid_dispatch(
                arena,
                generated.node_id,
                containing,
                working_cursor,
                context,
                pre_allocated_rect,
            )?
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
