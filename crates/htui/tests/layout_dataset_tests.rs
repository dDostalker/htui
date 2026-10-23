//! Tests for the `dataset` / `hidden` global attributes on the JS side:
//! `el.dataset` is a live `DOMStringMap` over the element's `data-*`
//! attributes (camelCase↔kebab-case mapping, live reads / writes, WebIDL
//! `DOMString` conversion on writes, `delete`-only removal, `Object.keys`
//! enumeration), and `el.hidden` toggles the `hidden` attribute (the UA
//! `[hidden] { display: none }` rule). Pure in-memory — no terminal required.

use htui::events::EventBindings;
use htui::layout::generate_box_tree;
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

/// The value of a `data-*` attribute, read from the live arena after JS ran.
fn data_attr(engine: &ScriptEngine, id: &str, name: &str) -> Option<String> {
    let arena = engine.arena();
    let node = find_node_by_id(&arena, id);
    arena
        .get(node)
        .and_then(|n| n.attributes.get(name).cloned())
}

/// Whether any rendered text node in the box tree equals `needle`.
fn flatten_text_contains(
    boxes: &[htui::layout::GeneratedBox],
    arena: &NodeArena,
    needle: &str,
) -> bool {
    fn walk(
        boxes: &[htui::layout::GeneratedBox],
        arena: &NodeArena,
        needle: &str,
        found: &mut bool,
    ) {
        for g in boxes {
            if arena[g.node_id].node_type == NodeType::Text
                && arena[g.node_id].data.as_deref() == Some(needle)
            {
                *found = true;
                return;
            }
            walk(&g.children, arena, needle, found);
            if *found {
                return;
            }
        }
    }
    let mut found = false;
    walk(boxes, arena, needle, &mut found);
    found
}

// ============================================================
// dataset reads
// ============================================================

#[test]
fn dataset_reads_data_attributes_with_camel_case_mapping() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d" data-foo="bar" data-max-count="3"></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').dataset.foo"),
        "bar"
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').dataset.maxCount"),
        "3",
        "camelCase property maps to the kebab-case data-* attribute"
    );
    assert_eq!(
        eval_str(
            &mut engine,
            "String(document.getElementById('d').dataset.missing)"
        ),
        "undefined",
        "an absent data-* attribute reads as undefined"
    );
    // A kebab-case key reads the same attribute (browser bracket access).
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('d').dataset['max-count']"
        ),
        "3"
    );
}

#[test]
fn dataset_empty_when_no_data_attributes() {
    let mut engine = ScriptEngine::new(r#"<div id="d"></div>"#, EventBindings::default())
        .expect("engine initializes");
    assert_eq!(
        eval_str(
            &mut engine,
            "Object.keys(document.getElementById('d').dataset).length"
        ),
        "0"
    );
    assert_eq!(
        eval_str(
            &mut engine,
            "String(document.getElementById('d').dataset.nope)"
        ),
        "undefined"
    );
}

// ============================================================
// dataset writes
// ============================================================

#[test]
fn dataset_write_sets_the_kebab_cased_attribute() {
    let mut engine = ScriptEngine::new(r#"<div id="d"></div>"#, EventBindings::default())
        .expect("engine initializes");
    eval_str(
        &mut engine,
        "document.getElementById('d').dataset.maxCount = '42'",
    );
    assert_eq!(data_attr(&engine, "d", "data-max-count"), Some("42".into()));
    // Live read-back through the same property.
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').dataset.maxCount"),
        "42"
    );
    // Existing attributes are updated in place.
    let mut engine = ScriptEngine::new(
        r#"<div id="e" data-src="a"></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    eval_str(
        &mut engine,
        "document.getElementById('e').dataset.src = 'b'",
    );
    assert_eq!(data_attr(&engine, "e", "data-src"), Some("b".into()));
}

#[test]
fn dataset_write_uses_webidl_string_conversion() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d" data-tmp="x" data-n="1" data-u="2"></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    eval_str(
        &mut engine,
        "globalThis.ds = document.getElementById('d').dataset",
    );
    // An empty string sets an empty attribute value (a browser keeps the
    // attribute present); removal is `delete`-only.
    eval_str(&mut engine, "globalThis.ds.tmp = ''");
    assert_eq!(data_attr(&engine, "d", "data-tmp"), Some("".into()));
    assert_eq!(eval_str(&mut engine, "globalThis.ds.tmp"), "");
    assert_eq!(
        eval_str(&mut engine, "'tmp' in globalThis.ds"),
        "true",
        "an empty-string write must keep the attribute present"
    );
    // WebIDL `DOMString` conversion: null / undefined stringify.
    eval_str(&mut engine, "globalThis.ds.n = null");
    assert_eq!(data_attr(&engine, "d", "data-n"), Some("null".into()));
    eval_str(&mut engine, "globalThis.ds.u = undefined");
    assert_eq!(data_attr(&engine, "d", "data-u"), Some("undefined".into()));
    // Removal stays `delete`-only.
    eval_str(&mut engine, "delete globalThis.ds.tmp");
    assert_eq!(data_attr(&engine, "d", "data-tmp"), None);
    assert_eq!(
        eval_str(&mut engine, "'tmp' in globalThis.ds"),
        "false",
        "delete must remove the attribute"
    );
}

