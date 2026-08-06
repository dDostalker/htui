use htui::html::html_parse::{self, Parser};
use htui::tdom::node::NodeType;

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
    assert_eq!(root_node.node_type, NodeType::Global);
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
    match &arena[title_node.children[0]].node_type {
        NodeType::Text(s) => assert_eq!(s, "My Doc"),
        other => panic!("expected Text child, got {other:?}"),
    }
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
fn test_script_src_loads_external_js() {
    // `<script src>` reads the file like `<link rel="stylesheet">`: local
    // filesystem path (absolute here so the test is CWD-independent).
    init();
    let path = std::env::temp_dir().join(format!(
        "htui_script_src_{}.js",
        std::process::id()
    ));
    std::fs::write(&path, "globalThis.__loaded = 42;").expect("write temp js");
    let href = path.to_string_lossy();
    let html = format!("<script src=\"{href}\"></script><p>x</p>");
    let doc = Parser::new(&html).parse_with_scripts();
    assert!(
        doc.scripts.iter().any(|s| s.contains("__loaded = 42")),
        "<script src> must load the external JS file"
    );
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) => eprintln!("temp js cleanup failed: {e}"),
    }
}

#[test]
fn test_script_src_wins_over_inline() {
    // A `src` attribute wins over the inline body (browser rule: external
    // script content is ignored).
    init();
    let path = std::env::temp_dir().join(format!(
        "htui_script_src_win_{}.js",
        std::process::id()
    ));
    std::fs::write(&path, "globalThis.__ext = true;").expect("write temp js");
    let href = path.to_string_lossy();
    let html = format!(
        "<script src=\"{href}\">globalThis.__inline = true;</script>"
    );
    let doc = Parser::new(&html).parse_with_scripts();
    assert!(
        doc.scripts.iter().any(|s| s.contains("__ext = true")),
        "external file content must be extracted"
    );
    assert!(
        doc.scripts.iter().all(|s| !s.contains("__inline")),
        "inline body must be ignored when src is present"
    );
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) => eprintln!("temp js cleanup failed: {e}"),
    }
}
