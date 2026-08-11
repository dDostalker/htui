use std::collections::HashMap;

use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use log::{debug, info, warn};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::resources;
use crate::tdom::node::{Node, NodeArena, NodeId, NodeType, document_base_href};

/// Parsed HTML document: the DOM arena plus the `<style>` CSS and `<script>`
/// JS sources extracted during parsing (inline, or external files loaded via
/// `<link rel="stylesheet">` / `<script src="...">`).
#[derive(Debug)]
pub struct ParsedDocument {
    pub root_id: NodeId,
    pub arena: NodeArena,
    pub css: String,
    pub scripts: Vec<String>,
}

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
        let doc = self.parse_with_scripts();
        (doc.root_id, doc.arena, doc.css)
    }

    /// Parse HTML and return the arena plus the embedded `<style>` CSS and
    /// `<script>` JS sources.
    pub fn parse_with_scripts(&mut self) -> ParsedDocument {
        let opts = ParseOpts::default();
        let dom = html5ever::parse_document(RcDom::default(), opts).one(self.input.clone());

        let mut arena = NodeArena::default();
        let root_id = arena.insert(Node::from(NodeType::Document));
        let mut css_sources: Vec<String> = Vec::new();
        let mut scripts: Vec<String> = Vec::new();

        convert_children(&dom.document, root_id, &mut arena, &mut css_sources, &mut scripts);

        debug!(
            "解析完成: root={}, {} 节点, CSS: {} 字节, JS: {} 字节",
            root_id,
            arena.len(),
            css_sources.join("\n").len(),
            scripts.join("\n").len()
        );
        ParsedDocument {
            root_id,
            arena,
            css: css_sources.join("\n"),
            scripts,
        }
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
    scripts: &mut Vec<String>,
) {
    convert_children_with_mode(handle, parent_id, arena, css_sources, scripts, false);
}

