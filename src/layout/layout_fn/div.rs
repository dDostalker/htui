use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox,
        tree::{block_left, collect_direct_text, layout_children, resolve_block_width},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Build a [`LayoutBox`] for a `<div>` element.
///
/// `<div>` is a generic block container. CSS `width` controls the block's
/// column count (default: fill containing block); the block sits flush left
/// inside its parent, matching CSS `margin-left: 0` defaults.
/// `background-color` triggers a full-rect background fill, and `text-align`
/// is applied to direct text descendants. CSS `color` cascades to descendants
/// through `inherit_css`, so the paint phase simply reads each descendant's
/// own `css_attr`.
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

    let block_width =
        resolve_block_width(node.css_attr.get_width(containing.width), containing.width);
    let x = block_left(containing, block_width);
    let y = cursor.allocate(1);

    let text = {
        let raw = collect_direct_text(arena, node);
        if raw.is_empty() { None } else { Some(raw) }
    };

    let fill_bg = node.css_attr.get_bg_color().is_some();

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
        fill_bg,
        0,
        text,
        children,
    )))
}
