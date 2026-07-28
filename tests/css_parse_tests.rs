use std::collections::HashMap;

use crossterm::style::Color;
use htui::html::css_parse::{
    BorderStyle, Combinator, CssAttr, CssRule, CssSelector, EdgeSizes, FontWeight, INHERITED_PROPS,
    Length, Parser, SUPPORTED_PROPERTIES, TextDecoration, css_into_node, parse_color_str,
    parse_declaration_text, selector_chain_matches,
};
use htui::tdom::node::{Node, NodeArena, NodeId, NodeType};
use indexmap::IndexMap;

/// Default font-size (in cells) used by [`Length::resolve`] tests so the
/// em / rem branches can be exercised without a DOM. Mirrors the
/// `FontContext::ROOT` value passed to layout.
const DEFAULT_FONT_SIZE: usize = 1;
const DEFAULT_ROOT_FONT_SIZE: usize = 1;

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
    css_into_node(&mut arena, root, &css);

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
    assert_eq!(
        Length::Auto.resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        80
    );
    assert_eq!(
        Length::Cells(40).resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        40
    );
    assert_eq!(
        Length::Cells(0).resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        0
    ); // caller clamps via resolve_block_width
    assert_eq!(
        Length::Cells(1000).resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        1000
    ); // ditto
}

