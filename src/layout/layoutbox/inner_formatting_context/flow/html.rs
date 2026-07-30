use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox, LayoutContext,
        inner_formatting_context::flow::{LayoutBlockOptions, layout_block},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Build a [`LayoutBox`] for an `<html>` element.
///
/// `<html>` is the document root. It is a transparent block container — its
/// children are laid out the same way as a `<div>`, so the typical use of
/// `body { ... }` to set the document background / foreground cascades down
/// without `<html>` adding any extra chrome.
pub fn layout_html(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Html {
        return None;
    }

    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions::default(),
        context,
        pre_allocated_rect,
    ))
}
