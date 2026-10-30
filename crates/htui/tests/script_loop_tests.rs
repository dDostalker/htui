//! Tests for the script-thread bridge: `apply_message` event semantics and
//! the `run_script_thread` loop, including the core decoupling property that
//! a blocking host function never stalls the input side's channel. Pure
//! in-memory — no terminal required.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use htui::events::{EventBindings, Geometry, HitMap, HoverState, KeyEventInfo, SharedHitMap};
use htui::layout::layout::TermMap;
use htui::layout::{BoxConstraint, Cursor, layout_tree};
use htui::resources::{ResourceRegistry, with_registry};
use htui::script::{
    ScriptEngine, ScriptEvent, ScriptMsg, ScriptResponse, SharedUiState, apply_message,
    run_script_thread,
};
use htui::tdom::node::{NodeArena, NodeId};
use htui::tui::graph_api::RenderCmd;

const COLS: u16 = 120;
const ROWS: u16 = 40;

#[test]
fn apply_message_move_tracks_hover_and_fires_over_out_in_order() {
    // A pointer move enters / leaves regions and fires `mouseover` /
    // `mouseout` in DOM order (leave before enter); the response reports the
    // new hover target and no DOM mutation.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var seen = [];
          document.getElementById('a').addEventListener('mouseover', function(){seen.push('a-in')});
          document.getElementById('a').addEventListener('mouseout', function(){seen.push('a-out')});
          document.getElementById('b').addEventListener('mouseover', function(){seen.push('b-in')});
          document.getElementById('b').addEventListener('mouseout', function(){seen.push('b-out')});
          globalThis.__seen = seen;
        </script>
        <div id="a">A</div><div id="b">B</div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let a = map.resolve_by_id("a").expect("a region");
    let b = map.resolve_by_id("b").expect("b region");
    let (ax, ay) = center(a.rect);
    let (bx, by) = center(b.rect);

    let mut hover = HoverState::new();
    let ev = apply_message(
        &mut engine,
        &mut hover,
        &map,
        ScriptMsg::Move { x: ax, y: ay },
    );
    assert_eq!(ev.hovered, Some(a.node_id), "hover must land on `a`");
    assert!(ev.dirty.is_none(), "a plain pointer move mutates nothing");

    let ev = apply_message(
        &mut engine,
        &mut hover,
        &map,
        ScriptMsg::Move { x: bx, y: by },
    );
    assert_eq!(ev.hovered, Some(b.node_id), "hover must move to `b`");
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))"),
        "a-in,a-out,b-in",
        "leave fires before enter; both dispatch along the ancestor chain"
    );
}

#[test]
fn apply_message_click_fires_the_matching_listener() {
    // A click dispatches through the hover sync and fires the element's
    // `onclick` host function.
    let calls = Arc::new(Mutex::new(0usize));
    let calls2 = Arc::clone(&calls);
    let (mut engine, geo) = build_geometry(
        r#"<div id="btn" onclick="tick()">click me</div>"#,
        counting_host(calls2),
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("btn").expect("btn region");
    let (x, y) = center(region.rect);

    let mut hover = HoverState::new();
    let ev = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Click { x, y });
    assert_eq!(
        *calls.lock().unwrap(),
        1,
        "click must call the host function"
    );
    assert_eq!(
        ev.hovered,
        Some(region.node_id),
        "click syncs the hover target"
    );
}

#[test]
fn apply_message_keydown_fires_body_listener_with_browser_fields() {
    // A keydown with no focused field targets the document `<body>` (the
    // browser's fallback target); the JS event object carries the
    // browser-style keyboard fields (`key` / `code` / `keyCode` / modifiers).
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var seen = [];
          globalThis.__seen = seen;
        </script>
        <body onkeydown="seen.push(event.type+':'+event.key+':'+event.code+':'+event.keyCode+(event.ctrlKey?':ctrl':'')+(event.shiftKey?':shift':'')+(event.repeat?':rep':''))">
          <div id="a">A</div>
        </body>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let mut hover = HoverState::new();
    let ev = apply_message(
        &mut engine,
        &mut hover,
        &map,
        ScriptMsg::KeyDown {
            key_id: 1,
            target: None,
            key: KeyEventInfo {
                key: "ArrowUp".into(),
                code: "ArrowUp".into(),
                key_code: 38,
                shift: true,
                repeat: true,
                ..Default::default()
            },
        },
    );
    assert!(ev.dirty.is_none(), "a plain key dispatch mutates nothing");
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))",),
        "keydown:ArrowUp:ArrowUp:38:shift:rep",
        "keydown must reach the body handler with the full keyboard payload"
    );
}

#[test]
fn apply_message_keydown_targets_focused_input_and_bubbles_to_ancestors() {
    // A keydown targeting a focused `<input>` fires the field's handler first,
    // then bubbles up the DOM ancestor chain (browser key event propagation).
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var seen = [];
          globalThis.__seen = seen;
        </script>
        <div id="wrap" onkeydown="seen.push('wrap:'+event.key)">
          <input id="inp" onkeydown="seen.push('inp:'+event.key)">
        </div>"#,
        EventBindings::default(),
    );
    let input_id = find_node_by_id(&engine.arena(), "inp");
    let map = HitMap::build(&geo);
    let mut hover = HoverState::new();
    apply_message(
        &mut engine,
        &mut hover,
        &map,
        ScriptMsg::KeyDown {
            key_id: 2,
            target: Some(input_id),
            key: KeyEventInfo {
                key: "Enter".into(),
                code: "Enter".into(),
                key_code: 13,
                ..Default::default()
            },
        },
    );
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))",),
        "inp:Enter,wrap:Enter",
        "target fires before its ancestors (bubbling)"
    );
}

#[test]
fn apply_message_keydown_falls_back_to_body_for_removed_target() {
    // A keydown targeted at a node the JS side removed must not panic: the
    // dispatch falls back to the body handler, exactly like a stale focus
    // reference in the input loop.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var seen = [];
          globalThis.__seen = seen;
        </script>
        <body onkeydown="seen.push('body:'+event.key)"><div id="a">A</div></body>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let mut hover = HoverState::new();
    // `NodeId::default()` is the null slotmap key — never present in the arena.
    let ev = apply_message(
        &mut engine,
        &mut hover,
        &map,
        ScriptMsg::KeyDown {
            key_id: 3,
            target: Some(NodeId::default()),
            key: KeyEventInfo {
                key: "F5".into(),
                code: "F5".into(),
                key_code: 116,
                ..Default::default()
            },
        },
    );
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))",),
        "body:F5",
        "stale target must degrade to the body handler without panicking"
    );
    assert!(ev.dirty.is_none());
}

