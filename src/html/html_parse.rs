use std::fs;
use std::path::Path;

use anyhow::Result;
use html5ever::Attribute;
use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use log::{debug, info, warn};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::tdom::{
    attributes::Attr,
    node::{Node, NodeArena, NodeId, NodeType},
};

/// HTML parser (slotmap arena-based).
#[derive(Debug)]
pub struct Parser {
    input: String,
}

impl Parser {
    pub fn new(input: &str) -> Self {
        Self {
            input: input.to_string(),
        }
    }

    /// Parse HTML and return (root_id, arena, embedded_css).
    pub fn parse(&mut self) -> Result<(NodeId, NodeArena, String)> {
        debug!("html5ever 解析 HTML，{} 字节", self.input.len());

        let opts = ParseOpts::default();
        let dom = html5ever::parse_document(RcDom::default(), opts).one(self.input.clone());

        let mut arena = NodeArena::default();
        let root_id = arena.insert(Node::new());
        let mut css_sources: Vec<String> = Vec::new();

        convert_children(&dom.document, root_id, &mut arena, &mut css_sources);

        debug!(
            "解析完成: root={}, {} 节点, CSS: {} 字节",
            root_id,
            arena.len(),
            css_sources.join("\n").len()
        );
        Ok((root_id, arena, css_sources.join("\n")))
    }
}

fn convert_attributes(html5_attrs: &[Attribute]) -> Attr {
    if html5_attrs.is_empty() {
        return Attr::new();
    }
    let attr_str: String = html5_attrs
        .iter()
        .map(|a| format!("{}=\"{}\"", a.name.local, a.value))
        .collect::<Vec<_>>()
        .join(" ");
    Attr::parse_from_str(&attr_str)
}

fn convert_children(
    handle: &Handle,
    parent_id: NodeId,
    arena: &mut NodeArena,
    css_sources: &mut Vec<String>,
) {
    let children = handle.children.borrow().clone();
    for child_handle in &children {
        match &child_handle.data {
            NodeData::Document => {
                convert_children(child_handle, parent_id, arena, css_sources);
            }
            NodeData::Element { name, attrs, .. } => {
                let tag = name.local.as_ref();
                match tag_name_to_node_type(tag) {
                    Some(node_type) => {
                        // ---- <style>: 提取 CSS ----
                        if node_type == NodeType::Style {
                            let css = extract_text_content(child_handle);
                            if !css.trim().is_empty() {
                                info!("提取 <style> CSS: {} 字节", css.len());
                                css_sources.push(css);
                            }
                            return;
                        }
                        // ---- <link>: 读取外部 CSS ----
                        if node_type == NodeType::Link {
                            let a = {
                                let ar = attrs.borrow();
                                convert_attributes(&ar)
                            };
                            if let (Some(rel), Some(href)) = (a.get("rel"), a.get("href"))
                                && rel.to_lowercase().contains("stylesheet")
                            {
                                info!("外部样式表: {}", href);
                                match fs::read_to_string(Path::new(href)) {
                                    Ok(c) => {
                                        css_sources.push(c);
                                    }
                                    Err(e) => warn!("读取 {} 失败: {}", href, e),
                                }
                            }
                            return;
                        }
                        let child_id = arena.insert(Node::from(node_type));
                        {
                            let ar = attrs.borrow();
                            arena[child_id].attributes = convert_attributes(&ar);
                        }
                        Node::append_child(arena, parent_id, child_id);
                        convert_children(child_handle, child_id, arena, css_sources);
                    }
                    None => {
                        warn!("不支持标签 <{}>，压平", tag);
                        convert_children(child_handle, parent_id, arena, css_sources);
                    }
                }
            }
            NodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() {
                    let text_id = arena.insert(Node::text(&trimmed));
                    Node::append_child(arena, parent_id, text_id);
                }
            }
            NodeData::Comment { .. } => {}
            NodeData::Doctype { .. } | NodeData::ProcessingInstruction { .. } => {
                convert_children(child_handle, parent_id, arena, css_sources);
            }
        }
    }
}

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

pub fn tag_name_to_node_type(tag: &str) -> Option<NodeType> {
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
