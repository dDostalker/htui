use anyhow::Result;
use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use html5ever::Attribute;
use log::{debug, trace, warn};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::tdom::{
    attributes::Attr,
    node::{Node, NodeType},
};

/// HTML 解析器，基于 html5ever 库实现
///
/// 将 HTML 字符串解析为项目内部的 tdome (Node) 树结构。
/// 保持了与原手写解析器相同的 `Parser::new(html)` / `Parser::parse()` 接口。
#[derive(Debug)]
pub struct Parser {
    input: String,
}

impl Parser {
    pub fn new(input: &str) -> Self {
        debug!("创建 html5ever Parser，输入长度 {} 字节", input.len());
        Self {
            input: input.to_string(),
        }
    }

    /// 解析 HTML 并返回 Node 树
    ///
    /// 使用 html5ever 进行符合 HTML5 规范的解析，
    /// 然后将 RcDom 树转换为项目内部的 Node 树。
    pub fn parse(&mut self) -> Result<Node> {
        debug!("开始使用 html5ever 解析 HTML，输入长度 {} 字节", self.input.len());

        let opts = ParseOpts::default();
        let dom = html5ever::parse_document(RcDom::default(), opts).one(self.input.clone());

        // 以 Global 类型作为根节点，从 Document 开始递归转换
        let mut root = Node::new();
        convert_children(&dom.document, &mut root);

        // 填充 parent 指针，供 CSS selectors 匹配引擎使用
        root.fix_parent_pointers();

        debug!(
            "html5ever 解析完成，根节点子节点数量: {}",
            root.children.len()
        );
        Ok(root)
    }
}

/// 将 html5ever 的 Attribute 转换为项目内部的 Attr
///
/// 通过将属性序列化为 `key="value"` 字符串后，
/// 复用 Attr::parse_from_str 进行解析。
fn convert_attributes(html5_attrs: &[Attribute]) -> Attr {
    if html5_attrs.is_empty() {
        return Attr::new();
    }

    let attr_str: String = html5_attrs
        .iter()
        .map(|a| format!("{}=\"{}\"", a.name.local, a.value))
        .collect::<Vec<_>>()
        .join(" ");

    trace!("转换属性: '{}'", attr_str);
    Attr::parse_from_str(&attr_str)
}

/// 从 html5ever 的 Handle (Rc<Node>) 递归转换子节点并添加到 parent
fn convert_children(handle: &Handle, parent: &mut Node) {
    let children = handle.children.borrow().clone();

    for child_handle in &children {
        match &child_handle.data {
            NodeData::Document => {
                // Document 节点本身不创建 Node，只递归处理其子节点
                trace!("遇到 Document 节点，递归处理子节点");
                convert_children(child_handle, parent);
            }
            NodeData::Element {
                name,
                attrs,
                ..
            } => {
                let tag = name.local.as_ref();
                trace!("遇到元素标签: <{}>", tag);

                match tag_name_to_node_type(tag) {
                    Some(node_type) => {
                        debug!("识别标签 <{}> → {:?}", tag, node_type);

                        let mut child_node = Node::from(node_type);
                        // 转换属性
                        let attrs_ref = attrs.borrow();
                        child_node.attributes = convert_attributes(&attrs_ref);

                        // 递归处理子节点
                        convert_children(child_handle, &mut child_node);
                        parent.children.push(child_node);
                    }
                    None => {
                        // 不支持的标签：压平，跳过该元素节点但递归处理其子节点
                        warn!(
                            "不支持的 HTML 标签 <{}>，压平处理（子节点提升到父节点）",
                            tag
                        );
                        convert_children(child_handle, parent);
                    }
                }
            }
            NodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                let trimmed = text.trim().to_string();

                if !trimmed.is_empty() {
                    trace!("发现文本节点 (长度 {}): '{}'", trimmed.len(), trimmed);
                    parent.children.push(Node::text(&trimmed));
                } else {
                    trace!("跳过空白文本节点");
                }
            }
            NodeData::Comment { contents } => {
                trace!("跳过注释节点: '{}'", contents);
            }
            NodeData::Doctype { .. } | NodeData::ProcessingInstruction { .. } => {
                trace!("跳过 Doctype / ProcessingInstruction 节点");
                // 这些节点类型也递归处理子节点（理论上它们没有子节点）
                convert_children(child_handle, parent);
            }
        }
    }
}