#[test]
fn dataset_delete_and_in_operator() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d" data-a="1" data-b="2"></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    assert_eq!(
        eval_str(&mut engine, "'a' in document.getElementById('d').dataset"),
        "true"
    );
    eval_str(&mut engine, "delete document.getElementById('d').dataset.a");
    assert_eq!(
        eval_str(&mut engine, "'a' in document.getElementById('d').dataset"),
        "false"
    );
    assert_eq!(data_attr(&engine, "d", "data-a"), None);
    assert_eq!(
        eval_str(&mut engine, "'b' in document.getElementById('d').dataset"),
        "true",
        "deleting one key must not touch the others"
    );
}

#[test]
fn dataset_enumerates_kebab_cased_keys() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d" data-alpha="1" data-max-count="2" data-z="3"></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    assert_eq!(
        eval_str(
            &mut engine,
            "JSON.stringify(Object.keys(document.getElementById('d').dataset))"
        ),
        r#"["alpha","max-count","z"]"#,
        "Object.keys(dataset) lists the kebab-cased data-* names in order"
    );
}

#[test]
fn dataset_unchanged_write_does_not_mark_dirty() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d" data-foo="bar"></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    eval_str(
        &mut engine,
        "document.getElementById('d').dataset.foo = 'bar'",
    );
    let dirty = engine.take_dirty();
    assert!(
        dirty.is_empty(),
        "writing the same value must not schedule a re-render"
    );
}

#[test]
fn dataset_is_live_across_mutations() {
    let mut engine =
        ScriptEngine::new(r#"<div id="d" data-a="1"></div>"#, EventBindings::default())
            .expect("engine initializes");
    // A held dataset reference reflects later DOM mutations.
    eval_str(
        &mut engine,
        "globalThis.ds = document.getElementById('d').dataset",
    );
    eval_str(&mut engine, "globalThis.ds.b = '2'");
    assert_eq!(data_attr(&engine, "d", "data-b"), Some("2".into()));
    assert_eq!(
        eval_str(&mut engine, "globalThis.ds.b"),
        "2",
        "a held dataset reference stays live"
    );
}

// ============================================================
// el.hidden
// ============================================================

#[test]
fn hidden_reads_the_hidden_attribute() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d" hidden></div><div id="e"></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').hidden"),
        "true"
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('e').hidden"),
        "false"
    );
}

