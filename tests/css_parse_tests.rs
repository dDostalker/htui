use crossterm::style::Color;
use htui::html::css_parse::{
    Combinator, CssAttr, CssRule, CssSelector, FontWeight, INHERITED_PROPS, Length, Parser,
    SUPPORTED_PROPERTIES, css_into_node, parse_color_str, parse_declaration_text,
    selector_chain_matches,
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
    css_into_node(&mut arena, root, &css).unwrap();

    let div_id = arena[root].children[0];
    let p_id = arena[div_id].children[0];
    assert_eq!(
        arena[div_id].css_attr.get_bg_color(),
        Some(Color::Yellow),
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
    css_into_node(&mut arena, root, &css).unwrap();

    let div_id = arena[root].children[0];
    let p_id = arena[div_id].children[0];
    assert_eq!(arena[div_id].css_attr.get_color(), Some(Color::Red));
    assert_eq!(
        arena[p_id].css_attr.get_color(),
        Some(Color::Red),
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
    let _rules = parser.parse().unwrap();

    let mut arena = NodeArena::default();
    let root = arena.insert(Node::new());
    let h1 = arena.insert(Node::from(NodeType::H1));
    Node::append_child(&mut arena, root, h1);

    let mut css = CssRule::new();
    let sel_h1: CssSelector = "h1".parse().unwrap();
    let mut decls = IndexMap::new();
    decls.insert("color".into(), "red".into());
    decls.insert("font-weight".into(), "bold".into());
    css.add(sel_h1, decls);
    css_into_node(&mut arena, root, &css).unwrap();

    assert_eq!(arena[h1].css_attr.get_color(), Some(Color::Red));
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
    assert_eq!(Length::parse("40px"), Some(Length::Cells(40)));
    assert_eq!(Length::parse("  40px  "), Some(Length::Cells(40)));
    // Case-insensitive unit.
    assert_eq!(Length::parse("20Px"), Some(Length::Cells(20)));
    assert_eq!(Length::parse("30PX"), Some(Length::Cells(30)));
    // Trailing whitespace tolerated.
    assert_eq!(Length::parse("20px\n"), Some(Length::Cells(20)));
}

#[test]
fn length_parse_cells_bare_number() {
    init();
    // Bare integers and floats are accepted as cells — convenient shorthand.
    assert_eq!(Length::parse("40"), Some(Length::Cells(40)));
    assert_eq!(Length::parse("0"), Some(Length::Cells(0)));
    assert_eq!(Length::parse("1.5"), Some(Length::Cells(2))); // round 1.5 → 2
    assert_eq!(Length::parse("0.5"), Some(Length::Cells(1))); // round 0.5 → 1
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
fn length_parse_rejects_unsupported_units() {
    init();
    // Viewport units — terminal has no viewport.
    assert_eq!(Length::parse("50vw"), None);
    assert_eq!(Length::parse("50vh"), None);
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
    assert_eq!(Length::Auto.resolve(80), 80);
    assert_eq!(Length::Cells(40).resolve(80), 40);
    assert_eq!(Length::Cells(0).resolve(80), 0); // caller clamps via resolve_block_width
    assert_eq!(Length::Cells(1000).resolve(80), 1000); // ditto
}

#[test]
fn length_resolve_pct_rounds_and_min_1() {
    init();
    assert_eq!(Length::Pct(50.0).resolve(80), 40);
    assert_eq!(Length::Pct(100.0).resolve(80), 80);
    // round(), not truncate: 80 * 33 / 100 = 26.4 → 26.
    assert_eq!(Length::Pct(33.0).resolve(80), 26);
    // Half-percent boundary: 80 * 50.5 / 100 = 40.4 → 40 (round).
    assert_eq!(Length::Pct(50.5).resolve(80), 40);
    // 80 * 50.6 / 100 = 40.48 → 40 (round).
    assert_eq!(Length::Pct(50.6).resolve(80), 40);
    // Minimum 1 column: a non-zero percentage of zero yields 1, not 0.
    assert_eq!(Length::Pct(50.0).resolve(0), 1);
    assert_eq!(Length::Pct(0.0).resolve(80), 1); // min-1 even at 0%
}

#[test]
fn length_resolve_em_rem_rounds_away_from_zero() {
    init();
    assert_eq!(Length::Em(2.0).resolve(80), 2);
    assert_eq!(Length::Em(1.5).resolve(80), 2); // 1.5 rounds away → 2
    assert_eq!(Length::Em(0.5).resolve(80), 1); // 0.5 rounds away → 1
    assert_eq!(Length::Em(2.5).resolve(80), 3); // 2.5 rounds away → 3
    assert_eq!(Length::Rem(1.5).resolve(80), 2);
    assert_eq!(Length::Rem(0.0).resolve(80), 0);
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
    assert_eq!(attr.get_width(80), None);

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "40px".into());
    attr.add(CssAttr(decls));
    assert_eq!(attr.get_width(80), Some(40));

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "50%".into());
    attr.add(CssAttr(decls));
    assert_eq!(attr.get_width(80), Some(40));

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "2em".into());
    attr.add(CssAttr(decls));
    assert_eq!(attr.get_width(80), Some(2));

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "1.5em".into());
    attr.add(CssAttr(decls));
    assert_eq!(attr.get_width(80), Some(2)); // 1.5 rounds away

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "2rem".into());
    attr.add(CssAttr(decls));
    assert_eq!(attr.get_width(80), Some(2));

    // Unsupported units collapse to None (fill the container).
    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "50vw".into());
    attr.add(CssAttr(decls));
    assert_eq!(attr.get_width(80), None);

    // Missing property → None.
    assert_eq!(CssAttr::new().get_width(80), None);
}
