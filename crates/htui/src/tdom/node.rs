use std::collections::HashMap;
use std::fmt;

use slotmap::{Key, SlotMap, new_key_type};

use crate::html::css_parse::{CssAttr, CssDisplay, FontContext, LengthContext, Viewport, WhiteSpace};

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

#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub enum NodeType {
    /// Document root (`nodeType` 9). The synthetic root node inserted by the
    /// HTML parser; never rendered directly.
    Document,
    /// `<!DOCTYPE html>` declaration (`nodeType` 10). The doctype name lives
    /// in [`Node::data`]; never rendered.
    DocumentType,
    /// Document fragment (`nodeType` 11). Created via
    /// `document.createDocumentFragment()`; renders nothing until its
    /// children are appended elsewhere.
    DocumentFragment,
    /// Text node (`nodeType` 3). Content lives in [`Node::data`].
    Text,
    /// Comment node (`nodeType` 8). Content lives in [`Node::data`]; never
    /// rendered.
    Comment,
    Span,
    Div,
    P,
    Pre,
    Img,
    A,
    /// `<em>` — emphasis. Inline element with browser-default `font-style:
    /// italic` that the layout pass applies when neither the element nor an
    /// ancestor has declared an explicit `font-style`.
    Em,
    Html,
    Body,
    Head,
    Title,
    Meta,
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
    Table,
    Thead,
    Tbody,
    Tfoot,
    Tr,
    Th,
    Td,
    Caption,
    Colgroup,
    Col,
    Input,
    Form,
    Select,
    HtmlOption,
    Button,
    Textarea,
    Label,
    Style,
    Link,
}

impl NodeType {
    /// Every [`NodeType`] variant, in declaration order. The single
    /// enumeration used by [`NodeType::from_tag_name`] and by completeness
    /// tests that prove every element variant is addressable (CSS type
    /// selector, HTML parser, …).
    pub const ALL: &[NodeType] = &[
        NodeType::Document,
        NodeType::DocumentType,
        NodeType::DocumentFragment,
        NodeType::Text,
        NodeType::Comment,
        NodeType::Span,
        NodeType::Div,
        NodeType::P,
        NodeType::Pre,
        NodeType::Img,
        NodeType::A,
        NodeType::Em,
        NodeType::Html,
        NodeType::Body,
        NodeType::Head,
        NodeType::Title,
        NodeType::Meta,
        NodeType::H1,
        NodeType::H2,
        NodeType::H3,
        NodeType::H4,
        NodeType::H5,
        NodeType::Header,
        NodeType::Footer,
        NodeType::Nav,
        NodeType::Main,
        NodeType::Section,
        NodeType::Article,
        NodeType::Aside,
        NodeType::Figure,
        NodeType::Figcaption,
        NodeType::Details,
        NodeType::Summary,
        NodeType::Ul,
        NodeType::Ol,
        NodeType::Li,
        NodeType::Table,
        NodeType::Thead,
        NodeType::Tbody,
        NodeType::Tfoot,
        NodeType::Tr,
        NodeType::Th,
        NodeType::Td,
        NodeType::Caption,
        NodeType::Colgroup,
        NodeType::Col,
        NodeType::Input,
        NodeType::Form,
        NodeType::Select,
        NodeType::HtmlOption,
        NodeType::Button,
        NodeType::Textarea,
        NodeType::Label,
        NodeType::Style,
        NodeType::Link,
    ];

    pub fn is_self_closing(&self) -> bool {
        matches!(self, NodeType::Img | NodeType::Input | NodeType::Meta)
    }

