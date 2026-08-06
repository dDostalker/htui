//! Top-level flex dispatch entry point.

use crate::{
    layout::{BoxConstraint, Cursor, LayoutBox, LayoutContext, Rect},
    tdom::node::{NodeArena, NodeId},
};

use super::container::layout_flex_container;

/// Top-level flex entry point. Registered first in `DEFAULT_LAYOUT`.
///
/// Returns `None` for nodes that do not opt into `display: flex` so the
/// registry falls through to the element-specific block layout.
pub fn layout_flex_dispatch(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    if !crate::layout::layoutbox::generation::generate_node(arena, node_id)
        .map(|generated| generated.inner_formatting_context == crate::layout::FormattingContext::Flex)
        .unwrap_or(false)
    {
        return None;
    }
    Some(layout_flex_container(
        arena,
        node_id,
        containing,
        cursor,
        context,
        pre_allocated_rect,
    ))
}
