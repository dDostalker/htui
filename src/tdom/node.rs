use std::fmt;

use slotmap::{Key, SlotMap, new_key_type};

use crate::html::css_parse::CssAttr;
use crate::tdom::attributes::Attr;

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

// ============================================================
// NodeType
// ============================================================

#[derive(PartialEq, Eq, Debug, Clone, Hash)]
pub enum NodeType {
    Global,
    Span,
    Div,
    P,
    Img,
    A,
    Html,
    Body,
    Head,
    H1,
    H2,
    H3,
    H4,
    H5,
    Header,
    Footer,
    Nav,
    Main,
    Section,
    Article,
    Aside,
    Figure,
    Figcaption,
    Details,
    Summary,
    Ul,
    Ol,
    Li,
    Input,
    Form,
    Select,
    HtmlOption,
    Button,
    Textarea,
    Label,
    Style,
    Link,
    Text(String),
}

impl NodeType {
    pub fn is_self_closing(&self) -> bool {
        matches!(self, NodeType::Img | NodeType::Input)
    }

    /// CSS default `font-size` in `em` units for this element type.
    ///
    /// Browser defaults for headings (CSS 2.1 / user-agent stylesheet):
    /// `h1 = 2em`, `h2 = 1.5em`, `h3 = 1.17em`, `h4 = 1em`,
    /// `h5 = 0.83em`. Every other element inherits the parent's font-size
    /// (returns `None`).
    ///
    /// Values are rounded toward the nearest integer cell when applied
    /// (`1.5em` → 2 cells, `0.83em` → 1 cell); see
    /// [`default_font_size_cells_for`].
    pub fn default_font_size_em(&self) -> Option<f32> {
        match self {
            NodeType::H1 => Some(2.0),
            NodeType::H2 => Some(1.5),
            NodeType::H3 => Some(1.17),
            NodeType::H4 => Some(1.0),
            NodeType::H5 => Some(0.83),
            _ => None,
        }
    }
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
/// [`NodeType::Global`] root). `em` units inside `font-size` resolve
/// against `parent_font_size_cells`; `rem` units resolve against
/// `root_font_size_cells`.
pub fn effective_font_size_cells(
    arena: &NodeArena,
    node_id: NodeId,
    parent_font_size_cells: usize,
    root_font_size_cells: usize,
) -> usize {
    let node = &arena[node_id];
    let declared = node.css_attr.get_font_size();
    if let Some(len) = declared {
        return len.resolve(parent_font_size_cells, parent_font_size_cells, root_font_size_cells);
    }
    if let Some(em) = node.node_type.default_font_size_em() {
        let raw = em * parent_font_size_cells as f32;
        return raw.round().max(1.0) as usize;
    }
    parent_font_size_cells.max(1)
}

// ============================================================
// Node — 纯数据，不含所有权
// ============================================================

#[derive(Debug)]
pub struct Node {
    pub node_type: NodeType,
    /// Child node IDs (insertion order).
    pub children: Vec<NodeId>,
    /// Parent node ID. `None` for the root node.
    pub parent: Option<NodeId>,
    pub attributes: Attr,
    pub css_attr: CssAttr,
}

impl Node {
    pub fn from(node_type: NodeType) -> Node {
        Node {
            node_type,
            children: vec![],
            parent: None,
            attributes: Attr::new(),
            css_attr: CssAttr::new(),
        }
    }

    pub fn text(data: &str) -> Node {
        Node {
            node_type: NodeType::Text(data.to_string()),
            children: vec![],
            parent: None,
            attributes: Attr::new(),
            css_attr: CssAttr::new(),
        }
    }

    // ============================================================
    // 树操作辅助（需配合 arena 使用）
    // ============================================================

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
