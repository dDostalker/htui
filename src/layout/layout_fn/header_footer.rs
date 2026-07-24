use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, BoxEdges, Cursor, LayoutBox,
        tree::{
            block_left, collect_runs, compute_outer_geometry,
            layout_children, reserve_border_box, resolve_block_width,
        },
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

    let fill_bg = block_bg.is_some() || fill_bg_default;

    // Reserve margin-top rows.
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

    // `spacing_after` is the header/footer's built-in `margin-bottom`. It
    // sits *inside* the rect (so the paint phase can skip those rows when
    // filling the bg) but also advances the cursor past it. User-specified
    // `margin-bottom` would stack on top — `reserve_border_box` already
    // added that, so we just add the spacing on the cursor side.
    let rect_height = outer_height + spacing_after;
    cursor.y = cursor.y.saturating_add(spacing_after);

    Ok(Some(LayoutBox::new_block(
        node_id,
        crate::layout::Rect::new(geom.outer_x, geom.outer_y, geom.outer_width, rect_height),
        edges,
        fill_bg,
        spacing_after,
        runs,
        children,
    )))
}