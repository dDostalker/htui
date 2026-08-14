use indexmap::IndexMap;

use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use html5ever::{LocalName, QualName, namespace_url, ns};
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

        convert_children(
            &dom.document,
            root_id,
            &mut arena,
            &mut css_sources,
            &mut scripts,
        );

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

fn convert_attributes(html5_attrs: &[html5ever::Attribute]) -> IndexMap<String, String> {
    // Insertion order mirrors the source document (html5ever reports
    // attributes in source order), so the `innerHTML` serializer can reproduce
    // a browser's attribute order.
    html5_attrs
        .iter()
        .map(|a| (a.name.local.to_string(), a.value.to_string()))
        .collect()
}

/// How embedded `<style>` / `<script>` content is treated during conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConvertMode {
    /// Top-level document parse: `<style>` CSS and `<script>` sources are
    /// extracted (CSS is applied, scripts run).
    Document,
    /// Inside a `<template>`: everything is inert — `<style>` stays a DOM
    /// node and `<script>` is flattened as plain text (template content never
    /// renders or executes).
    Template,
    /// `innerHTML` fragment: `<style>` CSS is extracted (the browser applies
    /// it), `<script>` is skipped entirely (the browser creates the element
    /// but never runs it; htui has no script DOM node).
    InnerHtml,
}

fn convert_children(
    handle: &Handle,
    parent_id: NodeId,
    arena: &mut NodeArena,
    css_sources: &mut Vec<String>,
    scripts: &mut Vec<String>,
) {
    convert_children_with_mode(
        handle,
        parent_id,
        arena,
        css_sources,
        scripts,
        ConvertMode::Document,
    );
}

/// Recursive DOM conversion, treating `<style>` / `<script>` according to the
/// conversion context (see [`ConvertMode`]).
fn convert_children_with_mode(
    handle: &Handle,
    parent_id: NodeId,
    arena: &mut NodeArena,
    css_sources: &mut Vec<String>,
    scripts: &mut Vec<String>,
    mode: ConvertMode,
) {
    let children = handle.children.borrow().clone();
    for child_handle in &children {
        convert_one_child(child_handle, parent_id, arena, css_sources, scripts, mode);
    }
}

