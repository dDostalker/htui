use crate::{
    layout::{
        BoxConstraint, Cursor, FontContext, InlineRun, LayoutBox,
        tree::{LayoutBlockOptions, layout_block},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::block_style,
};

/// Markdown-style prefix prepended to every heading level.
///
/// `<h1>` → `# `, `<h2>` → `## `, etc. Up to 5 levels match the supported
/// `NodeType::H1..H5` variants.
fn heading_prefix(node_type: &NodeType) -> Option<&'static str> {
    match node_type {
        NodeType::H1 => Some("# "),
        NodeType::H2 => Some("## "),
        NodeType::H3 => Some("### "),
        NodeType::H4 => Some("#### "),
        NodeType::H5 => Some("##### "),
        _ => None,
    }
}

/// Build a [`LayoutBox`] for an `<h1>`–`<h5>` element.
///
/// Headings are block elements with a font-size that defaults to
/// `2em / 1.5em / 1.17em / 1em / 0.83em` (see
/// [`NodeType::default_font_size_em`]) and a markdown-style prefix (`# `,
/// `## `, …) that paints in the heading's own foreground so visual nesting
/// mirrors the source level. `text-align`, `color`, and `background-color`
/// are applied the same way as on `<p>` / `<div>`.
pub fn layout_hn(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    let prefix_text = match heading_prefix(&node.node_type) {
        Some(p) => p,
        None => return None,
    };

    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let prefix_run = InlineRun::new(prefix_text, block_font_style, block_fg, block_bg);

    Some(layout_block(
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
    ))
}

/// Build a [`LayoutBox`] for a `<head>` element.
///
/// `<head>` is a structural-only container for document metadata (`<title>`,
/// `<style>`, `<link>`, `<meta>`, `<script>`). It produces no visible output
/// and reserves no rows on the terminal canvas, so it acts as a sink for
/// flattened children that would otherwise leak into the surrounding
/// inline runs (e.g. the text content of `<title>`). The actual side effects
/// — CSS extraction for `<style>` and `<link rel="stylesheet">` — happen at
/// parse time, so the layout pass can safely ignore the whole subtree.
pub fn layout_head(
    arena: &NodeArena,
    node_id: NodeId,
    _containing: BoxConstraint,
    _cursor: &mut Cursor,
    _font_ctx: FontContext,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Head {
        return None;
    }
    None
}
