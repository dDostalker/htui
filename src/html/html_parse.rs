use std::collections::HashMap;
use std::fs;
use std::path::Path;

use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use log::{debug, info, warn};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::tdom::node::{Node, NodeArena, NodeId, NodeType};

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
    pub fn parse(&mut self) -> (NodeId, NodeArena, String) {
        let opts = ParseOpts::default();
        let dom = html5ever::parse_document(RcDom::default(), opts).one(self.input.clone());

        let mut arena = NodeArena::default();
        let root_id = arena.insert(Node::from(NodeType::Global));
        let mut css_sources: Vec<String> = Vec::new();

        convert_children(&dom.document, root_id, &mut arena, &mut css_sources);

        debug!(
            "解析完成: root={}, {} 节点, CSS: {} 字节",
            root_id,
            arena.len(),
            css_sources.join("\n").len()
        );
        (root_id, arena, css_sources.join("\n"))
    }
}

fn convert_attributes(html5_attrs: &[html5ever::Attribute]) -> HashMap<String, String> {
    html5_attrs
        .iter()
        .map(|a| (a.name.local.to_string(), a.value.to_string()))
        .collect()
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
                        if node_type == NodeType::Style {
                            let css = extract_text_content(child_handle);
                            if !css.trim().is_empty() {
                                info!("提取 <style> CSS: {} 字节", css.len());
                                css_sources.push(css);
                            }
                            return;
                        }
                        if node_type == NodeType::Link {
                            let link_attrs = {
                                let ar = attrs.borrow();
                                convert_attributes(&ar)
                            };
                            if let (Some(rel), Some(href)) =
                                (link_attrs.get("rel"), link_attrs.get("href"))
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
                // Drop purely-whitespace text nodes (no printable content) but
                // preserve original whitespace within the text — `<pre>` and
                // inline runs that depend on leading/trailing spaces will
                // otherwise be silently truncated. Whitespace collapsing is a
                // paint-time concern driven by `white-space`; the HTML parser
                // should not mutate the source bytes.
                if text.chars().any(|c| !c.is_whitespace()) {
                    let text_id = arena.insert(Node::text(&text));
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

/// Maps an HTML tag name to its corresponding [`NodeType`].
///
/// Returns `None` for unsupported tags (they are flattened).
#[doc(hidden)]
pub fn tag_name_to_node_type(tag: &str) -> Option<NodeType> {
    match tag.to_ascii_lowercase().as_str() {
        "a" => Some(NodeType::A),
        "article" => Some(NodeType::Article),
        "aside" => Some(NodeType::Aside),
        "body" => Some(NodeType::Body),
        "caption" => Some(NodeType::Caption),
        "button" => Some(NodeType::Button),
        "col" => Some(NodeType::Col),
        "colgroup" => Some(NodeType::Colgroup),
        "details" => Some(NodeType::Details),
        "td" => Some(NodeType::Td),
        "th" => Some(NodeType::Th),
        "tr" => Some(NodeType::Tr),
        "thead" => Some(NodeType::Thead),
        "tbody" => Some(NodeType::Tbody),
        "tfoot" => Some(NodeType::Tfoot),
        "div" => Some(NodeType::Div),
        "em" => Some(NodeType::Em),
        "figcaption" => Some(NodeType::Figcaption),
        "figure" => Some(NodeType::Figure),
        "footer" => Some(NodeType::Footer),
        "form" => Some(NodeType::Form),
        "h1" => Some(NodeType::H1),
        "h2" => Some(NodeType::H2),
        "h3" => Some(NodeType::H3),
        "h4" => Some(NodeType::H4),
        "h5" => Some(NodeType::H5),
        "head" => Some(NodeType::Head),
        "header" => Some(NodeType::Header),
        "html" => Some(NodeType::Html),
        "img" => Some(NodeType::Img),
        "input" => Some(NodeType::Input),
        "label" => Some(NodeType::Label),
        "li" => Some(NodeType::Li),
        "link" => Some(NodeType::Link),
        "main" => Some(NodeType::Main),
        "meta" => Some(NodeType::Meta),
        "nav" => Some(NodeType::Nav),
        "ol" => Some(NodeType::Ol),
        "option" => Some(NodeType::HtmlOption),
        "p" => Some(NodeType::P),
        "pre" => Some(NodeType::Pre),
        "section" => Some(NodeType::Section),
        "select" => Some(NodeType::Select),
        "span" => Some(NodeType::Span),
        "style" => Some(NodeType::Style),
        "summary" => Some(NodeType::Summary),
        "table" => Some(NodeType::Table),
        "textarea" => Some(NodeType::Textarea),
        "title" => Some(NodeType::Title),
        "ul" => Some(NodeType::Ul),
        _ => None,
    }
}
