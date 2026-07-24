use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox,
        tree::{block_left, collect_runs, layout_children, resolve_block_width},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::block_style,
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
///
/// Inline children are collected as one run per `<span>` so styled spans
/// (`<span class="bold">bold</span>`) render with their own `FontStyle`
/// at paint time.
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

    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let runs = collect_runs(arena, node, block_fg, block_bg, block_font_style);

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
        runs,
        children,
    )))
}
