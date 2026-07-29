use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox,
        inner_formatting_context::flow::{FontContext, LayoutBlockOptions, layout_block},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Build a plain block box for block-level elements without specialised
/// rendering rules.
///
/// This is the final block fallback in the registry. Specialised handlers for
/// headings, lists, tables, images, chrome, and the document root run first;
/// this handler keeps ordinary structural elements usable in both normal block
/// flow and as flex items.
pub fn layout_generic(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if !is_generic_block(&node.node_type) {
        return None;
    }

    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions::default(),
        font_ctx,
        pre_allocated_rect,
    ))
}

fn is_generic_block(node_type: &NodeType) -> bool {
    matches!(
        node_type,
        NodeType::Nav
            | NodeType::Main
            | NodeType::Section
            | NodeType::Article
            | NodeType::Figure
            | NodeType::Figcaption
            | NodeType::Details
            | NodeType::Summary
            | NodeType::Input
            | NodeType::Form
            | NodeType::Select
            | NodeType::Button
            | NodeType::Textarea
            | NodeType::Label
    )
}

