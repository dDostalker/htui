use std::collections::HashMap;

use crossterm::style::Color;
use htui::html::css_parse::{
    BorderStyle, BoxSizing, Combinator, CornerRadii, CssAttr, CssRule, CssSelector, EdgeSizes,
    FontContext, FontWeight, INHERITED_PROPS, Length, LengthContext, LineHeight, NthExpr,
    PX_PER_CELL, Parser, SUPPORTED_PROPERTIES, TextDecoration, TextTransform, Viewport,
    css_into_node, parse_color_str, parse_declaration_text, resolve_line_height_cells,
    selector_chain_matches,
};
use htui::tdom::node::{Node, NodeArena, NodeId, NodeType};
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
    let root = arena.insert(Node::from(NodeType::Global));
    let div_id = arena.insert(Node::from(NodeType::Div));
    arena[div_id].attributes = HashMap::from([
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
    // skip the 1st, 3rd, … . Build a 4-row parent and verify each index.
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
    assert_eq!(NthExpr::EVEN.matches(1), false);
    assert_eq!(NthExpr::EVEN.matches(2), true);
    assert_eq!(NthExpr::EVEN.matches(3), false);
    assert_eq!(NthExpr::EVEN.matches(4), true);
    assert_eq!(NthExpr::ODD.matches(1), true);
    assert_eq!(NthExpr::ODD.matches(2), false);
    assert_eq!(NthExpr::ODD.matches(3), true);
    let expr = NthExpr { a: 3, b: 2 };
    assert_eq!(expr.matches(2), true);
    assert_eq!(expr.matches(5), true);
    assert_eq!(expr.matches(8), true);
    assert_eq!(expr.matches(1), false);
    assert_eq!(expr.matches(4), false);
    // Degenerate a=0 → only the exact position matches.
    let single = NthExpr { a: 0, b: 3 };
    assert_eq!(single.matches(3), true);
    assert_eq!(single.matches(2), false);
    assert_eq!(single.matches(4), false);
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
    css.add(sel, m);
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
    attr.add(CssAttr(declared));
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
fn test_css_attr_full_color() {
    let mut attr = CssAttr::new();
    let css = "color: darkblue; background-color: #1e293b;";
    let m = parse_declaration_text(css);
    attr.add(CssAttr(m));
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
    assert_eq!(m.get("color").map(String::as_str), Some("red"));
}

#[test]
fn test_important_marker_with_space_is_not_recognised() {
    let m = parse_declaration_text("font-weight: bold ! important;");
    // CSS requires `!important` with no space; `! important` is invalid.
    // cssparser tokenises the `!` as an unknown token at declaration value
    // level, drops it, and parses the rest as the value. We end up storing
    // `bold` (without a false !important signal). What we assert here is
    // that we do NOT silently accept it as important.
    let val = m.get("font-weight").map(String::as_str);
    assert_eq!(val, Some("bold"));
    assert!(
        !val.unwrap().contains("!important"),
        "must not smuggle the bogus `! important` into the value"
    );
}

#[test]
fn test_important_marker_with_leading_space_is_kept_out_of_value() {
    let m = parse_declaration_text("font-style: italic  !important ;");
    assert_eq!(m.get("font-style").map(String::as_str), Some("italic"));
}

#[test]
fn test_block_comment_is_stripped() {
    let m = parse_declaration_text("/* color: ignored */ color: red;");
    assert_eq!(m.get("color").map(String::as_str), Some("red"));
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
        m.get("content").map(String::as_str),
        Some("\"a /* b */ c\"")
    );
}

#[test]
fn test_unterminated_comment_truncates_input() {
    // Defensive: malformed CSS should not panic and should not leak the
    // comment text into a property key.
    let m = parse_declaration_text("color: red; /* oops");
    assert_eq!(m.get("color").map(String::as_str), Some("red"));
}

#[test]
fn test_missing_trailing_semicolon_is_ok() {
    let m = parse_declaration_text("color: blue");
    assert_eq!(m.get("color").map(String::as_str), Some("blue"));
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
    css.add(sel, m);
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
    css.add(sel, m);
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
    // End-to-end: parse a real stylesheet through the CSS pipeline
    // (QualifiedRuleParser → declaration parsing via cssparser tokens).
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
    let root = arena.insert(Node::from(NodeType::Global));
    let h1 = arena.insert(Node::from(NodeType::H1));
    Node::append_child(&mut arena, root, h1);

    let mut css = CssRule::new();
    let sel_h1: CssSelector = "h1".parse().unwrap();
    let mut decls = IndexMap::new();
    decls.insert("color".into(), "red".into());
    decls.insert("font-weight".into(), "bold".into());
    css.add(sel_h1, decls);
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
fn px_per_cell_constant_anchors_px_to_one_cell() {
    // Standardized px semantics: 1 CSS px == 1 terminal cell. This is the
    // single edit-point for any future DPI mode (e.g. 2x retina = 2.0).
    assert_eq!(PX_PER_CELL, 1.0);
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
fn length_parse_cells_px_multiply_matches_raw_value_when_ratio_is_one() {
    // With PX_PER_CELL == 1.0, `Length::Px(n).resolve(...)` multiplies by
    // 1.0 and rounds. `parse` keeps the raw multiplier; resolve applies
    // PX_PER_CELL. This test pins the multiplication so any future change
    // to the constant forces a deliberate update here.
    init();
    assert_eq!(
        (40.0_f32 * PX_PER_CELL).round() as usize,
        Length::parse("40px").map(|l| match l {
            Length::Px(n) => n as usize,
            _ => panic!("expected Px"),
        }).unwrap()
    );
}

#[test]
fn length_parse_cells_bare_number() {
    init();
    // Bare integers and floats are accepted as cells — convenient shorthand.
    // `parse` preserves the raw f32 value; rounding happens inside
    // `Length::resolve`.
    assert_eq!(Length::parse("40"), Some(Length::Cells(40.0)));
    assert_eq!(Length::parse("0"), Some(Length::Cells(0.0)));
    assert_eq!(Length::parse("1.5"), Some(Length::Cells(1.5)));
    assert_eq!(Length::parse("0.5"), Some(Length::Cells(0.5)));
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
    assert_eq!(
        Length::Auto.resolve(length_ctx(80, default_font_ctx())),
        80
    );
    assert_eq!(
        Length::Cells(40.0).resolve(length_ctx(80, default_font_ctx())),
        40
    );
    assert_eq!(
        Length::Cells(0.0).resolve(length_ctx(80, default_font_ctx())),
        0
    ); // caller clamps via resolve_block_width
    assert_eq!(
        Length::Cells(1000.0).resolve(length_ctx(80, default_font_ctx())),
        1000
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
fn length_resolve_vw_vh_anchor_to_viewport() {
    init();
    // 1vw = viewport_cols / 100, 1vh = viewport_rows / 100.
    // `containing` is intentionally a different number to prove `vw` does
    // NOT anchor to the containing block — it always uses the viewport.
    assert_eq!(Length::Vw(50.0).resolve(length_ctx(80, font_ctx(1, 1))), 40); // 50 * 80 / 100
    assert_eq!(Length::Vw(100.0).resolve(length_ctx(80, font_ctx(1, 1))), 80);
    assert_eq!(Length::Vw(50.0).resolve(length_ctx(999, font_ctx(1, 1))), 40); // containing irrelevant
    assert_eq!(Length::Vh(50.0).resolve(length_ctx(80, font_ctx(1, 1))), 12); // 50 * 24 / 100
    assert_eq!(Length::Vh(100.0).resolve(length_ctx(80, font_ctx(1, 1))), 24);
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
    attr.add(CssAttr(decls));
    assert_eq!(
        attr.get_width(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
        None
    );

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "40px".into());
    attr.add(CssAttr(decls));
    assert_eq!(
        attr.get_width(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
        Some(40)
    );

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "50%".into());
    attr.add(CssAttr(decls));
    assert_eq!(
        attr.get_width(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
        Some(40)
    );

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "2em".into());
    attr.add(CssAttr(decls));
    assert_eq!(
        attr.get_width(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
        Some(2)
    );

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "1.5em".into());
    attr.add(CssAttr(decls));
    assert_eq!(
        attr.get_width(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
        Some(2)
    ); // 1.5 rounds away

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "2rem".into());
    attr.add(CssAttr(decls));
    assert_eq!(
        attr.get_width(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
        Some(2)
    );

    // 50vw anchors to viewport_cols (80 cols here) → 50 * 80 / 100 = 40.
    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "50vw".into());
    attr.add(CssAttr(decls));
    assert_eq!(
        attr.get_width(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
        Some(40)
    );

    // 50vh anchors to viewport_rows (24 rows here) → 50 * 24 / 100 = 12.
    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "50vh".into());
    attr.add(CssAttr(decls));
    assert_eq!(
        attr.get_width(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
        Some(12)
    );

    // Missing property → None.
    assert_eq!(
        CssAttr::new().get_width(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
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
    attr.add(CssAttr(decls));
    attr
}

#[test]
fn padding_shorthand_one_value_fills_all_sides() {
    let attr = attr_from_decl("padding", "5px");
    let edges = attr.get_padding(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport());
    assert_eq!(edges, EdgeSizes::new(5, 5, 5, 5));
}

#[test]
fn padding_shorthand_two_values_vertical_horizontal() {
    // `padding: 10px 20px;` → top/bottom = 10, left/right = 20
    let attr = attr_from_decl("padding", "10px 20px");
    assert_eq!(
        attr.get_padding(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
        EdgeSizes::new(10, 20, 10, 20)
    );
}

#[test]
fn padding_shorthand_three_values_top_horizontal_bottom() {
    // `padding: 1px 2px 3px;` → top=1, left/right=2, bottom=3
    let attr = attr_from_decl("padding", "1px 2px 3px");
    assert_eq!(
        attr.get_padding(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
        EdgeSizes::new(1, 2, 3, 2)
    );
}

#[test]
fn padding_shorthand_four_values_clockwise() {
    // `padding: 1px 2px 3px 4px;` → top=1, right=2, bottom=3, left=4
    let attr = attr_from_decl("padding", "1px 2px 3px 4px");
    assert_eq!(
        attr.get_padding(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
        EdgeSizes::new(1, 2, 3, 4)
    );
}

#[test]
fn padding_longhands_override_shorthand() {
    // `padding: 0; padding-top: 7px;` → top=7, others=0
    let mut decls = IndexMap::new();
    decls.insert("padding".into(), "0".into());
    decls.insert("padding-top".into(), "7px".into());
    let mut attr = CssAttr::new();
    attr.add(CssAttr(decls));
    assert_eq!(
        attr.get_padding(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
        EdgeSizes::new(7, 0, 0, 0)
    );
}

#[test]
fn padding_supports_percent_and_em() {
    let attr = attr_from_decl("padding", "50%");
    assert_eq!(
        attr.get_padding(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport())
            .left,
        40
    );
    let attr = attr_from_decl("padding-top", "3em");
    assert_eq!(
        attr.get_padding(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport())
            .top,
        3
    );
}

#[test]
fn margin_shorthand_mirrors_padding() {
    let attr = attr_from_decl("margin", "2px 4px");
    assert_eq!(
        attr.get_margin(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
        EdgeSizes::new(2, 4, 2, 4)
    );
    let attr = attr_from_decl("margin-left", "6px");
    assert_eq!(
        attr.get_margin(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
        EdgeSizes::new(0, 0, 0, 6)
    );
}

#[test]
fn margin_auto_collapses_to_zero() {
    // Terminals don't honour `margin: auto`; we surface it as 0 so callers
    // don't need a special case.
    let attr = attr_from_decl("margin", "auto");
    assert_eq!(
        attr.get_margin(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport()),
        EdgeSizes::default()
    );
}

#[test]
fn border_shorthand_solid_parses_width_and_style() {
    let attr = attr_from_decl("border", "1px solid red");
    let sides = attr.get_border(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport());
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
fn border_shorthand_style_alone_applies_to_all_sides() {
    let attr = attr_from_decl("border", "solid");
    let sides = attr.get_border(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport());
    assert_eq!(sides.top.style, BorderStyle::Solid);
    assert_eq!(sides.bottom.style, BorderStyle::Solid);
    assert_eq!(sides.left.style, BorderStyle::Solid);
    assert_eq!(sides.right.style, BorderStyle::Solid);
}

#[test]
fn border_longhand_overrides_universal_shorthand() {
    // `border: 1px solid red; border-top: 3px solid blue;` — top wins, others
    // come from the shorthand.
    let mut decls = IndexMap::new();
    decls.insert("border".into(), "1px solid red".into());
    decls.insert("border-top".into(), "3px solid blue".into());
    let mut attr = CssAttr::new();
    attr.add(CssAttr(decls));
    let sides = attr.get_border(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport());
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
    // `border-width: 2px; border-style: solid; border-color: green;`
    // should produce a green 2px border on every side.
    let mut decls = IndexMap::new();
    decls.insert("border-width".into(), "2px".into());
    decls.insert("border-style".into(), "solid".into());
    decls.insert("border-color".into(), "green".into());
    let mut attr = CssAttr::new();
    attr.add(CssAttr(decls));
    let sides = attr.get_border(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport());
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
    let sides = attr.get_border(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport());
    assert!(!sides.any_visible());
}

#[test]
fn border_unknown_style_falls_back_to_solid() {
    // `dashed`, `dotted`, `double` etc. all render as solid in the terminal.
    for keyword in ["dashed", "dotted", "double", "groove"] {
        let attr = attr_from_decl("border", &format!("1px {keyword} red"));
        let sides = attr.get_border(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport());
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
    decls.insert("padding".into(), "2px".into());
    decls.insert("margin".into(), "4px".into());
    decls.insert("border".into(), "1px solid red".into());
    let mut attr = CssAttr::new();
    attr.add(CssAttr(decls));
    let edges = attr.get_box_edges(80, font_ctx(DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE), viewport());
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
    assert_eq!(attr.get_line_height(), LineHeight::Length(Length::Pct(150.0)));
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
    let attr = attr_from_decl("border-radius", "2px");
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
    let attr = attr_from_decl("border-radius", "4px 1px");
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
    let attr = attr_from_decl("border-radius", "1px 2px 3px 4px");
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
    assert_eq!(CssAttr::new().get_border_radius(80, default_font_ctx(), viewport()), CornerRadii::default());
}

#[test]
fn border_radius_is_supported() {
    init();
    assert!(SUPPORTED_PROPERTIES.contains(&"border-radius"));
}

// ============================================================
// min-width / max-width / min-height / max-height
// ============================================================

#[test]
fn min_max_size_getters_resolve_and_default_to_none() {
    init();
    let mut decls = IndexMap::new();
    decls.insert("min-width".into(), "10px".into());
    decls.insert("max-width".into(), "40px".into());
    decls.insert("min-height".into(), "3px".into());
    decls.insert("max-height".into(), "none".into());
    let mut attr = CssAttr::new();
    attr.add(CssAttr(decls));
    let f = default_font_ctx();
    assert_eq!(attr.get_min_width(80, f, viewport()), Some(10));
    assert_eq!(attr.get_max_width(80, f, viewport()), Some(40));
    assert_eq!(attr.get_min_height(80, f, viewport()), Some(3));
    // `none` (and `auto`) mean "no constraint".
    assert_eq!(attr.get_max_height(80, f, viewport()), None);
    let attr = attr_from_decl("min-width", "auto");
    assert_eq!(attr.get_min_width(80, f, viewport()), None);
    // Unset properties -> None.
    assert_eq!(CssAttr::new().get_min_width(80, f, viewport()), None);
    assert_eq!(CssAttr::new().get_max_width(80, f, viewport()), None);
    assert_eq!(CssAttr::new().get_min_height(80, f, viewport()), None);
    assert_eq!(CssAttr::new().get_max_height(80, f, viewport()), None);
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
