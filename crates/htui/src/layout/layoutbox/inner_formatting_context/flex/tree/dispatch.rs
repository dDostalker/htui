//! Top-level flex dispatch — entry point plus node-capability predicates.

use crate::{
    layout::{BoxConstraint, Cursor, LayoutBox, LayoutContext, Rect},
    tdom::node::{NodeArena, NodeId, NodeType},
};

use super::container::layout_flex_container;

/// Top-level flex entry point. Registered first in `DEFAULT_LAYOUT`.
///
/// Returns `None` for nodes that do not opt into `display: flex` so the
/// registry falls through to the element-specific block layout.
pub fn layout_flex_dispatch(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    if !crate::layout::layoutbox::generation::generate_node(arena, node_id)
        .map(|generated| generated.inner_formatting_context == crate::layout::FormattingContext::Flex)
        .unwrap_or(false)
    {
        return None;
    }
    Some(layout_flex_container(
        arena,
        node_id,
        containing,
        cursor,
        context,
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
#[allow(dead_code)]
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
/// Generation wraps inline and text children into anonymous flex items, so
/// the runtime never has to filter them by tag. This helper is kept for
/// callers that still need to know whether a direct child *should* be
/// promoted into a flex item box at all (e.g. measure-time width probing).
#[allow(dead_code)]
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
            | NodeType::A
    )
}
