use htui::html::html_parse::{self, Parser};
use htui::tdom::node::{NodeType, document_base_href};

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

#[test]
fn test_tag_map() {
    init();
    assert_eq!(
        html_parse::tag_name_to_node_type("div"),
        Some(NodeType::Div)
    );
    assert_eq!(
        html_parse::tag_name_to_node_type("style"),
        Some(NodeType::Style)
    );
    assert_eq!(
        html_parse::tag_name_to_node_type("html"),
        Some(NodeType::Html)
    );
    assert_eq!(
        html_parse::tag_name_to_node_type("body"),
        Some(NodeType::Body)
    );
    assert_eq!(html_parse::tag_name_to_node_type("nope"), None);
}

#[test]
fn test_tag_map_metadata() {
    init();
    assert_eq!(
        html_parse::tag_name_to_node_type("title"),
        Some(NodeType::Title)
    );
    assert_eq!(
        html_parse::tag_name_to_node_type("TITLE"),
        Some(NodeType::Title)
    );
    assert_eq!(
        html_parse::tag_name_to_node_type("meta"),
        Some(NodeType::Meta)
    );
    assert_eq!(
        html_parse::tag_name_to_node_type("META"),
        Some(NodeType::Meta)
    );
}

#[test]
fn test_tag_map_heading_group_and_address() {
    // `<h6>` / `<hgroup>` / `<address>` must map to their own `NodeType`
    // (they used to be flattened by the HTML parser), and the case
    // normalisation must match the browser's tag-name handling.
    init();
    assert_eq!(
        html_parse::tag_name_to_node_type("h6"),
        Some(NodeType::H6)
    );
    assert_eq!(
        html_parse::tag_name_to_node_type("H6"),
        Some(NodeType::H6)
    );
    assert_eq!(
        html_parse::tag_name_to_node_type("hgroup"),
        Some(NodeType::HGroup)
    );
    assert_eq!(
        html_parse::tag_name_to_node_type("address"),
        Some(NodeType::Address)
    );
}

#[test]
fn test_parse_h6_hgroup_address_keeps_dom_tree() {
    // The three tags must survive parsing as real element nodes (not be
    // flattened), so CSS selectors and the layout pass can address them.
    init();
    let html = "<h6>Small</h6><hgroup><h2>Grouped</h2></hgroup><address>Contact</address>";
    let (root, arena, _) = Parser::new(html).parse();
    // html5ever hoists flow content into an implicit <body>.
    let html_id = arena[root].children[0];
    let body_id = arena[html_id]
        .children
        .iter()
        .copied()
        .find(|&id| arena[id].node_type == NodeType::Body)
        .expect("flow content must land in <body>");
    let body = &arena[body_id];
    let node_types: Vec<NodeType> = body
        .children
        .iter()
        .map(|&id| arena[id].node_type)
        .collect();
    assert_eq!(
        node_types,
        vec![NodeType::H6, NodeType::HGroup, NodeType::Address],
        "h6/hgroup/address must not be flattened"
    );
    let hgroup_id = body.children[1];
    assert_eq!(arena[hgroup_id].children.len(), 1);
    assert_eq!(
        arena[arena[hgroup_id].children[0]].node_type,
        NodeType::H2,
        "<hgroup> must keep its heading members as children"
    );
}

#[test]
fn test_parse_basic() {
    init();
    let mut p = Parser::new("<div><p>Hello</p></div>");
    let (root, arena, _) = p.parse();
    assert!(arena[root].child_count() > 0);
}

#[test]
fn test_style_extract() {
    init();
    let mut p = Parser::new("<div><style>p{color:red}</style><p>Hi</p></div>");
    let (_root, _arena, css) = p.parse();
    assert!(css.contains("p{color:red}"));
}