/// Convert one rcdom child node into the tdom arena under `parent_id`.
fn convert_one_child(
    child_handle: &Handle,
    parent_id: NodeId,
    arena: &mut NodeArena,
    css_sources: &mut Vec<String>,
    scripts: &mut Vec<String>,
    mode: ConvertMode,
) {
    match &child_handle.data {
        NodeData::Document => {
            convert_children_with_mode(child_handle, parent_id, arena, css_sources, scripts, mode);
        }
        NodeData::Element {
            name,
            attrs,
            template_contents,
            ..
        } => {
            let tag = name.local.as_ref();
            if tag.eq_ignore_ascii_case("script") {
                match mode {
                    // Top-level parse: the source is extracted verbatim (like
                    // `<style>`) rather than becoming a DOM node — the script
                    // layer evaluates it against the rendered document.
                    ConvertMode::Document => {
                        // `<script src="...">` loads an external script
                        // through [`resources::load`] (a `mem://` virtual key;
                        // prefix optional). A `src` attribute wins over any
                        // inline content, mirroring the browser's rule that an
                        // external script's body is ignored.
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
                        } else {
                            let js = extract_text_content(child_handle);
                            if !js.trim().is_empty() {
                                info!("提取 <script> JS: {} 字节", js.len());
                                scripts.push(js);
                            }
                        }
                    }
                    // Template content: no script DOM node type exists, so the
                    // source is flattened as plain text (inert).
                    ConvertMode::Template => {
                        convert_children_with_mode(
                            child_handle,
                            parent_id,
                            arena,
                            css_sources,
                            scripts,
                            ConvertMode::Template,
                        );
                    }
                    // `innerHTML`: the browser creates the element but never
                    // executes it; htui has no script DOM node, so the block
                    // is dropped entirely (it must not render its source).
                    ConvertMode::InnerHtml => {}
                }
                return;
            }
            match tag_name_to_node_type(tag) {
                Some(node_type) => {
                    // `<style>` inside a template stays an inert DOM node;
                    // on the document / `innerHTML` paths its CSS is extracted
                    // and applied.
                    if node_type == NodeType::Style
                        && matches!(mode, ConvertMode::Document | ConvertMode::InnerHtml)
                    {
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
                        let loads_css =
                            matches!(mode, ConvertMode::Document | ConvertMode::InnerHtml);
                        if loads_css
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
                                ConvertMode::Template,
                            );
                        }
                    } else {
                        convert_children_with_mode(
                            child_handle,
                            child_id,
                            arena,
                            css_sources,
                            scripts,
                            mode,
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
                        mode,
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

/// Parse an HTML fragment and attach it to `parent_id`, replacing that node's
/// existing children (the `innerHTML` setter semantics).
///
/// The fragment is parsed with html5ever's fragment parser in the context of
/// the target element's tag, so table / list insertion modes match the
/// browser. `<style>` CSS (and `<link rel=stylesheet>` bodies loaded through
/// the resource layer) is pushed to `css_sources`; `<script>` blocks are
/// skipped entirely — the browser creates script elements but never runs
/// them, and htui has no script DOM node to keep.
pub fn parse_fragment_into(
    arena: &mut NodeArena,
    parent_id: NodeId,
    html: &str,
    css_sources: &mut Vec<String>,
) {
    let context_tag = arena
        .get(parent_id)
        .and_then(|n| n.node_type.tag_name())
        .unwrap_or("div");
    let ctx = QualName::new(None, ns!(html), LocalName::from(context_tag));
    let dom = html5ever::parse_fragment(RcDom::default(), ParseOpts::default(), ctx, Vec::new())
        .one(html);

    // Replacement semantics: the target's existing children are dropped first.
    let old_children = arena
        .get_mut(parent_id)
        .map(|n| std::mem::take(&mut n.children))
        .unwrap_or_default();
    for child in old_children {
        if let Some(c) = arena.get_mut(child) {
            c.parent = None;
        }
    }

    // `parse_fragment` wraps the fragment in a synthetic `<html>` element;
    // unwrap it so only the fragment's own nodes become children.
    let document_children = dom.document.children.borrow().clone();
    let mut scripts = Vec::new();
    for child in &document_children {
        if let NodeData::Element { name, .. } = &child.data
            && name.local.as_ref().eq_ignore_ascii_case("html")
        {
            convert_children_with_mode(
                child,
                parent_id,
                arena,
                css_sources,
                &mut scripts,
                ConvertMode::InnerHtml,
            );
            return;
        }
    }
    for child in &document_children {
        convert_one_child(
            child,
            parent_id,
            arena,
            css_sources,
            &mut scripts,
            ConvertMode::InnerHtml,
        );
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

/// Serialize a node's children as an HTML fragment (the `innerHTML` getter).
///
/// Mirrors the browser's fragment serialization algorithm: text escapes `&`
/// `<` `>` and non-breaking spaces, comments keep their delimiters, attribute
/// values escape `&` `"` and non-breaking spaces, and void elements emit no
/// closing tag. Attributes are emitted in source order (preserved by the
/// `IndexMap` attribute store).
pub fn serialize_inner_html(arena: &NodeArena, node_id: NodeId) -> String {
    let Some(node) = arena.get(node_id) else {
        return String::new();
    };
    let mut out = String::new();
    for &child in &node.children {
        serialize_node(arena, child, &mut out);
    }
    out
}

/// Append the serialized form of `node_id` to `out`.
fn serialize_node(arena: &NodeArena, node_id: NodeId, out: &mut String) {
    let node = &arena[node_id];
    match node.node_type {
        NodeType::Text => {
            if let Some(data) = &node.data {
                escape_text(data, out);
            }
        }
        NodeType::Comment => {
            out.push_str("<!--");
            if let Some(data) = &node.data {
                out.push_str(data);
            }
            out.push_str("-->");
        }
        NodeType::DocumentFragment => {
            for &child in &node.children {
                serialize_node(arena, child, out);
            }
        }
        _ => {
            let Some(tag) = node.node_type.tag_name() else {
                return;
            };
            // Attributes serialize in insertion order (source order), which
            // the fragment parser preserves via `IndexMap`.
            out.push('<');
            out.push_str(tag);
            for (k, v) in &node.attributes {
                out.push(' ');
                out.push_str(k);
                out.push_str("=\"");
                escape_attr(v, out);
                out.push('"');
            }
            out.push('>');
            if node.node_type.is_self_closing() {
                return;
            }
            for &child in &node.children {
                serialize_node(arena, child, out);
            }
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        }
    }
}

/// Escape text content for HTML serialization.
fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{00a0}' => out.push_str("&nbsp;"),
            c => out.push(c),
        }
    }
}

/// Escape an attribute value for HTML serialization.
fn escape_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\u{00a0}' => out.push_str("&nbsp;"),
            c => out.push(c),
        }
    }
}