/// 将 HTML 标签名映射为项目内部的 NodeType
///
/// 支持项目已定义的所有 HTML 元素类型。
/// 不支持的元素返回 None，调用方应进行压平处理。
fn tag_name_to_node_type(tag: &str) -> Option<NodeType> {
    match tag.to_ascii_lowercase().as_str() {
        "a" => Some(NodeType::A),
        "article" => Some(NodeType::Article),
        "aside" => Some(NodeType::Aside),
        "button" => Some(NodeType::Button),
        "details" => Some(NodeType::Details),
        "div" => Some(NodeType::Div),
        "figcaption" => Some(NodeType::Figcaption),
        "figure" => Some(NodeType::Figure),
        "footer" => Some(NodeType::Footer),
        "form" => Some(NodeType::Form),
        "h1" => Some(NodeType::H1),
        "h2" => Some(NodeType::H2),
        "h3" => Some(NodeType::H3),
        "h4" => Some(NodeType::H4),
        "h5" => Some(NodeType::H5),
        "header" => Some(NodeType::Header),
        "img" => Some(NodeType::Img),
        "input" => Some(NodeType::Input),
        "label" => Some(NodeType::Label),
        "li" => Some(NodeType::Li),
        "main" => Some(NodeType::Main),
        "nav" => Some(NodeType::Nav),
        "ol" => Some(NodeType::Ol),
        "option" => Some(NodeType::HtmlOption),
        "p" => Some(NodeType::P),
        "section" => Some(NodeType::Section),
        "select" => Some(NodeType::Select),
        "span" => Some(NodeType::Span),
        "summary" => Some(NodeType::Summary),
        "textarea" => Some(NodeType::Textarea),
        "ul" => Some(NodeType::Ul),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test_tag_name_to_node_type() {
        init();
        assert_eq!(tag_name_to_node_type("span"), Some(NodeType::Span));
        assert_eq!(tag_name_to_node_type("div"), Some(NodeType::Div));
        assert_eq!(tag_name_to_node_type("DIV"), Some(NodeType::Div)); // 大小写不敏感
        assert_eq!(tag_name_to_node_type("unsupported"), None);
    }

    #[test]
    fn test_parse_basic_html() {
        init();
        let html = "<div><p>Hello</p><p>World</p></div>";
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();
        dbg!(&node);

        // 验证根节点有子节点
        assert!(!node.children.is_empty());
    }

    #[test]
    fn test_parse_with_attributes() {
        init();
        let html = r#"<div class="container" id="main"><p>Hello</p></div>"#;
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();
        dbg!(&node);
    }

    #[test]
    fn test_parse_img() {
        init();
        let html = r#"<div><img src="test.jpg" alt="Test Image"></div>"#;
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();
        dbg!(&node);
    }

    #[test]
    fn test_parse_nested_headings() {
        init();
        let html = r#"
            <div>
                <h1>Chapter 1</h1>
                <p>Introduction</p>
                <h2>Section 1.1</h2>
                <p>Content</p>
                <h2>Section 1.2</h2>
                <p>More content</p>
            </div>
        "#;
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();
        dbg!(&node);
    }

    #[test]
    fn test_parse_heading_with_attributes() {
        init();
        let html = r#"<h1 id="title" class="main-title" src="./1/2/3">Welcome</h1>"#;
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();

        // 验证解析结果 - html5ever 会包裹 <html><head></head><body>...</body></html>
        // h1 应能在文档树中找到
        fn find_h1(node: &Node) -> Option<&Node> {
            if node.node_type == NodeType::H1 {
                return Some(node);
            }
            for child in &node.children {
                if let Some(found) = find_h1(child) {
                    return Some(found);
                }
            }
            None
        }

        let h1 = find_h1(&node).expect("应该能找到 H1 节点");
        assert_eq!(h1.node_type, NodeType::H1);
        assert_eq!(h1.attributes.get("id"), Some(&"title".to_string()));
        assert_eq!(h1.attributes.get("src"), Some(&"./1/2/3".to_string()));
    }

    #[test]
    fn test_parse_full_document() {
        init();
        let html = r#"<html><body><h1>Title</h1><div id="main" class="test"><p>Hello world!</p></div></body></html>"#;
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();
        dbg!(&node);

        // html/body 标签不被项目支持，会被压平
        // 最终 h1 和 div 应该作为根节点的子节点
        assert!(node.children.len() >= 2);
    }

    #[test]
    fn test_parse_text_with_inline_elements() {
        init();
        // <em> 不被项目支持，其文本 "world" 应被压平提升
        let html = "<p>Hello <em>world</em>!</p>";
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();
        dbg!(&node);
    }

    #[test]
    fn test_parse_empty_input() {
        init();
        let html = "";
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();
        assert!(node.children.is_empty());
    }

    /// 模拟 all_in_one.html 场景：注释 + 嵌套元素
    #[test]
    fn test_parse_with_comment_and_nested_div() {
        init();
        let html = "<!-- header comment -->\n<div><h1>Title</h1><p>Paragraph</p></div>";
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();

        dbg!(&node);

        assert!(!node.children.is_empty(), "根节点应有子节点");
        assert_eq!(node.children[0].node_type, NodeType::Div, "第一个子节点应为 Div");
        assert_eq!(node.children[0].children.len(), 2, "Div 应有 2 个子节点");
        assert_eq!(node.children[0].children[0].node_type, NodeType::H1);
        assert_eq!(node.children[0].children[1].node_type, NodeType::P);
    }
}
