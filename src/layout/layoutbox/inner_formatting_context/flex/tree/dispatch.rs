//! Top-level flex dispatch — entry point plus node-capability predicates.

use crate::{
    layout::{BoxConstraint, Cursor, FontContext, LayoutBox, Rect},
    tdom::node::{NodeArena, NodeId, NodeType},
};

use super::container::layout_flex_container;

/// Top-level flex entry point. Registered first in `DEFAULT_LAYOUT`.
///
/// Returns `None` for nodes that do not opt into `display: flex` so the
/// registry falls through to the element-specific block layout. Also
/// declines non-element nodes (text, head, style, …) which can never
/// host flex children.
pub fn layout_flex_dispatch(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if !node.css_attr.is_flex() {
        return None;
    }
    if !is_flex_capable_node(&node.node_type) {
        return None;
    }
    Some(layout_flex_container(
        arena,
        node_id,
        containing,
        cursor,
        font_ctx,
        pre_allocated_rect,
    ))
}

/// Whether a `NodeType` can host a flex container.
///
/// Text nodes, head/title/style/meta/link (which are parsed but never
/// reached as layout roots in normal documents), and the synthetic
/// `Global` root are all rejected. The `Selector::Tag` matcher
/// already prevents most of these from acquiring `display: flex` in the
/// first place — the guard is belt-and-braces.
pub(super) fn is_flex_capable_node(nt: &NodeType) -> bool {
    !matches!(
        nt,
        NodeType::Text(_)
            | NodeType::Global
            | NodeType::Head
            | NodeType::Title
            | NodeType::Style
            | NodeType::Link
            | NodeType::Meta
    )
}

/// True when the node type is a block-level element that can act as a
/// flex *item* (i.e. can be a direct child of a flex container).
///
/// Inline elements (`Span`, `A`, `Em`) and `Text` leaves are excluded —
/// they participate in the container's inline runs instead. `<img>` is
/// also a block-level layout element and is included so images can sit
/// inside a flex container.
pub(super) fn is_block_child_node(nt: &NodeType) -> bool {
    matches!(
        nt,
        NodeType::Div
            | NodeType::P
            | NodeType::H1
            | NodeType::H2
            | NodeType::H3
            | NodeType::H4
            | NodeType::H5
            | NodeType::Header
            | NodeType::Footer
            | NodeType::Aside
            | NodeType::Section
            | NodeType::Article
            | NodeType::Main
            | NodeType::Nav
            | NodeType::Figure
            | NodeType::Figcaption
            | NodeType::Details
            | NodeType::Summary
            | NodeType::Ul
            | NodeType::Ol
            | NodeType::Li
            | NodeType::Table
            | NodeType::Thead
            | NodeType::Tbody
            | NodeType::Tfoot
            | NodeType::Tr
            | NodeType::Th
            | NodeType::Td
            | NodeType::Img
            | NodeType::Form
            | NodeType::Button
            | NodeType::Input
            | NodeType::Select
            | NodeType::Textarea
            | NodeType::Label
    )
}
