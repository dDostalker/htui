//! DOM-to-box generation for the layout pipeline.
//!
//! This phase classifies DOM nodes and builds a geometry-free box tree. It
//! does not allocate rows or columns; geometry is assigned by the geometry
//! pass. The classifier mirrors the browser CSS display model:
//!
//! - [`BoxKind`](crate::layout::BoxKind) — the box's content shape.
//! - [`OuterDisplay`](crate::layout::OuterDisplay) — how the box participates
//!   in its parent's formatting context.
//! - [`FormattingContext`](crate::layout::FormattingContext) — how the box
//!   arranges its own children.
//!
//! After generation, the geometry pass dispatches exclusively on those three
//! classification axes; element tag is read inside an algorithm only when the
//! algorithm truly needs tag-specific data (e.g. resolving `<img src>`).

use crate::{
    layout::{FormattingContext, OuterDisplay},
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Content-shape classification produced before geometry is calculated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedBoxKind {
    /// A regular element box.
    Normal,
    /// A raw text node represented in the generated tree.
    Text,
    /// A synthetic wrapper around consecutive inline or text children.
    Anonymous,
    /// A replaced element whose content comes from outside the DOM tree.
    Replaced,
}

/// A geometry-free box generated from the styled DOM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedBox {
    /// DOM node associated with this box. Anonymous boxes use their parent's ID.
    pub node_id: NodeId,
    /// Content-shape classification.
    pub kind: GeneratedBoxKind,
    /// How this box participates in its parent's formatting context.
    pub outer_display: OuterDisplay,
    /// How this box arranges its children.
    pub inner_formatting_context: FormattingContext,
    /// Tag-specific data required by the geometry algorithm.
    pub payload: GeneratedPayload,
    /// Generated child boxes in document order.
    pub children: Vec<GeneratedBox>,
}

/// Optional payload attached to a [`GeneratedBox`] when the element carries
/// information that the geometry algorithm needs but classification alone
/// cannot express.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum GeneratedPayload {
    /// No tag-specific payload.
    #[default]
    None,
    /// `<img>` — the resource path the geometry pass will resolve.
    Image { src: String },
}

/// Generate a geometry-free forest for the children of `root_id`.
pub fn generate_box_tree(arena: &NodeArena, root_id: NodeId) -> Vec<GeneratedBox> {
    arena[root_id]
        .children
        .iter()
        .filter_map(|&child_id| generate_node(arena, child_id))
        .collect()
}

/// Generate the box corresponding to one DOM node.
pub fn generate_node(arena: &NodeArena, node_id: NodeId) -> Option<GeneratedBox> {
    let node = &arena[node_id];
    if is_non_rendered(&node.node_type) {
        return None;
    }

    if matches!(node.node_type, NodeType::Text(_)) {
        return Some(GeneratedBox {
            node_id,
            kind: GeneratedBoxKind::Text,
            outer_display: OuterDisplay::Inline,
            inner_formatting_context: FormattingContext::Flow,
            payload: GeneratedPayload::None,
            children: Vec::new(),
        });
    }

    let kind = if matches!(node.node_type, NodeType::Img) {
        GeneratedBoxKind::Replaced
    } else {
        GeneratedBoxKind::Normal
    };
    let inner_formatting_context = if matches!(node.node_type, NodeType::Table) {
        FormattingContext::Table
    } else if node.css_attr.is_flex() {
        FormattingContext::Flex
    } else {
        FormattingContext::Flow
    };
    let payload = match &node.node_type {
        NodeType::Img => GeneratedPayload::Image {
            src: node.get_src().cloned().unwrap_or_default(),
        },
        _ => GeneratedPayload::None,
    };

    Some(GeneratedBox {
        node_id,
        kind,
        outer_display: node.css_attr.outer_display(),
        inner_formatting_context,
        payload,
        children: generate_children(arena, node_id),
    })
}

fn generate_children(arena: &NodeArena, parent_id: NodeId) -> Vec<GeneratedBox> {
    let mut children = Vec::new();
    let mut inline_group = Vec::new();

    for &child_id in &arena[parent_id].children {
        let Some(child) = generate_node(arena, child_id) else {
            continue;
        };
        if child.kind == GeneratedBoxKind::Text || child.outer_display == OuterDisplay::Inline {
            inline_group.push(child);
            continue;
        }
        flush_inline_group(parent_id, &mut inline_group, &mut children);
        children.push(child);
    }
    flush_inline_group(parent_id, &mut inline_group, &mut children);
    children
}

fn flush_inline_group(
    parent_id: NodeId,
    inline_group: &mut Vec<GeneratedBox>,
    children: &mut Vec<GeneratedBox>,
) {
    if inline_group.is_empty() {
        return;
    }
    children.push(GeneratedBox {
        node_id: parent_id,
        kind: GeneratedBoxKind::Anonymous,
        outer_display: OuterDisplay::Block,
        inner_formatting_context: FormattingContext::Flow,
        payload: GeneratedPayload::None,
        children: std::mem::take(inline_group),
    });
}

fn is_non_rendered(node_type: &NodeType) -> bool {
    matches!(
        node_type,
        NodeType::Head | NodeType::Style | NodeType::Link | NodeType::Meta
    )
}