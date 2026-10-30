//! Tests for the `innerHTML` DOM property: fragment parsing on write, HTML
//! serialization on read, `<style>` CSS extraction, `<script>` skipping and
//! element-context parsing. Pure in-memory — no terminal required.

use std::sync::Arc;

use crossterm::style::Color;

use htui::events::{ElementRef, EventBindings, EventType, HitRegion};
use htui::html::css_parse;
use htui::layout::Rect;
use htui::resources::{ResourceRegistry, with_registry};
use htui::script::ScriptEngine;
use htui::tdom::node::{NodeArena, NodeId, NodeType};

/// Evaluate `String(expr)` and return its text.
fn eval_str(engine: &mut ScriptEngine, expr: &str) -> String {
    let v = engine
        .eval_source(&format!("String({expr})"))
        .expect("eval must succeed");
    v.as_string()
        .map(|s| s.to_std_string_lossy())
        .unwrap_or_default()
}

/// `NodeId` of the element whose `id` attribute equals `id`.
fn find_node_by_id(arena: &NodeArena, id: &str) -> NodeId {
    arena
        .iter()
        .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id))
        .map(|(nid, _)| nid)
        .expect("element with id exists")
}

/// Minimal hit region for `node_id` (no geometry) to fire events along the DOM
/// ancestor chain.
fn region_of(arena: &NodeArena, node_id: NodeId) -> HitRegion {
    let mut path = Vec::new();
    let mut current = Some(node_id);
    while let Some(id) = current {
        let node = &arena[id];
        path.push(ElementRef {
            id: node.get_id().cloned(),
            node_id: id,
        });
        current = node.parent;
    }
    HitRegion {
        node_id,
        rect: Rect::new(0, 0, 0, 0),
        client: Rect::new(0, 0, 0, 0),
        path: path.into(),
    }
}

// ============================================================
// `innerHTML` getter: HTML fragment serialization
// ============================================================

#[test]
fn inner_html_getter_serializes_children() {
    // Nested elements, attributes, void elements, comments and escaped text
    // must serialize back to faithful HTML.
    let mut engine = ScriptEngine::new(
        r#"<div id="d"><p class="a">Hi &amp; bye</p><br><img src="x.png"><!-- c --><span>a&lt;b&gt;</span></div>"#, "",

        EventBindings::default(),
    )
    .expect("engine initializes");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').innerHTML"),
        r#"<p class="a">Hi &amp; bye</p><br><img src="x.png"><!-- c --><span>a&lt;b&gt;</span>"#
    );
}

#[test]
fn inner_html_getter_round_trips_escaped_source() {
    // Text escapes `&` `<` `>`; attribute values escape `&` and quotes.
    let mut engine = ScriptEngine::new(
        r#"<div id="d"><p title="a&quot;b">x &amp; y &lt; z</p></div>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').innerHTML"),
        r#"<p title="a&quot;b">x &amp; y &lt; z</p>"#
    );
}

#[test]
fn inner_html_getter_preserves_attribute_source_order() {
    // Multi-attribute elements serialize in source order (browser behaviour),
    // not sorted.
    let mut engine = ScriptEngine::new(
        r#"<div id="d"><a id="x" class="btn" data-k="v" href="/z">t</a></div>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').innerHTML"),
        r#"<a id="x" class="btn" data-k="v" href="/z">t</a>"#
    );
}

#[test]
fn inner_html_getter_tracks_set_and_remove_attribute_order() {
    // `setAttribute` appends new attributes; `removeAttribute` keeps the
    // remaining ones in their relative source order (browser behaviour).
    let mut engine = ScriptEngine::new(
        r#"<div id="d"><p class="x"></p></div>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    let html = eval_str(
        &mut engine,
        "(function(){ var el = document.querySelector('#d p'); el.setAttribute('a', '1'); el.setAttribute('b', '2'); el.removeAttribute('a'); return document.getElementById('d').innerHTML; })()",
    );
    assert_eq!(html, r#"<p class="x" b="2"></p>"#);
}

#[test]
fn inner_html_getter_empty_when_no_children() {
    let mut engine = ScriptEngine::new(r#"<div id="d"></div>"#, "", EventBindings::default())
        .expect("engine initializes");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').innerHTML"),
        "",
    );
}

// ============================================================
// `innerHTML` setter: fragment parsing
// ============================================================

#[test]
fn inner_html_setter_replaces_children() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d">old<p>text</p></div>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    eval_str(
        &mut engine,
        "document.getElementById('d').innerHTML = '<span>X</span><p>Y</p>'",
    );
    let arena = engine.arena();
    let d = find_node_by_id(&arena, "d");
    let children: Vec<NodeType> = arena[d]
        .children
        .iter()
        .map(|&c| arena[c].node_type)
        .collect();
    assert_eq!(
        children,
        vec![NodeType::Span, NodeType::P],
        "old children replaced"
    );
    for &c in &arena[d].children {
        assert_eq!(
            arena[c].parent,
            Some(d),
            "parsed children must link to the target"
        );
    }
    // Reading back round-trips the fragment.
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').innerHTML"),
        "<span>X</span><p>Y</p>"
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').textContent"),
        "XY"
    );
}

