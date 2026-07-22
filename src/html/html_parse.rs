use std::fs;
use std::path::Path;

use anyhow::Result;
use html5ever::Attribute;
use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use log::{debug, info, trace, warn};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::tdom::{
    attributes::Attr,
    node::{Node, NodeType},
};

/// HTML 解析器，基于 html5ever 库实现。
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

    pub fn parse(&mut self) -> Result<Node> {
        let (node, _css) = self.parse_with_css()?;
        Ok(node)
    }

    /// 解析 HTML，同时返回 Node 树和从 `<style>` / `<link>` 中提取的 CSS 文本
    pub fn parse_with_css(&mut self) -> Result<(Node, String)> {
        debug!(
            "开始使用 html5ever 解析 HTML，输入长度 {} 字节",
            self.input.len()
        );

        let opts = ParseOpts::default();
        let dom = html5ever::parse_document(RcDom::default(), opts).one(self.input.clone());

        let mut root = Node::new();
        let mut css_sources: Vec<String> = Vec::new();
        convert_children(&dom.document, &mut root, &mut css_sources);

        root.fix_parent_pointers();

        let embedded_css = css_sources.join("\n");
        debug!(
            "html5ever 解析完成，子节点: {}，嵌入 CSS: {} 字节",
            root.children.len(),
            embedded_css.len()
        );
        Ok((root, embedded_css))
    }
}

/// 将 html5ever 的 Attribute 转换为项目内部的 Attr
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

/// 递归转换 html5ever RcDom → Node 树，同时收集 CSS 源
fn convert_children(handle: &Handle, parent: &mut Node, css_sources: &mut Vec<String>) {
    let children = handle.children.borrow().clone();

    for child_handle in &children {
        match &child_handle.data {
            NodeData::Document => {
                trace!("遇到 Document 节点，递归处理子节点");
                convert_children(child_handle, parent, css_sources);
            }
            NodeData::Element { name, attrs, .. } => {
                let tag = name.local.as_ref();
                trace!("遇到元素标签: <{}>", tag);

                match tag_name_to_node_type(tag) {
                    Some(node_type) => {
                        debug!("识别标签 <{}> → {:?}", tag, node_type);

                        // ---- <style> 特殊处理：提取 CSS ----
                        if node_type == NodeType::Style {
                            let css_text = extract_text_content(child_handle);
                            if !css_text.trim().is_empty() {
                                info!("提取 <style> CSS: {} 字节", css_text.len());
                                css_sources.push(css_text);
                            }
                            // Style 节点不加入渲染树
                            return;
                        }

                        // ---- <link> 特殊处理：读取外部 CSS ----
                        if node_type == NodeType::Link {
                            let attr = {
                                let attrs_ref = attrs.borrow();
                                convert_attributes(&attrs_ref)
                            };
                            if let (Some(rel), Some(href)) = (attr.get("rel"), attr.get("href")) {
                                if rel.to_lowercase().contains("stylesheet") {
                                    info!("发现外部样式表: {}", href);
                                    match fs::read_to_string(Path::new(href)) {
                                        Ok(content) => {
                                            info!(
                                                "读取外部 CSS: {} ({} 字节)",
                                                href,
                                                content.len()
                                            );
                                            css_sources.push(content);
                                        }
                                        Err(e) => {
                                            warn!("无法读取外部样式表 '{}': {}", href, e);
                                        }
                                    }
                                }
                            }
                            // Link 节点不加入渲染树
                            return;
                        }

                        let mut child_node = Node::from(node_type);
                        {
                            let attrs_ref = attrs.borrow();
                            child_node.attributes = convert_attributes(&attrs_ref);
                        }
                        convert_children(child_handle, &mut child_node, css_sources);
                        parent.children.push(child_node);
                    }
                    None => {
                        warn!("不支持的 HTML 标签 <{}>，压平处理", tag);
                        convert_children(child_handle, parent, css_sources);
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
                convert_children(child_handle, parent, css_sources);
            }
        }
    }
}

/// 提取元素的文本内容（用于 `<style>` 等原始文本元素）
fn extract_text_content(handle: &Handle) -> String {
    let children = handle.children.borrow();
    let mut text = String::new();
    for child in children.iter() {
        if let NodeData::Text { contents } = &child.data {
            text.push_str(&contents.borrow());
        }
    }
    text
}

/// 将 HTML 标签名映射为项目内部的 NodeType
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
        "link" => Some(NodeType::Link),
        "main" => Some(NodeType::Main),
        "nav" => Some(NodeType::Nav),
        "ol" => Some(NodeType::Ol),
        "option" => Some(NodeType::HtmlOption),
        "p" => Some(NodeType::P),
        "section" => Some(NodeType::Section),
        "select" => Some(NodeType::Select),
        "span" => Some(NodeType::Span),
        "style" => Some(NodeType::Style),
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
        assert_eq!(tag_name_to_node_type("style"), Some(NodeType::Style));
        assert_eq!(tag_name_to_node_type("link"), Some(NodeType::Link));
        assert_eq!(tag_name_to_node_type("unsupported"), None);
    }

    #[test]
    fn test_parse_style_extracts_css() {
        init();
        let html = "<div><style>p { color: red; }</style><p>Hello</p></div>";
        let mut p = Parser::new(html);
        let (node, css) = p.parse_with_css().unwrap();

        // CSS 应被提取
        assert!(css.contains("p { color: red; }"), "应提取到 CSS: {}", css);
        // <style> 不应出现在渲染树中
        assert!(node.children.len() >= 1, "应有渲染节点");
    }

    #[test]
    fn test_parse_basic_html() {
        init();
        let html = "<div><p>Hello</p><p>World</p></div>";
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();
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
    fn test_parse_with_comment_and_nested_div() {
        init();
        let html = "<!-- header comment -->\n<div><h1>Title</h1><p>Paragraph</p></div>";
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();
        assert!(!node.children.is_empty());
        assert_eq!(node.children[0].node_type, NodeType::Div);
    }
}
