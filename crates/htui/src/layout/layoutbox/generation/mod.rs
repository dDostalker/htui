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
    html::css_parse::CssDisplay,
    layout::{FormattingContext, OuterDisplay},
    tdom::node::{NodeArena, NodeId, NodeType, document_base_href},
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
    /// `<input>` — a single-line text field; the geometry pass sizes it from
    /// the `value` attribute (and the active edit state).
    Input,
    /// `<select>` — a single-line dropdown; options come from `<option>`
    /// children, the geometry pass sizes it from the longest label.
    Select,
    /// `<textarea>` — a multi-line editable field; the geometry pass sizes
    /// it from the `cols` / `rows` attributes and the paint pass shows the
    /// (possibly edited) value with a caret.
    Textarea,
    /// `<button>` — a push-button whose text content is painted inside a
    /// frame; the geometry pass sizes it from the text width.
    Button,
    /// Marker produced by [`generate_node`] for a `display: contents`
    /// element. The [`generate_children`] walker detects it and
    /// substitutes the contents element's own children in its place; the
    /// marker itself never reaches the geometry phase.
    Transparent,
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
///
/// The classifier is driven entirely by CSS `display` (with a per-`NodeType`
/// UA default fallback) plus a small set of element-specific payload rules
/// (e.g. `<img>` carries an `Image { src }` payload). The dispatch in
/// [`crate::layout::layoutbox::geometry`] never inspects `NodeType` to decide
/// which algorithm to run — it consumes only the `kind`, `outer_display` and
/// `inner_formatting_context` fields.
pub fn generate_node(arena: &NodeArena, node_id: NodeId) -> Option<GeneratedBox> {
    let node = &arena[node_id];
    if is_non_rendered(&node.node_type) {
        return None;
    }

    if matches!(node.node_type, NodeType::Text) {
        return Some(GeneratedBox {
            node_id,
            kind: GeneratedBoxKind::Text,
            outer_display: OuterDisplay::Inline { block: false },
            inner_formatting_context: FormattingContext::Flow,
            payload: GeneratedPayload::None,
            children: Vec::new(),
        });
    }

    // Resolve the effective `display`: author-declared value, or UA default.
    let display = node
        .css_attr
        .display()
        .unwrap_or_else(|| node.node_type.ua_default_display());

    // `display: none` removes the element and its subtree from the box tree.
    if display.is_dropped() {
        return None;
    }

    // `display: contents` flattens the element's children into the parent
    // instead of producing a box. We can't do that here (we only own one
    // node), so we emit a `Transparent` marker; `generate_children` will
    // detect it and substitute the contents element's own children.
    if display.is_transparent() {
        return Some(GeneratedBox {
            node_id,
            kind: GeneratedBoxKind::Anonymous,
            outer_display: OuterDisplay::Inline { block: false },
            inner_formatting_context: FormattingContext::Flow,
            payload: GeneratedPayload::Transparent,
            children: Vec::new(),
        });
    }

    // Replaced-element payload is determined by the element's HTML
    // semantics, not by the `display` value. `<img>` and `<input>` are the
    // replaced elements we render today; new tags (e.g. `<video>`) follow the
    // same pattern. The UA `display: inline-block` is what makes the geometry
    // dispatcher route through the replaced path — the dispatch never
    // inspects `NodeType`.
    let (kind, payload) = if matches!(node.node_type, NodeType::Img) {
        // Relative `src` resolves against the document base URL (`<base
        // href>`) once here, so the geometry and paint phases only ever see
        // the absolute `mem://` key.
        let raw_src = node.get_src().cloned().unwrap_or_default();
        let src = if raw_src.is_empty() {
            raw_src
        } else {
            let base = document_base_href(arena, node_id);
            crate::resources::resolve_url(base.as_deref(), &raw_src)
        };
        (GeneratedBoxKind::Replaced, GeneratedPayload::Image { src })
    } else if matches!(node.node_type, NodeType::Input) {
        // `<input type=hidden>` carries form data but never renders — the
        // browser UA stylesheet treats it like `display: none`, so no box
        // is generated (the element stays in the DOM, addressable by CSS/JS).
        if node
            .get_type()
            .is_some_and(|t| t.eq_ignore_ascii_case("hidden"))
        {
            return None;
        }
        (GeneratedBoxKind::Replaced, GeneratedPayload::Input)
    } else if matches!(node.node_type, NodeType::Textarea) {
        (GeneratedBoxKind::Replaced, GeneratedPayload::Textarea)
    } else if matches!(node.node_type, NodeType::Button) {
        (GeneratedBoxKind::Replaced, GeneratedPayload::Button)
    } else if matches!(node.node_type, NodeType::Select) {
        (GeneratedBoxKind::Replaced, GeneratedPayload::Select)
    } else {
        (GeneratedBoxKind::Normal, GeneratedPayload::None)
    };

    let outer_display = OuterDisplay::from_css_display(display);
    let inner_formatting_context = if display.is_table_family() {
        FormattingContext::Table
    } else if display.is_flex() {
        FormattingContext::Flex
    } else if matches!(display, CssDisplay::Grid | CssDisplay::InlineGrid) {
        FormattingContext::Grid
    } else {
        FormattingContext::Flow
    };

    Some(GeneratedBox {
        node_id,
        kind,
        outer_display,
        inner_formatting_context,
        payload,
        children: generate_children(arena, node_id),
    })
}

