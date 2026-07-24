use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox, tree::{LayoutBlockOptions, layout_block},
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
) -> Result<Option<LayoutBox>> {
    layout_block_with_tag(arena, node_id, containing, cursor, NodeType::Header)
}

/// Build a [`LayoutBox`] for a `<footer>` element.
pub fn layout_footer(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>> {
    layout_block_with_tag(arena, node_id, containing, cursor, NodeType::Footer)
}

fn layout_block_with_tag(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    expected: NodeType,
) -> Result<Option<LayoutBox>> {
    let node = &arena[node_id];
    if node.node_type != expected {
        return Ok(None);
    }

    Ok(Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions {
            fill_bg_default: true,
            spacing_after: HEADER_FOOTER_SPACING,
            ..Default::default()
        },
    )?))
}
