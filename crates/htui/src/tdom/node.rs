use std::fmt;

use indexmap::IndexMap;
use slotmap::{Key, SlotMap, new_key_type};

use crate::html::css_parse::{CssAttr, FontContext, LengthContext, Viewport, WhiteSpace};

mod node_type;

pub use node_type::NodeType;

// ============================================================
// NodeId — slotmap 生成的类型安全索引
// ============================================================

new_key_type! {
    /// DOM tree node ID.
    ///
    /// Allocated by `SlotMap<NodeId, Node>`, guarantees:
    ///   - Type-safe (not interchangeable with plain u64)
    ///   - Versioned (accessing a removed node will panic)
    ///   - Copy + Send + Sync
    pub struct NodeId;
}

/// Global DOM node storage.
pub type NodeArena = SlotMap<NodeId, Node>;

/// The document base URL (`<base href>`), if one is present.
///
/// Walks `from` up to the document root, then scans the tree for the first
/// `<base>` element carrying a non-empty `href` attribute — mirroring the
/// browser rule that the first `href`-bearing `<base>` establishes the
/// document base URL. Relative resource references (`<script src>`,
/// `<link href>`, `<img src>`, JS `fetch`) resolve against the returned
/// value via [`crate::resources::resolve_url`].
///
/// Calling with any node id works: the lookup only needs to reach the
/// document root through the `parent` chain.
pub fn document_base_href(arena: &NodeArena, from: NodeId) -> Option<String> {
    let mut root = from;
    while let Some(parent) = arena[root].parent {
        root = parent;
    }
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let node = &arena[id];
        if node.node_type == NodeType::Base
            && let Some(href) = node.attributes.get("href")
            && !href.trim().is_empty()
        {
            return Some(href.clone());
        }
        stack.extend(node.children.iter().copied());
    }
    None
}

/// Resolve the effective font-size of `node_id` to a concrete cell count.
///
/// CSS precedence (highest to lowest):
/// 1. Explicit `font-size` rule on the node
/// 2. The element's built-in default (headings only — see
///    [`NodeType::default_font_size_em`])
/// 3. The inherited value from `parent_font_size_cells` (which itself
///    follows the same chain)
///
/// The root element's font-size is **always** `1` cell unless a CSS rule
/// explicitly sets `font-size` on `<html>` (rendered as the
/// [`NodeType::Document`] root). `em` units inside `font-size` resolve
/// against `parent_font_size_cells`; `rem` units resolve against
/// `root_font_size_cells`.
pub fn effective_font_size_cells(
    arena: &NodeArena,
    node_id: NodeId,
    font_ctx: FontContext,
    viewport: Viewport,
) -> usize {
    let node = &arena[node_id];
    let declared = node.css_attr.get_font_size();
    if let Some(len) = declared {
        // `font-size` never resolves as a percentage, so `containing` is
        // unused here — pass `0` rather than borrowing the parent's font-size
        // to keep the placeholder obvious.
        return len.resolve(LengthContext {
            containing: 0,
            font: font_ctx,
            viewport,
        });
    }
    if let Some(em) = node.node_type.default_font_size_em() {
        let raw = em * font_ctx.font_size_cells as f32;
        return raw.round().max(1.0) as usize;
    }
    font_ctx.font_size_cells.max(1)
}

// ============================================================
// Node — 纯数据，不含所有权
// ============================================================

#[derive(Debug, Clone)]
pub struct Node {
    pub node_type: NodeType,
    /// Text / comment content ([`NodeType::Text`] / [`NodeType::Comment`]) or
    /// the doctype name ([`NodeType::DocumentType`]).
    pub data: Option<String>,
    /// Child node IDs (insertion order).
    pub children: Vec<NodeId>,
    /// Parent node ID. `None` for the root node.
    pub parent: Option<NodeId>,
    /// Raw HTML attributes as a `(name, value)` map preserving insertion
    /// order (the `innerHTML` serializer emits them in source order, like a
    /// browser).
    ///
    /// Use the inherent helpers below (`get_id`, `get_class`, …) for the few
    /// lookups that carry domain semantics; everything else should go through
    /// `attributes.get(...)` directly so the surface stays minimal.
    pub attributes: IndexMap<String, String>,
    pub css_attr: CssAttr,
}