#[test]
fn test_parse_html_body_basic() {
    init();
    let mut p = Parser::new(
        "<html><head><title>t</title><style>p{color:red}</style></head><body><p>Hi</p></body></html>",
    );
    let (root, arena, css) = p.parse();
    let root_node = &arena[root];
    assert_eq!(root_node.node_type, NodeType::Document);
    assert_eq!(
        root_node.child_count(),
        1,
        "<html> should be the only top-level child"
    );

    let html_id = root_node.children[0];
    let html_node = &arena[html_id];
    assert_eq!(html_node.node_type, NodeType::Html);

    let body_id = html_node
        .children
        .iter()
        .copied()
        .find(|&cid| arena[cid].node_type == NodeType::Body)
        .expect("<body> should be a child of <html>");
    let body_node = &arena[body_id];
    assert_eq!(body_node.child_count(), 1);
    assert_eq!(arena[body_node.children[0]].node_type, NodeType::P);
    assert!(
        css.contains("p{color:red}"),
        "<style> inside <head> should still be extracted"
    );
}

#[test]
fn test_parse_html_body_style_inside_head() {
    init();
    let mut p =
        Parser::new("<html><head><style>body{color:white}</style></head><body>x</body></html>");
    let (_root, _arena, css) = p.parse();
    assert!(css.contains("body{color:white}"));
}

#[test]
fn test_parse_title_preserves_tree() {
    // After promoting <title> to a recognised NodeType, the title becomes
    // its own element with the title text as its child. The title is not
    // flattened up to <head> any more.
    init();
    let mut p = Parser::new("<html><head><title>My Doc</title></head><body></body></html>");
    let (root, arena, _) = p.parse();
    let html_id = arena[root].children[0];
    let head_id = arena[html_id].children[0];
    let head_node = &arena[head_id];
    assert_eq!(head_node.node_type, NodeType::Head);

    let title_id = head_node
        .children
        .iter()
        .copied()
        .find(|&cid| arena[cid].node_type == NodeType::Title)
        .expect("<title> should be a child of <head>");
    let title_node = &arena[title_id];
    assert_eq!(title_node.child_count(), 1, "<title> retains its text child");
    let text_child = &arena[title_node.children[0]];
    assert_eq!(text_child.node_type, NodeType::Text);
    assert_eq!(text_child.data.as_deref(), Some("My Doc"));
}

#[test]
fn test_parse_meta_void_element() {
    // <meta> is a void element: html5ever should produce a Meta node with
    // no children, while the next sibling is parsed normally.
    init();
    let mut p = Parser::new(
        "<html><head><meta charset=\"utf-8\"><title>T</title></head><body></body></html>",
    );
    let (root, arena, _) = p.parse();
    let html_id = arena[root].children[0];
    let head_id = arena[html_id].children[0];
    let head_node = &arena[head_id];
    let meta_id = head_node
        .children
        .iter()
        .copied()
        .find(|&cid| arena[cid].node_type == NodeType::Meta)
        .expect("<meta> should be a child of <head>");
    let meta_node = &arena[meta_id];
    assert_eq!(meta_node.child_count(), 0, "<meta> is void");
    assert_eq!(
        meta_node.attributes.get("charset").map(String::as_str),
        Some("utf-8")
    );
}

#[test]
fn test_script_inline_extract() {
    init();
    let mut p = Parser::new("<script>globalThis.__a = 1;</script><p>x</p>");
    let doc = p.parse_with_scripts();
    assert!(
        doc.scripts.iter().any(|s| s.contains("__a = 1")),
        "inline <script> content must be extracted"
    );
}

#[test]
fn test_script_src_wins_over_inline() {
    // A `src` attribute wins over the inline body (browser rule: external
    // script content is ignored). The external content now comes from a
    // `mem://` resource (no file system).
    init();
    let key = format!("mem://script/srcwin_{}.js", std::process::id());
    htui::resources::register(&key, b"globalThis.__ext = true;".to_vec())
        .expect("register mem script");
    let html = format!(
        "<script src=\"{key}\">globalThis.__inline = true;</script>"
    );
    let doc = Parser::new(&html).parse_with_scripts();
    assert!(
        doc.scripts.iter().any(|s| s.contains("__ext = true")),
        "external resource content must be extracted"
    );
    assert!(
        doc.scripts.iter().all(|s| !s.contains("__inline")),
        "inline body must be ignored when src is present"
    );
    htui::resources::unregister(&key);
}