/// Recursive DOM conversion with a template-inertness flag.
///
/// `in_template` is set while walking the contents of a `<template>`:
/// template content is a document fragment that must stay inert — `<style>` /
/// `<link rel=stylesheet>` are not extracted (they would apply globally) and
/// `<script>` is not executed (it has no DOM node type, so its source is
/// flattened as plain text instead).
fn convert_children_with_mode(
    handle: &Handle,
    parent_id: NodeId,
    arena: &mut NodeArena,
    css_sources: &mut Vec<String>,
    scripts: &mut Vec<String>,
    in_template: bool,
) {
    let children = handle.children.borrow().clone();
    for child_handle in &children {
        match &child_handle.data {
            NodeData::Document => {
                convert_children_with_mode(
                    child_handle,
                    parent_id,
                    arena,
                    css_sources,
                    scripts,
                    in_template,
                );
            }
            NodeData::Element {
                name,
                attrs,
                template_contents,
                ..
            } => {
                let tag = name.local.as_ref();
                // `<script>` is inert inside a template (never executed).
                // It has no DOM node type, so its source is flattened as
                // plain text like an unknown tag would.
                if in_template && tag.eq_ignore_ascii_case("script") {
                    convert_children_with_mode(
                        child_handle,
                        parent_id,
                        arena,
                        css_sources,
                        scripts,
                        true,
                    );
                    continue;
                }
                // `<script>` is extracted verbatim (like `<style>`) rather than
                // becoming a DOM node: the script layer evaluates its source
                // against the rendered document.
                if tag.eq_ignore_ascii_case("script") {
                    // `<script src="...">` loads an external script through
                    // [`resources::load`] (a `mem://` virtual key; prefix
                    // optional). A `src` attribute wins over any inline
                    // content, mirroring the browser's rule that an external
                    // script's body is ignored.
                    let script_attrs = {
                        let ar = attrs.borrow();
                        convert_attributes(&ar)
                    };
                    if let Some(src) = script_attrs.get("src") {
                        // Relative `src` resolves against the document base
                        // URL (`<base href>`) established earlier in the
                        // document.
                        let src = resources::resolve_url(
                            document_base_href(arena, parent_id).as_deref(),
                            src,
                        );
                        info!("外部脚本: {}", src);
                        match load_text(&src) {
                            Ok(js) if !js.trim().is_empty() => scripts.push(js),
                            Ok(_) => warn!("外部脚本 {} 为空", src),
                            Err(e) => warn!("读取 {} 失败: {}", src, e),
                        }
                        continue;
                    }
                    // `<script>` is extracted verbatim (like `<style>`) rather than
                    // becoming a DOM node: the script layer evaluates its source
                    // against the rendered document.
                    let js = extract_text_content(child_handle);
                    if !js.trim().is_empty() {
                        info!("提取 <script> JS: {} 字节", js.len());
                        scripts.push(js);
                    }
                    continue;
                }
                match tag_name_to_node_type(tag) {
                    Some(node_type) => {
                        // `<style>` inside a template stays an inert DOM node;
                        // elsewhere its CSS is extracted and applied.
                        if node_type == NodeType::Style && !in_template {
                            let css = extract_text_content(child_handle);
                            if !css.trim().is_empty() {
                                info!("提取 <style> CSS: {} 字节", css.len());
                                css_sources.push(css);
                            }
                            continue;
                        }
                        if node_type == NodeType::Link {
                            let link_attrs = {
                                let ar = attrs.borrow();
                                convert_attributes(&ar)
                            };
                            if !in_template
                                && let (Some(rel), Some(href)) =
                                    (link_attrs.get("rel"), link_attrs.get("href"))
                                && rel.to_lowercase().contains("stylesheet")
                            {
                                // Relative `href` resolves against the
                                // document base URL (`<base href>`).
                                let href = resources::resolve_url(
                                    document_base_href(arena, parent_id).as_deref(),
                                    href,
                                );
                                info!("外部样式表: {}", href);
                                match load_text(&href) {
                                    Ok(c) => {
                                        css_sources.push(c);
                                    }
                                    Err(e) => warn!("读取 {} 失败: {}", href, e),
                                }
                            }
                            // Fall through: `<link>` stays in the DOM as a
                            // non-rendered node (stylesheets are extracted
                            // above), so it is addressable by CSS / DOM
                            // queries like a browser's `<link>`.
                        }
                        let child_id = arena.insert(Node::from(node_type));
                        {
                            let ar = attrs.borrow();
                            arena[child_id].attributes = convert_attributes(&ar);
                        }
                        Node::append_child(arena, parent_id, child_id);
                        if node_type == NodeType::Template {
                            // The parser moves a template's children into a
                            // dedicated contents fragment; walk that instead of
                            // the (empty) element child list. The content stays
                            // inert and never renders.
                            if let Some(contents) = template_contents.borrow().as_ref() {
                                convert_children_with_mode(
                                    contents,
                                    child_id,
                                    arena,
                                    css_sources,
                                    scripts,
                                    true,
                                );
                            }
                        } else {
                            convert_children_with_mode(
                                child_handle,
                                child_id,
                                arena,
                                css_sources,
                                scripts,
                                in_template,
                            );
                        }
                    }
                    None => {
                        warn!("不支持标签 <{}>，压平", tag);
                        convert_children_with_mode(
                            child_handle,
                            parent_id,
                            arena,
                            css_sources,
                            scripts,
                            in_template,
                        );
                    }
                }
            }
            NodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                // Every text node is preserved verbatim (including
                // whitespace-only runs) so the DOM mirrors the source and
                // `childNodes` / `textContent` stay faithful. The layout pass
                // decides whether a whitespace-only run actually paints
                // (see `Node::text_is_painted`), so rendering is unchanged.
                let text_id = arena.insert(Node::text(&text));
                Node::append_child(arena, parent_id, text_id);
            }
            NodeData::Comment { contents } => {
                // Comments are DOM nodes (`nodeType` 8) that never render.
                let comment_id = arena.insert(Node::comment(contents));
                Node::append_child(arena, parent_id, comment_id);
            }
            NodeData::Doctype { name, .. } => {
                let doctype_id = arena.insert(Node::doctype(name));
                Node::append_child(arena, parent_id, doctype_id);
            }
            NodeData::ProcessingInstruction { .. } => {}
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

/// Load a `src` / `href` reference as UTF-8 text through [`resources::load`]
/// (a `mem://` key; the prefix is optional).
///
/// Non-UTF-8 content yields an error, matching how a script or stylesheet
/// body must be text.
fn load_text(src: &str) -> Result<String, String> {
    let bytes = resources::load(src).map_err(|e| e.to_string())?;
    String::from_utf8(bytes.as_ref().to_vec()).map_err(|e| e.to_string())
}

/// Maps an HTML tag name to its corresponding [`NodeType`].
///
/// Returns `None` for unsupported tags (they are flattened).
///
/// Delegates to [`NodeType::from_tag_name`] — the single tag table shared
/// with CSS selector parsing — so both mappings can never drift apart.
#[doc(hidden)]
pub fn tag_name_to_node_type(tag: &str) -> Option<NodeType> {
    NodeType::from_tag_name(tag)
}
