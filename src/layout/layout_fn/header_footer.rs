use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox,
        tree::{block_left, collect_runs, layout_children, resolve_block_width},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::block_style,
};

/// Build a [`LayoutBox`] for a `<header>` element.
pub fn layout_header(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>> {
    layout_block_with_tag(
        arena,
        node_id,
        containing,
        cursor,
        NodeType::Header,
        true,
        2,
    )
}

/// Build a [`LayoutBox`] for a `<footer>` element.
pub fn layout_footer(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>> {
    layout_block_with_tag(
        arena,
        node_id,
        containing,
        cursor,
        NodeType::Footer,
        true,
        2,
    )
}

#[allow(clippy::too_many_arguments)]
fn layout_block_with_tag(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    expected: NodeType,
    fill_bg_default: bool,
    spacing_after: usize,
) -> Result<Option<LayoutBox>> {
    let node = &arena[node_id];
    if node.node_type != expected {
        return Ok(None);
    }

    let block_width =
        resolve_block_width(node.css_attr.get_width(containing.width), containing.width);
    let x = block_left(containing, block_width);
    let content_y = cursor.allocate(1);
    // Reserve `spacing_after` trailing rows so the next block starts with
    // visual breathing room, matching the previous `new_line(); new_line();`.
    cursor.y = cursor.y.saturating_add(spacing_after);

    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let runs = collect_runs(arena, node, block_fg, block_bg, block_font_style);

    let fill_bg = block_bg.is_some() || fill_bg_default;

    let child_containing = BoxConstraint {
        x,
        width: block_width,
    };
    let mut child_cursor = Cursor { y: content_y };
    let block_children = node.children.clone();
    let children = layout_children(arena, &block_children, child_containing, &mut child_cursor)?;
    if child_cursor.y > cursor.y {
        cursor.y = child_cursor.y;
    }

    Ok(Some(LayoutBox::new_block(
        node_id,
        crate::layout::Rect::new(x, content_y, block_width, 1 + spacing_after),
        fill_bg,
        spacing_after,
        runs,
        children,
    )))
}