#[test]
fn test_script_src_loads_mem_resource() {
    // `<script src="mem://…">` resolves through the unified resource layer
    // instead of the file system.
    init();
    let key = format!("mem://script/{}.js", std::process::id());
    htui::resources::register(&key, b"globalThis.__memLoaded = 7;".to_vec())
        .expect("register mem script");
    let html = format!("<script src=\"{key}\"></script><p>x</p>");
    let doc = Parser::new(&html).parse_with_scripts();
    assert!(
        doc.scripts.iter().any(|s| s.contains("__memLoaded = 7")),
        "<script src> must load the mem:// resource"
    );
    htui::resources::unregister(&key);
}

#[test]
fn test_link_href_loads_mem_stylesheet() {
    // `<link rel="stylesheet" href="mem://…">` loads CSS from the resource
    // layer.
    init();
    let key = format!("mem://css/{}.css", std::process::id());
    htui::resources::register(&key, b"p { color: red; }".to_vec())
        .expect("register mem css");
    let html = format!(
        "<link rel=\"stylesheet\" href=\"{key}\"><p>x</p>"
    );
    let (_root, _arena, css) = Parser::new(&html).parse();
    assert!(
        css.contains("color: red"),
        "<link> must load the mem:// stylesheet"
    );
    htui::resources::unregister(&key);
}

// ============================================================
// <base> — document base URL
// ============================================================

#[test]
fn test_parse_base_keeps_dom_node_with_href() {
    // `<base>` must survive parsing as a real (non-rendered) element so its
    // `href` can establish the document base URL.
    init();
    let html = "<html><head><base href=\"mem://app/\"></head><body></body></html>";
    let (root, arena, _) = Parser::new(html).parse();
    let html_id = arena[root].children[0];
    let head_id = arena[html_id].children[0];
    let base_id = arena[head_id].children[0];
    assert_eq!(arena[base_id].node_type, NodeType::Base);
    assert_eq!(
        arena[base_id].attributes.get("href").map(String::as_str),
        Some("mem://app/"),
        "<base href> must be kept as an attribute"
    );
    assert_eq!(
        document_base_href(&arena, root),
        Some("mem://app/".to_string()),
        "the first <base href> must be the document base URL"
    );
}

#[test]
fn test_base_resolves_script_src() {
    // `<script src>` relative paths resolve against the `<base href>`.
    init();
    let pid = std::process::id();
    let key = format!("mem://app/main_{pid}.js");
    htui::resources::register(&key, b"globalThis.__basedScript = true;".to_vec())
        .expect("register mem script");
    let html = format!(
        "<base href=\"mem://app/\"><script src=\"main_{pid}.js\"></script><p>x</p>"
    );
    let doc = Parser::new(&html).parse_with_scripts();
    assert!(
        doc.scripts.iter().any(|s| s.contains("__basedScript")),
        "<script src> must resolve against <base href>"
    );
    htui::resources::unregister(&key);
}

#[test]
fn test_base_resolves_link_stylesheet_href() {
    // `<link rel=stylesheet>` relative `href` resolves against `<base href>`.
    init();
    let pid = std::process::id();
    let key = format!("mem://assets/style_{pid}.css");
    htui::resources::register(&key, b"p { color: green; }".to_vec())
        .expect("register mem css");
    let html = format!(
        "<base href=\"mem://assets/\"><link rel=\"stylesheet\" href=\"style_{pid}.css\"><p>x</p>"
    );
    let (_root, _arena, css) = Parser::new(&html).parse();
    assert!(
        css.contains("color: green"),
        "<link> stylesheet href must resolve against <base href>"
    );
    htui::resources::unregister(&key);
}

#[test]
fn test_link_enters_dom_as_non_rendered_node() {
    // `<link>` is now a real DOM node (addressable by CSS / DOM queries),
    // while its stylesheet is still extracted during parsing.
    init();
    let html = "<html><head><link rel=\"stylesheet\" href=\"mem://x/y.css\"></head><body></body></html>";
    let (root, arena, _) = Parser::new(html).parse();
    let html_id = arena[root].children[0];
    let head_id = arena[html_id].children[0];
    assert_eq!(
        arena[arena[head_id].children[0]].node_type,
        NodeType::Link,
        "<link> must stay in the DOM"
    );
}