#[test]
fn apply_message_keyup_fires_the_keyup_listener() {
    // `keyup` dispatches with the same targeting and a `type` of `keyup`.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var seen = [];
          globalThis.__seen = seen;
        </script>
        <body onkeyup="seen.push(event.type+':'+event.key)"><div id="a">A</div></body>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let mut hover = HoverState::new();
    apply_message(
        &mut engine,
        &mut hover,
        &map,
        ScriptMsg::KeyUp {
            target: None,
            key: KeyEventInfo {
                key: "Backspace".into(),
                code: "Backspace".into(),
                key_code: 8,
                ..Default::default()
            },
        },
    );
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))",),
        "keyup:Backspace",
        "keyup must reach the body handler"
    );
}

#[test]
fn apply_message_keydown_handler_can_mutate_dom_and_report_dirty() {
    // A keydown handler is real JS: it can rewrite the DOM and the response
    // reports the mutated nodes for a `RenderArena` — the same contract as
    // mouse / form handlers.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          globalThis.__seen = [];
        </script>
        <body onkeydown="document.getElementById('a').textContent = 'x'"><div id="a">A</div></body>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let mut hover = HoverState::new();
    let ev = apply_message(
        &mut engine,
        &mut hover,
        &map,
        ScriptMsg::KeyDown {
            key_id: 4,
            target: None,
            key: KeyEventInfo {
                key: "Space".into(),
                code: "Space".into(),
                key_code: 32,
                ..Default::default()
            },
        },
    );
    let dirty = ev.dirty.expect("keydown DOM mutation must be reported");
    assert!(
        dirty.dirty.nodes.iter().any(|&id| {
            dirty
                .arena
                .get(id)
                .is_some_and(|n| n.attributes.get("id").map(String::as_str) == Some("a"))
        }),
        "the mutated node must appear in the dirty set"
    );
}

#[test]
fn apply_message_keydown_prevent_default_is_reported_in_the_gate() {
    // A keydown handler that calls `event.preventDefault()` must make the
    // gate verdict report `prevented = true` (echoing the key id), so the
    // input loop skips the built-in shortcut (scroll / zoom / quit) and the
    // page owns that key.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var seen = [];
          globalThis.__seen = seen;
        </script>
        <body onkeydown="event.preventDefault(); seen.push(event.defaultPrevented ? 'pd' : 'no')">
          <div id="a">A</div>
        </body>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let mut hover = HoverState::new();
    let ev = apply_message(
        &mut engine,
        &mut hover,
        &map,
        ScriptMsg::KeyDown {
            key_id: 42,
            target: None,
            key: KeyEventInfo {
                key: "ArrowUp".into(),
                code: "ArrowUp".into(),
                key_code: 38,
                ..Default::default()
            },
        },
    );
    assert_eq!(
        ev.key_gate,
        Some((42, true)),
        "preventDefault must be echoed with the matching key id"
    );
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))"),
        "pd",
        "defaultPrevented must read back true inside the handler"
    );
}

#[test]
fn apply_message_keydown_without_prevent_default_reports_clear_verdict() {
    // Without any listener calling `preventDefault()` the gate reports a clear
    // verdict (`prevented = false`), so the built-in shortcut runs normally.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var seen = [];
          globalThis.__seen = seen;
        </script>
        <body onkeydown="seen.push('ran')"><div id="a">A</div></body>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let mut hover = HoverState::new();
    let ev = apply_message(
        &mut engine,
        &mut hover,
        &map,
        ScriptMsg::KeyDown {
            key_id: 7,
            target: None,
            key: KeyEventInfo {
                key: "j".into(),
                code: "KeyJ".into(),
                key_code: 106,
                ..Default::default()
            },
        },
    );
    assert_eq!(
        ev.key_gate,
        Some((7, false)),
        "an unprevented keydown clears the gate for the built-in action"
    );
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))"),
        "ran"
    );
}

#[test]
fn initial_session_js_runs_after_document_scripts() {
    let mut engine = ScriptEngine::new(
        r#"<script>globalThis.__order = ["inline"];</script><p>x</p>"#,
        r#"globalThis.__order.push("session")"#,
        EventBindings::default(),
    )
    .expect("engine initializes");
    assert_eq!(
        eval_str(&mut engine, "globalThis.__order.join(',')"),
        "inline,session"
    );
}

