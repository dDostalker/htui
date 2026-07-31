//! Compatibility wrapper around the geometry-driven replaced-element path.
//!
//! `<img>` is now classified as [`crate::layout::layoutbox::generation::GeneratedBoxKind::Replaced`]
//! during the generation phase and dispatched through the geometry entry
//! point. This entry point remains so external callers and the existing
//! flex / table integration continue to compile; it simply defers to the
//! geometry dispatcher and returns `None` when the node isn't an image.

use crate::{
    layout::{BoxConstraint, Cursor, LayoutBox, LayoutContext},
    tdom::node::{NodeArena, NodeId, NodeType},
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
    if !matches!(arena[node_id].node_type, NodeType::Img) {
        return None;
    }
    crate::layout::layoutbox::geometry::layout_generated_box_via_node(
        arena,
        node_id,
        containing,
        cursor,
        context,
        pre_allocated_rect,
    )
}