#[test]
fn length_resolve_pct_rounds_and_min_1() {
    init();
    assert_eq!(
        Length::Pct(50.0).resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        40
    );
    assert_eq!(
        Length::Pct(100.0).resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        80
    );
    // round(), not truncate: 80 * 33 / 100 = 26.4 → 26.
    assert_eq!(
        Length::Pct(33.0).resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        26
    );
    // Half-percent boundary: 80 * 50.5 / 100 = 40.4 → 40 (round).
    assert_eq!(
        Length::Pct(50.5).resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        40
    );
    // 80 * 50.6 / 100 = 40.48 → 40 (round).
    assert_eq!(
        Length::Pct(50.6).resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        40
    );
    // Minimum 1 column: a non-zero percentage of zero yields 1, not 0.
    assert_eq!(
        Length::Pct(50.0).resolve(0, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        1
    );
    assert_eq!(
        Length::Pct(0.0).resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        1
    ); // min-1 even at 0%
}

#[test]
fn length_resolve_em_rem_rounds_away_from_zero() {
    init();
    // With the default font-size / root-font-size of 1, em and rem collapse
    // to `n.round()`, so the existing round-away-from-zero behaviour holds.
    assert_eq!(
        Length::Em(2.0).resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        2
    );
    assert_eq!(
        Length::Em(1.5).resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        2
    ); // 1.5 rounds away → 2
    assert_eq!(
        Length::Em(0.5).resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        1
    ); // 0.5 rounds away → 1
    assert_eq!(
        Length::Em(2.5).resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        3
    ); // 2.5 rounds away → 3
    assert_eq!(
        Length::Rem(1.5).resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        2
    );
    assert_eq!(
        Length::Rem(0.0).resolve(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        0
    );
}

#[test]
fn length_resolve_em_scales_with_font_size() {
    init();
    // em resolves against the element's font-size: 2em at font-size=2
    // yields 4 cells, 1.5em at font-size=3 yields 5 cells (round away).
    assert_eq!(Length::Em(2.0).resolve(80, 2, 1), 4);
    assert_eq!(Length::Em(1.5).resolve(80, 3, 1), 5);
    assert_eq!(Length::Em(0.5).resolve(80, 2, 1), 1);
}

#[test]
fn length_resolve_rem_scales_with_root_font_size() {
    init();
    // rem ignores the element's font-size and uses the root font-size: 2rem
    // at root font-size=3 yields 6 cells, regardless of the element's size.
    assert_eq!(Length::Rem(2.0).resolve(80, 1, 3), 6);
    assert_eq!(Length::Rem(1.5).resolve(80, 2, 4), 6);
    assert_eq!(Length::Rem(0.0).resolve(80, 5, 5), 0);
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
        attr.get_width(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        None
    );

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "40px".into());
    attr.add(CssAttr(decls));
    assert_eq!(
        attr.get_width(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        Some(40)
    );

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "50%".into());
    attr.add(CssAttr(decls));
    assert_eq!(
        attr.get_width(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        Some(40)
    );

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "2em".into());
    attr.add(CssAttr(decls));
    assert_eq!(
        attr.get_width(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        Some(2)
    );

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "1.5em".into());
    attr.add(CssAttr(decls));
    assert_eq!(
        attr.get_width(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        Some(2)
    ); // 1.5 rounds away

    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "2rem".into());
    attr.add(CssAttr(decls));
    assert_eq!(
        attr.get_width(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        Some(2)
    );

    // Unsupported units collapse to None (fill the container).
    let mut attr = CssAttr::new();
    let mut decls = IndexMap::new();
    decls.insert("width".into(), "50vw".into());
    attr.add(CssAttr(decls));
    assert_eq!(
        attr.get_width(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        None
    );

    // Missing property → None.
    assert_eq!(
        CssAttr::new().get_width(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
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
    let edges = attr.get_padding(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE);
    assert_eq!(edges, EdgeSizes::new(5, 5, 5, 5));
}

#[test]
fn padding_shorthand_two_values_vertical_horizontal() {
    // `padding: 10px 20px;` → top/bottom = 10, left/right = 20
    let attr = attr_from_decl("padding", "10px 20px");
    assert_eq!(
        attr.get_padding(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        EdgeSizes::new(10, 20, 10, 20)
    );
}

#[test]
fn padding_shorthand_three_values_top_horizontal_bottom() {
    // `padding: 1px 2px 3px;` → top=1, left/right=2, bottom=3
    let attr = attr_from_decl("padding", "1px 2px 3px");
    assert_eq!(
        attr.get_padding(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        EdgeSizes::new(1, 2, 3, 2)
    );
}

#[test]
fn padding_shorthand_four_values_clockwise() {
    // `padding: 1px 2px 3px 4px;` → top=1, right=2, bottom=3, left=4
    let attr = attr_from_decl("padding", "1px 2px 3px 4px");
    assert_eq!(
        attr.get_padding(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
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
        attr.get_padding(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        EdgeSizes::new(7, 0, 0, 0)
    );
}

#[test]
fn padding_supports_percent_and_em() {
    let attr = attr_from_decl("padding", "50%");
    assert_eq!(
        attr.get_padding(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE)
            .left,
        40
    );
    let attr = attr_from_decl("padding-top", "3em");
    assert_eq!(
        attr.get_padding(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE)
            .top,
        3
    );
}

#[test]
fn margin_shorthand_mirrors_padding() {
    let attr = attr_from_decl("margin", "2px 4px");
    assert_eq!(
        attr.get_margin(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        EdgeSizes::new(2, 4, 2, 4)
    );
    let attr = attr_from_decl("margin-left", "6px");
    assert_eq!(
        attr.get_margin(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        EdgeSizes::new(0, 0, 0, 6)
    );
}

#[test]
fn margin_auto_collapses_to_zero() {
    // Terminals don't honour `margin: auto`; we surface it as 0 so callers
    // don't need a special case.
    let attr = attr_from_decl("margin", "auto");
    assert_eq!(
        attr.get_margin(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE),
        EdgeSizes::default()
    );
}

#[test]
fn border_shorthand_solid_parses_width_and_style() {
    let attr = attr_from_decl("border", "1px solid red");
    let sides = attr.get_border(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE);
    assert_eq!(sides.top.width, 1);
    assert_eq!(sides.top.style, BorderStyle::Solid);
    assert_eq!(sides.top.color, Some(Color::Red));
    assert_eq!(sides.right.width, 1);
    assert_eq!(sides.bottom.width, 1);
    assert_eq!(sides.left.width, 1);
}

#[test]
fn border_shorthand_style_alone_applies_to_all_sides() {
    let attr = attr_from_decl("border", "solid");
    let sides = attr.get_border(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE);
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
    let sides = attr.get_border(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE);
    assert_eq!(sides.top.width, 3);
    assert_eq!(sides.top.color, Some(Color::Blue));
    assert_eq!(sides.right.width, 1);
    assert_eq!(sides.right.color, Some(Color::Red));
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
    let sides = attr.get_border(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE);
    assert_eq!(sides.top.width, 2);
    assert_eq!(sides.top.style, BorderStyle::Solid);
    assert_eq!(sides.top.color, Some(Color::Green));
}

#[test]
fn border_style_none_hides_the_border() {
    let attr = attr_from_decl("border-style", "none");
    let sides = attr.get_border(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE);
    assert!(!sides.any_visible());
}

#[test]
fn border_unknown_style_falls_back_to_solid() {
    // `dashed`, `dotted`, `double` etc. all render as solid in the terminal.
    for keyword in ["dashed", "dotted", "double", "groove"] {
        let attr = attr_from_decl("border", &format!("1px {keyword} red"));
        let sides = attr.get_border(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE);
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
    let edges = attr.get_box_edges(80, DEFAULT_FONT_SIZE, DEFAULT_ROOT_FONT_SIZE);
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
    assert_eq!(attr.get_font_size(), Some(Length::Cells(16)));
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
    assert_eq!(attr.get_padding(80, 2, 1).top, 2);
    // Same property on an element with font-size 3 yields 3 cells.
    assert_eq!(attr.get_padding(80, 3, 1).top, 3);
    // On the default font-size of 1 it stays at 1.
    assert_eq!(attr.get_padding(80, 1, 1).top, 1);
}

#[test]
fn margin_rem_uses_root_font_size_not_element() {
    init();
    // rem ignores the element's font-size; it always resolves against the
    // root's font-size. So `margin: 2rem` is 6 cells regardless of the
    // element's own size.
    let attr = attr_from_decl("margin", "2rem");
    assert_eq!(attr.get_margin(80, 1, 3).top, 6);
    assert_eq!(attr.get_margin(80, 5, 3).top, 6);
    assert_eq!(attr.get_margin(80, 1, 1).top, 2);
}
