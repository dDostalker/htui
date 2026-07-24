use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, BoxEdges, Cursor, LayoutBox,
        tree::{
            block_left, build_border_rect, collect_runs, compute_outer_geometry,
            layout_children, reserve_border_box, resolve_block_width,
        },
    },
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::block_style,
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

    let edges: BoxEdges = node.css_attr.get_box_edges(containing.width);

    // CSS `width` is resolved against the space available after the box's own
    // margins — same as CSS containing-block semantics.
    let available = containing.width.saturating_sub(edges.margin.horizontal());
    let block_width =
        resolve_block_width(node.css_attr.get_width(available), available).max(1);
    let outer_width = block_width;
    let x = block_left(containing, block_width);

    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let runs = collect_runs(arena, node, block_fg, block_bg, block_font_style);

    let fill_bg = block_bg.is_some();

    // Reserve margin-top before the border-box.
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

    // Children fill the content area. They claim rows *below* the div's
    // own padding-top row.
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
    let outer_height = reserve_border_box(
        cursor,
        geom.outer_y,
        edges,
        content_height,
        // `<div>` always claims at least one row of content even when empty.
        1,
    );

    Ok(Some(LayoutBox::new_block(
        node_id,
        build_border_rect(geom, outer_height),
        edges,
        fill_bg,
        0,
        runs,
        children,
    )))
}