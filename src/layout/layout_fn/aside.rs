use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox,
        tree::{block_left, collect_direct_text, layout_children, resolve_block_width},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Build a [`LayoutBox`] for an `<aside>` element.
///
/// `<aside>` is a block container. Its rectangle does **not** reserve an extra
/// empty row for itself — the bg wraps the children directly so the rendering
/// matches a browser (no blank stripe above the content). With no nested block
/// children the aside still claims one row so siblings don't collide.
pub fn layout_aside(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Aside {
        return Ok(None);
    }

    let block_width =
        resolve_block_width(node.css_attr.get_width(containing.width), containing.width);
    let x = block_left(containing, block_width);

    let text = {
        let raw = collect_direct_text(arena, node);
        if raw.is_empty() { None } else { Some(raw) }
    };

    // Peek the next row without reserving — the aside's children will start
    // here, so the aside's rect should encompass the children's rows rather
    // than sitting on top of an empty "own row".
    let y = cursor.peek();

    let child_containing = BoxConstraint {
        x,
        width: block_width,
    };
    let mut child_cursor = Cursor { y };
    let block_children = node.children.clone();
    let children = layout_children(arena, &block_children, child_containing, &mut child_cursor)?;

    // Aside height equals the number of rows occupied by its children (or 1
    // when empty). This keeps the bg flush with the content — no empty
    // stripe above — and leaves an empty aside at height 1 so siblings don't
    // collide.
    let height = child_cursor.y.saturating_sub(y).max(1);
    cursor.y = y + height;

    Ok(Some(LayoutBox::new_block(
        node_id,
        crate::layout::Rect::new(x, y, block_width, height),
        true,
        0,
        text,
        children,
    )))
}