/// Whether `c` is one of the HTML/CSS collapsible white-space characters
/// (space, tab, and the segment-break set). Non-breaking space (`U+00A0`) is
/// deliberately excluded: browsers treat it as a non-collapsible, non-breaking
/// space that must survive `white-space: normal` collapsing.
pub(crate) fn is_collapsible_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000C}')
}

impl Node {
    pub fn from(node_type: NodeType) -> Node {
        Node {
            node_type,
            data: None,
            children: vec![],
            parent: None,
            attributes: IndexMap::new(),
            css_attr: CssAttr::new(),
        }
    }

    pub fn text(data: &str) -> Node {
        Node {
            node_type: NodeType::Text,
            data: Some(data.to_string()),
            children: vec![],
            parent: None,
            attributes: IndexMap::new(),
            css_attr: CssAttr::new(),
        }
    }

    /// Build a comment node (`nodeType` 8).
    pub fn comment(data: &str) -> Node {
        Node {
            node_type: NodeType::Comment,
            data: Some(data.to_string()),
            children: vec![],
            parent: None,
            attributes: IndexMap::new(),
            css_attr: CssAttr::new(),
        }
    }

    /// Build a detached document fragment (`nodeType` 11).
    pub fn fragment() -> Node {
        Node {
            node_type: NodeType::DocumentFragment,
            data: None,
            children: vec![],
            parent: None,
            attributes: IndexMap::new(),
            css_attr: CssAttr::new(),
        }
    }

    /// Build a document-type node (`nodeType` 10) carrying the doctype name.
    pub fn doctype(name: &str) -> Node {
        Node {
            node_type: NodeType::DocumentType,
            data: Some(name.to_string()),
            children: vec![],
            parent: None,
            attributes: IndexMap::new(),
            css_attr: CssAttr::new(),
        }
    }

    /// DOM `nodeType` constant (the spec's `ELEMENT_NODE`…`DOCUMENT_FRAGMENT_NODE`
    /// values).
    pub fn node_type_constant(&self) -> u8 {
        match self.node_type {
            NodeType::Document => 9,
            NodeType::DocumentType => 10,
            NodeType::DocumentFragment => 11,
            NodeType::Text => 3,
            NodeType::Comment => 8,
            _ => 1,
        }
    }

    /// DOM `nodeName`: `"#text"` / `"#comment"` / `"#document"` for the
    /// document-structure nodes, the uppercased tag name for elements (e.g.
    /// `"DIV"`), and the doctype name for a document type.
    pub fn node_name(&self) -> String {
        match self.node_type {
            NodeType::Document => "#document".to_string(),
            NodeType::DocumentType => self.data.clone().unwrap_or_else(|| "html".to_string()),
            NodeType::DocumentFragment => "#document-fragment".to_string(),
            NodeType::Text => "#text".to_string(),
            NodeType::Comment => "#comment".to_string(),
            _ => self.node_type.tag_name().unwrap_or_default().to_uppercase(),
        }
    }

    /// Whether this text node contributes rendered / measured content under
    /// the given `white-space` mode: pure-whitespace runs are dropped when the
    /// inline context collapses whitespace (`normal` / `nowrap` / `pre-line`),
    /// while `pre` / `pre-wrap` preserve them verbatim and `pre-line` keeps
    /// newlines. Keeps the DOM faithful to the source while the rendered
    /// output stays unchanged.
    pub fn text_is_painted(&self, white_space: WhiteSpace) -> bool {
        let Some(s) = self.data.as_deref() else {
            return true;
        };
        if white_space.preserves_whitespace() {
            return true;
        }
        if matches!(white_space, WhiteSpace::PreLine) && s.contains('\n') {
            return true;
        }
        !s.chars().all(is_collapsible_whitespace)
    }

