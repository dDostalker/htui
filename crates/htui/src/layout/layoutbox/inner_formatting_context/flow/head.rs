use crate::{
    layout::{
        BoxConstraint, Cursor, InlineRun, LayoutBox, LayoutContext,
        inner_formatting_context::flow::{LayoutBlockOptions, layout_block},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::block_style,
};

/// Markdown-style prefix prepended to every heading level.
///
/// `<h1>` → `# `, `<h2>` → `## `, etc. Up to 6 levels match the supported
/// `NodeType::H1..H6` variants.
fn heading_prefix(node_type: &NodeType) -> Option<&'static str> {
    match node_type {
        NodeType::H1 => Some("# "),
        NodeType::H2 => Some("## "),
        NodeType::H3 => Some("### "),
        NodeType::H4 => Some("#### "),
        NodeType::H5 => Some("##### "),
        NodeType::H6 => Some("###### "),
        _ => None,
    }
}

/// Build a [`LayoutBox`] for an `<h1>`–`<h6>` element.
///
/// Headings are block elements with a font-size that defaults to
/// `2em / 1.5em / 1.17em / 1em / 0.83em / 0.67em` (see
/// [`NodeType::default_font_size_em`]) and a markdown-style prefix (`# `,
/// `## `, … `###### `) that paints in the heading's own foreground so visual
/// nesting mirrors the source level. `text-align`, `color`, and
/// `background-color` are applied the same way as on `<p>` / `<div>`.
pub fn layout_hn(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
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
        context,
        pre_allocated_rect,
    ))
}

/// Build a [`LayoutBox`] for a `<title>` element.
///
/// `<title>` is document metadata rendered as the terminal title (or browser
/// window title). It must not appear in the rendered output, so this layout
/// function is a sink that drops the title's text content — the parsed `Text`
/// child of `<title>` is never reached because the dispatcher only recurses
/// into children of nodes that claim a [`LayoutBox`]. Treating it as a sink
/// also makes the title invisible regardless of where it appears in the DOM
/// (e.g. outside `<head>`), matching browser behaviour.
pub fn layout_title(
    arena: &NodeArena,
    node_id: NodeId,
    _containing: BoxConstraint,
    _cursor: &mut Cursor,
    _context: LayoutContext,
    _pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Title {
        return None;
    }
    None
}

/// Build a [`LayoutBox`] for a `<meta>` element.
///
/// `<meta>` is a void element carrying document metadata (`charset`, `name`,
/// `content`, `http-equiv`, etc.). It has no rendered content and reserves no
/// rows on the terminal canvas, so it acts as a sink in the layout pass.
/// CSS rules targeting `<meta>` still parse and apply to its [`CssAttr`], but
/// have no visible effect because the element is never painted.
pub fn layout_meta(
    arena: &NodeArena,
    node_id: NodeId,
    _containing: BoxConstraint,
    _cursor: &mut Cursor,
    _context: LayoutContext,
    _pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Meta {
        return None;
    }
    None
}
