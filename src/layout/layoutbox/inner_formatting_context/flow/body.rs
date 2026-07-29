use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox,
        inner_formatting_context::flow::{FontContext, LayoutBlockOptions, layout_block},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Build a [`LayoutBox`] for a `<body>` element.
///
/// `<body>` is a transparent block container. It behaves like `<div>` so CSS
/// rules on `body` (e.g. `body { background-color: ...; color: ...; }`) apply
/// to the whole document while nested block children lay out exactly as they
/// would inside any other block element.
pub fn layout_body(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Body {
        return None;
    }

    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions::default(),
        font_ctx,
        pre_allocated_rect,
    ))
}
