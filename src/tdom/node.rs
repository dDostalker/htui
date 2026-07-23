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
    None,
    Global,
    Span,
    Div,
    P,
    Img,
    A,
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
    pub fn new() -> Node {
        Node {
            node_type: NodeType::Global,
            children: vec![],
            parent: None,
            attributes: Attr::new(),
            css_attr: CssAttr::new(),
        }
    }

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

    pub fn child(&self, index: usize) -> Option<NodeId> {
        self.children.get(index).copied()
    }

    /// Number of child nodes.
    pub fn child_count(&self) -> usize {
        self.children.len()
    }

    /// Iterate over child node IDs.
    pub fn child_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.children.iter().copied()
    }

    /// Get the previous sibling node.
    pub fn prev_sibling(&self, arena: &NodeArena) -> Option<NodeId> {
        let parent_id = self.parent?;
        let parent = &arena[parent_id];
        let mut prev = None;
        for &id in &parent.children {
            if id == self.node_id(arena) {
                return prev;
            }
            prev = Some(id);
        }
        None
    }

    /// Get the next sibling node.
    pub fn next_sibling(&self, arena: &NodeArena) -> Option<NodeId> {
        let parent_id = self.parent?;
        let parent = &arena[parent_id];
        let children = &parent.children;
        let self_id = self.node_id(arena);
        for (i, &id) in children.iter().enumerate() {
            if id == self_id && i + 1 < children.len() {
                return Some(children[i + 1]);
            }
        }
        None
    }

    /// Recursively rebuild parent pointers. Call once after tree construction.
    /// (Normally maintained automatically by `append_child`; this is a bulk-repair tool.)
    pub fn fix_parent_pointers(arena: &mut NodeArena, node_id: NodeId) {
        let children: Vec<NodeId> = arena[node_id].children.clone();
        for child_id in children {
            arena[child_id].parent = Some(node_id);
            Self::fix_parent_pointers(arena, child_id);
        }
    }

    /// Depth-first traversal of all descendants (including self).
    pub fn descendants<'a>(arena: &'a NodeArena, start_id: NodeId) -> Descendants<'a> {
        Descendants {
            arena,
            stack: vec![start_id],
        }
    }

    fn node_id(&self, arena: &NodeArena) -> NodeId {
        // 在父节点的 children 中查找自己
        if let Some(parent_id) = self.parent {
            let parent = &arena[parent_id];
            let self_ptr = self as *const Node;
            for &id in &parent.children {
                if std::ptr::eq(&arena[id] as *const Node, self_ptr) {
                    return id;
                }
            }
        }
        panic!("Node not found in its parent's children");
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

pub fn collect_text(arena: &NodeArena, node: &Node) -> String {
    let mut s = String::new();
    for child_id in node.child_ids() {
        if let NodeType::Text(t) = &arena[child_id].node_type {
            s.push_str(t);
        }
    }
    s
}
