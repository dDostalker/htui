use std::sync::Arc;

use crossterm::style::Color;
use htui::html::css_parse::{
    BorderStyle, BoxSizing, Combinator, CornerRadii, CssAttr, CssRule, CssRuleEntry, CssSelector,
    DynamicState, EdgeAuto, EdgeSizes, FontContext, FontStyleKind, FontWeight, INHERITED_PROPS,
    Length, LengthContext, LineHeight, MarginSizes, NthExpr, PX_PER_CELL, Parser,
    SUPPORTED_PROPERTIES, Selector, TextAlign, TextDecoration, TextTransform, Viewport, WhiteSpace,
    css_into_node, css_into_node_with_state, parse_color_str, parse_declaration_text,
    reset_css_attr, resolve_line_height_cells, selector_chain_matches,
    selector_chain_matches_with_state,
};
use htui::html::html_parse;
use htui::tdom::node::{Node, NodeArena, NodeId, NodeType, effective_font_size_cells};
use indexmap::IndexMap;

/// Default font-size (in cells) used by [`Length::resolve`] tests so the
/// em / rem branches can be exercised without a DOM. Mirrors the
/// `FontContext::root(80, 24)` value passed to layout.
const DEFAULT_FONT_SIZE: usize = 1;
const DEFAULT_ROOT_FONT_SIZE: usize = 1;
/// Default viewport cols / rows for `vw` / `vh` resolution tests.
const DEFAULT_VIEWPORT_COLS: usize = 80;
const DEFAULT_VIEWPORT_ROWS: usize = 24;

fn font_ctx(font_size_cells: usize, root_font_size_cells: usize) -> FontContext {
    FontContext {
        font_size_cells,
        root_font_size_cells,
    }
}

fn viewport() -> Viewport {
    Viewport {
        cols: DEFAULT_VIEWPORT_COLS,
        rows: DEFAULT_VIEWPORT_ROWS,
    }
}

fn length_ctx(containing: usize, font: FontContext) -> LengthContext {
    LengthContext {
        containing,
        font,
        viewport: viewport(),
    }
}

fn default_font_ctx() -> FontContext {
    font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE)
}

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

fn build_test(arena: &mut NodeArena) -> (NodeId, NodeId) {
    let root = arena.insert(Node::from(NodeType::Document));
    let div_id = arena.insert(Node::from(NodeType::Div));
    arena[div_id].attributes = IndexMap::from([
        ("class".to_string(), "active".to_string()),
        ("id".to_string(), "main".to_string()),
    ]);
    let p_id = arena.insert(Node::from(NodeType::P));
    let text_id = arena.insert(Node::text("Hello"));
    Node::append_child(arena, p_id, text_id);
    Node::append_child(arena, div_id, p_id);
    Node::append_child(arena, root, div_id);
    (root, div_id)
}

/// Build `root -> div -> p(class="a b")`, returning `(root, p)`. Used by the
/// cascade tests that need same-specificity class selectors matching one node.
fn build_p_with_classes(arena: &mut NodeArena) -> (NodeId, NodeId) {
    let root = arena.insert(Node::from(NodeType::Document));
    let div_id = arena.insert(Node::from(NodeType::Div));
    let p_id = arena.insert(Node::from(NodeType::P));
    arena[p_id]
        .attributes
        .insert("class".to_string(), "a b".to_string());
    let text_id = arena.insert(Node::text("x"));
    Node::append_child(arena, p_id, text_id);
    Node::append_child(arena, div_id, p_id);
    Node::append_child(arena, root, div_id);
    (root, p_id)
}

