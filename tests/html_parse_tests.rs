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