    /// Browser-style UA `display` default for this element.
    ///
    /// Returns the `display` value the element would carry if neither
    /// author CSS nor an inherited value has set `display` on it. The
    /// matches mirror the HTML5 spec / browser defaults:
    ///
    /// - `<img>` → `inline-block` (it's a replaced element)
    /// - `<table>` / `inline-table`-family → their table keyword
    /// - `<li>` → `list-item`
    /// - everything else → `flow` (block-level block container)
    pub fn ua_default_display(&self) -> CssDisplay {
        match self {
            NodeType::Img => CssDisplay::InlineBlock,
            // Inline text-level elements: `<span>` / `<a>` / `<em>` participate
            // in the parent's inline flow. Classifying them block-level would
            // make intrinsic measurement sum each one as a full block row and
            // inflate flex-item / column heights.
            NodeType::Span | NodeType::A | NodeType::Em => CssDisplay::Inline,
            NodeType::Table => CssDisplay::Table,
            NodeType::Thead => CssDisplay::TableHeaderGroup,
            NodeType::Tbody => CssDisplay::TableRowGroup,
            NodeType::Tfoot => CssDisplay::TableFooterGroup,
            NodeType::Tr => CssDisplay::TableRow,
            NodeType::Th | NodeType::Td => CssDisplay::TableCell,
            NodeType::Caption => CssDisplay::TableCaption,
            NodeType::Colgroup => CssDisplay::TableColumnGroup,
            NodeType::Col => CssDisplay::TableColumn,
            NodeType::Li => CssDisplay::ListItem,
            _ => CssDisplay::Flow,
        }
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

    /// Browser-style UA `white-space` default for this element type.
    ///
    /// Mirrors the HTML5 user-agent stylesheet: `<pre>` ships with
    /// `white-space: pre` so leading whitespace, tabs, and newlines are
    /// preserved verbatim. Every other element returns `None` — the
    /// layout falls back to the regular `WhiteSpace::Normal` default.
    pub fn default_white_space(&self) -> Option<WhiteSpace> {
        match self {
            NodeType::Pre => Some(WhiteSpace::Pre),
            _ => None,
        }
    }

    /// Lowercase HTML tag name for element variants, `None` for the
    /// document-structure nodes (`Document` / `DocumentType` /
    /// `DocumentFragment` / `Text` / `Comment`).
    pub fn tag_name(&self) -> Option<&'static str> {
        Some(match self {
            NodeType::Span => "span",
            NodeType::Div => "div",
            NodeType::P => "p",
            NodeType::Pre => "pre",
            NodeType::Img => "img",
            NodeType::A => "a",
            NodeType::Em => "em",
            NodeType::Html => "html",
            NodeType::Body => "body",
            NodeType::Head => "head",
            NodeType::Title => "title",
            NodeType::Meta => "meta",
            NodeType::H1 => "h1",
            NodeType::H2 => "h2",
            NodeType::H3 => "h3",
            NodeType::H4 => "h4",
            NodeType::H5 => "h5",
            NodeType::Header => "header",
            NodeType::Footer => "footer",
            NodeType::Nav => "nav",
            NodeType::Main => "main",
            NodeType::Section => "section",
            NodeType::Article => "article",
            NodeType::Aside => "aside",
            NodeType::Figure => "figure",
            NodeType::Figcaption => "figcaption",
            NodeType::Details => "details",
            NodeType::Summary => "summary",
            NodeType::Ul => "ul",
            NodeType::Ol => "ol",
            NodeType::Li => "li",
            NodeType::Table => "table",
            NodeType::Thead => "thead",
            NodeType::Tbody => "tbody",
            NodeType::Tfoot => "tfoot",
            NodeType::Tr => "tr",
            NodeType::Th => "th",
            NodeType::Td => "td",
            NodeType::Caption => "caption",
            NodeType::Colgroup => "colgroup",
            NodeType::Col => "col",
            NodeType::Input => "input",
            NodeType::Form => "form",
            NodeType::Select => "select",
            NodeType::HtmlOption => "option",
            NodeType::Button => "button",
            NodeType::Textarea => "textarea",
            NodeType::Label => "label",
            NodeType::Style => "style",
            NodeType::Link => "link",
            _ => return None,
        })
    }

    /// Reverse of [`NodeType::tag_name`]: the element variant whose lowercase
    /// tag name is `tag`, or `None` when `tag` does not name a supported
    /// element (document-structure nodes have no tag name and never match).
    ///
    /// Derived from [`NodeType::ALL`] + [`NodeType::tag_name`] so there is
    /// exactly one tag table to keep in sync. The CSS selector parser
    /// ([`crate::html::css_parse::selector`]) and the HTML parser
    /// ([`crate::html::html_parse::tag_name_to_node_type`]) both delegate
    /// here.
    pub fn from_tag_name(tag: &str) -> Option<NodeType> {
        let t = tag.to_ascii_lowercase();
        Self::ALL
            .iter()
            .copied()
            .find(|nt| nt.tag_name() == Some(t.as_str()))
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
    /// Raw HTML attributes as a `(name, value)` map.
    ///
    /// Use the inherent helpers below (`get_id`, `get_class`, …) for the few
    /// lookups that carry domain semantics; everything else should go through
    /// `attributes.get(...)` directly so the surface stays minimal.
    pub attributes: HashMap<String, String>,
    pub css_attr: CssAttr,
}

impl Node {
    pub fn from(node_type: NodeType) -> Node {
        Node {
            node_type,
            data: None,
            children: vec![],
            parent: None,
            attributes: HashMap::new(),
            css_attr: CssAttr::new(),
        }
    }

    pub fn text(data: &str) -> Node {
        Node {
            node_type: NodeType::Text,
            data: Some(data.to_string()),
            children: vec![],
            parent: None,
            attributes: HashMap::new(),
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
            attributes: HashMap::new(),
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
            attributes: HashMap::new(),
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
            attributes: HashMap::new(),
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
        !s.chars().all(char::is_whitespace)
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