/// Children of `parent_id` that participate in layout, in document order.
///
/// A `<details>` element without an `open` attribute renders only its first
/// `<summary>` child — the browser's closed disclosure widget hides every
/// other child (and everything when no `<summary>` exists) until opened. Any
/// other element returns all of its children unchanged.
pub fn visible_children(arena: &NodeArena, parent_id: NodeId) -> Vec<NodeId> {
    let parent = &arena[parent_id];
    let ids = parent.children.clone();
    if parent.node_type != NodeType::Details || parent.attributes.contains_key("open") {
        return ids;
    }
    match ids.iter().position(|&id| arena[id].node_type == NodeType::Summary) {
        Some(pos) => vec![ids[pos]],
        None => Vec::new(),
    }
}

fn generate_children(arena: &NodeArena, parent_id: NodeId) -> Vec<GeneratedBox> {
    let mut children = Vec::new();
    let mut inline_group = Vec::new();

    for &child_id in &visible_children(arena, parent_id) {
        // `display: contents` flattens the element's own children into
        // `parent_id`. We can't do that inside `generate_node` because
        // the wrapper is needed to *detect* the case here.
        let child_node = &arena[child_id];
        if let Some(d) = child_node.css_attr.display() && d.is_transparent() {
                for &grandchild_id in &child_node.children {
                    append_child_box(
                        arena,
                        parent_id,
                        grandchild_id,
                        &mut inline_group,
                        &mut children,
                    );
                }
                continue;
            
        }
        append_child_box(arena, parent_id, child_id, &mut inline_group, &mut children);
    }
    flush_inline_group(parent_id, &mut inline_group, &mut children);
    children
}

fn append_child_box(
    arena: &NodeArena,
    parent_id: NodeId,
    child_id: NodeId,
    inline_group: &mut Vec<GeneratedBox>,
    children: &mut Vec<GeneratedBox>,
) {
    let Some(child) = generate_node(arena, child_id) else {
        return;
    };
    if child.kind == GeneratedBoxKind::Text || child.outer_display.is_inline() {
        inline_group.push(child);
        return;
    }
    flush_inline_group(parent_id, inline_group, children);
    children.push(child);
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
        outer_display: OuterDisplay::BlockLevel { container: true },
        inner_formatting_context: FormattingContext::Flow,
        payload: GeneratedPayload::None,
        children: std::mem::take(inline_group),
    });
}

fn is_non_rendered(node_type: &NodeType) -> bool {
    matches!(
        node_type,
        NodeType::Head
            | NodeType::Style
            | NodeType::Link
            | NodeType::Meta
            | NodeType::Base
            | NodeType::Noscript
            | NodeType::Comment
            | NodeType::DocumentType
            | NodeType::DocumentFragment
    )
}