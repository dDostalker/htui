use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox,
        tree::{block_left, collect_direct_text, layout_children, resolve_block_width},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Build a [`LayoutBox`] for a `<p>` element.
///
/// Honors CSS `width` (default: fill containing block) and `text-align`
/// (inherited from ancestors via `inherit_css`) — the paint phase uses
/// the resolved `block_width` for both background fill and text-align
/// padding, so nested `<div><p>…</p></div>` correctly positions text
/// within the parent's width. Block sits flush left inside its parent,
/// matching CSS `margin-left: 0` defaults; `text-align` only moves text
/// within the block, not the block itself.
pub fn layout_p(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>> {
    let node = &arena[node_id];
    if node.node_type != NodeType::P {
        return Ok(None);
    }

    let block_width =
        resolve_block_width(node.css_attr.get_width(containing.width), containing.width);
    let x = block_left(containing, block_width);
    let y = cursor.allocate(1);

    let text = {
        let raw = collect_direct_text(arena, node);
        if raw.is_empty() { None } else { Some(raw) }
    };

    let child_containing = BoxConstraint {
        x,
        width: block_width,
    };
    let child_cursor = &mut Cursor { y: cursor.peek() };
    let block_children = node.children.clone();
    let children = layout_children(arena, &block_children, child_containing, child_cursor)?;
    cursor.y = child_cursor.y;

    Ok(Some(LayoutBox::new_block(
        node_id,
        crate::layout::Rect::new(x, y, block_width, 1),
        false,
        0,
        text,
        children,
    )))
}
