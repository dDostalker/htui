use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox, tree::{LayoutBlockOptions, layout_block},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Build a [`LayoutBox`] for a `<div>` element.
///
/// `<div>` is a generic block container. CSS `width` controls the **border-box**
/// column count (default: fill containing block). The element sits at
/// `containing.x + margin.left`, and its inner content box (passed to nested
/// children) is shrunk by `padding` + `border`. `background-color` triggers a
/// full-rect background fill covering the border-box; `text-align` is applied
/// to direct text descendants inside the content area.
///
/// Inline children are collected as one run per `<span>` so styled spans
/// (`<span class="bold">bold</span>`) render with their own `FontStyle`
/// at paint time.
pub fn layout_div(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Div {
        return Ok(None);
    }

    Ok(Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions::default(),
    )?))
}
