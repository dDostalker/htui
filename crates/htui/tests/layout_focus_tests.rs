//! Tests for JS `el.focus()` / `el.blur()`: the request travels from the
//! script thread to the input loop (which owns the edit session), and the
//! `focus` / `blur` (+ `change`) events route back through the script-message
//! path. Pure in-memory — no terminal required.

use htui::events::{EventBindings, HitMap, interaction::HoverState};
use htui::html::{css_parse, html_parse};
use htui::script::{ScriptEngine, apply_message};
use htui::tdom::node::{NodeArena, NodeId};
use htui::tui::graph_api::{InputState, RenderCmd, apply_focus_requests};

/// Evaluate `String(expr)` against the engine and return its text.
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

/// One JS-initiated focus transition round trip: drain the engine's pending
/// commands (`el.focus()` / `el.blur()` queued by the script), apply the
/// focus requests to the input-loop `state` (seeding the edit session and
/// producing `focus` / `blur` script messages), then feed those messages back
/// through the engine so the events fire. Returns the render commands.
fn drive(engine: &mut ScriptEngine, state: &mut InputState, arena: &NodeArena) -> Vec<RenderCmd> {
    let cmds = engine.take_pending_cmds();
    let mut msgs = Vec::new();
    let cmds = apply_focus_requests(state, cmds, Some(arena), &mut msgs);
    let mut hover = HoverState::new();
    let map = HitMap::new();
    let mut out = cmds;
    for msg in msgs {
        let ev = apply_message(engine, &mut hover, &map, msg);
        out.extend(ev.cmds);
    }
    out
}

// ============================================================
// `el.focus()`
// ============================================================

#[test]
fn el_focus_focuses_the_field_and_fires_focus_event() {
    let mut engine = ScriptEngine::new(
        r#"<script>
          var seen = [];
          document.getElementById('a').addEventListener('focus', function () { seen.push('a-focus'); });
          globalThis.__seen = seen;
        </script>
        <input id="a"><input id="b">"#, "",

        EventBindings::default(),
    )
    .expect("engine initializes");
    let mut state = InputState::new();
    engine
        .eval_source("document.getElementById('a').focus()")
        .expect("focus() runs");
    let arena = engine.arena();
    let a = find_node_by_id(&arena, "a");
    let cmds = drive(&mut engine, &mut state, &arena);
    assert_eq!(
        state.focused_input,
        Some(a),
        "el.focus() must focus the field"
    );
    assert_eq!(
        eval_str(&mut engine, "globalThis.__seen.join(',')"),
        "a-focus"
    );
    assert!(
        cmds.iter().any(|c| {
            matches!(c, RenderCmd::RenderInput { node_id, focused: true, .. } if *node_id == a)
        }),
        "the focused field must be repainted"
    );
}

