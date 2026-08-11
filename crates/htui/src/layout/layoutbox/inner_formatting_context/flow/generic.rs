use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox, LayoutContext,
        inner_formatting_context::flow::{LayoutBlockOptions, layout_block},
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
    context: LayoutContext,
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
        context,
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
            | NodeType::Pre
    )
}

/// Build a [`LayoutBox`] for an `<hr>` element.
///
/// `<hr>` renders a thematic break as a horizontal rule: a single row of
/// `─` glyphs spanning the content-box width (the browser equivalent of
/// `display: block` + a 1px top border). The rule run is synthesised at
/// layout time from the element's own fg / bg / font style, and one blank
/// row is reserved below so the rule does not abut the next block.
pub fn layout_hr(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Hr {
        return None;
    }
    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions {
            hr_rule: true,
            spacing_after: 1,
            ..Default::default()
        },
        context,
        pre_allocated_rect,
    ))
}