#[test]
fn hidden_js_property_toggles_rendering() {
    let mut engine = ScriptEngine::new(
        r#"<div id="wrap"><p id="a">visible</p><p id="b">gone</p></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    let root_id = engine.root_id();

    // Initial: both <p> boxes exist.
    let boxes = generate_box_tree(&engine.arena(), root_id);
    assert!(flatten_text_contains(&boxes, &engine.arena(), "gone"));

    // `el.hidden = true` sets the attribute and marks the node dirty.
    eval_str(&mut engine, "document.getElementById('b').hidden = true");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('b').hidden"),
        "true"
    );
    let dirty = engine.take_dirty();
    assert!(
        !dirty.is_empty(),
        "hiding an element must schedule a re-render"
    );

    // The re-layout drops the hidden element and its subtree.
    let boxes = generate_box_tree(&engine.arena(), root_id);
    assert!(
        !flatten_text_contains(&boxes, &engine.arena(), "gone"),
        "a hidden element must not render"
    );
    assert!(flatten_text_contains(&boxes, &engine.arena(), "visible"));

    // `el.hidden = false` restores it.
    eval_str(&mut engine, "document.getElementById('b').hidden = false");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('b').hidden"),
        "false"
    );
    let boxes = generate_box_tree(&engine.arena(), root_id);
    assert!(flatten_text_contains(&boxes, &engine.arena(), "gone"));
}

#[test]
fn hidden_unchanged_write_does_not_mark_dirty() {
    let mut engine = ScriptEngine::new(r#"<div id="d" hidden></div>"#, EventBindings::default())
        .expect("engine initializes");
    eval_str(&mut engine, "document.getElementById('d').hidden = true");
    assert!(
        engine.take_dirty().is_empty(),
        "re-setting an already-present hidden attribute must not re-render"
    );
    eval_str(&mut engine, "document.getElementById('d').hidden = false");
    assert!(
        !engine.take_dirty().is_empty(),
        "clearing the hidden attribute must re-render"
    );
}

#[test]
fn dataset_access_returns_the_same_live_instance() {
    let mut engine = ScriptEngine::new(r#"<div id="d"></div>"#, EventBindings::default())
        .expect("engine initializes");
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('d').dataset === document.getElementById('d').dataset"
        ),
        "true",
        "repeated access returns the same instance (browser identity)"
    );
}

#[test]
fn dataset_proto_keys_keep_object_semantics_and_names_are_validated() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d" data-foo="1" data-constructor="x"></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    eval_str(
        &mut engine,
        "globalThis.ds = document.getElementById('d').dataset",
    );
    // WebIDL named-property visibility: Object.prototype's own property
    // names keep their normal semantics and are never shadowed by data-*.
    assert_eq!(
        eval_str(&mut engine, "typeof globalThis.ds.constructor"),
        "function",
        "constructor must come from the prototype, not the data-constructor attr"
    );
    assert_eq!(eval_str(&mut engine, "'toString' in globalThis.ds"), "true");
    assert_eq!(
        eval_str(&mut engine, "globalThis.ds.hasOwnProperty('foo')"),
        "true",
        "prototype methods must work and see the data keys"
    );
    // The setter validates the transformed candidate name.
    assert_eq!(
        eval_str(
            &mut engine,
            "(function () { try { globalThis.ds['a b'] = 1; return 'no-throw'; } catch (e) { return e.name; } })()"
        ),
        "SyntaxError"
    );
    assert_eq!(
        eval_str(
            &mut engine,
            "(function () { try { globalThis.ds.UPPER = 1; return 'no-throw'; } catch (e) { return e.name; } })()"
        ),
        "SyntaxError",
        "UPPER transforms to '-u-p-p-e-r', which cannot start a name"
    );
    // Valid names still write; the deleter silently ignores invalid names.
    eval_str(&mut engine, "globalThis.ds['a-b'] = 2");
    assert_eq!(data_attr(&engine, "d", "data-a-b"), Some("2".into()));
    assert_eq!(
        eval_str(&mut engine, "delete globalThis.ds['a b']"),
        "true",
        "deleting an invalid name is a no-op, not a throw"
    );
}