#[test]
fn reload_js_keeps_the_dom_and_reports_mutations() {
    let (mut engine, _geo) = build_geometry(r#"<p id="value">old</p>"#, EventBindings::default());
    engine
        .reload_js(r#"document.getElementById('value').textContent = "fresh";"#)
        .expect("session JS reloads");

    let dirty = engine.take_dirty();
    assert!(
        !dirty.is_empty(),
        "DOM writes from reloaded JS must report a repaint"
    );
    let arena = engine.arena();
    assert!(
        arena
            .iter()
            .any(|(_, n)| n.text_content(&arena).contains("fresh")),
        "the live DOM must survive and reflect the reloaded JS"
    );
}

#[test]
fn reload_html_after_mem_change_resolves_the_new_resource() {
    let registry = Arc::new(ResourceRegistry::new());
    registry
        .register("mem-change.js", b"globalThis.__mem = \"old\";".to_vec())
        .expect("register initial resource");

    let result = with_registry(Arc::clone(&registry), || {
        let mut parser =
            htui::html::html_parse::Parser::new(r#"<script src="mem-change.js"></script><p>x</p>"#);
        let document = parser.parse_with_scripts();
        let mut engine = ScriptEngine::new_from_document(document, "", EventBindings::default())
            .expect("engine initializes");
        assert_eq!(eval_str(&mut engine, "globalThis.__mem"), "old");

        registry
            .register("mem-change.js", b"globalThis.__mem = \"new\";".to_vec())
            .expect("register changed resource");

        let mut hover = HoverState::new();
        let map = HitMap::new();
        let event = apply_message(
            &mut engine,
            &mut hover,
            &map,
            ScriptMsg::ReloadHtml {
                html: Arc::from(r#"<script src="mem-change.js"></script><p id="fresh">fresh</p>"#),
            },
        );
        assert!(event.dirty.is_some(), "a mem-driven reparse must repaint");
        eval_str(&mut engine, "globalThis.__mem")
    });
    assert_eq!(result, "new", "reparse must read the changed mem resource");
}

#[test]
fn reload_html_message_reports_document_css_and_clears_hover() {
    let (mut engine, geo) = build_geometry(r#"<div id="old">old</div>"#, EventBindings::default());
    let map = HitMap::build(&geo);
    let old = map.resolve_by_id("old").expect("old region");
    let (x, y) = center(old.rect);
    let mut hover = HoverState::new();

    let entered = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Move { x, y });
    assert_eq!(entered.hovered, Some(old.node_id));

    let reloaded = apply_message(
        &mut engine,
        &mut hover,
        &map,
        ScriptMsg::ReloadHtml {
            html: Arc::from(r#"<style>p { color: red; }</style><p id="fresh">fresh</p>"#),
        },
    );
    assert_eq!(reloaded.hovered, None, "old node ids cannot survive reload");

    let dirty = reloaded.dirty.expect("reload marks the fresh root dirty");
    let document_css = dirty
        .document_css
        .as_deref()
        .expect("reload carries document CSS");
    assert!(
        document_css.contains("color: red"),
        "document CSS must reach the render bridge: {document_css}"
    );
    assert!(
        dirty.dirty.nodes.contains(&dirty.root_id),
        "the fresh root must be marked dirty"
    );
    find_node_by_id(&dirty.arena, "fresh");
}

#[test]
fn set_timeout_fires_once_then_clears() {
    // `setTimeout` fires exactly once when the deadline passes.
    let (mut engine, _geo) = build_geometry(
        r#"<script>
          var seen = [];
          setTimeout(function(){ seen.push('t'); }, 5);
          globalThis.__seen = seen;
        </script><p>x</p>"#,
        EventBindings::default(),
    );
    // Not yet due: nothing fires.
    assert!(engine.run_due_timers().is_empty());
    std::thread::sleep(Duration::from_millis(30));
    engine.run_due_timers();
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))"),
        "t",
        "one-shot timeout must fire once"
    );
    // One-shot: a later run does not re-fire.
    std::thread::sleep(Duration::from_millis(30));
    engine.run_due_timers();
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))"),
        "t"
    );
}

#[test]
fn set_interval_repeats_until_cleared() {
    // `setInterval` re-schedules after each fire; `clearInterval` stops it.
    let (mut engine, _geo) = build_geometry(
        r#"<script>
          var seen = [];
          var iv = setInterval(function(){ seen.push('i'); }, 5);
          globalThis.__seen = seen;
          globalThis.__iv = iv;
        </script><p>x</p>"#,
        EventBindings::default(),
    );
    std::thread::sleep(Duration::from_millis(30));
    engine.run_due_timers();
    std::thread::sleep(Duration::from_millis(30));
    engine.run_due_timers();
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))"),
        "i,i",
        "interval must re-schedule and fire again"
    );
    eval_str(&mut engine, "clearInterval(globalThis.__iv)");
    eval_str(&mut engine, "globalThis.__seen = []");
    std::thread::sleep(Duration::from_millis(30));
    engine.run_due_timers();
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))"),
        "",
        "cleared interval must not fire"
    );
}

#[test]
fn set_interval_zero_period_is_clamped_to_browser_minimum() {
    // `setInterval(fn, 0)` must not register a zero period: the live script
    // thread would wake `recv_timeout(0)`, fire, re-schedule at +0 ms and
    // spin at 100% CPU with an unbounded response rate. The browser clamps
    // sub-4 ms intervals to ~4 ms — the registered period here must behave
    // the same way.
    let (mut engine, _geo) = build_geometry(
        r#"<script>
          var seen = [];
          var iv = setInterval(function(){ seen.push('i'); }, 0);
          globalThis.__seen = seen;
          globalThis.__iv = iv;
        </script><p>x</p>"#,
        EventBindings::default(),
    );
    fn fires(engine: &mut ScriptEngine) -> usize {
        eval_str(engine, "String(globalThis.__seen.length)")
            .parse()
            .expect("seen length must be a number")
    }

    std::thread::sleep(Duration::from_millis(20));

    // Hot-drain: after the first fire the callback is re-scheduled 4 ms out,
    // so a tight loop over due timers cannot re-fire it (a zero period would
    // fire on every one of these 50 calls — the busy-spin shape).
    let before = fires(&mut engine);
    for _ in 0..50 {
        engine.run_due_timers();
    }
    let hot_fires = fires(&mut engine) - before;
    assert!(
        (1..=2).contains(&hot_fires),
        "zero-period interval must clamp to the 4 ms minimum \
         (fired {hot_fires} times in a tight 50-call drain)"
    );

    // Still repeating: after the clamp window passes it fires again.
    let after_hot = fires(&mut engine);
    std::thread::sleep(Duration::from_millis(10));
    engine.run_due_timers();
    assert!(
        fires(&mut engine) > after_hot,
        "clamped interval must keep repeating"
    );

    eval_str(&mut engine, "clearInterval(globalThis.__iv)");
}

#[test]
fn timer_callback_dom_write_marks_dirty() {
    // A timer callback that rewrites the DOM behaves like any other JS
    // mutation: it marks the touched nodes dirty for the next RenderArena.
    let (mut engine, _geo) = build_geometry(
        r#"<script>
          var el = document.getElementById('f');
          setTimeout(function(){ el.textContent = 'tick'; }, 5);
        </script><p id="f">x</p>"#,
        EventBindings::default(),
    );
    assert!(engine.take_dirty().is_empty(), "nothing mutated yet");
    std::thread::sleep(Duration::from_millis(30));
    engine.run_due_timers();
    assert!(
        !engine.take_dirty().is_empty(),
        "timer DOM write must mark dirty"
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('f').textContent"),
        "tick"
    );
}

#[test]
fn timer_callback_can_push_render_commands() {
    // Host functions called from a timer callback push into the same render
    // command queue as any other JS execution.
    let bindings = host_bindings("pushIt", move |call| {
        call.render().push(RenderCmd::Redraw {
            scroll_y: 0,
            zoom: 1.0,
        });
        boa_engine::JsValue::undefined()
    });
    let mut engine = ScriptEngine::new(
        r#"<script>
          setTimeout(function(){ pushIt(); }, 5);
        </script><p>x</p>"#,
        "",
        bindings,
    )
    .expect("engine");
    std::thread::sleep(Duration::from_millis(30));
    let cmds = engine.run_due_timers();
    assert!(
        cmds.iter().any(|c| matches!(c, RenderCmd::Redraw { .. })),
        "host push from a timer must surface in the returned commands"
    );
}