#[test]
fn inner_html_setter_empty_clears_children() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d"><p>a</p><p>b</p></div>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    eval_str(&mut engine, "document.getElementById('d').innerHTML = ''");
    let arena = engine.arena();
    let d = find_node_by_id(&arena, "d");
    assert!(
        arena[d].children.is_empty(),
        "empty fragment must clear children"
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').textContent"),
        "",
    );
}

#[test]
fn inner_html_setter_parses_in_element_context() {
    // The fragment parser uses the target element as its parsing context, so
    // table fragments that would be dropped in a `<div>` context survive.
    let mut engine = ScriptEngine::new(
        r#"<table><tbody id="b"></tbody></table>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    eval_str(
        &mut engine,
        "document.getElementById('b').innerHTML = '<tr><td>X</td></tr>'",
    );
    let arena = engine.arena();
    let b = find_node_by_id(&arena, "b");
    assert_eq!(arena[b].children.len(), 1, "tr must be kept under tbody");
    let tr = arena[b].children[0];
    assert!(matches!(arena[tr].node_type, NodeType::Tr));
    assert_eq!(arena[tr].children.len(), 1);
    assert!(matches!(
        arena[arena[tr].children[0]].node_type,
        NodeType::Td
    ));
}

#[test]
fn inner_html_content_is_query_selectorable() {
    let mut engine = ScriptEngine::new(r#"<div id="d"></div>"#, "", EventBindings::default())
        .expect("engine initializes");
    eval_str(
        &mut engine,
        "document.getElementById('d').innerHTML = '<p class=\"x\">hi</p><ul><li>1</li></ul>'",
    );
    assert_eq!(
        eval_str(&mut engine, "document.querySelector('.x').textContent"),
        "hi"
    );
    assert_eq!(
        eval_str(&mut engine, "document.querySelectorAll('#d li').length"),
        "1"
    );
}

// ============================================================
// `<style>` / `<script>` semantics inside the fragment
// ============================================================

#[test]
fn inner_html_setter_extracts_style_css() {
    let mut engine = ScriptEngine::new(r#"<div id="d"></div>"#, "", EventBindings::default())
        .expect("engine initializes");
    eval_str(
        &mut engine,
        "document.getElementById('d').innerHTML = '<style>p { color: red; }</style><p id=\"x\">hi</p>'",
    );
    let dirty = engine.take_dirty();
    assert!(
        dirty
            .injected_css
            .values()
            .any(|c| c.contains("color: red")),
        "extracted <style> CSS must be carried to the render thread: {:?}",
        dirty.injected_css
    );
    assert!(
        !dirty.nodes.is_empty(),
        "the mutated element must still be marked dirty"
    );
    // The `<style>` element itself never becomes a DOM node.
    let arena = engine.arena();
    let d = find_node_by_id(&arena, "d");
    let children: Vec<NodeType> = arena[d]
        .children
        .iter()
        .map(|&c| arena[c].node_type)
        .collect();
    assert_eq!(children, vec![NodeType::P]);
}

#[test]
fn inner_html_style_css_applies_to_fragment() {
    // The extracted CSS is re-parsed and cascaded exactly like the render
    // thread does, so the injected rule styles the fragment's elements.
    let mut engine = ScriptEngine::new(r#"<div id="d"></div>"#, "", EventBindings::default())
        .expect("engine initializes");
    eval_str(
        &mut engine,
        "document.getElementById('d').innerHTML = '<style>p { color: red; }</style><p id=\"x\">hi</p>'",
    );
    let dirty = engine.take_dirty();
    let mut arena = engine.arena();
    let root_id = engine.root_id();
    let injected = dirty
        .injected_css
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let rules = css_parse::Parser::new(&injected).parse();
    css_parse::css_into_node(&mut arena, root_id, &rules);
    let x = find_node_by_id(&arena, "x");
    assert_eq!(
        arena[x].css_attr.get_color(),
        Some(Color::Rgb {
            r: 0xFF,
            g: 0x00,
            b: 0x00
        }),
        "innerHTML-injected CSS must cascade onto the fragment"
    );
}

#[test]
fn inner_html_setter_loads_link_stylesheet() {
    let reg = Arc::new(ResourceRegistry::new());
    reg.register("inner-html-test.css", b"li { color: blue; }".to_vec())
        .expect("register css");
    with_registry(Arc::clone(&reg), || {
        let mut engine = ScriptEngine::new(r#"<div id="d"></div>"#, "", EventBindings::default())
            .expect("engine initializes");
        eval_str(
            &mut engine,
            "document.getElementById('d').innerHTML = '<link rel=\"stylesheet\" href=\"inner-html-test.css\"><ul><li id=\"l\">x</li></ul>'",
        );
        let dirty = engine.take_dirty();
        assert!(
            dirty
                .injected_css
                .values()
                .any(|c| c.contains("color: blue")),
            "linked stylesheet CSS must be extracted: {:?}",
            dirty.injected_css
        );
        let mut arena = engine.arena();
        let root_id = engine.root_id();
        let injected = dirty
            .injected_css
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let rules = css_parse::Parser::new(&injected).parse();
        css_parse::css_into_node(&mut arena, root_id, &rules);
        let l = find_node_by_id(&arena, "l");
        assert_eq!(
            arena[l].css_attr.get_color(),
            Some(Color::Rgb {
                r: 0x00,
                g: 0x00,
                b: 0xFF
            }),
            "linked stylesheet must style the fragment"
        );
    });
}

#[test]
fn inner_html_setter_skips_scripts() {
    let mut engine = ScriptEngine::new(r#"<div id="d"></div>"#, "", EventBindings::default())
        .expect("engine initializes");
    eval_str(
        &mut engine,
        "document.getElementById('d').innerHTML = '<script>globalThis.__ran = true;</script><b>ok</b>'",
    );
    // The script source was never evaluated (browser `innerHTML` semantics).
    assert_eq!(
        eval_str(&mut engine, "typeof globalThis.__ran"),
        "undefined"
    );
    // ... and never became a DOM node.
    let arena = engine.arena();
    let d = find_node_by_id(&arena, "d");
    let children: Vec<NodeType> = arena[d]
        .children
        .iter()
        .map(|&c| arena[c].node_type)
        .collect();
    assert_eq!(children, vec![NodeType::B]);
}

// ============================================================
// Element-only semantics
// ============================================================

#[test]
fn inner_html_on_non_element_is_noop() {
    let mut engine = ScriptEngine::new(r#"<div id="d"></div>"#, "", EventBindings::default())
        .expect("engine initializes");
    // `innerHTML` is an Element-only property: the document has none.
    assert_eq!(
        eval_str(&mut engine, "String(document.innerHTML)"),
        "undefined"
    );
    // Setting it must neither crash nor mutate the document.
    eval_str(&mut engine, "document.innerHTML = '<p>nope</p>'");
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('d').childNodes.length"
        ),
        "0"
    );
}

#[test]
fn inner_html_click_handler_mutation_carries_style_css() {
    // A JS event handler that rewrites `innerHTML` (the common interactive
    // use) must carry the extracted CSS through the ScriptEvent dirty batch.
    let mut engine = ScriptEngine::new(
        r#"<div id="d" onclick="this.innerHTML = '<style>i { color: green; }</style><i id=&quot;q&quot;>hi</i>'"></div>"#, "",

        EventBindings::default(),
    )
    .expect("engine initializes");
    let arena = engine.arena();
    let d = find_node_by_id(&arena, "d");
    let region = region_of(&arena, d);
    engine.fire(&region, EventType::Click, 0, 0);
    let dirty = engine.take_dirty();
    assert!(
        dirty
            .injected_css
            .values()
            .any(|c| c.contains("color: green")),
        "handler-injected CSS must be reported: {:?}",
        dirty.injected_css
    );
    let arena = engine.arena();
    let q = find_node_by_id(&arena, "q");
    assert!(
        matches!(arena[q].node_type, NodeType::I),
        "fragment must be in the live DOM"
    );
}

#[test]
fn inner_html_rewrites_replace_host_styles_instead_of_accumulating() {
    // Timer-refresh pattern: the same host rewrites its innerHTML every tick.
    // The injected-CSS set is keyed by the host, so a rewrite *replaces* the
    // entry (the browser drops the host's old `<style>` elements) instead of
    // appending an unbounded history that the render side re-parses O(n²).
    let mut engine = ScriptEngine::new(r#"<div id="d"></div>"#, "", EventBindings::default())
        .expect("engine initializes");

    eval_str(
        &mut engine,
        "document.getElementById('d').innerHTML = '<style>p { color: red; }</style><p id=\"x\">1</p>'",
    );
    let first = engine.take_dirty();
    assert_eq!(
        first.injected_css.len(),
        1,
        "one host write produces exactly one entry"
    );

    eval_str(
        &mut engine,
        "document.getElementById('d').innerHTML = '<style>p { color: blue; }</style><p id=\"x\">2</p>'",
    );
    let second = engine.take_dirty();
    let combined = second
        .injected_css
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        combined.contains("color: blue") && !combined.contains("color: red"),
        "the rewrite must replace the host's entry, got: {combined:?}"
    );
}
