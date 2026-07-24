use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, BoxEdges, Cursor, InlineRun, LayoutBox,
        tree::{
            block_left, build_border_rect, collect_runs, compute_outer_geometry,
            layout_children, reserve_border_box, resolve_block_width,
        },
    },
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::block_style,
};

/// Build a [`LayoutBox`] for `<h1>`…`<h5>`.
///
/// Renders a `#`-marker prefix equal to the heading level (`# Heading`,
/// `## Sub`, …) so the paint phase includes it in `text-align` width
/// calculations, matching the previous behaviour. The prefix is its own
/// `InlineRun` with the heading's resolved style; body runs are collected
/// through `collect_runs` so `<span>` children keep their own styling.
///
/// Padding / border / margin on a heading shrinks the content area and
/// shifts the inline runs into the content row.
pub fn layout_hn(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>> {
    let node = &arena[node_id];
    let level = match node.node_type {
        NodeType::H1 => 1,
        NodeType::H2 => 2,
        NodeType::H3 => 3,
        NodeType::H4 => 4,
        NodeType::H5 => 5,
        _ => return Ok(None),
    };

    let edges: BoxEdges = node.css_attr.get_box_edges(containing.width);

    let available = containing.width.saturating_sub(edges.margin.horizontal());
    let block_width =
        resolve_block_width(node.css_attr.get_width(available), available).max(1);
    let outer_width = block_width;
    let x = block_left(containing, block_width);

    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let body_runs = collect_runs(arena, node, block_fg, block_bg, block_font_style);

    let runs = if body_runs.is_empty() {
        Vec::new()
    } else {
        // `#`-prefix is its own run carried at the heading's resolved style,
        // so a span in the body can still apply its own flags on top.
        let prefix = format!("{} ", "#".repeat(level));
        let mut all = Vec::with_capacity(body_runs.len() + 1);
        all.push(InlineRun::new(
            prefix,
            block_font_style,
            block_fg,
            block_bg,
        ));
        all.extend(body_runs);
        all
    };

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
        build_border_rect(geom, outer_height),
        edges,
        false,
        0,
        runs,
        children,
    )))
}