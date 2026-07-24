use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, BoxEdges, Cursor, LayoutBox,
        tree::{
            block_left, collect_runs, compute_outer_geometry, layout_children,
            reserve_border_box, resolve_block_width,
        },
    },
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::block_style,
};

/// Build a [`LayoutBox`] for an `<aside>` element.
///
/// `<aside>` is a block container. Its rectangle does **not** reserve an extra
/// empty row for itself — the bg wraps the children directly so the rendering
/// matches a browser (no blank stripe above the content). With no nested block
/// children the aside still claims one row so siblings don't collide.
///
/// Padding / border are honoured: the inner containing block is shrunk, the
/// background fills the full border-box, and border glyphs are drawn when
/// `border-*` rules are present.
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

    let edges: BoxEdges = node.css_attr.get_box_edges(containing.width);

    let available = containing.width.saturating_sub(edges.margin.horizontal());
    let block_width =
        resolve_block_width(node.css_attr.get_width(available), available).max(1);
    let outer_width = block_width;
    let x = block_left(containing, block_width);

    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let runs = collect_runs(arena, node, block_fg, block_bg, block_font_style);

    // Peek the next row (after any margin-top) without reserving — the aside's
    // children will start on this row, so the aside's rect should encompass
    // them rather than sitting on top of an empty "own row".
    cursor.skip(edges.margin.top);
    let outer_y_top = cursor.y;

    let geom = compute_outer_geometry(
        BoxConstraint {
            x,
            width: outer_width,
        },
        outer_width,
        edges,
        outer_y_top,
        1,
    );

    let mut child_cursor = Cursor {
        y: geom.outer_y
            + edges.border.top.thickness()
            + edges.padding.top,
    };
    let block_children = node.children.clone();
    let children = layout_children(
        arena,
        &block_children,
        geom.content_constraint,
        &mut child_cursor,
    )?;

    let content_height = child_cursor.y.saturating_sub(
        geom.outer_y + edges.border.top.thickness() + edges.padding.top,
    );
    let outer_height = reserve_border_box(cursor, geom.outer_y, edges, content_height, 1);

    Ok(Some(LayoutBox::new_block(
        node_id,
        crate::layout::Rect::new(geom.outer_x, geom.outer_y, geom.outer_width, outer_height),
        edges,
        true,
        0,
        runs,
        children,
    )))
}