    /// The effective `white-space` governing this node's inline content: the
    /// node's own declaration when it has one, otherwise the parent inline
    /// context's mode. Browsers inherit `white-space`; the cascade does not,
    /// so the inline walk threads it explicitly.
    pub fn white_space_or(&self, parent: WhiteSpace) -> WhiteSpace {
        if self.css_attr.is_set("white-space") {
            self.css_attr.get_white_space(&self.node_type)
        } else {
            parent
        }
    }

    /// The display label of an `<input>` push-button element: the `value`
    /// attribute, or a browser-appropriate default when absent (`submit` →
    /// `Submit`, `reset` → `Reset`, `image` → `alt` / `Submit Query`).
    /// Returns `None` for inputs that are not push-buttons.
    pub fn input_button_label(&self) -> Option<String> {
        let input_type = self.get_type()?.to_ascii_lowercase();
        if !matches!(input_type.as_str(), "button" | "submit" | "reset" | "image") {
            return None;
        }
        let value = self.attributes.get("value").cloned().unwrap_or_default();
        match input_type.as_str() {
            "submit" if value.is_empty() => Some("Submit".to_string()),
            "reset" if value.is_empty() => Some("Reset".to_string()),
            // `image` is a submit button whose label is the `alt` text (or
            // the `value`, falling back to the browser's default query name).
            "image" if value.is_empty() => {
                let alt = self.attributes.get("alt").cloned().unwrap_or_default();
                if alt.is_empty() {
                    Some("Submit Query".to_string())
                } else {
                    Some(alt)
                }
            }
            _ => Some(value),
        }
    }

    /// Append a child node to the current node (also sets child's parent).
    pub fn append_child(arena: &mut NodeArena, parent_id: NodeId, child_id: NodeId) {
        let child_parent = &mut arena[child_id].parent;
        assert!(child_parent.is_none(), "Child already has a parent");
        *child_parent = Some(parent_id);
        arena[parent_id].children.push(child_id);
    }

    /// Number of child nodes.
    pub fn child_count(&self) -> usize {
        self.children.len()
    }

    /// Iterate over child node IDs.
    pub fn child_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.children.iter().copied()
    }

    /// Concatenated text of every descendant [`NodeType::Text`] node in
    /// document order (the DOM `textContent` value). `<textarea>` uses this
    /// as its initial value — textarea content lives in child text nodes,
    /// not an attribute.
    pub fn text_content(&self, arena: &NodeArena) -> String {
        let mut out = String::new();
        let mut stack: Vec<NodeId> = self.children.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let node = &arena[id];
            if node.node_type == NodeType::Text
                && let Some(s) = &node.data
            {
                out.push_str(s);
            }
            stack.extend(node.children.iter().rev().copied());
        }
        out
    }

    /// Depth-first traversal of all descendants (including self).
    pub fn descendants<'a>(arena: &'a NodeArena, start_id: NodeId) -> Descendants<'a> {
        Descendants {
            arena,
            stack: vec![start_id],
        }
    }
}

// 迭代器

pub struct Descendants<'a> {
    arena: &'a NodeArena,
    stack: Vec<NodeId>,
}

impl<'a> Iterator for Descendants<'a> {
    type Item = (NodeId, &'a Node);

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.stack.pop()?;
        let node = &self.arena[id];
        // 子节点逆序入栈保证前序遍历
        for &child_id in node.children.iter().rev() {
            self.stack.push(child_id);
        }
        Some((id, node))
    }
}

// 使 NodeId 可在 logging 中使用

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // NodeId 内部是 u64，这里只显示低 32 位版本号
        write!(f, "n#{}", self.data().as_ffi() as u32)
    }
}
