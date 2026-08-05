use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox, LayoutContext,
        inner_formatting_context::flow::{LayoutBlockOptions, layout_block},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
};

const HEADER_FOOTER_SPACING: usize = 2;

/// Build a [`LayoutBox`] for a `<header>` element.
pub fn layout_header(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    layout_block_with_tag(
        arena,
        node_id,
        containing,
        cursor,
        NodeType::Header,
        context,
        pre_allocated_rect,
    )
}

/// Build a [`LayoutBox`] for a `<footer>` element.
pub fn layout_footer(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    layout_block_with_tag(
        arena,
        node_id,
        containing,
        cursor,
        NodeType::Footer,
        context,
        pre_allocated_rect,
    )
}

fn layout_block_with_tag(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    expected: NodeType,
    context: LayoutContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != expected {
        return None;
    }

    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions {
            fill_bg_default: true,
            chrome: true,
            spacing_after: HEADER_FOOTER_SPACING,
            ..Default::default()
        },
        context,
        pre_allocated_rect,
    ))
}
