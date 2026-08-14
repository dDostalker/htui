//! Tests for `el.classList` (live DOMTokenList): token reads / writes,
//! browser token semantics (order preservation, empty / whitespace rejection),
//! liveness and dirty tracking. Pure in-memory — no terminal required.

use htui::events::EventBindings;
use htui::script::ScriptEngine;
use htui::tdom::node::{NodeArena, NodeId};

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

/// The element's `className`, read from the live arena after JS ran.
fn class_name(engine: &ScriptEngine, id: &str) -> String {
    let arena = engine.arena();
    let node = find_node_by_id(&arena, id);
    arena
        .get(node)
        .and_then(|n| n.attributes.get("class").cloned())
        .unwrap_or_default()
}

// ============================================================
// Basic reads
// ============================================================

#[test]
fn class_list_reads_the_class_attribute() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d" class="a b c"></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').classList.length"),
        "3"
    );
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('d').classList.item(1)"
        ),
        "b"
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').classList[2]"),
        "c"
    );
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('d').classList.contains('a')"
        ),
        "true"
    );
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('d').classList.contains('x')"
        ),
        "false"
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').classList.value"),
        "a b c",
        "classList.value mirrors className"
    );
    assert_eq!(
        eval_str(
            &mut engine,
            "String(document.getElementById('d').classList)"
        ),
        "a b c",
        "String(classList) is the attribute value"
    );
}

#[test]
fn class_list_empty_when_no_class() {
    let mut engine = ScriptEngine::new(r#"<div id="d"></div>"#, EventBindings::default())
        .expect("engine initializes");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').classList.length"),
        "0"
    );
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('d').classList.contains('x')"
        ),
        "false"
    );
}

// ============================================================
// add / remove
// ============================================================

#[test]
fn class_list_add_appends_and_preserves_order() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d" class="a b"></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    eval_str(
        &mut engine,
        "(function(){ var l = document.getElementById('d').classList; l.add('x', 'y'); l.add('a'); })()",
    );
    // New tokens append at the end; an existing token is not duplicated.
    assert_eq!(class_name(&engine, "d"), "a b x y");
    // The change is visible in the live token list.
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').classList.length"),
        "4"
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').classList[3]"),
        "y"
    );
}

#[test]
fn class_list_remove_preserves_relative_order() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d" class="a b c d"></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    eval_str(
        &mut engine,
        "(function(){ var l = document.getElementById('d').classList; l.remove('b', 'a', 'zz'); })()",
    );
    assert_eq!(
        class_name(&engine, "d"),
        "c d",
        "remaining tokens keep their order"
    );
    // Removing everything empties the class attribute.
    eval_str(
        &mut engine,
        "(function(){ var l = document.getElementById('d').classList; l.remove('c', 'd'); })()",
    );
    assert_eq!(class_name(&engine, "d"), "");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').classList.length"),
        "0"
    );
}

#[test]
fn class_list_add_unchanged_token_does_not_mark_dirty() {
    let mut engine = ScriptEngine::new(r#"<div id="d" class="a"></div>"#, EventBindings::default())
        .expect("engine initializes");
    // Adding an already-present token changes nothing: no re-render.
    eval_str(
        &mut engine,
        "(function(){ document.getElementById('d').classList.add('a'); })()",
    );
    let dirty = engine.take_dirty();
    assert!(
        dirty.is_empty(),
        "no-op add must not schedule a re-render: {dirty:?}"
    );
}

// ============================================================
// toggle / replace
// ============================================================

#[test]
fn class_list_toggle_returns_the_new_state() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d" class="a b"></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('d').classList.toggle('a')"
        ),
        "false"
    );
    assert_eq!(class_name(&engine, "d"), "b");
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('d').classList.toggle('a')"
        ),
        "true"
    );
    assert_eq!(class_name(&engine, "d"), "b a", "toggled-on token appends");
    // Forced states never flip the opposite way and report the forced value.
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('d').classList.toggle('b', true)"
        ),
        "true"
    );
    assert_eq!(class_name(&engine, "d"), "b a");
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('d').classList.toggle('a', false)"
        ),
        "false"
    );
    assert_eq!(class_name(&engine, "d"), "b");
}

#[test]
fn class_list_replace_swaps_a_present_token() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d" class="a b"></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('d').classList.replace('a', 'z')"
        ),
        "true"
    );
    assert_eq!(class_name(&engine, "d"), "b z");
    // Replacing an absent token is a no-op returning false.
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('d').classList.replace('nope', 'q')"
        ),
        "false"
    );
    assert_eq!(class_name(&engine, "d"), "b z");
}

// ============================================================
// value / iteration
// ============================================================