#[test]
fn timer_callback_can_register_new_timers() {
    // Timers scheduled from inside a timer callback must fire too (the due
    // ids are snapshotted before the callbacks run).
    let (mut engine, _geo) = build_geometry(
        r#"<script>
          var seen = [];
          setTimeout(function(){
            seen.push('outer');
            setTimeout(function(){ seen.push('inner'); }, 5);
          }, 5);
          globalThis.__seen = seen;
        </script><p>x</p>"#,
        EventBindings::default(),
    );
    std::thread::sleep(Duration::from_millis(30));
    engine.run_due_timers();
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))"),
        "outer"
    );
    std::thread::sleep(Duration::from_millis(30));
    engine.run_due_timers();
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))"),
        "outer,inner",
        "a timer registered inside a callback must fire later"
    );
}

#[test]
fn apply_message_blur_commits_the_value_and_fires_blur_then_change() {
    // `BlurInput` commits the edited value and fires `blur` + `change` in
    // order (change only when the value differs).
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var seen = [];
          var el = document.getElementById('f');
          el.addEventListener('blur', function(){seen.push('blur');});
          el.addEventListener('change', function(){seen.push('change');});
          globalThis.__seen = seen;
        </script>
        <input id="f" value="old">"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let node_id = find_node_by_id(&engine.arena(), "f");

    let mut hover = HoverState::new();
    let ev = apply_message(
        &mut engine,
        &mut hover,
        &map,
        ScriptMsg::BlurInput {
            node_id,
            value: "new".into(),
        },
    );
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))"),
        "blur,change",
        "blur must fire before change"
    );
    let committed = engine
        .arena()
        .get(node_id)
        .and_then(|n| n.attributes.get("value"))
        .cloned();
    assert_eq!(committed.as_deref(), Some("new"), "value must be committed");
    let dirty = ev.dirty.expect("blur commit must report a repaint");
    assert!(
        dirty.dirty.nodes.contains(&node_id),
        "the committed node must be in the dirty set"
    );
}

#[test]
fn blur_handler_rewrite_reports_the_final_value_in_the_dirty_arena() {
    // The Codex scenario: a blur handler rewrites `el.value` after the blur
    // commit. Because the commit itself marks the node dirty, the resulting
    // `RenderArena` carries the *handler's* value — the field clears on screen
    // instead of keeping the stale edited text.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var el = document.getElementById('f');
          el.addEventListener('blur', function(){ el.value = ''; });
        </script>
        <input id="f" value="old">"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let node_id = find_node_by_id(&engine.arena(), "f");

    let mut hover = HoverState::new();
    let ev = apply_message(
        &mut engine,
        &mut hover,
        &map,
        ScriptMsg::BlurInput {
            node_id,
            value: "new".into(),
        },
    );
    let dirty = ev.dirty.expect("blur commit must report a repaint");
    let shown = dirty
        .arena
        .get(node_id)
        .and_then(|n| n.attributes.get("value"))
        .cloned();
    assert_eq!(
        shown.as_deref(),
        Some(""),
        "the repaint arena must carry the handler-cleared value"
    );
}

#[test]
fn apply_message_select_pick_commits_and_reports_dirty() {
    // `SelectPick` writes the new value with a dirty flag (label swap needs a
    // re-render) and fires `change` with the committed value readable in JS.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var seen = [];
          var s = document.getElementById('s');
          s.addEventListener('change', function(){seen.push(s.value);});
          globalThis.__seen = seen;
        </script>
        <div><select id="s"><option value="a">A</option><option value="b">B</option></select></div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let node_id = find_node_by_id(&engine.arena(), "s");

    let mut hover = HoverState::new();
    let ev = apply_message(
        &mut engine,
        &mut hover,
        &map,
        ScriptMsg::SelectPick {
            node_id,
            value: "b".into(),
        },
    );
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))"),
        "b",
        "change must fire with the new value"
    );
    let dirty = ev
        .dirty
        .expect("a select value change must request a repaint");
    let committed = dirty
        .arena
        .get(node_id)
        .and_then(|n| n.attributes.get("value"))
        .cloned();
    assert_eq!(
        committed.as_deref(),
        Some("b"),
        "dirty snapshot must carry the new value"
    );
}

#[test]
fn run_script_thread_ready_then_processes_messages() {
    // End to end: the script thread constructs the engine, answers `Ready`
    // with the initial arena, then processes a click against a hit map built
    // from that same arena (matching node ids).
    let calls = Arc::new(Mutex::new(0usize));
    let calls2 = Arc::clone(&calls);
    let bindings = counting_host(calls2);
    let hit_map: SharedHitMap = Arc::new(Mutex::new(Arc::new(HitMap::new())));

    let (tx, out_rx, handle) = spawn_script(
        r#"<div id="btn" onclick="tick()">click me</div>"#,
        bindings,
        Arc::clone(&hit_map),
    );

    let (arena, root_id) = match out_rx.recv().expect("Ready must arrive") {
        ScriptResponse::Ready { arena, root_id } => (arena, root_id),
        other => panic!("expected Ready, got {other:?}"),
    };
    publish_hit_map(&hit_map, &arena, root_id);

    let region = {
        let map = hit_map.lock().unwrap();
        map.resolve_by_id("btn").expect("btn region").clone()
    };
    let (x, y) = center(region.rect);
    tx.send(ScriptMsg::Click { x, y }).expect("send click");

    match out_rx.recv().expect("event must arrive") {
        ScriptResponse::Event(ev) => {
            assert_eq!(*calls.lock().unwrap(), 1, "onclick host must have run");
            assert_eq!(ev.hovered, Some(region.node_id));
        }
        other => panic!("expected Event, got {other:?}"),
    }

    tx.send(ScriptMsg::Quit).expect("send quit");
    handle.join().expect("script thread must exit cleanly");
}

