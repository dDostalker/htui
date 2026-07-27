use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, Cursor, FontContext, InlineRun, LayoutBox,
        tree::{LayoutBlockOptions, layout_block},
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
/// through the shared pipeline so `<span>` children keep their own styling.
///
/// Padding / border / margin on a heading shrinks the content area and
/// shifts the inline runs into the content row.
pub fn layout_hn(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
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

    // `#`-prefix is its own run carried at the heading's resolved style,
    // so a span in the body can still apply its own flags on top.
    let prefix = format!("{} ", "#".repeat(level));
    let block_font_style = block_style(&node.css_attr);
    let prefix_run = InlineRun::new(
        prefix,
        block_font_style,
        node.css_attr.get_color(),
        node.css_attr.get_bg_color(),
    );

    Ok(Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions {
            prefix: Some(prefix_run),
            ..Default::default()
        },
        font_ctx,
    )?))
}