#[test]
fn el_focus_switches_focus_blurring_the_previous_field() {
    let mut engine = ScriptEngine::new(
        r#"<script>
          var seen = [];
          var a = document.getElementById('a');
          var b = document.getElementById('b');
          a.addEventListener('focus', function () { seen.push('a-focus'); });
          a.addEventListener('blur', function () { seen.push('a-blur'); });
          b.addEventListener('focus', function () { seen.push('b-focus'); });
          globalThis.__seen = seen;
        </script>
        <input id="a"><input id="b">"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    let mut state = InputState::new();
    let arena = engine.arena();
    let a = find_node_by_id(&arena, "a");
    let b = find_node_by_id(&arena, "b");

    engine
        .eval_source("document.getElementById('a').focus()")
        .expect("focus a");
    drive(&mut engine, &mut state, &arena);
    assert_eq!(state.focused_input, Some(a));
    assert_eq!(
        eval_str(&mut engine, "globalThis.__seen.join(',')"),
        "a-focus"
    );

    // Focusing `b` blurs `a` first (browser order: blur then focus).
    engine
        .eval_source("document.getElementById('b').focus()")
        .expect("focus b");
    drive(&mut engine, &mut state, &arena);
    assert_eq!(state.focused_input, Some(b), "focus must move to b");
    assert_eq!(
        eval_str(&mut engine, "globalThis.__seen.join(',')"),
        "a-focus,a-blur,b-focus",
        "blur(prev) must fire before focus(new)"
    );
}

#[test]
fn el_focus_on_already_focused_field_is_a_noop() {
    let mut engine = ScriptEngine::new(
        r#"<script>
          globalThis.__n = 0;
          document.getElementById('a').addEventListener('focus', function () { globalThis.__n++; });
        </script>
        <input id="a">"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    let mut state = InputState::new();
    let arena = engine.arena();
    engine
        .eval_source("document.getElementById('a').focus()")
        .expect("focus a");
    drive(&mut engine, &mut state, &arena);
    assert_eq!(eval_str(&mut engine, "globalThis.__n"), "1");
    // Refocusing the same field must not re-fire `focus`.
    engine
        .eval_source("document.getElementById('a').focus()")
        .expect("refocus a");
    drive(&mut engine, &mut state, &arena);
    assert_eq!(eval_str(&mut engine, "globalThis.__n"), "1");
}

#[test]
fn el_focus_works_on_textarea() {
    let mut engine = ScriptEngine::new(
        r#"<textarea id="t">hello</textarea>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    let mut state = InputState::new();
    let arena = engine.arena();
    let t = find_node_by_id(&arena, "t");
    engine
        .eval_source("document.getElementById('t').focus()")
        .expect("focus textarea");
    drive(&mut engine, &mut state, &arena);
    assert_eq!(state.focused_input, Some(t));
    assert!(
        state.focused_is_textarea,
        "textarea must enter multi-line editing"
    );
    assert_eq!(
        state.input_value, "hello",
        "edit buffer seeds from the text content"
    );
}

#[test]
fn el_focus_on_non_focusable_element_is_a_noop() {
    let mut engine = ScriptEngine::new(
        r#"<div id="d"></div><input id="a">"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    let mut state = InputState::new();
    let arena = engine.arena();
    engine
        .eval_source("document.getElementById('d').focus()")
        .expect("div.focus() runs");
    let cmds = drive(&mut engine, &mut state, &arena);
    assert_eq!(state.focused_input, None, "div is not focusable");
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, RenderCmd::RenderInput { .. })),
        "no field repaint for a non-focusable target"
    );
}

// ============================================================
// `el.blur()`
// ============================================================

#[test]
fn el_blur_removes_focus_from_the_focused_field() {
    let mut engine = ScriptEngine::new(
        r#"<script>
          var seen = [];
          var a = document.getElementById('a');
          a.addEventListener('focus', function () { seen.push('a-focus'); });
          a.addEventListener('blur', function () { seen.push('a-blur'); });
          globalThis.__seen = seen;
        </script>
        <input id="a" value="x">"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    let mut state = InputState::new();
    let arena = engine.arena();
    let a = find_node_by_id(&arena, "a");
    engine
        .eval_source("document.getElementById('a').focus()")
        .expect("focus a");
    drive(&mut engine, &mut state, &arena);
    assert_eq!(state.focused_input, Some(a));
    assert_eq!(
        eval_str(&mut engine, "globalThis.__seen.join(',')"),
        "a-focus"
    );

    engine
        .eval_source("document.getElementById('a').blur()")
        .expect("blur a");
    drive(&mut engine, &mut state, &arena);
    assert_eq!(state.focused_input, None, "el.blur() must clear the focus");
    assert_eq!(
        eval_str(&mut engine, "globalThis.__seen.join(',')"),
        "a-focus,a-blur"
    );
}

#[test]
fn el_blur_on_non_focused_element_is_a_noop() {
    let mut engine = ScriptEngine::new(
        r#"<script>
          globalThis.__n = 0;
          var a = document.getElementById('a');
          a.addEventListener('focus', function () { globalThis.__n++; });
          a.addEventListener('blur', function () { globalThis.__n++; });
        </script>
        <input id="a"><input id="b">"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    let mut state = InputState::new();
    let arena = engine.arena();
    engine
        .eval_source("document.getElementById('a').focus()")
        .expect("focus a");
    drive(&mut engine, &mut state, &arena);
    assert_eq!(eval_str(&mut engine, "globalThis.__n"), "1");

    // Blurring the *other* field does nothing (browser semantics).
    engine
        .eval_source("document.getElementById('b').blur()")
        .expect("blur b");
    drive(&mut engine, &mut state, &arena);
    assert_eq!(
        eval_str(&mut engine, "globalThis.__n"),
        "1",
        "blur() on a non-focused element must be a no-op"
    );
    assert!(
        state.focused_input.is_some(),
        "focus must stay on a after b.blur()"
    );
}

// ============================================================
// CSS `:focus` pseudo-class
// ============================================================

#[test]
fn focus_pseudo_class_styles_only_the_focused_field() {
    let mut parser =
        html_parse::Parser::new(r#"<div id="wrap"><input id="a"><input id="b"></div>"#);
    let (root_id, mut arena, _embedded) = parser.parse();
    let rules = css_parse::Parser::new("input:focus { color: red; }").parse();
    let a = find_node_by_id(&arena, "a");
    let b = find_node_by_id(&arena, "b");

    // No focus: neither field matches `:focus`.
    css_parse::css_into_node_with_state(
        &mut arena,
        root_id,
        &rules,
        &css_parse::DynamicState::default(),
    );
    assert_eq!(arena[a].css_attr.get_color(), None, "unfocused: no match");
    assert_eq!(arena[b].css_attr.get_color(), None);

    // Focusing `a` styles only `a`.
    css_parse::reset_css_attr(&mut arena, root_id);
    css_parse::css_into_node_with_state(
        &mut arena,
        root_id,
        &rules,
        &css_parse::DynamicState {
            hovered: None,
            focused: Some(a),
        },
    );
    assert!(
        arena[a].css_attr.get_color().is_some(),
        "the focused field must match :focus"
    );
    assert_eq!(
        arena[b].css_attr.get_color(),
        None,
        "sibling stays unstyled"
    );
}

#[test]
fn focus_pseudo_class_matches_only_the_element_itself() {
    // Unlike `:hover` (which matches every ancestor of the hovered node),
    // `:focus` matches only the focused element — `:focus-within` would match
    // the ancestor chain.
    let mut parser = html_parse::Parser::new(r#"<div id="wrap"><input id="a"></div>"#);
    let (root_id, mut arena, _embedded) = parser.parse();
    let rules = css_parse::Parser::new("div:focus { color: red; }").parse();
    let wrap = find_node_by_id(&arena, "wrap");
    let a = find_node_by_id(&arena, "a");
    css_parse::css_into_node_with_state(
        &mut arena,
        root_id,
        &rules,
        &css_parse::DynamicState {
            hovered: None,
            focused: Some(a),
        },
    );
    assert_eq!(
        arena[wrap].css_attr.get_color(),
        None,
        ":focus must not match ancestors of the focused field"
    );
}

#[test]
fn focus_pseudo_class_rejects_arguments() {
    let err: Result<css_parse::CssSelector, _> = "input:focus()".parse();
    assert!(err.is_err(), ":focus takes no arguments");
    let ok: Result<css_parse::CssSelector, _> = "input:focus".parse();
    assert!(ok.is_ok());
}

// ============================================================
// focus state published to the render thread / autofocus
// ============================================================

#[test]
fn el_focus_pushes_focus_command_for_the_render_thread() {
    let mut engine = ScriptEngine::new(r#"<input id="a">"#, "", EventBindings::default())
        .expect("engine initializes");
    let mut state = InputState::new();
    let arena = engine.arena();
    let a = find_node_by_id(&arena, "a");
    engine
        .eval_source("document.getElementById('a').focus()")
        .expect("focus a");
    let cmds = drive(&mut engine, &mut state, &arena);
    assert_eq!(state.focused_input, Some(a));
    assert!(
        cmds.iter()
            .any(|c| { matches!(c, RenderCmd::Focus { node_id: Some(id) } if *id == a) }),
        "focusing must publish a Focus command for the render thread's :focus cascade"
    );
    // And blurring clears it.
    engine
        .eval_source("document.getElementById('a').blur()")
        .expect("blur a");
    let cmds = drive(&mut engine, &mut state, &arena);
    assert_eq!(state.focused_input, None);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, RenderCmd::Focus { node_id: None })),
        "blurring must publish Focus{{node_id: None}} to clear the highlight"
    );
}

#[test]
fn autofocus_attribute_focuses_the_first_field_on_load() {
    let mut engine = ScriptEngine::new(
        r#"<input id="a" autofocus><input id="b">"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    let arena = engine.arena();
    let a = find_node_by_id(&arena, "a");
    let cmds = engine.take_pending_cmds();
    assert!(
        cmds.iter()
            .any(|c| { matches!(c, RenderCmd::FocusRequest { node_id } if *node_id == a) }),
        "autofocus must request focus for the first autofocus element"
    );
    // Full round trip: the request reaches the input loop and focuses it.
    let mut state = InputState::new();
    let cmds = apply_focus_requests(&mut state, cmds, Some(&arena), &mut Vec::new());
    assert_eq!(state.focused_input, Some(a), "autofocus must take effect");
    assert!(
        cmds.iter()
            .any(|c| { matches!(c, RenderCmd::Focus { node_id: Some(id) } if *id == a) }),
        "autofocus must publish the :focus repaint"
    );
}

#[test]
fn autofocus_ignores_non_focusable_elements() {
    // `autofocus` on a `<div>` must not focus anything (browser behaviour).
    let mut engine = ScriptEngine::new(
        r#"<div id="d" autofocus></div>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    let cmds = engine.take_pending_cmds();
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, RenderCmd::FocusRequest { .. })),
        "autofocus on a non-focusable element must be ignored"
    );
}

// ============================================================
// dirty / commit
// ============================================================

#[test]
fn el_blur_commits_edited_value_back_to_the_dom() {
    // A blur() triggered by JS commits the edit buffer like a click-outside
    // blur would: the arena text content / value attribute is updated and a
    // dirty marker routes the re-render.
    let mut engine = ScriptEngine::new(
        r#"<textarea id="t">old</textarea>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    let mut state = InputState::new();
    let arena = engine.arena();
    let t = find_node_by_id(&arena, "t");
    engine
        .eval_source("document.getElementById('t').focus()")
        .expect("focus t");
    drive(&mut engine, &mut state, &arena);
    // Simulate typed edits in the edit buffer, then blur() from JS.
    state.input_value = "new text".into();
    state.input_cursor = 8;
    engine
        .eval_source("document.getElementById('t').blur()")
        .expect("blur t");
    drive(&mut engine, &mut state, &arena);
    let arena = engine.arena();
    assert_eq!(
        arena.get(t).unwrap().text_content(&arena),
        "new text",
        "blur must commit the edit buffer into the text content"
    );
    assert_eq!(state.focused_input, None);
}
