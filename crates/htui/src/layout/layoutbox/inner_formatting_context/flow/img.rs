//! Compatibility wrapper around the geometry-driven replaced-element path.
//!
//! `<img>` is classified as [`crate::layout::layoutbox::generation::GeneratedBoxKind::Replaced`]
//! and dispatched through the geometry entry point. This wrapper preserves the
//! existing flex / table integration; it defers to the geometry dispatcher and
//! returns `None` for non-`<img>` nodes (the geometry phase decides whether
//! the payload qualifies as a replaced element).

use crate::{
    layout::{BoxConstraint, Cursor, LayoutBox, LayoutContext},
    tdom::node::{NodeArena, NodeId},
};

/// Build a normal-flow image box without a parent pre-allocation.
pub fn layout_img(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
) -> Option<LayoutBox> {
    layout_img_with_rect(arena, node_id, containing, cursor, context, None)
}

/// Build an `<img>` element, optionally honoring a rectangle allocated by a
/// parent formatting context.
pub fn layout_img_with_rect(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    crate::layout::layoutbox::geometry::layout_generated_box_via_node(
        arena,
        node_id,
        containing,
        cursor,
        context,
        pre_allocated_rect,
    )
}
