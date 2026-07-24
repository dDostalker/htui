use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, Cursor, InlineRun, LayoutBox,
        tree::{block_left, collect_runs, layout_children, resolve_block_width},
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

    let block_width =
        resolve_block_width(node.css_attr.get_width(containing.width), containing.width);
    let x = block_left(containing, block_width);
    let y = cursor.allocate(1);

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
