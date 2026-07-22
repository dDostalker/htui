use crossterm::style::Color;
use htui::html::css_parse::{
    Combinator, CssAttr, CssRule, CssSelector, Parser, css_into_node, parse_color_str,
    parse_declaration_text, selector_chain_matches,
};
use htui::tdom::{
    attributes::Attr,
    node::{Node, NodeArena, NodeId, NodeType},
};
use indexmap::IndexMap;

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

fn build_test(arena: &mut NodeArena) -> (NodeId, NodeId) {
    let root = arena.insert(Node::new());
    let div_id = arena.insert(Node::from(NodeType::Div));
    arena[div_id].attributes = Attr::parse_from_str(r#"class="active" id="main""#);
    let p_id = arena.insert(Node::from(NodeType::P));
    let text_id = arena.insert(Node::text("Hello"));
    Node::append_child(arena, p_id, text_id);
    Node::append_child(arena, div_id, p_id);
    Node::append_child(arena, root, div_id);
    (root, div_id)
}

#[test]
fn test_parse_simple() {
    init();
    let mut p = Parser::new("p { color: red; }");
    assert_eq!(p.parse().unwrap().0.len(), 1);
}

#[test]
fn test_parse_desc() {
    init();
    let s: CssSelector = "div p".parse().unwrap();
    assert_eq!(s.combinators[0], Combinator::Descendant);
}

#[test]
fn test_parse_child() {
    init();
    let s: CssSelector = "div > p".parse().unwrap();
    assert_eq!(s.combinators[0], Combinator::Child);
}

#[test]
fn test_matches_descendant() {
    init();
    let mut arena = NodeArena::default();
    let (root, _) = build_test(&mut arena);
    let sel: CssSelector = "div p".parse().unwrap();
    let div_id = arena[root].children[0];
    let p_id = arena[div_id].children[0];
    assert!(selector_chain_matches(&arena, p_id, &sel));
}

#[test]
fn test_css_into_node() {
    init();
    let mut arena = NodeArena::default();
    let (root, _) = build_test(&mut arena);
    let mut css = CssRule::new();
    let sel: CssSelector = "div p".parse().unwrap();
    let mut m = IndexMap::new();
    m.insert("color".into(), "red".into());
    css.add(sel, m);
    css_into_node(&mut arena, root, &css).unwrap();
    let div_id = arena[root].children[0];
    let p_id = arena[div_id].children[0];
    assert!(arena[p_id].css_attr.get_color().is_some());
}

#[test]
fn test_color_name_parsing() {
    assert_eq!(parse_color_str("darkblue"), Some(Color::DarkBlue));
    assert_eq!(parse_color_str("DarkBlue"), Some(Color::DarkBlue));
    assert_eq!(parse_color_str("red"), Some(Color::Red));
    assert_eq!(parse_color_str("darkred"), Some(Color::DarkRed));
    assert_eq!(parse_color_str("blue"), Some(Color::Blue));
    assert_eq!(parse_color_str("grey"), Some(Color::Grey));
    assert_eq!(parse_color_str("gray"), Some(Color::Grey));
}

#[test]
fn test_color_darkblue_parsing() {}

#[test]
fn test_color_hex_parsing() {
    assert_eq!(
        parse_color_str("#1e293b"),
        Some(Color::Rgb {
            r: 0x1e,
            g: 0x29,
            b: 0x3b
        })
    );
    assert_eq!(
        parse_color_str("#fff"),
        Some(Color::Rgb {
            r: 255,
            g: 255,
            b: 255
        })
    );
}

#[test]
fn test_color_rgb_parsing() {
    assert_eq!(
        parse_color_str("rgb(30,41,59)"),
        Some(Color::Rgb {
            r: 30,
            g: 41,
            b: 59
        })
    );
}

#[test]
fn test_css_attr_full_color() {
    let mut attr = CssAttr::new();
    let css = "color: darkblue; background-color: #1e293b;";
    let m = parse_declaration_text(css);
    attr.add(CssAttr(m));
    assert_eq!(attr.get_color(), Some(Color::DarkBlue));
    assert_eq!(
        attr.get_bg_color(),
        Some(Color::Rgb {
            r: 0x1e,
            g: 0x29,
            b: 0x3b
        })
    );
}
