use crate::{html::css_parse::CssAttr, tdom::attributes::Attr};

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
    Text(String),
}

impl NodeType {
    pub fn is_self_closing(&self) -> bool {
        match self {
            NodeType::Img => true,
            NodeType::Input => true,
            _ => false,
        }
    }
}

#[derive(Debug)]
pub struct Node {
    pub node_type: NodeType,
    pub children: Vec<Node>,
    pub attributes: Attr,
    pub css_attr: CssAttr,
    /// 父节点指针，树构建完成后通过 `fix_parent_pointers()` 填充。
    /// 仅在 CSS 匹配阶段（树稳定后）访问，原始指针在此期间有效。
    pub parent: Option<*const Node>,
}

impl Node {
    pub fn new() -> Node {
        Node {
            node_type: NodeType::Global,
            children: vec![],
            attributes: Attr::new(),
            css_attr: CssAttr::new(),
            parent: None,
        }
    }
    pub fn from(node_type: NodeType) -> Node {
        Node {
            node_type,
            children: vec![],
            attributes: Attr::new(),
            css_attr: CssAttr::new(),
            parent: None,
        }
    }
    pub fn children(&self) -> impl Iterator<Item = &Node> {
        self.children.iter()
    }
    pub fn text(data: &str) -> Node {
        Node {
            node_type: NodeType::Text(data.to_string()),
            children: vec![],
            attributes: Attr::new(),
            css_attr: CssAttr::new(),
            parent: None,
        }
    }
    pub fn div() -> Node {
        Node {
            node_type: NodeType::Div,
            children: vec![],
            attributes: Attr::new(),
            css_attr: CssAttr::new(),
            parent: None,
        }
    }

    /// 递归设置所有子节点的 `parent` 指针。
    ///
    /// 必须在树构建完毕后、CSS 匹配 / Layout 之前调用。
    /// 未来若重构为 arena（如 slotmap）存储，此方法改为填充索引。
    pub fn fix_parent_pointers(&mut self) {
        let self_ptr = self as *const Node;
        for child in &mut self.children {
            child.parent = Some(self_ptr);
            child.fix_parent_pointers();
        }
    }

    /// 获取父节点引用（仅在 `parent` 有效时调用）
    pub fn parent_ref(&self) -> Option<&Node> {
        // Safety: parent 指针在 fix_parent_pointers 后有效，
        // 且在 CSS/Layout 期间树不发生移动。
        self.parent.map(|p| unsafe { &*p })
    }
}

// ============================================================
// 未来 Node 重构指引
// ============================================================
//
// 当前 Node 使用 `Vec<Node>` 自有树 + `*const Node` 父指针。
// 这种设计简单直接，但有以下局限：
//   1. 原始指针在树修改后可能悬空
//   2. `Vec` 扩容时子节点地址会变
//   3. 无法在删除节点后保持指针有效
//
// 若需要更安全的父节点导航（如支持 DOM 操作），推荐以下方案：
//
// 方案 A: slotmap 索引（推荐）
//   - 将 Node 存入 `SlotMap<NodeId, Node>`（crate: slotmap）
//   - parent 改为 `Option<NodeId>`
//   - children 改为 `Vec<NodeId>`
//   - 优势：无 unsafe，O(1) 访问，增删安全
//   - 代价：所有遍历需要传入 &SlotMap
//
// 方案 B: Rc<RefCell<Node>> 
//   - 用 Weak 做 parent，Rc 做 children
//   - 优势：内存安全，自动管理生命周期
//   - 代价：运行时 borrow check 开销，循环引用需手动处理
//
// 方案 C: indextree arena
//   - crate: indextree，专用的树 arena
//   - 优势：API 完整（parent/children/siblings 均 O(1)）
//   - 代价：所有 Node 访问需通过 arena
//
// 迁移步骤（以方案 A 为例）：
//   1. 引入 slotmap，定义 `type NodeId = slotmap::new_key_type!();`
//   2. Node 改存 NodeId 而非 *const Node
//   3. 全局 `SlotMap<NodeId, Node>` 管理所有权
//   4. html_parse 构建时返回 (NodeId, SlotMap)
//   5. css_into_node / layout 遍历传 &SlotMap
