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
    assert_eq!(root_node.child_count(), 1, "<html> should be the only top-level child");

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
    assert!(css.contains("p{color:red}"), "<style> inside <head> should still be extracted");
}

#[test]
fn test_parse_html_body_style_inside_head() {
    init();
    let mut p = Parser::new("<html><head><style>body{color:white}</style></head><body>x</body></html>");
    let (_root, _arena, css) = p.parse();
    assert!(css.contains("body{color:white}"));
}
