use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox,
        tree::{centered_left, collect_direct_text, layout_children, resolve_block_width},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Build a [`LayoutBox`] for an `<aside>` element.
///
/// `<aside>` always fills its rect with the background color (even when
/// `background-color` is unset), so the rect reads as a solid stripe. The
/// box's height grows to encompass any nested block-level children so the
/// background truly wraps the content; `<div>` is identical otherwise.
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
    let x = centered_left(containing, block_width);
    let y = cursor.allocate(1);

    let text = {
        let raw = collect_direct_text(arena, node);
        if raw.is_empty() { None } else { Some(raw) }
    };

    let child_containing = BoxConstraint {
        x,
        width: block_width,
    };
    let mut child_cursor = Cursor { y: cursor.peek() };
    let block_children = node.children.clone();
    let children = layout_children(arena, &block_children, child_containing, &mut child_cursor)?;
    cursor.y = child_cursor.y;

    // Grow the rect to wrap nested block children. Floor at 1 so an empty
    // aside still occupies its allocated row.
    let height = cursor.y.saturating_sub(y).max(1);

    Ok(Some(LayoutBox::new_block(
        node_id,
        crate::layout::Rect::new(x, y, block_width, height),
        true,
        0,
        text,
        children,
    )))
}