#[test]
fn class_list_value_setter_replaces_all_tokens() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d" class="a b"></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    eval_str(
        &mut engine,
        "(function(){ document.getElementById('d').classList.value = 'x y'; })()",
    );
    assert_eq!(class_name(&engine, "d"), "x y");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').classList.length"),
        "2"
    );
}

#[test]
fn class_list_is_iterable() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d" class="a b c"></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    assert_eq!(
        eval_str(
            &mut engine,
            "[...document.getElementById('d').classList].join(',')"
        ),
        "a,b,c"
    );
    assert_eq!(
        eval_str(
            &mut engine,
            "Array.from(document.getElementById('d').classList).join('|')"
        ),
        "a|b|c"
    );
    assert_eq!(
        eval_str(
            &mut engine,
            "Array.from(document.getElementById('d').classList.keys()).join(',')"
        ),
        "0,1,2"
    );
    assert_eq!(
        eval_str(
            &mut engine,
            "Array.from(document.getElementById('d').classList.values()).join(',')"
        ),
        "a,b,c"
    );
    assert_eq!(
        eval_str(
            &mut engine,
            "Array.from(document.getElementById('d').classList.entries()).map(function (e) { return e[0] + ':' + e[1]; }).join(',')"
        ),
        "0:a,1:b,2:c"
    );
    // forEach passes (token, index, list).
    assert_eq!(
        eval_str(
            &mut engine,
            "(function () { var out = []; document.getElementById('d').classList.forEach(function (t, i) { out.push(i + '=' + t); }); return out.join(','); })()"
        ),
        "0=a,1=b,2=c"
    );
}

// ============================================================
// liveness
// ============================================================

#[test]
fn class_list_is_live_across_attribute_changes() {
    let mut engine = ScriptEngine::new(r#"<div id="d" class="a"></div>"#, EventBindings::default())
        .expect("engine initializes");
    let held = eval_str(
        &mut engine,
        "(function () {
          var el = document.getElementById('d');
          var list = el.classList;               // held reference
          el.className = 'x y';                   // className rewrite
          var afterClassName = list.length;       // must see the new tokens
          el.setAttribute('class', 'p q');        // setAttribute rewrite
          var afterAttr = list.contains('p');
          return afterClassName + ':' + afterAttr;
        })()",
    );
    assert_eq!(
        held, "2:true",
        "held classList must reflect later attribute writes"
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').classList.value"),
        "p q"
    );
}

#[test]
fn class_list_mutations_are_visible_to_inner_html() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d"><p id="p" class="a"></p></div>"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    eval_str(
        &mut engine,
        "(function(){ document.getElementById('p').classList.add('hot', 'b'); })()",
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('d').innerHTML"),
        r#"<p id="p" class="a hot b"></p>"#,
        "classList changes appear in serialization in token order"
    );
}

// ============================================================
// token validation
// ============================================================

#[test]
fn class_list_rejects_empty_and_whitespace_tokens() {
    let mut engine = ScriptEngine::new(r#"<div id="d" class="a"></div>"#, EventBindings::default())
        .expect("engine initializes");
    // Empty tokens throw SyntaxError across the API surface.
    for expr in [
        "document.getElementById('d').classList.add('')",
        "document.getElementById('d').classList.remove('')",
        "document.getElementById('d').classList.toggle('')",
        "document.getElementById('d').classList.contains('')",
        "document.getElementById('d').classList.replace('', 'x')",
        "document.getElementById('d').classList.replace('x', '')",
    ] {
        let name = eval_str(
            &mut engine,
            &format!(
                "(function () {{ try {{ {expr}; return 'no-throw'; }} catch (e) {{ return e.name; }} }})()"
            ),
        );
        assert_eq!(name, "SyntaxError", "{expr} must throw");
    }
    // Whitespace inside a token is also rejected.
    assert_eq!(
        eval_str(
            &mut engine,
            "(function () { try { document.getElementById('d').classList.add('a b'); return 'no-throw'; } catch (e) { return e.name; } })()"
        ),
        "SyntaxError"
    );
    // A rejected batch leaves the class untouched.
    assert_eq!(class_name(&engine, "d"), "a");
}

// ============================================================
// dirty tracking
// ============================================================

#[test]
fn class_list_changes_mark_the_element_dirty() {
    let mut engine = ScriptEngine::new(r#"<div id="d" class="a"></div>"#, EventBindings::default())
        .expect("engine initializes");
    eval_str(
        &mut engine,
        "(function(){ document.getElementById('d').classList.add('b'); })()",
    );
    let dirty = engine.take_dirty();
    let arena = engine.arena();
    let d = find_node_by_id(&arena, "d");
    assert!(
        dirty.nodes.contains(&d),
        "a real classList change must be reported for re-render: {dirty:?}"
    );
}
