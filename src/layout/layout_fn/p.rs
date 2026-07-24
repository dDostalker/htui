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

/// Build a [`LayoutBox`] for a `<p>` element.
///
/// Honors CSS `width` (default: fill containing block) — the resolved value is
/// the **border-box** width, including padding + border. Padding/border
/// shrink the content area passed to nested block children, and the inline
/// runs are painted on the content row (after `border.top` + `padding.top`).
/// `text-align` (inherited from ancestors via `inherit_css`) is computed
/// against the content area's width so `text-align: center` correctly
/// positions text within the inner content box rather than the full rect.
///
/// Margin / border / padding values are surfaced through `box_edges`; the
/// paint phase reads them to draw border glyphs and offset content.
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