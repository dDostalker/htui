use crate::{
    layout::{
        BoxConstraint, Cursor, FontContext, LayoutBox,
        tree::{LayoutBlockOptions, layout_block},
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
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    layout_block_with_tag(
        arena,
        node_id,
        containing,
        cursor,
        NodeType::Header,
        font_ctx,
    )
}

/// Build a [`LayoutBox`] for a `<footer>` element.
pub fn layout_footer(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    layout_block_with_tag(
        arena,
        node_id,
        containing,
        cursor,
        NodeType::Footer,
        font_ctx,
    )
}

fn layout_block_with_tag(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    expected: NodeType,
    font_ctx: FontContext,
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
        font_ctx,
    ))
}