#[test]
fn test_parse_simple() {
    init();
    let mut p = Parser::new("p { color: red; }");
    assert_eq!(p.parse().0.len(), 1);
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
fn test_nth_child_even_matches_second_child() {
    // `:nth-child(even)` (== `2n`) must match the 2nd, 4th, … siblings and
    // skip the 1st, 3rd, …. Build a 4-row parent and verify each index.
    init();
    let mut arena = NodeArena::default();
    let parent = arena.insert(Node::from(NodeType::Div));
    let mut children = Vec::new();
    for _ in 0..4 {
        let id = arena.insert(Node::from(NodeType::P));
        Node::append_child(&mut arena, parent, id);
        children.push(id);
    }
    let sel: CssSelector = ":nth-child(even)".parse().unwrap();
    assert!(!selector_chain_matches(&arena, children[0], &sel));
    assert!(selector_chain_matches(&arena, children[1], &sel));
    assert!(!selector_chain_matches(&arena, children[2], &sel));
    assert!(selector_chain_matches(&arena, children[3], &sel));
}

#[test]
fn test_nth_child_odd_matches_first_child() {
    // `:nth-child(odd)` (== `2n+1`) must match the 1st, 3rd, … siblings.
    init();
    let mut arena = NodeArena::default();
    let parent = arena.insert(Node::from(NodeType::Div));
    let mut children = Vec::new();
    for _ in 0..3 {
        let id = arena.insert(Node::from(NodeType::P));
        Node::append_child(&mut arena, parent, id);
        children.push(id);
    }
    let sel: CssSelector = ":nth-child(odd)".parse().unwrap();
    assert!(selector_chain_matches(&arena, children[0], &sel));
    assert!(!selector_chain_matches(&arena, children[1], &sel));
    assert!(selector_chain_matches(&arena, children[2], &sel));
}

#[test]
fn test_nth_child_bare_integer_matches_exact_position() {
    // `:nth-child(2)` must match only the 2nd child. `a=0` in the NthExpr
    // exercises the "single position" branch.
    init();
    let mut arena = NodeArena::default();
    let parent = arena.insert(Node::from(NodeType::Div));
    let mut children = Vec::new();
    for _ in 0..4 {
        let id = arena.insert(Node::from(NodeType::P));
        Node::append_child(&mut arena, parent, id);
        children.push(id);
    }
    let sel: CssSelector = ":nth-child(2)".parse().unwrap();
    assert!(!selector_chain_matches(&arena, children[0], &sel));
    assert!(selector_chain_matches(&arena, children[1], &sel));
    assert!(!selector_chain_matches(&arena, children[2], &sel));
}

#[test]
fn test_nth_child_an_plus_b_form() {
    // `:nth-child(3n+2)` must match indices 2, 5, 8, …
    init();
    let mut arena = NodeArena::default();
    let parent = arena.insert(Node::from(NodeType::Div));
    let mut children = Vec::new();
    for _ in 0..9 {
        let id = arena.insert(Node::from(NodeType::P));
        Node::append_child(&mut arena, parent, id);
        children.push(id);
    }
    let sel: CssSelector = ":nth-child(3n+2)".parse().unwrap();
    for (i, id) in children.iter().enumerate() {
        let expected = (i + 1 == 2) || (i + 1 == 5) || (i + 1 == 8);
        assert_eq!(
            selector_chain_matches(&arena, *id, &sel),
            expected,
            "3n+2 should match index {} → {}",
            i + 1,
            expected
        );
    }
}

#[test]
fn test_nth_child_compound_with_tag() {
    // `tr:nth-child(even)` — the visual-test form. The tag prefix must not
    // disturb the nth-child evaluation.
    init();
    let mut arena = NodeArena::default();
    let table = arena.insert(Node::from(NodeType::Table));
    let tr_ids: Vec<_> = (0..3)
        .map(|_| {
            let id = arena.insert(Node::from(NodeType::Tr));
            Node::append_child(&mut arena, table, id);
            id
        })
        .collect();
    let sel: CssSelector = "tr:nth-child(even)".parse().unwrap();
    assert!(!selector_chain_matches(&arena, tr_ids[0], &sel));
    assert!(selector_chain_matches(&arena, tr_ids[1], &sel));
    assert!(!selector_chain_matches(&arena, tr_ids[2], &sel));

    // A non-`<tr>` element at the same index must not match — the tag
    // filter is applied first.
    let span = arena.insert(Node::from(NodeType::Span));
    Node::append_child(&mut arena, table, span);
    assert!(!selector_chain_matches(&arena, span, &sel));
}

#[test]
fn test_nth_child_descendant_combinator() {
    // `table tr:nth-child(even)` — the exact selector used by the
    // `table/index.html` visual test for striping. The descendant
    // combinator must still walk ancestors correctly.
    init();
    let mut arena = NodeArena::default();
    let table = arena.insert(Node::from(NodeType::Table));
    let tr_ids: Vec<_> = (0..4)
        .map(|_| {
            let id = arena.insert(Node::from(NodeType::Tr));
            Node::append_child(&mut arena, table, id);
            id
        })
        .collect();
    let sel: CssSelector = "table tr:nth-child(even)".parse().unwrap();
    assert!(!selector_chain_matches(&arena, tr_ids[0], &sel));
    assert!(selector_chain_matches(&arena, tr_ids[1], &sel));
    assert!(!selector_chain_matches(&arena, tr_ids[2], &sel));
    assert!(selector_chain_matches(&arena, tr_ids[3], &sel));
}

#[test]
fn test_nth_child_nth_expr_matches_predicate() {
    // Direct unit test on NthExpr — covers the even/odd constants and the
    // `a*n + b` math without needing an arena.
    init();
    assert!(!NthExpr::EVEN.matches(1));
    assert!(NthExpr::EVEN.matches(2));
    assert!(!NthExpr::EVEN.matches(3));
    assert!(NthExpr::EVEN.matches(4));
    assert!(NthExpr::ODD.matches(1));
    assert!(!NthExpr::ODD.matches(2));
    assert!(NthExpr::ODD.matches(3));
    let expr = NthExpr { a: 3, b: 2 };
    assert!(expr.matches(2));
    assert!(expr.matches(5));
    assert!(expr.matches(8));
    assert!(!expr.matches(1));
    assert!(!expr.matches(4));
    // Degenerate a=0 — only the exact position matches.
    let single = NthExpr { a: 0, b: 3 };
    assert!(single.matches(3));
    assert!(!single.matches(2));
    assert!(!single.matches(4));
}

#[test]
fn test_nth_child_unsupported_forms_fail_parse() {
    // Forms outside the supported subset (bare `n`, negative coefficients,
    // signed offsets) must fail to parse so we never silently mis-match.
    init();
    assert!(":nth-child(n)".parse::<CssSelector>().is_err());
    assert!(":nth-child(-n+3)".parse::<CssSelector>().is_err());
    assert!(":nth-child(2n-1)".parse::<CssSelector>().is_err());
    assert!(":nth-child(0)".parse::<CssSelector>().is_err());
    // The bare-nth-child token without parentheses must also be rejected.
    assert!(":nth-child".parse::<CssSelector>().is_err());
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
    css.add(sel, CssAttr::from(m));
    css_into_node(&mut arena, root, &css);
    let div_id = arena[root].children[0];
    let p_id = arena[div_id].children[0];
    assert!(arena[p_id].css_attr.get_color().is_some());
}

#[test]
fn test_css_selector_matches_title_and_meta() {
    // The `tag` branch of the selector parser must recognise the metadata
    // tags so that user CSS targeting `<title>` / `<meta>` parses and
    // matches the corresponding DOM nodes.
    init();
    let mut arena = NodeArena::default();
    let root = arena.insert(Node::from(NodeType::Html));
    let head_id = arena.insert(Node::from(NodeType::Head));
    Node::append_child(&mut arena, root, head_id);
    let title_id = arena.insert(Node::from(NodeType::Title));
    let meta_id = arena.insert(Node::from(NodeType::Meta));
    Node::append_child(&mut arena, head_id, title_id);
    Node::append_child(&mut arena, head_id, meta_id);

    let title_sel: CssSelector = "title".parse().unwrap();
    let meta_sel: CssSelector = "meta".parse().unwrap();
    assert!(selector_chain_matches(&arena, title_id, &title_sel));
    assert!(selector_chain_matches(&arena, meta_id, &meta_sel));
    // Cross-tag sanity: the wrong selector must not match.
    assert!(!selector_chain_matches(&arena, meta_id, &title_sel));
    assert!(!selector_chain_matches(&arena, title_id, &meta_sel));
}

#[test]
fn test_universal_selector_parses_and_matches_elements_only() {
    // `*` must parse to the `Universal` variant, match every element node,
    // and never match non-element nodes (text / comment / document).
    init();
    let mut arena = NodeArena::default();
    let (root, div_id) = build_test(&mut arena);
    let sel: CssSelector = "*".parse().unwrap();
    assert_eq!(sel.compounds.len(), 1);
    assert_eq!(sel.compounds[0], vec![Selector::Universal]);

    let p_id = arena[div_id].children[0];
    let text_id = arena[p_id].children[0];
    // Elements match the universal selector.
    assert!(selector_chain_matches(&arena, div_id, &sel));
    assert!(selector_chain_matches(&arena, p_id, &sel));
    // Non-element nodes never match.
    assert!(!selector_chain_matches(&arena, root, &sel));
    assert!(!selector_chain_matches(&arena, text_id, &sel));
}

#[test]
fn test_universal_selector_in_compound_and_descendant() {
    // `*.foo` and `div *` are the standard ways the universal selector
    // combines with classes and combinators.
    init();
    let mut arena = NodeArena::default();
    let (_, div_id) = build_test(&mut arena);
    let compound_sel: CssSelector = "*.active".parse().unwrap();
    assert!(selector_chain_matches(&arena, div_id, &compound_sel));

    let descendant_sel: CssSelector = "div *".parse().unwrap();
    let p_id = arena[div_id].children[0];
    assert!(selector_chain_matches(&arena, p_id, &descendant_sel));
    // The div itself is not a descendant of itself.
    assert!(!selector_chain_matches(&arena, div_id, &descendant_sel));
}

#[test]
fn test_universal_selector_specificity_is_zero() {
    // Per the CSS spec `*` contributes (0,0,0), so `* div` and `div` tie;
    // a rule combining the two must not outrank the plain type selector.
    init();
    let univ: CssSelector = "*".parse().unwrap();
    let tag: CssSelector = "div".parse().unwrap();
    let chain: CssSelector = "* div".parse().unwrap();
    assert_eq!(univ.specificity(), (0, 0, 0));
    assert_eq!(chain.specificity(), tag.specificity());
}

#[test]
fn test_universal_selector_cascades_to_elements_not_text() {
    // A `* { background-color: red }` rule must paint every element but leave
    // text nodes untouched — the cascade visits the whole subtree, so the
    // matcher's element-only guard is what keeps text unstyled. (An inheritable
    // property like `color` would reach text nodes through inheritance, so the
    // test uses `background-color`, which does not inherit.)
    init();
    let mut arena = NodeArena::default();
    let (root, _) = build_test(&mut arena);
    let mut css = CssRule::new();
    let sel: CssSelector = "*".parse().unwrap();
    let mut m = IndexMap::new();
    m.insert("background-color".into(), "red".into());
    css.add(sel, CssAttr::from(m));
    css_into_node(&mut arena, root, &css);

    let div_id = arena[root].children[0];
    let p_id = arena[div_id].children[0];
    let text_id = arena[p_id].children[0];
    assert!(arena[div_id].css_attr.get_bg_color().is_some());
    assert!(arena[p_id].css_attr.get_bg_color().is_some());
    assert!(arena[text_id].css_attr.get_bg_color().is_none());
}

#[test]
fn test_e_type_selector_covers_every_element_tag() {
    // Completeness: every element `NodeType` must be addressable by a CSS
    // type selector, and the three tag lookups (DOM `tag_name`, HTML parser
    // map, selector parser) must agree. `NodeType::ALL` is the single
    // enumeration — a new element variant that forgets `tag_name` /
    // `from_tag_name` fails here instead of silently matching nothing.
    init();
    let mut arena = NodeArena::default();
    for nt in NodeType::ALL {
        match nt.tag_name() {
            Some(tag) => {
                // Round-trip through the shared reverse lookup.
                assert_eq!(NodeType::from_tag_name(tag), Some(*nt));
                assert_eq!(html_parse::tag_name_to_node_type(tag), Some(*nt));
                // The selector parser must accept the tag as a type selector
                // and match a node of exactly that type.
                let sel: CssSelector = tag
                    .parse()
                    .unwrap_or_else(|e| panic!("tag {tag} failed as a selector: {e}"));
                let node_id = arena.insert(Node::from(*nt));
                assert!(
                    selector_chain_matches(&arena, node_id, &sel),
                    "selector `{tag}` must match its node type"
                );
            }
            None => {
                // Document-structure nodes have no tag name and must not be
                // creatable from a tag string.
                assert!(
                    matches!(
                        nt,
                        NodeType::Document
                            | NodeType::DocumentType
                            | NodeType::DocumentFragment
                            | NodeType::Text
                            | NodeType::Comment
                    ),
                    "{nt:?} is an element but has no tag name"
                );
            }
        }
    }
}

#[test]
fn test_e_type_selector_rejects_unknown_tag() {
    // Unknown tags must still fail to parse (the rule is dropped), never
    // silently matching nothing.
    init();
    assert!("madeup".parse::<CssSelector>().is_err());
    // `strong` is a supported element today; use a genuinely unknown tag.
    assert!("div blink".parse::<CssSelector>().is_err());
}

#[test]
fn test_is_set_distinguishes_unset_from_explicit_none() {
    // `is_set` is the public hook the `<a>` renderer uses to decide whether
    // to apply the user-agent default underline — it must report `false`
    // when the property was never declared, and `true` once a declaration
    // (even `none`) is recorded in the underlying map.
    init();
    let mut attr = CssAttr::new();
    assert!(!attr.is_set("text-decoration"));
    assert!(!attr.is_set("color"));

    let declared = parse_declaration_text("text-decoration: none;");
    attr.add(&CssAttr(declared));
    assert!(
        attr.is_set("text-decoration"),
        "explicit `none` declaration must still count as set"
    );
    // Parsed accessor folds `none` back to the variant, but `is_set` only
    // looks at the raw key.
    assert_eq!(attr.get_text_decoration(), TextDecoration::None);
}

#[test]
fn test_color_name_parsing() {
    assert_eq!(
        parse_color_str("darkblue"),
        Some(Color::Rgb {
            r: 0x00,
            g: 0x00,
            b: 0x8B
        })
    );
    assert_eq!(
        parse_color_str("DarkBlue"),
        Some(Color::Rgb {
            r: 0x00,
            g: 0x00,
            b: 0x8B
        })
    );
    assert_eq!(
        parse_color_str("red"),
        Some(Color::Rgb {
            r: 0xFF,
            g: 0x00,
            b: 0x00
        })
    );
    assert_eq!(
        parse_color_str("darkred"),
        Some(Color::Rgb {
            r: 0x8B,
            g: 0x00,
            b: 0x00
        })
    );
    assert_eq!(
        parse_color_str("blue"),
        Some(Color::Rgb {
            r: 0x00,
            g: 0x00,
            b: 0xFF
        })
    );
    assert_eq!(
        parse_color_str("grey"),
        Some(Color::Rgb {
            r: 0x80,
            g: 0x80,
            b: 0x80
        })
    );
    assert_eq!(
        parse_color_str("gray"),
        Some(Color::Rgb {
            r: 0x80,
            g: 0x80,
            b: 0x80
        })
    );
}

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
fn test_color_rgba_and_modern_syntax() {
    // Legacy comma syntax with a 4th alpha component.
    assert_eq!(
        parse_color_str("rgba(255, 0, 0, 0.5)"),
        Some(Color::Rgb { r: 255, g: 0, b: 0 }),
        "rgba alpha is dropped (opaque)"
    );
    // CSS Color 4 space syntax with a `/ alpha` separator.
    assert_eq!(
        parse_color_str("rgb(255 0 0 / 50%)"),
        Some(Color::Rgb { r: 255, g: 0, b: 0 })
    );
    // Percentage components + out-of-range clamping (CSS Color 4).
    assert_eq!(
        parse_color_str("rgb(100% 0% 0%)"),
        Some(Color::Rgb { r: 255, g: 0, b: 0 })
    );
    assert_eq!(
        parse_color_str("rgb(300, 0, 0)"),
        Some(Color::Rgb { r: 255, g: 0, b: 0 }),
        "out-of-range components clamp to 0…255"
    );
    assert_eq!(
        parse_color_str("rgb(20% 40% 60%)"),
        Some(Color::Rgb {
            r: 51,
            g: 102,
            b: 153
        })
    );
}

#[test]
fn test_color_hsl_parsing() {
    // hsl(0, 100%, 50%) == red; hsl(120, 100%, 50%) == green.
    assert_eq!(
        parse_color_str("hsl(0, 100%, 50%)"),
        Some(Color::Rgb { r: 255, g: 0, b: 0 })
    );
    assert_eq!(
        parse_color_str("hsl(120, 100%, 50%)"),
        Some(Color::Rgb { r: 0, g: 255, b: 0 })
    );
    // Space syntax + `/ alpha` (dropped).
    assert_eq!(
        parse_color_str("hsl(120 100% 50% / 0.25)"),
        Some(Color::Rgb { r: 0, g: 255, b: 0 })
    );
    assert_eq!(
        parse_color_str("hsl(480, 100%, 50%)"),
        Some(Color::Rgb { r: 0, g: 255, b: 0 })
    );
    // Zero saturation → gray.
    assert_eq!(
        parse_color_str("hsl(0, 0%, 50%)"),
        Some(Color::Rgb {
            r: 128,
            g: 128,
            b: 128
        })
    );
    // hsla legacy comma syntax with the 4th alpha component.
    assert_eq!(
        parse_color_str("hsla(0, 100%, 50%, 0.5)"),
        Some(Color::Rgb { r: 255, g: 0, b: 0 })
    );
}

#[test]
fn test_color_hex_alpha_parsing() {
    assert_eq!(
        parse_color_str("#ff000080"),
        Some(Color::Rgb { r: 255, g: 0, b: 0 }),
        "#RRGGBBAA drops the alpha channel"
    );
    assert_eq!(
        parse_color_str("#f008"),
        Some(Color::Rgb { r: 255, g: 0, b: 0 }),
        "#RGBA drops the alpha channel"
    );
}

#[test]
fn test_color_transparent_and_malformed() {
    // `transparent` is accepted for backgrounds / borders as "no paint"
    // (callers treat None that way), so a document background falls through
    // to the parent's, matching the browser.
    assert_eq!(parse_color_str("transparent"), None);
    // Malformed functional colors are dropped, not guessed.
    assert_eq!(parse_color_str("rgb(0, 0)"), None);
    assert_eq!(parse_color_str("rgb(0 0)"), None);
    assert_eq!(parse_color_str("hsl(0, 100%)"), None);
    assert_eq!(parse_color_str("rgba(0, 0, 0, 1.5)"), None);
    assert_eq!(parse_color_str("hsl(0, 0%, 0%, 2)"), None);
}

#[test]
fn test_css_attr_full_color() {
    let mut attr = CssAttr::new();
    let css = "color: darkblue; background-color: #1e293b;";
    let m = parse_declaration_text(css);
    attr.add(&CssAttr(m));
    assert_eq!(
        attr.get_color(),
        Some(Color::Rgb {
            r: 0x00,
            g: 0x00,
            b: 0x8B
        })
    );
    assert_eq!(
        attr.get_bg_color(),
        Some(Color::Rgb {
            r: 0x1e,
            g: 0x29,
            b: 0x3b
        })
    );
}

// ============================================================
// #1 refactor: property registry & parse robustness
// ============================================================

#[test]
fn test_supported_properties_lists_known_names() {
    // The registry is the single source of truth for property names.
    assert!(SUPPORTED_PROPERTIES.contains(&"color"));
    assert!(SUPPORTED_PROPERTIES.contains(&"background-color"));
    assert!(SUPPORTED_PROPERTIES.contains(&"background"));
    assert!(SUPPORTED_PROPERTIES.contains(&"text-align"));
    assert!(SUPPORTED_PROPERTIES.contains(&"font-weight"));
    assert!(SUPPORTED_PROPERTIES.contains(&"font-style"));
    assert!(SUPPORTED_PROPERTIES.contains(&"text-decoration"));
    assert!(SUPPORTED_PROPERTIES.contains(&"width"));
    // Box-model longhands + shorthands.
    assert!(SUPPORTED_PROPERTIES.contains(&"padding"));
    assert!(SUPPORTED_PROPERTIES.contains(&"padding-top"));
    assert!(SUPPORTED_PROPERTIES.contains(&"margin"));
    assert!(SUPPORTED_PROPERTIES.contains(&"margin-left"));
    assert!(SUPPORTED_PROPERTIES.contains(&"border"));
    assert!(SUPPORTED_PROPERTIES.contains(&"border-top"));
    assert!(SUPPORTED_PROPERTIES.contains(&"border-width"));
    assert!(SUPPORTED_PROPERTIES.contains(&"border-style"));
    assert!(SUPPORTED_PROPERTIES.contains(&"border-color"));
    // Visibility / overflow (overflow keeps the renderer's default clip).
    assert!(SUPPORTED_PROPERTIES.contains(&"visibility"));
    assert!(SUPPORTED_PROPERTIES.contains(&"overflow"));
}

#[test]
fn visibility_is_supported_and_inheritable() {
    init();
    assert!(SUPPORTED_PROPERTIES.contains(&"visibility"));
    assert!(INHERITED_PROPS.contains(&"visibility"));
}

#[test]
fn overflow_hidden_is_parsed_and_kept_by_the_cascade() {
    init();
    // `overflow` is a supported property: the declaration survives the
    // cascade (the renderer's default already clips a box's static children
    // to its content area, so `overflow: hidden` matches that behaviour).
    let mut arena = NodeArena::default();
    let (root, div) = build_test(&mut arena);
    let css = Parser::new("div { overflow: hidden }").parse();
    css_into_node(&mut arena, root, &css);
    assert_eq!(
        arena[div].css_attr.0.get("overflow").map(|s| s.as_ref()),
        Some("hidden"),
        "overflow: hidden must survive the cascade"
    );
}

#[test]
fn test_inherited_props_is_a_subset_of_supported() {
    for p in INHERITED_PROPS {
        assert!(
            SUPPORTED_PROPERTIES.contains(p),
            "{p} is in INHERITED_PROPS but missing from SUPPORTED_PROPERTIES"
        );
    }
}

#[test]
fn test_inherited_props_contains_visual_inheritable_set() {
    // The set is curated by hand, so guard against accidental removal of
    // the five classic inheritable CSS properties.
    assert!(INHERITED_PROPS.contains(&"color"));
    assert!(INHERITED_PROPS.contains(&"text-align"));
    assert!(INHERITED_PROPS.contains(&"font-weight"));
    assert!(INHERITED_PROPS.contains(&"font-style"));
    assert!(INHERITED_PROPS.contains(&"text-decoration"));
    // Properties that should NOT inherit (negative set).
    assert!(!INHERITED_PROPS.contains(&"background-color"));
    assert!(!INHERITED_PROPS.contains(&"background"));
    assert!(!INHERITED_PROPS.contains(&"width"));
}

#[test]
fn test_important_marker_is_stripped_from_value() {
    let m = parse_declaration_text("color: red !important;");
    assert_eq!(m.get("color").map(|s| s.as_ref()), Some("red"));
}

#[test]
fn test_important_marker_with_space_is_not_recognised() {
    let m = parse_declaration_text("font-weight: bold ! important;");
    // CSS requires `!important` with no space; `! important` is invalid.
    // cssparser tokenises the `!` as an unknown token at declaration value
    // level, drops it, and parses the rest as the value. We end up storing
    // `bold` (without a false !important signal). What we assert here is
    // that we do NOT silently accept it as important.
    let val = m.get("font-weight").map(|s| s.as_ref());
    assert_eq!(val, Some("bold"));
    assert!(
        !val.unwrap().contains("!important"),
        "must not smuggle the bogus `! important` into the value"
    );
}

#[test]
fn test_important_marker_with_leading_space_is_kept_out_of_value() {
    let m = parse_declaration_text("font-style: italic  !important ;");
    assert_eq!(m.get("font-style").map(|s| s.as_ref()), Some("italic"));
}

#[test]
fn test_block_comment_is_stripped() {
    let m = parse_declaration_text("/* color: ignored */ color: red;");
    assert_eq!(m.get("color").map(|s| s.as_ref()), Some("red"));
    assert_eq!(m.len(), 1, "commented-out decl must not appear as a key");
}

#[test]
fn test_block_comment_inside_string_is_preserved() {
    // cssparser tracks string-literal state, so a `/* */` token inside
    // `"..."` is part of the value and survives parsing intact. (Before
    // the cssparser-based rewrite, the comment was textually stripped,
    // corrupting quoted strings.)
    let m = parse_declaration_text("content: \"a /* b */ c\";");
    assert_eq!(
        m.get("content").map(|s| s.as_ref()),
        Some("\"a /* b */ c\"")
    );
}

#[test]
fn test_unterminated_comment_truncates_input() {
    // Defensive: malformed CSS should not panic and should not leak the
    // comment text into a property key.
    let m = parse_declaration_text("color: red; /* oops");
    assert_eq!(m.get("color").map(|s| s.as_ref()), Some("red"));
}

#[test]
fn test_missing_trailing_semicolon_is_ok() {
    let m = parse_declaration_text("color: blue");
    assert_eq!(m.get("color").map(|s| s.as_ref()), Some("blue"));
}

// ============================================================
// Cascade rewrite: specificity ordering, `!important` and inline style
// ============================================================

#[test]
fn test_parse_splits_important_declarations() {
    init();
    // The stylesheet parser records `!important` per property: normal and
    // important declarations end up in separate bags on the rule entry.
    let css = Parser::new("p { color: red !important; width: 10px }").parse();
    assert_eq!(css.0.len(), 1);
    let entry = &css.0[0];
    assert_eq!(
        entry.normal.0.get("width").map(|s| s.as_ref()),
        Some("10px")
    );
    assert!(entry.normal.0.get("color").is_none());
    assert_eq!(
        entry.important.0.get("color").map(|s| s.as_ref()),
        Some("red")
    );
    assert!(entry.important.0.get("width").is_none());
}

#[test]
fn test_cascade_specificity_units() {
    init();
    let spec = |s: &str| s.parse::<CssSelector>().unwrap().specificity();
    assert_eq!(spec("p"), (0, 0, 1));
    assert_eq!(spec(".a"), (0, 1, 0));
    assert_eq!(spec("#i"), (1, 0, 0));
    assert_eq!(spec("div p span"), (0, 0, 3));
    assert_eq!(spec(".a .b"), (0, 2, 0));
    assert_eq!(spec("div.active"), (0, 1, 1));
    assert_eq!(spec("p:hover"), (0, 1, 1));
    assert_eq!(spec("li:nth-child(2)"), (0, 1, 1));
    assert_eq!(spec("div > p + span"), (0, 0, 3));
}

#[test]
fn test_cascade_specificity_wins_regardless_of_order() {
    init();
    // `div p` (0,0,2) must beat `p` (0,0,1) no matter the source order.
    for css_text in [
        "div p { color: red } p { color: blue }",
        "p { color: blue } div p { color: red }",
    ] {
        let mut arena = NodeArena::default();
        let (root, div_id) = build_test(&mut arena);
        let p_id = arena[div_id].children[0];
        let css = Parser::new(css_text).parse();
        css_into_node(&mut arena, root, &css);
        assert_eq!(
            arena[p_id].css_attr.get_color(),
            Some(Color::Rgb {
                r: 0xFF,
                g: 0x00,
                b: 0x00
            }),
            "css: {css_text}"
        );
    }
}

#[test]
fn test_cascade_important_beats_normal() {
    init();
    // `!important` beats normal regardless of source order.
    for css_text in [
        "p { color: blue } p { color: red !important }",
        "p { color: red !important } p { color: blue }",
    ] {
        let mut arena = NodeArena::default();
        let (root, div_id) = build_test(&mut arena);
        let p_id = arena[div_id].children[0];
        let css = Parser::new(css_text).parse();
        css_into_node(&mut arena, root, &css);
        assert_eq!(
            arena[p_id].css_attr.get_color(),
            Some(Color::Rgb {
                r: 0xFF,
                g: 0x00,
                b: 0x00
            }),
            "css: {css_text}"
        );
    }
}

#[test]
fn test_cascade_important_beats_higher_specificity_normal() {
    init();
    // `!important` outranks specificity: `p { … !important }` beats
    // `.b { … }` even though the class selector is more specific.
    let mut arena = NodeArena::default();
    let (root, p_id) = build_p_with_classes(&mut arena);
    let css = Parser::new("p { color: red !important } .b { color: blue }").parse();
    css_into_node(&mut arena, root, &css);
    assert_eq!(
        arena[p_id].css_attr.get_color(),
        Some(Color::Rgb {
            r: 0xFF,
            g: 0x00,
            b: 0x00
        })
    );
}

#[test]
fn test_cascade_inline_style_beats_stylesheet_normal() {
    init();
    let mut arena = NodeArena::default();
    let (root, div_id) = build_test(&mut arena);
    let p_id = arena[div_id].children[0];
    arena[p_id]
        .attributes
        .insert("style".to_string(), "color: blue".to_string());
    let css = Parser::new("p { color: red }").parse();
    css_into_node(&mut arena, root, &css);
    assert_eq!(
        arena[p_id].css_attr.get_color(),
        Some(Color::Rgb {
            r: 0x00,
            g: 0x00,
            b: 0xFF
        })
    );
}

#[test]
fn test_cascade_important_precedence_with_inline_style() {
    init();
    // Stylesheet `!important` beats inline normal declarations.
    let mut arena = NodeArena::default();
    let (root, div_id) = build_test(&mut arena);
    let p_id = arena[div_id].children[0];
    arena[p_id]
        .attributes
        .insert("style".to_string(), "color: blue".to_string());
    let css = Parser::new("p { color: red !important }").parse();
    css_into_node(&mut arena, root, &css);
    assert_eq!(
        arena[p_id].css_attr.get_color(),
        Some(Color::Rgb {
            r: 0xFF,
            g: 0x00,
            b: 0x00
        })
    );

    // Inline `!important` beats stylesheet `!important`.
    let mut arena2 = NodeArena::default();
    let (root2, div2) = build_test(&mut arena2);
    let p2 = arena2[div2].children[0];
    arena2[p2]
        .attributes
        .insert("style".to_string(), "color: blue !important".to_string());
    let css2 = Parser::new("p { color: red !important }").parse();
    css_into_node(&mut arena2, root2, &css2);
    assert_eq!(
        arena2[p2].css_attr.get_color(),
        Some(Color::Rgb {
            r: 0x00,
            g: 0x00,
            b: 0xFF
        })
    );
}

#[test]
fn test_cascade_same_specificity_later_source_wins() {
    init();
    // `.a` appears before and after `.b` at equal specificity; the last
    // declaration in source order wins (green). The old same-selector rule
    // merging would have collapsed `.a`'s two occurrences and reported blue.
    let mut arena = NodeArena::default();
    let (root, p_id) = build_p_with_classes(&mut arena);
    let css = Parser::new(".a { color: red } .b { color: blue } .a { color: green }").parse();
    css_into_node(&mut arena, root, &css);
    assert_eq!(
        arena[p_id].css_attr.get_color(),
        Some(Color::Rgb {
            r: 0x00,
            g: 0x80,
            b: 0x00
        })
    );
}

#[test]
fn test_background_color_does_not_inherit() {
    // End-to-end: even though `background-color` is in SUPPORTED_PROPERTIES,
    // it must NOT propagate from parent to child. Set it on the parent and
    // verify the child does not pick it up.
    init();
    let mut arena = NodeArena::default();
    let (root, _) = build_test(&mut arena);
    let mut css = CssRule::new();
    let sel: CssSelector = "div".parse().unwrap();
    let mut m = IndexMap::new();
    m.insert("background-color".into(), "yellow".into());
    css.add(sel, CssAttr::from(m));
    css_into_node(&mut arena, root, &css);

    let div_id = arena[root].children[0];
    let p_id = arena[div_id].children[0];
    assert_eq!(
        arena[div_id].css_attr.get_bg_color(),
        Some(Color::Rgb {
            r: 0xFF,
            g: 0xFF,
            b: 0x00
        }),
        "div should have its own bg"
    );
    assert_eq!(
        arena[p_id].css_attr.get_bg_color(),
        None,
        "non-inheritable property must not leak to descendant"
    );
}

#[test]
fn test_color_does_inherit() {
    // Symmetric counterpart to the above — color IS inheritable and the
    // grandchild <p> must pick up the div's color.
    init();
    let mut arena = NodeArena::default();
    let (root, _) = build_test(&mut arena);
    let mut css = CssRule::new();
    let sel: CssSelector = "div".parse().unwrap();
    let mut m = IndexMap::new();
    m.insert("color".into(), "red".into());
    css.add(sel, CssAttr::from(m));
    css_into_node(&mut arena, root, &css);

    let div_id = arena[root].children[0];
    let p_id = arena[div_id].children[0];
    assert_eq!(
        arena[div_id].css_attr.get_color(),
        Some(Color::Rgb {
            r: 0xFF,
            g: 0x00,
            b: 0x00
        })
    );
    assert_eq!(
        arena[p_id].css_attr.get_color(),
        Some(Color::Rgb {
            r: 0xFF,
            g: 0x00,
            b: 0x00
        }),
        "inheritable property must propagate to descendant"
    );
}

#[test]
fn test_full_parse_pipeline_handles_important_and_comments() {
    // Pins !important stripping + comment removal at the call sites used
    // by the rest of the codebase (Parser::parse + apply_css_rule).
    init();
    let mut parser = Parser::new(
        "/* heading style */\n\
         h1 { color: red !important; font-weight: bold; }\n\
         h2 { /* nothing */ font-style: italic; }",
    );
    let _rules = parser.parse();

    let mut arena = NodeArena::default();
    let root = arena.insert(Node::from(NodeType::Document));
    let h1 = arena.insert(Node::from(NodeType::H1));
    Node::append_child(&mut arena, root, h1);

    let mut css = CssRule::new();
    let sel_h1: CssSelector = "h1".parse().unwrap();
    let mut decls = IndexMap::new();
    decls.insert("color".into(), "red".into());
    decls.insert("font-weight".into(), "bold".into());
    css.add(sel_h1, CssAttr::from(decls));
    css_into_node(&mut arena, root, &css);

    assert_eq!(
        arena[h1].css_attr.get_color(),
        Some(Color::Rgb {
            r: 0xFF,
            g: 0x00,
            b: 0x00
        })
    );
    assert_eq!(arena[h1].css_attr.get_font_weight(), FontWeight::Bold);
}

// =====================================================================
// Length value type tests
// =====================================================================

#[test]
fn length_parse_auto() {
    init();
    assert_eq!(Length::parse("auto"), Some(Length::Auto));
    assert_eq!(Length::parse("AUTO"), Some(Length::Auto));
    assert_eq!(Length::parse("Auto"), Some(Length::Auto));
    assert_eq!(Length::parse("  auto  "), Some(Length::Auto));
}

#[test]
fn length_parse_cells_px() {
    init();
    // `px` is now kept as a distinct `Px` variant and resolves through
    // `Length::resolve` instead of being collapsed at parse time. The browser
    // pipeline treats `px` as an absolute unit and only normalises to cells
    // at resolution; we mirror that.
    assert_eq!(Length::parse("40px"), Some(Length::Px(40.0)));
    assert_eq!(Length::parse("  40px  "), Some(Length::Px(40.0)));
    // Case-insensitive unit.
    assert_eq!(Length::parse("20Px"), Some(Length::Px(20.0)));
    assert_eq!(Length::parse("30PX"), Some(Length::Px(30.0)));
    // Trailing whitespace tolerated.
    assert_eq!(Length::parse("20px\n"), Some(Length::Px(20.0)));
}

#[test]
fn px_per_cell_constant_anchors_px_to_browser_scale() {
    // Browser-scale px semantics: one terminal cell is 8 CSS px (a monospace
    // glyph is 0.5em wide at the 16px default font-size). px lengths divide
    // by this at resolution; this is the single edit-point for any future
    // DPI mode (e.g. 2x retina = 16.0).
    assert_eq!(PX_PER_CELL, 8.0);
}

#[test]
fn length_parse_cells_px_fractional_rounds_half_away_from_zero() {
    init();
    // `parse` stores the raw f32 multiplier; the rounding to integer cells
    // happens inside `Length::resolve` (which mirrors the browser's
    // "rendered pixel" step). We use round-half-away-from-zero (Rust's
    // f32::round), not banker's rounding, because terminals need a stable,
    // monotonic mapping. These edge cases pin the standardized behavior.
    assert_eq!(Length::parse("0.4px"), Some(Length::Px(0.4)));
    assert_eq!(Length::parse("0.5px"), Some(Length::Px(0.5)));
    assert_eq!(Length::parse("0.6px"), Some(Length::Px(0.6)));
    assert_eq!(Length::parse("1.5px"), Some(Length::Px(1.5)));
    assert_eq!(Length::parse("2.5px"), Some(Length::Px(2.5)));
    assert_eq!(Length::parse("100.4px"), Some(Length::Px(100.4)));
}

#[test]
fn length_parse_cells_px_zero_and_integer_exact() {
    init();
    // 0px must parse to Px(0), not None -- otherwise zero-sized boxes would
    // silently fall through to the containing block width.
    assert_eq!(Length::parse("0px"), Some(Length::Px(0.0)));
    assert_eq!(Length::parse("1px"), Some(Length::Px(1.0)));
    assert_eq!(Length::parse("1000px"), Some(Length::Px(1000.0)));
}

#[test]
fn length_parse_cells_px_divide_matches_browser_scale() {
    // `Length::Px(n).resolve(...)` divides by PX_PER_CELL (8px per cell) and
    // rounds, mirroring a browser rendered on a character grid: `parse`
    // keeps the raw px multiplier; resolve applies the ratio. This test pins
    // the division so any future change to the constant forces a deliberate
    // update here.
    init();
    assert_eq!(
        (40.0_f32 / PX_PER_CELL).round() as usize,
        Length::parse("40px")
            .unwrap()
            .resolve(length_ctx(80, default_font_ctx()))
    );
}

#[test]
fn length_parse_cells_bare_number() {
    init();
    // Bare integers and floats parse as `Px`; CSS treats a unitless number
    // as px, so `Length::resolve` divides by PX_PER_CELL. `parse` preserves
    // the raw f32 value; rounding happens inside `Length::resolve`.
    assert_eq!(Length::parse("40"), Some(Length::Px(40.0)));
    assert_eq!(Length::parse("0"), Some(Length::Px(0.0)));
    assert_eq!(Length::parse("1.5"), Some(Length::Px(1.5)));
    assert_eq!(Length::parse("0.5"), Some(Length::Px(0.5)));
}

#[test]
fn length_parse_pct() {
    init();
    // Integer percentages round-trip through f32 exactly; non-integer
    // percentages (e.g. 33.33%) inherit f32 rounding error from the
    // tokenizer. The resolve tests below cover fractional percentages.
    assert_eq!(Length::parse("50%"), Some(Length::Pct(50.0)));
    assert_eq!(Length::parse("33%"), Some(Length::Pct(33.0)));
    assert_eq!(Length::parse("0%"), Some(Length::Pct(0.0)));
    assert_eq!(Length::parse("100%"), Some(Length::Pct(100.0)));
}

#[test]
fn length_parse_em_rem() {
    init();
    assert_eq!(Length::parse("2em"), Some(Length::Em(2.0)));
    assert_eq!(Length::parse("1.5em"), Some(Length::Em(1.5)));
    assert_eq!(Length::parse("2REM"), Some(Length::Rem(2.0)));
    assert_eq!(Length::parse("0.5rem"), Some(Length::Rem(0.5)));
}

#[test]
fn length_parse_vw_vh() {
    init();
    // 1vw = viewport_cols / 100, 1vh = viewport_rows / 100.
    // The parser only stores the raw multiplier; the viewport is supplied
    // at `Length::resolve` time.
    assert_eq!(Length::parse("50vw"), Some(Length::Vw(50.0)));
    assert_eq!(Length::parse("50vh"), Some(Length::Vh(50.0)));
    assert_eq!(Length::parse("100VW"), Some(Length::Vw(100.0)));
    assert_eq!(Length::parse("12.5vh"), Some(Length::Vh(12.5)));
}

#[test]
fn length_parse_ch() {
    init();
    // `ch` is the CSS character-width unit: 1ch == the advance width of the
    // `0` glyph, which is exactly one cell on a terminal's monospace grid.
    // Unlike `em`, `ch` measures characters, so widths stay aligned with
    // character counts regardless of font-size.
    assert_eq!(Length::parse("12ch"), Some(Length::Ch(12.0)));
    assert_eq!(Length::parse("1.5ch"), Some(Length::Ch(1.5)));
    assert_eq!(Length::parse("8CH"), Some(Length::Ch(8.0)));
    assert_eq!(Length::parse(" 4ch  "), Some(Length::Ch(4.0)));
}

#[test]
fn length_parse_rejects_unsupported_units() {
    init();
    // vmin / vmax not yet wired through (see doc/layout.md 10.6).
    assert_eq!(Length::parse("50vmin"), None);
    assert_eq!(Length::parse("50vmax"), None);
    // Physical units — no DPI.
    assert_eq!(Length::parse("12pt"), None);
    assert_eq!(Length::parse("5cm"), None);
    assert_eq!(Length::parse("5mm"), None);
    assert_eq!(Length::parse("1in"), None);
    assert_eq!(Length::parse("1pc"), None);
}

#[test]
fn length_parse_rejects_signed_and_junk() {
    init();
    assert_eq!(Length::parse("-10px"), None);
    assert_eq!(Length::parse("+10px"), None);
    assert_eq!(Length::parse(""), None);
    assert_eq!(Length::parse("   "), None);
    // CSS forbids whitespace between number and unit (Number then Ident).
    assert_eq!(Length::parse("40 px"), None);
    assert_eq!(Length::parse("40 PX"), None);
    // Trailing junk after a valid value.
    assert_eq!(Length::parse("40px garbage"), None);
    // Bare garbage.
    assert_eq!(Length::parse("hello"), None);
    assert_eq!(Length::parse("!important"), None);
}

#[test]
fn length_resolve_auto_and_cells() {
    init();
    assert_eq!(Length::Auto.resolve(length_ctx(80, default_font_ctx())), 80);
    // A bare number is px per CSS, so it divides by PX_PER_CELL like `px`:
    // 40 -> 5 cells at 8px/cell.
    assert_eq!(
        Length::Px(40.0).resolve(length_ctx(80, default_font_ctx())),
        5
    );
    assert_eq!(
        Length::Px(0.0).resolve(length_ctx(80, default_font_ctx())),
        0
    ); // caller clamps via resolve_block_width
    assert_eq!(
        Length::Px(1000.0).resolve(length_ctx(80, default_font_ctx())),
        125
    ); // ditto
}

#[test]
fn length_resolve_pct_rounds_and_min_1() {
    init();
    assert_eq!(
        Length::Pct(50.0).resolve(length_ctx(80, default_font_ctx())),
        40
    );
    assert_eq!(
        Length::Pct(100.0).resolve(length_ctx(80, default_font_ctx())),
        80
    );
    // round(), not truncate: 80 * 33 / 100 = 26.4 → 26.
    assert_eq!(
        Length::Pct(33.0).resolve(length_ctx(80, default_font_ctx())),
        26
    );
    // Half-percent boundary: 80 * 50.5 / 100 = 40.4 → 40 (round).
    assert_eq!(
        Length::Pct(50.5).resolve(length_ctx(80, default_font_ctx())),
        40
    );
    // 80 * 50.6 / 100 = 40.48 → 40 (round).
    assert_eq!(
        Length::Pct(50.6).resolve(length_ctx(80, default_font_ctx())),
        40
    );
    // Minimum 1 column: a non-zero percentage of zero yields 1, not 0.
    assert_eq!(
        Length::Pct(50.0).resolve(length_ctx(0, default_font_ctx())),
        1
    );
    assert_eq!(
        Length::Pct(0.0).resolve(length_ctx(80, default_font_ctx())),
        1
    ); // min-1 even at 0%
}

#[test]
fn length_resolve_em_rem_rounds_away_from_zero() {
    init();
    // With the default font-size / root-font-size of 1, em and rem collapse
    // to `n.round()`, so the existing round-away-from-zero behaviour holds.
    assert_eq!(
        Length::Em(2.0).resolve(length_ctx(80, default_font_ctx())),
        2
    );
    assert_eq!(
        Length::Em(1.5).resolve(length_ctx(80, default_font_ctx())),
        2
    ); // 1.5 rounds away → 2
    assert_eq!(
        Length::Em(0.5).resolve(length_ctx(80, default_font_ctx())),
        1
    ); // 0.5 rounds away → 1
    assert_eq!(
        Length::Em(2.5).resolve(length_ctx(80, default_font_ctx())),
        3
    ); // 2.5 rounds away → 3
    assert_eq!(
        Length::Rem(1.5).resolve(length_ctx(80, default_font_ctx())),
        2
    );
    assert_eq!(
        Length::Rem(0.0).resolve(length_ctx(80, default_font_ctx())),
        0
    );
}

#[test]
fn length_resolve_em_scales_with_font_size() {
    init();
    // em resolves against the element's font-size: 2em at font-size=2
    // yields 4 cells, 1.5em at font-size=3 yields 5 cells (round away).
    assert_eq!(Length::Em(2.0).resolve(length_ctx(80, font_ctx(2, 1))), 4);
    assert_eq!(Length::Em(1.5).resolve(length_ctx(80, font_ctx(3, 1))), 5);
    assert_eq!(Length::Em(0.5).resolve(length_ctx(80, font_ctx(2, 1))), 1);
}

#[test]
fn length_resolve_rem_scales_with_root_font_size() {
    init();
    // rem ignores the element's font-size and uses the root font-size: 2rem
    // at root font-size=3 yields 6 cells, regardless of the element's size.
    assert_eq!(Length::Rem(2.0).resolve(length_ctx(80, font_ctx(1, 3))), 6);
    assert_eq!(Length::Rem(1.5).resolve(length_ctx(80, font_ctx(2, 4))), 6);
    assert_eq!(Length::Rem(0.0).resolve(length_ctx(80, font_ctx(5, 5))), 0);
}

#[test]
fn length_resolve_ch_is_font_size_independent() {
    init();
    // 1ch == 1 cell no matter the effective font-size — unlike `em`, `ch`
    // counts characters, and a terminal cell *is* a character. Rounding
    // follows the same away-from-zero rule as the other multipliers.
    assert_eq!(Length::Ch(12.0).resolve(length_ctx(80, default_font_ctx())), 12);
    assert_eq!(Length::Ch(1.5).resolve(length_ctx(80, font_ctx(2, 1))), 2); // 1.5 rounds away
    assert_eq!(Length::Ch(0.4).resolve(length_ctx(80, font_ctx(3, 2))), 0);
}

#[test]
fn length_resolve_vw_vh_anchor_to_viewport() {
    init();
    // 1vw = viewport_cols / 100, 1vh = viewport_rows / 100.
    // `containing` is intentionally a different number to prove `vw` does
    // NOT anchor to the containing block — it always uses the viewport.
    assert_eq!(Length::Vw(50.0).resolve(length_ctx(80, font_ctx(1, 1))), 40); // 50 * 80 / 100
    assert_eq!(
        Length::Vw(100.0).resolve(length_ctx(80, font_ctx(1, 1))),
        80
    );
    assert_eq!(
        Length::Vw(50.0).resolve(length_ctx(999, font_ctx(1, 1))),
        40
    ); // containing irrelevant
    assert_eq!(Length::Vh(50.0).resolve(length_ctx(80, font_ctx(1, 1))), 12); // 50 * 24 / 100
    assert_eq!(
        Length::Vh(100.0).resolve(length_ctx(80, font_ctx(1, 1))),
        24
    );
    // Rounding: 33.3 * 80 / 100 = 26.64 → 27.
    assert_eq!(Length::Vw(33.3).resolve(length_ctx(80, font_ctx(1, 1))), 27);
    // Min-1 clamp: 0.5vw at viewport_cols=80 → 0.4 → 1.
    assert_eq!(Length::Vw(0.5).resolve(length_ctx(80, font_ctx(1, 1))), 1);
}

#[test]
fn css_attr_get_width_end_to_end() {
    init();
    // Mirrors the existing `width: ...` semantics: `auto` / missing /
    // unsupported returns None (caller fills the container); everything
    // else resolves into a concrete column count.
    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "auto".into());
    attr.add(&CssAttr::from(decls));
    assert_eq!(
        attr.get_width(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        None
    );

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "40ch".into());
    attr.add(&CssAttr::from(decls));
    assert_eq!(
        attr.get_width(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        Some(40)
    );

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "50%".into());
    attr.add(&CssAttr::from(decls));
    assert_eq!(
        attr.get_width(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        Some(40)
    );

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "2em".into());
    attr.add(&CssAttr::from(decls));
    assert_eq!(
        attr.get_width(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        Some(2)
    );

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "1.5em".into());
    attr.add(&CssAttr::from(decls));
    assert_eq!(
        attr.get_width(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        Some(2)
    ); // 1.5 rounds away

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "2rem".into());
    attr.add(&CssAttr::from(decls));
    assert_eq!(
        attr.get_width(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        Some(2)
    );

    // 50vw anchors to viewport_cols (80 cols here) → 50 * 80 / 100 = 40.
    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "50vw".into());
    attr.add(&CssAttr::from(decls));
    assert_eq!(
        attr.get_width(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        Some(40)
    );

    // 50vh anchors to viewport_rows (24 rows here) → 50 * 24 / 100 = 12.
    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "50vh".into());
    attr.add(&CssAttr::from(decls));
    assert_eq!(
        attr.get_width(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        Some(12)
    );

    // Missing property → None.
    assert_eq!(
        CssAttr::new().get_width(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        None
    );
}

// ============================================================
// Box-model: padding / margin / border parsing
// ============================================================

fn attr_from_decl(name: &str, value: &str) -> CssAttr {
    let mut decls = IndexMap::new();
    decls.insert(name.into(), value.into());
    let mut attr = CssAttr::new();
    attr.add(&CssAttr::from(decls));
    attr
}

#[test]
fn padding_shorthand_one_value_fills_all_sides() {
    let attr = attr_from_decl("padding", "5ch");
    let edges = attr.get_padding(
        80,
        font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        viewport(),
    );
    assert_eq!(edges, EdgeSizes::new(5, 5, 5, 5));
}

#[test]
fn padding_shorthand_two_values_vertical_horizontal() {
    // `padding: 10ch 20ch;` → top/bottom = 10, left/right = 20
    let attr = attr_from_decl("padding", "10ch 20ch");
    assert_eq!(
        attr.get_padding(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        EdgeSizes::new(10, 20, 10, 20)
    );
}

#[test]
fn padding_shorthand_three_values_top_horizontal_bottom() {
    // `padding: 1ch 2ch 3ch;` → top=1, left/right=2, bottom=3
    let attr = attr_from_decl("padding", "1ch 2ch 3ch");
    assert_eq!(
        attr.get_padding(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        EdgeSizes::new(1, 2, 3, 2)
    );
}

#[test]
fn padding_shorthand_four_values_clockwise() {
    // `padding: 1ch 2ch 3ch 4ch;` → top=1, right=2, bottom=3, left=4
    let attr = attr_from_decl("padding", "1ch 2ch 3ch 4ch");
    assert_eq!(
        attr.get_padding(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        EdgeSizes::new(1, 2, 3, 4)
    );
}

#[test]
fn padding_longhands_override_shorthand() {
    // `padding: 0; padding-top: 7ch;` → top=7, others=0
    let mut decls = IndexMap::new();
    decls.insert("padding".into(), "0".into());
    decls.insert("padding-top".into(), "7ch".into());
    let mut attr = CssAttr::new();
    attr.add(&CssAttr::from(decls));
    assert_eq!(
        attr.get_padding(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        EdgeSizes::new(7, 0, 0, 0)
    );
}

#[test]
fn padding_supports_percent_and_em() {
    let attr = attr_from_decl("padding", "50%");
    assert_eq!(
        attr.get_padding(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        )
        .left,
        40
    );
    let attr = attr_from_decl("padding-top", "3em");
    assert_eq!(
        attr.get_padding(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        )
        .top,
        3
    );
}

#[test]
fn margin_shorthand_mirrors_padding() {
    let attr = attr_from_decl("margin", "2ch 4ch");
    assert_eq!(
        attr.get_margin(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        MarginSizes::new(2, 4, 2, 4)
    );
    let attr = attr_from_decl("margin-left", "6ch");
    assert_eq!(
        attr.get_margin(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        MarginSizes::new(0, 0, 0, 6)
    );
}

#[test]
fn margin_auto_collapses_to_zero() {
    // Terminals don't honour `margin: auto`; we surface it as 0 so callers
    // don't need a special case.
    let attr = attr_from_decl("margin", "auto");
    assert_eq!(
        attr.get_margin(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport()
        ),
        MarginSizes::default()
    );
}

#[test]
fn border_shorthand_solid_parses_width_and_style() {
    let attr = attr_from_decl("border", "1ch solid red");
    let sides = attr.get_border(
        80,
        font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        viewport(),
    );
    assert_eq!(sides.top.width, 1);
    assert_eq!(sides.top.style, BorderStyle::Solid);
    assert_eq!(
        sides.top.color,
        Some(Color::Rgb {
            r: 0xFF,
            g: 0x00,
            b: 0x00
        })
    );
    assert_eq!(sides.right.width, 1);
    assert_eq!(sides.bottom.width, 1);
    assert_eq!(sides.left.width, 1);
}

#[test]
fn border_shorthand_parses_functional_colors() {
    // `border: 1ch solid rgb(255, 0, 0)` — the parenthesised color must stay
    // one token through the shorthand's whitespace split.
    let attr = attr_from_decl("border", "1ch solid rgb(255, 0, 0)");
    let sides = attr.get_border(
        80,
        font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        viewport(),
    );
    assert_eq!(
        sides.top.color,
        Some(Color::Rgb { r: 255, g: 0, b: 0 }),
        "comma-syntax rgb() must survive the border shorthand"
    );
    assert_eq!(sides.top.style, BorderStyle::Solid);

    // Space-syntax hsl() with internal spaces also works.
    let attr = attr_from_decl("border", "2ch solid hsl(120 100% 50%)");
    let sides = attr.get_border(
        80,
        font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        viewport(),
    );
    assert_eq!(
        sides.top.color,
        Some(Color::Rgb { r: 0, g: 255, b: 0 }),
        "space-syntax hsl() must survive the border shorthand"
    );
    assert_eq!(sides.top.width, 2);
}

#[test]
fn border_shorthand_style_alone_applies_to_all_sides() {
    let attr = attr_from_decl("border", "solid");
    let sides = attr.get_border(
        80,
        font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        viewport(),
    );
    assert_eq!(sides.top.style, BorderStyle::Solid);
    assert_eq!(sides.bottom.style, BorderStyle::Solid);
    assert_eq!(sides.left.style, BorderStyle::Solid);
    assert_eq!(sides.right.style, BorderStyle::Solid);
}

#[test]
fn border_longhand_overrides_universal_shorthand() {
    // `border: 1ch solid red; border-top: 3ch solid blue;` — top wins, others
    // come from the shorthand.
    let mut decls = IndexMap::new();
    decls.insert("border".into(), "1ch solid red".into());
    decls.insert("border-top".into(), "3ch solid blue".into());
    let mut attr = CssAttr::new();
    attr.add(&CssAttr::from(decls));
    let sides = attr.get_border(
        80,
        font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        viewport(),
    );
    assert_eq!(sides.top.width, 3);
    assert_eq!(
        sides.top.color,
        Some(Color::Rgb {
            r: 0x00,
            g: 0x00,
            b: 0xFF
        })
    );
    assert_eq!(sides.right.width, 1);
    assert_eq!(
        sides.right.color,
        Some(Color::Rgb {
            r: 0xFF,
            g: 0x00,
            b: 0x00
        })
    );
}

#[test]
fn border_universal_width_style_color_layers_correctly() {
    // `border-width: 2ch; border-style: solid; border-color: green;`
    // should produce a green 2ch border on every side.
    let mut decls = IndexMap::new();
    decls.insert("border-width".into(), "2ch".into());
    decls.insert("border-style".into(), "solid".into());
    decls.insert("border-color".into(), "green".into());
    let mut attr = CssAttr::new();
    attr.add(&CssAttr::from(decls));
    let sides = attr.get_border(
        80,
        font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        viewport(),
    );
    assert_eq!(sides.top.width, 2);
    assert_eq!(sides.top.style, BorderStyle::Solid);
    assert_eq!(
        sides.top.color,
        Some(Color::Rgb {
            r: 0x00,
            g: 0x80,
            b: 0x00
        })
    );
}

#[test]
fn border_style_none_hides_the_border() {
    let attr = attr_from_decl("border-style", "none");
    let sides = attr.get_border(
        80,
        font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        viewport(),
    );
    assert!(!sides.any_visible());
}

#[test]
fn border_unknown_style_falls_back_to_solid() {
    // `dashed`, `dotted`, `double` etc. all render as solid in the terminal.
    for keyword in ["dashed", "dotted", "double", "groove"] {
        let attr = attr_from_decl("border", &format!("1ch {keyword} red"));
        let sides = attr.get_border(
            80,
            font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
            viewport(),
        );
        assert_eq!(
            sides.top.style,
            BorderStyle::Solid,
            "keyword {keyword} should fall back to solid"
        );
    }
}

#[test]
fn get_box_edges_returns_combined_padding_border_margin() {
    let mut decls = IndexMap::new();
    decls.insert("padding".into(), "2ch".into());
    decls.insert("margin".into(), "4ch".into());
    decls.insert("border".into(), "1ch solid red".into());
    let mut attr = CssAttr::new();
    attr.add(&CssAttr::from(decls));
    let edges = attr.get_box_edges(
        80,
        font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        viewport(),
    );
    assert_eq!(edges.padding.top, 2);
    assert_eq!(edges.margin.left, 4);
    assert_eq!(edges.border.top.width, 1);
    assert_eq!(edges.border.top.style, BorderStyle::Solid);
    assert_eq!(edges.inner_horizontal(), 6); // 2 + 2 + 1 + 1
}

// ============================================================
// font-size — em / rem resolve against the active font-size context.
// ============================================================

#[test]
fn font_size_parses_to_length() {
    init();
    // `font-size: 2em` → Length::Em(2.0); missing property → None.
    let attr = attr_from_decl("font-size", "2em");
    assert_eq!(attr.get_font_size(), Some(Length::Em(2.0)));
    let attr = attr_from_decl("font-size", "1.5rem");
    assert_eq!(attr.get_font_size(), Some(Length::Rem(1.5)));
    let attr = attr_from_decl("font-size", "16px");
    assert_eq!(attr.get_font_size(), Some(Length::Px(16.0)));
    let attr = attr_from_decl("font-size", "120%");
    assert_eq!(attr.get_font_size(), Some(Length::Pct(120.0)));
    assert_eq!(CssAttr::new().get_font_size(), None);
}

#[test]
fn font_size_property_is_supported_and_inheritable() {
    init();
    assert!(
        SUPPORTED_PROPERTIES.contains(&"font-size"),
        "font-size should be a supported CSS property"
    );
    assert!(
        INHERITED_PROPS.contains(&"font-size"),
        "font-size should inherit from the parent so nested spans keep the same em base"
    );
}

#[test]
fn padding_em_scales_with_element_font_size() {
    init();
    // `padding: 1em` on an element whose effective font-size is 2 should
    // resolve to 2 cells, not 1.
    let attr = attr_from_decl("padding", "1em");
    assert_eq!(attr.get_padding(80, font_ctx(2, 1), viewport()).top, 2);
    // Same property on an element with font-size 3 yields 3 cells.
    assert_eq!(attr.get_padding(80, font_ctx(3, 1), viewport()).top, 3);
    // On the default font-size of 1 it stays at 1.
    assert_eq!(attr.get_padding(80, font_ctx(1, 1), viewport()).top, 1);
}

#[test]
fn margin_rem_uses_root_font_size_not_element() {
    init();
    // rem ignores the element's font-size; it always resolves against the
    // root's font-size. So `margin: 2rem` is 6 cells regardless of the
    // element's own size.
    let attr = attr_from_decl("margin", "2rem");
    assert_eq!(attr.get_margin(80, font_ctx(1, 3), viewport()).top, 6);
    assert_eq!(attr.get_margin(80, font_ctx(5, 3), viewport()).top, 6);
    assert_eq!(attr.get_margin(80, font_ctx(1, 1), viewport()).top, 2);
}

// ============================================================
// text-transform
// ============================================================

#[test]
fn text_transform_parses_and_defaults_to_none() {
    init();
    let attr = attr_from_decl("text-transform", "uppercase");
    assert_eq!(attr.get_text_transform(), TextTransform::Uppercase);
    let attr = attr_from_decl("text-transform", "LOWERCASE");
    assert_eq!(attr.get_text_transform(), TextTransform::Lowercase);
    let attr = attr_from_decl("text-transform", "capitalize");
    assert_eq!(attr.get_text_transform(), TextTransform::Capitalize);
    // Unset / garbage -> none.
    assert_eq!(CssAttr::new().get_text_transform(), TextTransform::None);
    let attr = attr_from_decl("text-transform", "full-width");
    assert_eq!(attr.get_text_transform(), TextTransform::None);
}

#[test]
fn text_transform_is_supported_and_inheritable() {
    init();
    assert!(SUPPORTED_PROPERTIES.contains(&"text-transform"));
    assert!(INHERITED_PROPS.contains(&"text-transform"));
}

// ============================================================
// line-height
// ============================================================

#[test]
fn line_height_parses_normal_number_and_length() {
    init();
    let attr = attr_from_decl("line-height", "normal");
    assert_eq!(attr.get_line_height(), LineHeight::Normal);
    let attr = attr_from_decl("line-height", "1.5");
    assert_eq!(attr.get_line_height(), LineHeight::Number(1.5));
    let attr = attr_from_decl("line-height", "2em");
    assert_eq!(attr.get_line_height(), LineHeight::Length(Length::Em(2.0)));
    let attr = attr_from_decl("line-height", "150%");
    assert_eq!(
        attr.get_line_height(),
        LineHeight::Length(Length::Pct(150.0))
    );
    assert_eq!(CssAttr::new().get_line_height(), LineHeight::Normal);
}

#[test]
fn line_height_resolves_to_whole_rows() {
    init();
    // normal -> font size (1 row per cell).
    assert_eq!(
        resolve_line_height_cells(LineHeight::Normal, 1, default_font_ctx(), viewport()),
        1
    );
    // A unitless multiplier scales the font-size, rounding to whole rows.
    assert_eq!(
        resolve_line_height_cells(LineHeight::Number(1.5), 1, default_font_ctx(), viewport()),
        2
    );
    assert_eq!(
        resolve_line_height_cells(LineHeight::Number(2.0), 2, default_font_ctx(), viewport()),
        4
    );
    // em resolves against the element font-size.
    assert_eq!(
        resolve_line_height_cells(
            LineHeight::Length(Length::Em(1.0)),
            3,
            font_ctx(3, 1),
            viewport()
        ),
        3
    );
    // Percentage resolves against the font-size (not the containing block).
    assert_eq!(
        resolve_line_height_cells(
            LineHeight::Length(Length::Pct(150.0)),
            2,
            font_ctx(2, 1),
            viewport()
        ),
        3
    );
}

#[test]
fn line_height_is_supported_and_inheritable() {
    init();
    assert!(SUPPORTED_PROPERTIES.contains(&"line-height"));
    assert!(INHERITED_PROPS.contains(&"line-height"));
}

// ============================================================
// box-sizing
// ============================================================

#[test]
fn box_sizing_defaults_to_border_box_and_parses() {
    init();
    // Unset -> the engine's historical border-box model.
    assert_eq!(CssAttr::new().get_box_sizing(), BoxSizing::BorderBox);
    let attr = attr_from_decl("box-sizing", "content-box");
    assert_eq!(attr.get_box_sizing(), BoxSizing::ContentBox);
    let attr = attr_from_decl("box-sizing", "BORDER-BOX");
    assert_eq!(attr.get_box_sizing(), BoxSizing::BorderBox);
    // Garbage falls back to the default.
    let attr = attr_from_decl("box-sizing", "padding-box");
    assert_eq!(attr.get_box_sizing(), BoxSizing::BorderBox);
}

#[test]
fn box_sizing_is_supported() {
    init();
    assert!(SUPPORTED_PROPERTIES.contains(&"box-sizing"));
}

// ============================================================
// border-radius
// ============================================================

#[test]
fn border_radius_parses_one_to_four_values() {
    init();
    // One value -> every corner.
    let attr = attr_from_decl("border-radius", "2ch");
    let r = attr.get_border_radius(80, default_font_ctx(), viewport());
    assert_eq!(
        r,
        CornerRadii {
            top_left: 2,
            top_right: 2,
            bottom_right: 2,
            bottom_left: 2
        }
    );
    // Two values -> diagonal pairs.
    let attr = attr_from_decl("border-radius", "4ch 1ch");
    let r = attr.get_border_radius(80, default_font_ctx(), viewport());
    assert_eq!(
        r,
        CornerRadii {
            top_left: 4,
            top_right: 1,
            bottom_right: 4,
            bottom_left: 1
        }
    );
    // Four values -> clockwise order.
    let attr = attr_from_decl("border-radius", "1ch 2ch 3ch 4ch");
    let r = attr.get_border_radius(80, default_font_ctx(), viewport());
    assert_eq!(
        r,
        CornerRadii {
            top_left: 1,
            top_right: 2,
            bottom_right: 3,
            bottom_left: 4
        }
    );
    // Unset -> square corners.
    assert_eq!(
        CssAttr::new().get_border_radius(80, default_font_ctx(), viewport()),
        CornerRadii::default()
    );
}

#[test]
fn border_radius_is_supported() {
    init();
    assert!(SUPPORTED_PROPERTIES.contains(&"border-radius"));
}

#[test]
fn border_radius_corner_longhands_override_shorthand() {
    init();
    // `border-radius: 2ch` then `border-top-left-radius: 5ch` — the longhand
    // wins on that corner, the others keep the shorthand value.
    let mut decls = IndexMap::new();
    decls.insert("border-radius".into(), "2ch".into());
    decls.insert("border-top-left-radius".into(), "5ch".into());
    let mut attr = CssAttr::new();
    attr.add(&CssAttr::from(decls));
    assert_eq!(
        attr.get_border_radius(80, default_font_ctx(), viewport()),
        CornerRadii {
            top_left: 5,
            top_right: 2,
            bottom_right: 2,
            bottom_left: 2,
        }
    );
}

#[test]
fn border_radius_corner_longhand_alone() {
    init();
    // Only the longhand declared — the other corners stay square.
    let attr = attr_from_decl("border-top-right-radius", "3ch");
    assert_eq!(
        attr.get_border_radius(80, default_font_ctx(), viewport()),
        CornerRadii {
            top_left: 0,
            top_right: 3,
            bottom_right: 0,
            bottom_left: 0,
        }
    );
}

#[test]
fn border_radius_corner_longhands_are_supported() {
    init();
    for p in [
        "border-top-left-radius",
        "border-top-right-radius",
        "border-bottom-right-radius",
        "border-bottom-left-radius",
    ] {
        assert!(
            SUPPORTED_PROPERTIES.contains(&p),
            "{p} must be in SUPPORTED_PROPERTIES"
        );
    }
}

#[test]
fn border_side_component_longhand_overrides_side_shorthand() {
    init();
    // `border: 1px solid red; border-top: 3px solid blue; border-top-width:
    // 5px;` — the component longhand overrides just the width of the side
    // shorthand; style / color stay from `border-top`.
    let mut decls = IndexMap::new();
    decls.insert("border".into(), "1ch solid red".into());
    decls.insert("border-top".into(), "3ch solid blue".into());
    decls.insert("border-top-width".into(), "5ch".into());
    let mut attr = CssAttr::new();
    attr.add(&CssAttr::from(decls));
    let sides = attr.get_border(
        80,
        font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        viewport(),
    );
    assert_eq!(sides.top.width, 5, "component longhand wins the width");
    assert_eq!(sides.top.style, BorderStyle::Solid);
    assert_eq!(
        sides.top.color,
        Some(Color::Rgb {
            r: 0,
            g: 0,
            b: 255
        }),
        "style/color come from the side shorthand"
    );
    // The other sides are untouched by the top longhand.
    assert_eq!(sides.right.width, 1);
}

#[test]
fn border_side_component_longhand_alone() {
    init();
    // Only `border-left-color` declared — width 0 / style none, color set.
    let attr = attr_from_decl("border-left-color", "green");
    let sides = attr.get_border(
        80,
        font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        viewport(),
    );
    assert_eq!(sides.left.width, 0);
    assert_eq!(sides.left.style, BorderStyle::None);
    assert_eq!(
        sides.left.color,
        Some(Color::Rgb {
            r: 0,
            g: 128,
            b: 0
        })
    );
    assert_eq!(sides.top.color, None, "other sides unaffected");
}

#[test]
fn border_side_component_longhands_are_supported() {
    init();
    for side in ["top", "right", "bottom", "left"] {
        for prop in ["width", "style", "color"] {
            let name = format!("border-{side}-{prop}");
            assert!(
                SUPPORTED_PROPERTIES.contains(&name.as_str()),
                "{name} must be in SUPPORTED_PROPERTIES"
            );
        }
    }
}

#[test]
fn margin_auto_flags_parse_from_shorthand_and_longhand() {
    init();
    // `margin: 0 auto` -> left+right auto.
    let attr = attr_from_decl("margin", "0 auto");
    let auto = attr.get_margin_auto();
    assert!(!auto.top && !auto.bottom, "vertical auto stays false");
    assert!(auto.left && auto.right, "horizontal edges are auto");

    // Four-value shorthand: `margin: 1px auto 2px 3px` -> right auto.
    let attr = attr_from_decl("margin", "1px auto 2px 3px");
    let auto = attr.get_margin_auto();
    assert!(!auto.top && auto.right && !auto.bottom && !auto.left);

    // Single-value shorthand applies to all four edges.
    let attr = attr_from_decl("margin", "auto");
    let auto = attr.get_margin_auto();
    assert!(auto.top && auto.right && auto.bottom && auto.left);

    // Per-side longhand wins over the shorthand position.
    let mut decls = IndexMap::new();
    decls.insert("margin".into(), "auto".into());
    decls.insert("margin-right".into(), "5px".into());
    let mut attr = CssAttr::new();
    attr.add(&CssAttr::from(decls));
    let auto = attr.get_margin_auto();
    assert!(auto.left, "shorthand left stays auto");
    assert!(!auto.right, "longhand clears the right auto");

    // No margin declarations -> all false.
    assert_eq!(CssAttr::new().get_margin_auto(), EdgeAuto::default());
}

// ============================================================
// min-width / max-width / min-height / max-height
// ============================================================

#[test]
fn percentage_height_resolves_against_definite_containing_height_only() {
    init();
    // CSS 10.5: percentage heights resolve against the containing block's
    // *height*; with an indefinite containing height they compute to `auto`
    // — never against the width.
    let attr = attr_from_decl("height", "50%");
    assert_eq!(
        attr.get_height(Some(100), font_ctx(1, 1), viewport()),
        Some(50),
        "definite containing height: 50% of 100"
    );
    assert_eq!(
        attr.get_height(None, font_ctx(1, 1), viewport()),
        None,
        "indefinite containing height: percentage height is auto"
    );

    let mh = attr_from_decl("min-height", "25%");
    assert_eq!(
        mh.get_min_height(Some(200), font_ctx(1, 1), viewport()),
        Some(50)
    );
    assert_eq!(mh.get_min_height(None, font_ctx(1, 1), viewport()), None);
}

#[test]
fn min_max_size_getters_resolve_and_default_to_none() {
    init();
    let mut decls = IndexMap::new();
    decls.insert("min-width".into(), "10ch".into());
    decls.insert("max-width".into(), "40ch".into());
    decls.insert("min-height".into(), "3ch".into());
    decls.insert("max-height".into(), "none".into());
    let mut attr = CssAttr::new();
    attr.add(&CssAttr::from(decls));
    let f = default_font_ctx();
    assert_eq!(attr.get_min_width(80, f, viewport()), Some(10));
    assert_eq!(attr.get_max_width(80, f, viewport()), Some(40));
    assert_eq!(attr.get_min_height(Some(80), f, viewport()), Some(3));
    // `none` (and `auto`) mean "no constraint".
    assert_eq!(attr.get_max_height(Some(80), f, viewport()), None);
    let attr = attr_from_decl("min-width", "auto");
    assert_eq!(attr.get_min_width(80, f, viewport()), None);
    // Unset properties -> None.
    assert_eq!(CssAttr::new().get_min_width(80, f, viewport()), None);
    assert_eq!(CssAttr::new().get_max_width(80, f, viewport()), None);
    assert_eq!(CssAttr::new().get_min_height(Some(80), f, viewport()), None);
    assert_eq!(CssAttr::new().get_max_height(Some(80), f, viewport()), None);
}

#[test]
fn min_max_size_properties_are_supported_not_inherited() {
    init();
    assert!(SUPPORTED_PROPERTIES.contains(&"min-width"));
    assert!(SUPPORTED_PROPERTIES.contains(&"max-width"));
    assert!(SUPPORTED_PROPERTIES.contains(&"min-height"));
    assert!(SUPPORTED_PROPERTIES.contains(&"max-height"));
    // Sizes do not inherit in CSS.
    assert!(!INHERITED_PROPS.contains(&"min-width"));
    assert!(!INHERITED_PROPS.contains(&"max-height"));
}


/// Test-only oracle: the pre-optimization cascade (per-rule full-tree walks,
/// per-call specificity sort, per-node inherited-map clones). `style`
/// attributes are not modelled (the inline declaration parser is
/// crate-private); documents compared against it must not use inline styles
/// (`cascade_inline_style_precedence_unchanged` covers those separately).
fn ref_apply_rule(
    arena: &mut NodeArena,
    node_id: NodeId,
    selector: &CssSelector,
    decls: &CssAttr,
    state: &DynamicState,
) {
    if selector_chain_matches_with_state(arena, node_id, selector, state) {
        arena[node_id].css_attr.add(decls);
    }
    let children: Vec<NodeId> = arena[node_id].children.clone();
    for child in children {
        ref_apply_rule(arena, child, selector, decls, state);
    }
}

fn ref_inherit(arena: &mut NodeArena, node_id: NodeId, parent: &IndexMap<Arc<str>, Arc<str>>) {
    {
        let node = &mut arena[node_id];
        for key in INHERITED_PROPS {
            if let Some(value) = parent.get(*key) {
                node.css_attr
                    .0
                    .entry(Arc::from(*key))
                    .or_insert_with(|| value.clone());
            }
        }
    }
    let node = &arena[node_id];
    let mut merged = parent.clone();
    for key in INHERITED_PROPS {
        if let Some(value) = node.css_attr.0.get(*key) {
            merged.insert(Arc::from(*key), value.clone());
        }
    }
    let children: Vec<NodeId> = node.children.clone();
    for child in children {
        ref_inherit(arena, child, &merged);
    }
}

fn reference_cascade(arena: &mut NodeArena, root: NodeId, css: &CssRule, state: &DynamicState) {
    reset_css_attr(arena, root);
    let mut rules: Vec<&CssRuleEntry> = css.0.iter().collect();
    rules.sort_by_key(|entry| entry.selector.specificity());
    for entry in &rules {
        if !entry.normal.0.is_empty() {
            ref_apply_rule(arena, root, &entry.selector, &entry.normal, state);
        }
    }
    for entry in &rules {
        if !entry.important.0.is_empty() {
            ref_apply_rule(arena, root, &entry.selector, &entry.important, state);
        }
    }
    ref_inherit(arena, root, &IndexMap::new());
}

/// Assert every node's `css_attr` equals what the reference cascade computes
/// for the same document + stylesheet + interaction state.
fn assert_matches_reference(arena: &NodeArena, root: NodeId, css: &str, state: &DynamicState) {
    let mut reference = arena.clone();
    let mut parser = Parser::new(css);
    let rules = parser.parse();
    reference_cascade(&mut reference, root, &rules, state);
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        assert_eq!(
            arena[id].css_attr.0,
            reference[id].css_attr.0,
            "css_attr mismatch at node {:?} (class {:?})",
            arena[id].node_type,
            arena[id].attributes.get("class")
        );
        stack.extend(arena[id].children.iter().copied());
    }
}

/// `Document > div.menu#main > (p#lead, ul > li×3, a.nav, em)` — a tree that
/// exercises every bucket key (tag / class / id / wildcard) and every
/// combinator.
fn build_cascade_tree(arena: &mut NodeArena) -> (NodeId, NodeId, NodeId, NodeId, NodeId) {
    let root = arena.insert(Node::from(NodeType::Document));
    let div = arena.insert(Node::from(NodeType::Div));
    arena[div].attributes = IndexMap::from([
        ("class".to_string(), "menu".to_string()),
        ("id".to_string(), "main".to_string()),
    ]);
    let p = arena.insert(Node::from(NodeType::P));
    arena[p]
        .attributes
        .insert("id".to_string(), "lead".to_string());
    let ul = arena.insert(Node::from(NodeType::Ul));
    let li1 = arena.insert(Node::from(NodeType::Li));
    let li2 = arena.insert(Node::from(NodeType::Li));
    let li3 = arena.insert(Node::from(NodeType::Li));
    let a = arena.insert(Node::from(NodeType::A));
    arena[a]
        .attributes
        .insert("class".to_string(), "nav".to_string());
    let em = arena.insert(Node::from(NodeType::Em));
    // `ul` children are exactly three `li` so `:nth-child` indices are
    // unambiguous (no whitespace text nodes).
    for (parent, children) in [
        (root, vec![div]),
        (div, vec![p, ul, a, em]),
        (ul, vec![li1, li2, li3]),
    ] {
        for child in children {
            Node::append_child(&mut *arena, parent, child);
        }
    }
    (root, div, ul, li2, em)
}

/// One stylesheet touching tag / class / id / universal / pseudo-class keys
/// and every combinator, with specificity ties resolved by source order and
/// an `!important` override.
const CASCADE_SHEET: &str = "
div { padding-top: 1px; }
div { color: purple; }
.menu { color: red; }
#lead { font-weight: bold; }
* { line-height: 2px; }
ul li { text-align: center; }
div > a { text-decoration: underline; }
li + li { color: green; }
p ~ em { font-style: italic; }
p:nth-child(odd) { font-style: italic; }
li:hover { background-color: blue; }
li:nth-child(even) { text-transform: uppercase; }
a { color: yellow; }
a { color: orange; }
em { color: black !important; }
";

#[test]
fn bucketed_cascade_matches_reference_cascade() {
    init();
    let mut arena = NodeArena::default();
    let (root, div, _ul, li2, em) = build_cascade_tree(&mut arena);

    // Static pass.
    css_into_node_with_state(&mut arena, root, &parse_sheet(CASCADE_SHEET), &DynamicState::default());
    assert_matches_reference(&arena, root, CASCADE_SHEET, &DynamicState::default());
    // Spot-check the cascade semantics the buckets must preserve.
    assert_eq!(arena[div].css_attr.0.get("color").map(|s| s.as_ref()), Some("red"));
    assert_eq!(arena[li2].css_attr.0.get("color").map(|s| s.as_ref()), Some("green"));
    assert_eq!(arena[em].css_attr.0.get("color").map(|s| s.as_ref()), Some("black"));
    assert_eq!(arena[em].css_attr.0.get("font-style").map(|s| s.as_ref()), Some("italic"));
    assert_eq!(
        arena[div].css_attr.0.get("padding-top").map(|s| s.as_ref()),
        Some("1px")
    );

    // Hovered pass: `:hover` must restyle only the matching chain, and the
    // rest of the tree must stay byte-identical to the reference.
    let hovered = DynamicState {
        hovered: Some(li2),
        focused: None,
    };
    let mut arena = NodeArena::default();
    let (root, _div, _ul, _li2, _em) = build_cascade_tree(&mut arena);
    css_into_node_with_state(&mut arena, root, &parse_sheet(CASCADE_SHEET), &hovered);
    assert_matches_reference(&arena, root, CASCADE_SHEET, &hovered);
}

fn parse_sheet(css: &str) -> CssRule {
    let mut parser = Parser::new(css);
    parser.parse()
}

#[test]
fn cascade_index_is_rebuilt_after_push() {
    init();
    let mut arena = NodeArena::default();
    let (root, div) = build_test(&mut arena);
    let rules = parse_sheet("div { color: red; }");
    let mut cascaded = arena.clone();
    css_into_node_with_state(&mut cascaded, root, &rules, &DynamicState::default());
    assert_eq!(
        cascaded[div].css_attr.0.get("color").map(|s| s.as_ref()),
        Some("red")
    );

    // A rule appended after the first cascade must take part in the next
    // one (same specificity → later source order wins).
    let mut rules = rules;
    let selector: CssSelector = "div".parse().unwrap();
    let mut decls = IndexMap::new();
    decls.insert("color".to_string(), "blue".to_string());
    rules.push(selector, CssAttr::from(decls), CssAttr::new());
    let mut recascaded = arena.clone();
    css_into_node_with_state(&mut recascaded, root, &rules, &DynamicState::default());
    assert_eq!(
        recascaded[div].css_attr.0.get("color").map(|s| s.as_ref()),
        Some("blue")
    );
}

#[test]
fn cascade_inline_style_precedence_unchanged() {
    init();
    let mut arena = NodeArena::default();
    let root = arena.insert(Node::from(NodeType::Document));
    let div = arena.insert(Node::from(NodeType::Div));
    arena[div].attributes = IndexMap::from([
        (
            "style".to_string(),
            "color: green; background-color: white !important".to_string(),
        ),
    ]);
    Node::append_child(&mut arena, root, div);
    let rules = parse_sheet("div { color: red !important; }");
    css_into_node_with_state(&mut arena, root, &rules, &DynamicState::default());
    // Stylesheet `!important` beats inline normal (green loses)…
    assert_eq!(
        arena[div].css_attr.0.get("color").map(|s| s.as_ref()),
        Some("red")
    );
    // …while inline `!important` still lands (no competing declaration).
    assert_eq!(
        arena[div].css_attr.0.get("background-color").map(|s| s.as_ref()),
        Some("white")
    );

    // Inline normal beats stylesheet normal.
    let mut arena = NodeArena::default();
    let root = arena.insert(Node::from(NodeType::Document));
    let div = arena.insert(Node::from(NodeType::Div));
    arena[div]
        .attributes
        .insert("style".to_string(), "color: green".to_string());
    Node::append_child(&mut arena, root, div);
    let rules = parse_sheet("div { color: red; }");
    css_into_node_with_state(&mut arena, root, &rules, &DynamicState::default());
    assert_eq!(
        arena[div].css_attr.0.get("color").map(|s| s.as_ref()),
        Some("green")
    );
}

#[test]
fn cascade_inheritance_values_survive_copy_on_write() {
    init();
    // div(red) > p(red, equal to inherited — shared-map path) >
    // span(blue, diverging — cloned-map path) > em (inherits blue).
    let mut arena = NodeArena::default();
    let root = arena.insert(Node::from(NodeType::Document));
    let div = arena.insert(Node::from(NodeType::Div));
    let p = arena.insert(Node::from(NodeType::P));
    let span = arena.insert(Node::from(NodeType::Span));
    let em = arena.insert(Node::from(NodeType::Em));
    for (parent, child) in [(root, div), (div, p), (p, span), (span, em)] {
        Node::append_child(&mut arena, parent, child);
    }
    let rules = parse_sheet(
        "div { color: red; } p { color: red; } span { color: blue; }",
    );
    css_into_node_with_state(&mut arena, root, &rules, &DynamicState::default());
    assert_eq!(arena[div].css_attr.0.get("color").map(|s| s.as_ref()), Some("red"));
    assert_eq!(arena[p].css_attr.0.get("color").map(|s| s.as_ref()), Some("red"));
    assert_eq!(arena[span].css_attr.0.get("color").map(|s| s.as_ref()), Some("blue"));
    assert_eq!(arena[em].css_attr.0.get("color").map(|s| s.as_ref()), Some("blue"));
}

// ============================================================
// B: CSS property-name and keyword case-insensitivity + logical keywords
// ============================================================

#[test]
fn declaration_property_names_are_case_insensitive() {
    init();
    // `parse_declaration_text` backs the inline `style` attribute and `el.style`.
    // Declaration names normalise to lowercase so the canonical keys the
    // `CssAttr` accessors read are produced (`WIDTH` → `width`, …).
    let m = parse_declaration_text("WIDTH: 10px; BACKGROUND-COLOR: red; Text-Transform: uppercase;");
    assert_eq!(m.get("width").map(|s| s.as_ref()), Some("10px"));
    assert_eq!(m.get("background-color").map(|s| s.as_ref()), Some("red"));
    assert_eq!(m.get("text-transform").map(|s| s.as_ref()), Some("uppercase"));
}

#[test]
fn text_align_keywords_are_case_insensitive_and_support_start_end() {
    init();
    assert_eq!(attr_from_decl("text-align", "CENTER").get_text_align(), TextAlign::Center);
    assert_eq!(attr_from_decl("text-align", "Right").get_text_align(), TextAlign::Right);
    assert_eq!(attr_from_decl("text-align", "end").get_text_align(), TextAlign::Right);
    assert_eq!(attr_from_decl("text-align", "start").get_text_align(), TextAlign::Left);
    // `justify` collapses to left (no inter-word justification).
    assert_eq!(attr_from_decl("text-align", "justify").get_text_align(), TextAlign::Left);
}

#[test]
fn font_weight_is_case_insensitive_and_600_is_bold() {
    init();
    assert_eq!(attr_from_decl("font-weight", "BOLD").get_font_weight(), FontWeight::Bold);
    assert_eq!(attr_from_decl("font-weight", "bolder").get_font_weight(), FontWeight::Bold);
    assert_eq!(attr_from_decl("font-weight", "600").get_font_weight(), FontWeight::Bold);
    assert_eq!(attr_from_decl("font-weight", "500").get_font_weight(), FontWeight::Normal);
}

#[test]
fn font_style_is_case_insensitive() {
    init();
    assert_eq!(attr_from_decl("font-style", "Italic").get_font_style_kind(), FontStyleKind::Italic);
    assert_eq!(
        attr_from_decl("font-style", "OBLIQUE").get_font_style_kind(),
        FontStyleKind::Oblique
    );
    assert_eq!(
        attr_from_decl("font-style", "normal").get_font_style_kind(),
        FontStyleKind::Normal
    );
}

// ============================================================
// C: signed negative lengths — margins and positioning insets
// ============================================================

#[test]
fn negative_margin_resolves_signed_horizontally() {
    init();
    // `margin: -4ch` applies to every side: vertical clamps to 0 (no
    // overlapping rows), horizontal keeps the sign (gutter pull-out).
    let attr = attr_from_decl("margin", "-4ch");
    let m = attr.get_margin(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport());
    assert_eq!(m, MarginSizes::new(0, -4, 0, -4));
    // A per-side longhand overrides the shorthand.
    let attr = attr_from_decl("margin-left", "-6ch");
    let m = attr.get_margin(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport());
    assert_eq!(m, MarginSizes::new(0, 0, 0, -6));
}

#[test]
fn negative_inset_resolves_signed() {
    init();
    // `top: -2ch`, `left: -10ch` — negative absolute / relative offsets.
    let mut attr = attr_from_decl("top", "-2ch");
    attr.add(&attr_from_decl("left", "-10ch"));
    let pos = attr.get_positioning(
        80,
        font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        viewport(),
    );
    assert_eq!(pos.inset.top, Some(-2));
    assert_eq!(pos.inset.left, Some(-10));
}

// ============================================================
// C: font-size percentage resolves against the parent font-size
// ============================================================

#[test]
fn font_size_percentage_resolves_against_parent() {
    init();
    let mut arena = NodeArena::default();
    let root = arena.insert(Node::from(NodeType::Document));
    let div_id = arena.insert(Node::from(NodeType::Div));
    Node::append_child(&mut arena, root, div_id);
    let rules = parse_sheet("div { font-size: 200%; }");
    css_into_node(&mut arena, root, &rules);
    // Parent font-size = 2 cells; `200%` → 4 cells (not the containing width).
    let ctx = FontContext {
        font_size_cells: 2,
        root_font_size_cells: 1,
    };
    assert_eq!(effective_font_size_cells(&arena, div_id, ctx, viewport()), 4);
}

// ============================================================
// D: white-space inherits to inline children
// ============================================================

#[test]
fn white_space_inherits_to_inline_children() {
    init();
    let mut arena = NodeArena::default();
    let root = arena.insert(Node::from(NodeType::Document));
    let div_id = arena.insert(Node::from(NodeType::Div));
    let span_id = arena.insert(Node::from(NodeType::Span));
    Node::append_child(&mut arena, root, div_id);
    Node::append_child(&mut arena, div_id, span_id);
    let rules = parse_sheet("div { white-space: pre-wrap; }");
    css_into_node(&mut arena, root, &rules);
    // The (unstyled) span picks up the div's `pre-wrap` by inheritance.
    assert_eq!(
        arena[span_id].css_attr.get_white_space(&arena[span_id].node_type),
        WhiteSpace::PreWrap
    );
}