#[test]
fn run_script_thread_stays_silent_when_idle() {
    // An idle page (no timers, no fetches, no messages) must not tick the
    // loop: the thread blocks until a message arrives instead of emitting
    // empty `Event` responses on a poll interval.
    let hit_map: SharedHitMap = Arc::new(Mutex::new(Arc::new(HitMap::new())));

    let (tx, out_rx, handle) = spawn_script(
        r#"<div id="btn">plain</div>"#,
        EventBindings::default(),
        Arc::clone(&hit_map),
    );

    match out_rx.recv().expect("Ready must arrive") {
        ScriptResponse::Ready { .. } => {}
        other => panic!("expected Ready, got {other:?}"),
    }

    // The event-driven loop must produce no `Event` while the input is idle.
    assert!(
        out_rx.recv_timeout(Duration::from_millis(150)).is_err(),
        "idle script thread must not emit empty responses"
    );

    tx.send(ScriptMsg::Quit).expect("send quit");
    handle.join().expect("script thread must exit cleanly");
}

#[test]
fn blocking_host_fn_does_not_block_the_input_side_channel() {
    // The decoupling guarantee: while a host function sleeps inside the
    // script thread, sending further messages from the input side returns
    // immediately (the input loop never waits on the engine).
    let started = Arc::new(AtomicBool::new(false));
    let started2 = Arc::clone(&started);
    let bindings = host_bindings("slow", move |_| {
        started2.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(300));
        boa_engine::JsValue::undefined()
    });
    let hit_map: SharedHitMap = Arc::new(Mutex::new(Arc::new(HitMap::new())));

    let (tx, out_rx, _handle) = spawn_script(
        r#"<div id="btn" onclick="slow()">click me</div>"#,
        bindings,
        Arc::clone(&hit_map),
    );

    let (arena, root_id) = match out_rx.recv().expect("Ready must arrive") {
        ScriptResponse::Ready { arena, root_id } => (arena, root_id),
        other => panic!("expected Ready, got {other:?}"),
    };
    publish_hit_map(&hit_map, &arena, root_id);

    let (x, y) = {
        let map = hit_map.lock().unwrap();
        let region = map.resolve_by_id("btn").expect("btn region");
        center(region.rect)
    };
    tx.send(ScriptMsg::Click { x, y }).expect("send click");

    // Wait until the host function is actually inside its sleep.
    let deadline = Instant::now() + Duration::from_secs(2);
    while !started.load(Ordering::Relaxed) {
        assert!(
            Instant::now() < deadline,
            "host function never entered the slow path"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    // The script thread is blocked for ~300 ms; a send must not wait on it.
    let t0 = Instant::now();
    tx.send(ScriptMsg::Quit)
        .expect("send quit while script thread is busy");
    assert!(
        t0.elapsed() < Duration::from_millis(100),
        "sending to the script thread must never block on a busy engine"
    );
}

// ============================================================
// fetch / event loop
// ============================================================

#[test]
fn fetch_mem_resource_resolves_promise_and_text() {
    // A `mem://` resource is resolved synchronously by `poll_fetches`; the
    // promise's `.then` (microtask) runs through the job queue — the engine's
    // event loop.
    let reg = Arc::new(ResourceRegistry::new());
    reg.register("mem://hello", b"hi from memory".to_vec())
        .expect("register mem resource");
    with_registry(Arc::clone(&reg), || {
        let mut engine = ScriptEngine::new(
            r#"<script>
              fetch('mem://hello').then(function (res) {
                return res.text();
              }).then(function (t) {
                document.getElementById('out').textContent = t;
              });
            </script>
            <div id="out"></div>"#,
            "",
            EventBindings::default(),
        )
        .expect("engine initializes");

        // The fetch was started during script evaluation but the load reports
        // back only when the loop polls — mirror the script thread's per-tick
        // drain.
        engine.poll_fetches();
        engine.poll_fetches();

        assert_eq!(
            eval_str(&mut engine, "document.getElementById('out').textContent"),
            "hi from memory",
            "the .then chain must run to completion and write the fetched body"
        );
    });
}

#[test]
fn fetch_mem_missing_rejects_promise() {
    // Fetching an unregistered `mem://` key rejects the promise; the rejection
    // handler (microtask) must run. An empty registry is installed so the
    // error is the key's absence (not a missing session scope).
    let reg = Arc::new(ResourceRegistry::new());
    with_registry(Arc::clone(&reg), || {
        let mut engine = ScriptEngine::new(
            r#"<script>
              globalThis.__rejected = 'no';
              fetch('mem://nope').then(function () {
                globalThis.__rejected = 'resolved';
              }, function (err) {
                globalThis.__rejected = 'rejected:' + String(err);
              });
            </script>"#,
            "",
            EventBindings::default(),
        )
        .expect("engine initializes");

        engine.poll_fetches();
        engine.poll_fetches();

        let rejected = eval_str(&mut engine, "String(globalThis.__rejected)");
        assert!(
            rejected.starts_with("rejected:"),
            "unregistered mem key must reject, got {rejected}"
        );
        assert!(
            rejected.contains("not registered"),
            "rejection reason should carry the resource error, got {rejected}"
        );
    });
}

#[test]
fn fetch_mem_resource_resolves_through_poll() {
    // A `mem://` fetch resolves synchronously inside `poll_fetches` (the
    // registry is in-memory), but the `.then` microtask chain still runs
    // through the job queue.
    let reg = Arc::new(ResourceRegistry::new());
    let key = format!("mem://fetch/file_{}.txt", std::process::id());
    reg.register(&key, b"file body".to_vec())
        .expect("register mem file");

    let html = format!(
        r#"<script>
          fetch('{key}').then(function (res) {{ return res.text(); }})
            .then(function (t) {{ document.getElementById('out').textContent = t; }});
        </script>
        <div id="out"></div>"#
    );
    with_registry(Arc::clone(&reg), || {
        let mut engine =
            ScriptEngine::new(&html, "", EventBindings::default()).expect("engine initializes");

        // A single drain settles the promise; run the microtask queue to land the
        // text in the DOM.
        engine.poll_fetches();
        engine.poll_fetches();

        assert_eq!(
            eval_str(&mut engine, "document.getElementById('out').textContent"),
            "file body",
            "mem fetch must resolve through poll_fetches"
        );
    });
}

#[test]
fn fetch_relative_url_resolves_against_document_base() {
    // JS `fetch()` resolves relative URLs against the document base URL
    // (`<base href>`) the same way `<img src>` / `<script src>` do.
    let reg = Arc::new(ResourceRegistry::new());
    let pid = std::process::id();
    let key = format!("mem://api/data_{pid}.json");
    reg.register(&key, br#"{"ok":true}"#.to_vec())
        .expect("register mem file");

    let html = format!(
        r#"<base href="mem://api/">
        <script>
          fetch('data_{pid}.json').then(function (res) {{ return res.text(); }})
            .then(function (t) {{ document.getElementById('out').textContent = t; }});
        </script>
        <div id="out"></div>"#
    );
    with_registry(Arc::clone(&reg), || {
        let mut engine =
            ScriptEngine::new(&html, "", EventBindings::default()).expect("engine initializes");
        engine.poll_fetches();
        engine.poll_fetches();

        assert_eq!(
            eval_str(&mut engine, "document.getElementById('out').textContent"),
            r#"{"ok":true}"#,
            "relative fetch must resolve against <base href>"
        );
    });
}

#[test]
fn base_uri_reflects_document_base_href() {
    // `Node.baseURI` reports the document base URL established by `<base>`;
    // without one it is an empty string.
    let pid = std::process::id();
    let html = format!(r#"<base href="mem://site/{pid}/"><div id="x"></div>"#);
    let mut engine =
        ScriptEngine::new(&html, "", EventBindings::default()).expect("engine initializes");
    let expected_uri = format!("mem://site/{pid}/");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('x').baseURI"),
        expected_uri
    );

    let mut no_base = ScriptEngine::new(r#"<div id="y"></div>"#, "", EventBindings::default())
        .expect("engine initializes");
    assert_eq!(
        eval_str(&mut no_base, "document.getElementById('y').baseURI"),
        "",
        "no <base href>: baseURI must be empty"
    );
}

// ============================================================
// Helpers
// ============================================================

fn build_geometry(html: &str, bindings: EventBindings) -> (ScriptEngine, Geometry) {
    let engine = ScriptEngine::new(html, "", bindings).expect("engine initializes");
    let arena = engine.arena();
    let root_id = engine.root_id();
    let term_map = TermMap {
        columns: COLS,
        rows: ROWS,
    };
    let containing = BoxConstraint::root(&term_map);
    let mut cursor = Cursor::new();
    let boxes = layout_tree(
        &arena,
        root_id,
        containing,
        &mut cursor,
        term_map.columns as usize,
        term_map.rows as usize,
    );
    let geo = Geometry {
        arena: Arc::new(arena),
        boxes,
        root_id,
    };
    (engine, geo)
}

fn spawn_script(
    html: &str,
    bindings: EventBindings,
    hit_map: SharedHitMap,
) -> (Sender<ScriptMsg>, Receiver<ScriptResponse>, JoinHandle<()>) {
    spawn_script_with_ui(html, bindings, hit_map, Arc::new(SharedUiState::default()))
}

/// Variant publishing a specific viewport / scroll offset so tests can drive
/// the `window.innerWidth` & co getters and `matchMedia` evaluation.
fn spawn_script_with_ui(
    html: &str,
    bindings: EventBindings,
    hit_map: SharedHitMap,
    ui: Arc<SharedUiState>,
) -> (Sender<ScriptMsg>, Receiver<ScriptResponse>, JoinHandle<()>) {
    let mut parser = htui::html::html_parse::Parser::new(html);
    let document = parser.parse_with_scripts();
    let (tx, rx) = mpsc::channel::<ScriptMsg>();
    let (out_tx, out_rx) = mpsc::channel::<ScriptResponse>();
    let handle = std::thread::spawn(move || {
        run_script_thread(
            document,
            bindings,
            Arc::from(""),
            rx,
            out_tx,
            hit_map,
            ui,
            Arc::new(Mutex::new(String::new())),
        )
    });
    (tx, out_rx, handle)
}

/// Lay out `arena` and publish the resulting hit map for the script thread
/// (its region node ids come from the same arena the script thread owns).
fn publish_hit_map(hit_map: &SharedHitMap, arena: &NodeArena, root_id: NodeId) {
    let term_map = TermMap {
        columns: COLS,
        rows: ROWS,
    };
    let containing = BoxConstraint::root(&term_map);
    let mut cursor = Cursor::new();
    let boxes = layout_tree(
        arena,
        root_id,
        containing,
        &mut cursor,
        term_map.columns as usize,
        term_map.rows as usize,
    );
    let geo = Geometry {
        arena: Arc::new(arena.clone()),
        boxes,
        root_id,
    };
    let map = Arc::new(HitMap::build(&geo));
    match hit_map.lock() {
        Ok(mut guard) => *guard = map,
        Err(e) => panic!("hit map lock poisoned: {e}"),
    }
}

fn center(rect: htui::layout::Rect) -> (u32, u32) {
    (
        rect.x as u32 + rect.width.max(1) as u32 / 2,
        rect.y as u32 + rect.height.max(1) as u32 / 2,
    )
}

fn find_node_by_id(arena: &NodeArena, id: &str) -> NodeId {
    arena
        .iter()
        .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id))
        .map(|(nid, _)| nid)
        .expect("element with id exists")
}

fn host_bindings(
    name: &str,
    mut f: impl FnMut(&mut htui::script::HostCtx) -> boa_engine::JsValue + Send + 'static,
) -> EventBindings {
    let mut bindings = EventBindings::default();
    bindings
        .host
        .insert(name.to_string(), Box::new(move |call| Ok(f(call))));
    bindings
}

fn counting_host(counter: Arc<Mutex<usize>>) -> EventBindings {
    host_bindings("tick", move |_| {
        *counter.lock().unwrap() += 1;
        boa_engine::JsValue::undefined()
    })
}

fn eval_str(engine: &mut ScriptEngine, expr: &str) -> String {
    let v = engine
        .eval_source(&format!("String({expr})"))
        .expect("eval must succeed");
    v.as_string()
        .map(|s| s.to_std_string_lossy())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// innerHTML replaced-subtree sweep (arena does not grow under refresh churn)
// ---------------------------------------------------------------------------

#[test]
fn inner_html_churn_does_not_grow_the_arena() {
    let mut engine = ScriptEngine::new(
        r#"<div id="host"><p id="old">stale</p></div>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");

    // Timer-refresh pattern: replace the generated content every tick. The
    // generated nodes are never wrapped by JS, so each sweep frees the
    // previous tick's subtree instead of leaking it into the slotmap.
    for i in 0..5 {
        engine
            .eval_source(&format!(
                "document.getElementById('host').innerHTML = '<b id=\"gen{i}\">{i}</b>'"
            ))
            .expect("innerHTML write evaluates");
    }
    let after_five = engine.arena().len();

    for i in 5..10 {
        engine
            .eval_source(&format!(
                "document.getElementById('host').innerHTML = '<b id=\"gen{i}\">{i}</b>'"
            ))
            .expect("innerHTML write evaluates");
    }
    let arena = engine.arena();

    assert_eq!(
        arena.len(),
        after_five,
        "repeated innerHTML refreshes must not grow the arena"
    );
    assert!(
        !arena
            .iter()
            .any(|(_, n)| n.get_id().is_some_and(|id| id == "old")),
        "the originally replaced subtree is freed"
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('host').textContent"),
        "9",
        "the latest fragment is live and correct"
    );
}

#[test]
fn inner_html_keeps_a_script_held_node_usable() {
    let mut engine = ScriptEngine::new(
        r#"<div id="host"><p id="kid">keepme</p></div>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");

    // JS holds the replaced child (browser: it stays alive as a detached
    // node) — the wrapper cache keeps it in the arena and fully usable.
    engine
        .eval_source(
            "globalThis.kid = document.getElementById('kid'); \
             document.getElementById('host').innerHTML = '';",
        )
        .expect("hold + replace evaluates");

    assert_eq!(eval_str(&mut engine, "kid.tagName"), "P");
    assert_eq!(eval_str(&mut engine, "kid.textContent"), "keepme");

    let arena = engine.arena();
    assert!(
        arena
            .iter()
            .any(|(_, n)| n.get_id().is_some_and(|id| id == "kid")),
        "the held node stays in the arena (kept via its wrapper)"
    );
    assert_eq!(
        eval_str(&mut engine, "kid.childNodes.length"),
        "1",
        "the held node's subtree stays attached to it"
    );
}

// ---------------------------------------------------------------------------
// DOM cycle rejection (browser `HierarchyRequestError` semantics)
// ---------------------------------------------------------------------------

/// Eval `expr` and return `"ok"` on success or `"error:…"` on a JS throw.
fn eval_catch(engine: &mut ScriptEngine, expr: &str) -> String {
    let src = format!("try {{ {expr} ; 'ok' }} catch (e) {{ 'error:' + String(e) }}");
    let v = engine.eval_source(&src).expect("eval runs");
    v.as_string()
        .map(|s| s.to_std_string_lossy())
        .unwrap_or_default()
}

#[test]
fn append_child_rejects_inserting_a_node_into_itself() {
    let mut engine = ScriptEngine::new(
        r#"<div id="outer"><div id="inner"></div></div>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");

    let result = eval_catch(
        &mut engine,
        "document.getElementById('outer').appendChild(document.getElementById('outer'))",
    );
    assert!(
        result.starts_with("error:"),
        "self-append must throw, got: {result}"
    );

    // The failed mutation must leave the arena untouched and acyclic:
    // `contains` walks the ancestor chain and must terminate.
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('outer').contains(document.getElementById('inner'))"
        ),
        "true",
        "the tree shape is unchanged"
    );
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('outer').childNodes.length"
        ),
        "1",
        "the failed append must not push a duplicate child"
    );
    // The engine stays usable after the rejection.
    assert_eq!(
        eval_catch(
            &mut engine,
            "document.getElementById('outer').appendChild(document.createElement('p'))"
        ),
        "ok",
        "a valid append still works after a rejected one"
    );
}

#[test]
fn append_child_rejects_inserting_a_node_into_its_own_descendant() {
    // Appending `outer` under its own child would make an ancestor a
    // descendant — the ancestor walk must reject it before any mutation.
    let mut engine = ScriptEngine::new(
        r#"<div id="outer"><div id="inner"></div></div>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");

    let result = eval_catch(
        &mut engine,
        "document.getElementById('inner').appendChild(document.getElementById('outer'))",
    );
    assert!(
        result.starts_with("error:"),
        "descendant append must throw, got: {result}"
    );
    assert_eq!(
        eval_str(
            &mut engine,
            "document.getElementById('inner').childNodes.length"
        ),
        "0",
        "no child may be attached on the rejected path"
    );
}

#[test]
fn insert_before_and_replace_child_reject_cycles() {
    let mut engine = ScriptEngine::new(
        r#"<div id="outer"><div id="inner"></div></div>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");

    let outer = "document.getElementById('outer')";
    let inner = "document.getElementById('inner')";

    // insertBefore with the node itself as both target and reference.
    assert!(
        eval_catch(
            &mut engine,
            &format!("{outer}.insertBefore({outer}, {inner})")
        )
        .starts_with("error:"),
        "insertBefore(self, …) must throw"
    );
    // insertBefore into an own descendant (reference `null` = append).
    assert!(
        eval_catch(&mut engine, &format!("{inner}.insertBefore({outer}, null)"))
            .starts_with("error:"),
        "insertBefore into an own descendant must throw"
    );
    // replaceChild with the node itself.
    assert!(
        eval_catch(
            &mut engine,
            &format!("{outer}.replaceChild({outer}, {inner})")
        )
        .starts_with("error:"),
        "replaceChild(self, …) must throw"
    );

    // Every rejected path leaves the DOM intact.
    assert_eq!(
        eval_str(&mut engine, &format!("{outer}.childNodes.length")),
        "1",
        "the tree shape survives all three rejections"
    );
    assert_eq!(
        eval_str(&mut engine, &format!("{outer}.contains({inner})")),
        "true",
        "`contains` terminates on the intact tree"
    );
}

// ============================================================
// window view API: viewport getters, resize, matchMedia, scrollTo
// ============================================================

fn empty_hit_map() -> SharedHitMap {
    Arc::new(Mutex::new(Arc::new(HitMap::new())))
}

/// Wait for the next `Event` response (skipping nothing — the script thread
/// only answers messages / timers / fetches).
fn recv_event(out_rx: &Receiver<ScriptResponse>) -> ScriptEvent {
    match out_rx.recv().expect("event must arrive") {
        ScriptResponse::Event(ev) => ev,
        other => panic!("expected Event, got {other:?}"),
    }
}

/// Read the text content of node `id` from a dirty arena snapshot.
fn dirty_text(arena: &NodeArena, id: &str) -> String {
    arena
        .iter()
        .find(|(_, n)| {
            n.node_type == htui::tdom::node::NodeType::Div && n.get_id().is_some_and(|v| v == id)
        })
        .map(|(_, n)| {
            n.children
                .iter()
                .filter_map(|&c| arena[c].data.as_deref())
                .collect::<String>()
        })
        .unwrap_or_default()
}

#[test]
fn viewport_getters_and_match_media_follow_the_shared_ui_state() {
    let ui = Arc::new(SharedUiState::default());
    ui.cols.store(80, Ordering::Relaxed);
    ui.rows.store(24, Ordering::Relaxed);
    ui.scroll_y.store(15, Ordering::Relaxed);
    let html = r#"<body><div id="out">init</div><script>
      globalThis._initial = window.innerWidth + 'x' + window.innerHeight +
        'y' + window.scrollY;
      window.addEventListener('resize', function () {
        document.getElementById('out').textContent =
          'resized:' + window.innerWidth;
      });
      globalThis._mql = window.matchMedia('(max-width: 70ch)');
      globalThis._initialMatches = globalThis._mql.matches;
      globalThis._mql.addEventListener('change', function (e) {
        document.getElementById('out').textContent =
          'change:' + e.matches + ':' + window.scrollY;
      });
    </script></body>"#;
    let _ = env_logger::builder().is_test(true).try_init();
    let (tx, out_rx, handle) =
        spawn_script_with_ui(html, EventBindings::default(), empty_hit_map(), ui.clone());
    match out_rx.recv().expect("ready must arrive") {
        ScriptResponse::Ready { .. } => {}
        other => panic!("expected Ready, got {other:?}"),
    }

    // Same-size resize: the event fires, no matchMedia flip.
    tx.send(ScriptMsg::Resize).expect("send resize");
    let ev = recv_event(&out_rx);
    let d = ev.dirty.expect("resize listener wrote the output node");
    assert_eq!(dirty_text(&d.arena, "out"), "resized:80");

    // Shrink past the query boundary: the change event fires with
    // `matches: true`, and `window.scrollY` still reads the shared state.
    ui.cols.store(60, Ordering::Relaxed);
    ui.rows.store(20, Ordering::Relaxed);
    tx.send(ScriptMsg::Resize).expect("send shrink resize");
    let ev = recv_event(&out_rx);
    let d = ev.dirty.expect("change listener wrote the output node");
    assert_eq!(dirty_text(&d.arena, "out"), "change:true:15");

    tx.send(ScriptMsg::Quit).expect("send quit");
    handle.join().expect("script thread must exit cleanly");
}

#[test]
fn scroll_to_pushes_scroll_markers() {
    let ui = Arc::new(SharedUiState::default());
    ui.scroll_y.store(10, Ordering::Relaxed);
    // Top-level scripts run during engine construction; their pushed render
    // commands ride out with the next message reply.
    let html = r#"<body><script>
      window.scrollBy(0, 30);
      window.scrollTo({ top: 100, behavior: 'smooth' });
    </script></body>"#;
    let (tx, out_rx, handle) =
        spawn_script_with_ui(html, EventBindings::default(), empty_hit_map(), ui.clone());
    match out_rx.recv().expect("ready must arrive") {
        ScriptResponse::Ready { .. } => {}
        other => panic!("expected Ready, got {other:?}"),
    }
    tx.send(ScriptMsg::Resize).expect("send resize");
    let ev = recv_event(&out_rx);
    let has = |cmds: &[htui::tui::graph_api::RenderCmd], want: &str| {
        cmds.iter().any(|c| format!("{c:?}").contains(want))
    };
    // `scrollBy` reads the shared offset (10 + 30) and the object form with
    // `behavior: "smooth"` routes into the animation pipeline.
    assert!(
        has(&ev.cmds, "ScrollSet { to: 40 }"),
        "scrollBy(0, 30) at scrollY 10 must target 40, got {:?}",
        ev.cmds
    );
    assert!(
        has(&ev.cmds, "ScrollAnimate") && has(&ev.cmds, "to: 100"),
        "smooth scrollTo must animate to 100, got {:?}",
        ev.cmds
    );

    tx.send(ScriptMsg::Quit).expect("send quit");
    handle.join().expect("script thread must exit cleanly");
}

#[test]
fn scroll_into_view_pushes_a_marker_with_block_alignment() {
    let html = r#"<body><div id="target">t</div><script>
      document.getElementById('target').scrollIntoView();
      document.getElementById('target').scrollIntoView({ block: 'center', behavior: 'smooth' });
    </script></body>"#;
    let (tx, out_rx, handle) = spawn_script_with_ui(
        html,
        EventBindings::default(),
        empty_hit_map(),
        Arc::new(SharedUiState::default()),
    );
    match out_rx.recv().expect("ready must arrive") {
        ScriptResponse::Ready { .. } => {}
        other => panic!("expected Ready, got {other:?}"),
    }
    // The markers ride out with the next message reply; the run loop (not
    // covered here) resolves them against the hit map.
    tx.send(ScriptMsg::Resize).expect("send resize");
    let ev = recv_event(&out_rx);
    let has = |want: &str| ev.cmds.iter().any(|c| format!("{c:?}").contains(want));
    assert!(
        has("ScrollIntoView") && has("Start"),
        "default block is start, got {:?}",
        ev.cmds
    );
    assert!(
        has("Center") && has("smooth: true"),
        "options object parsed, got {:?}",
        ev.cmds
    );

    tx.send(ScriptMsg::Quit).expect("send quit");
    handle.join().expect("script thread must exit cleanly");
}

#[test]
fn scroll_event_fires_window_and_body_listeners() {
    let html = r#"<body><div id="out">init</div><script>
      window.addEventListener('scroll', function () {
        document.getElementById('out').textContent = 'win-scroll:' + window.scrollY;
      });
      document.body.addEventListener('scroll', function () {
        var el = document.getElementById('out');
        el.textContent = 'body-scroll:' + window.scrollY;
      });
    </script></body>"#;
    let (tx, out_rx, handle) = spawn_script_with_ui(
        html,
        EventBindings::default(),
        empty_hit_map(),
        Arc::new(SharedUiState::default()),
    );
    match out_rx.recv().expect("ready must arrive") {
        ScriptResponse::Ready { .. } => {}
        other => panic!("expected Ready, got {other:?}"),
    }
    tx.send(ScriptMsg::Scroll).expect("send scroll");
    let ev = recv_event(&out_rx);
    let d = ev.dirty.expect("scroll listener wrote the output node");
    // The body listener runs after the window one (the bubbling chain), so
    // its write is the final text; both read the shared scroll offset.
    assert_eq!(dirty_text(&d.arena, "out"), "body-scroll:0");

    tx.send(ScriptMsg::Quit).expect("send quit");
    handle.join().expect("script thread must exit cleanly");
}
