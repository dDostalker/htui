//! Tests for the script-thread bridge: `apply_message` event semantics and
//! the `run_script_thread` loop, including the core decoupling property that
//! a blocking host function never stalls the input side's channel. Pure
//! in-memory — no terminal required.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use htui::events::{EventBindings, Geometry, HitMap, HoverState, KeyEventInfo};
use htui::layout::layout::TermMap;
use htui::layout::{BoxConstraint, Cursor, layout_tree};
use htui::script::{ScriptEngine, ScriptMsg, ScriptResponse, apply_message, run_script_thread};
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
fn host_render_html_reloads_in_engine_instead_of_render_thread() {
    // A host function that pushes `RenderHtml` must rebuild the document in
    // the engine (the single source of truth): the fresh arena + a dirty root
    // reach the render thread as a `RenderArena`, never as an independent
    // re-parse on the render side (the former dual-source bug).
    let bindings = host_bindings("rebuild", move |call| {
        call.render().push(RenderCmd::RenderHtml {
            html: Arc::from(r#"<div id="fresh">hello</div>"#),
        });
        boa_engine::JsValue::undefined()
    });
    let mut engine = ScriptEngine::new(r#"<div id="old">bye</div>"#, bindings).expect("engine");
    find_node_by_id(&engine.arena(), "old"); // panics if absent

    let cmds = engine.invoke_action("rebuild");
    assert!(
        cmds.iter()
            .all(|c| !matches!(c, RenderCmd::RenderHtml { .. })),
        "RenderHtml must be consumed in-engine, not forwarded to the render thread"
    );
    find_node_by_id(&engine.arena(), "fresh"); // panics if absent — must be live in the engine
    assert!(
        engine
            .arena()
            .iter()
            .all(|(_, n)| n.attributes.get("id").map(String::as_str) != Some("old")),
        "the old document must be gone"
    );
    assert!(
        !engine.take_dirty().is_empty(),
        "reload must mark the root dirty so the response carries the fresh arena"
    );

    // The host map survives the reload: a second call works again.
    let cmds2 = engine.invoke_action("rebuild");
    assert!(cmds2.is_empty(), "second reload also consumes the command");
    assert!(
        !engine.take_dirty().is_empty(),
        "each reload marks its root dirty"
    );
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
            value: "new".to_string(),
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
            value: "new".to_string(),
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
            value: "b".to_string(),
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
    let hit_map: Arc<Mutex<Arc<HitMap>>> = Arc::new(Mutex::new(Arc::new(HitMap::new())));

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
    let hit_map: Arc<Mutex<Arc<HitMap>>> = Arc::new(Mutex::new(Arc::new(HitMap::new())));

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
    htui::resources::register("mem://hello", b"hi from memory".to_vec())
        .expect("register mem resource");
    let mut engine = ScriptEngine::new(
        r#"<script>
          fetch('mem://hello').then(function (res) {
            return res.text();
          }).then(function (t) {
            document.getElementById('out').textContent = t;
          });
        </script>
        <div id="out"></div>"#,
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
    htui::resources::unregister("mem://hello");
}

#[test]
fn fetch_mem_missing_rejects_promise() {
    // Fetching an unregistered `mem://` key rejects the promise; the rejection
    // handler (microtask) must run.
    let mut engine = ScriptEngine::new(
        r#"<script>
          globalThis.__rejected = 'no';
          fetch('mem://nope').then(function () {
            globalThis.__rejected = 'resolved';
          }, function (err) {
            globalThis.__rejected = 'rejected:' + String(err);
          });
        </script>"#,
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
}

#[test]
fn fetch_mem_resource_resolves_through_poll() {
    // A `mem://` fetch resolves synchronously inside `poll_fetches` (the
    // registry is in-memory), but the `.then` microtask chain still runs
    // through the job queue.
    let key = format!("mem://fetch/file_{}.txt", std::process::id());
    htui::resources::register(&key, b"file body".to_vec()).expect("register mem file");

    let html = format!(
        r#"<script>
          fetch('{key}').then(function (res) {{ return res.text(); }})
            .then(function (t) {{ document.getElementById('out').textContent = t; }});
        </script>
        <div id="out"></div>"#
    );
    let mut engine =
        ScriptEngine::new(&html, EventBindings::default()).expect("engine initializes");

    // A single drain settles the promise; run the microtask queue to land the
    // text in the DOM.
    engine.poll_fetches();
    engine.poll_fetches();

    assert_eq!(
        eval_str(&mut engine, "document.getElementById('out').textContent"),
        "file body",
        "mem fetch must resolve through poll_fetches"
    );
    htui::resources::unregister(&key);
}

#[test]
fn fetch_relative_url_resolves_against_document_base() {
    // JS `fetch()` resolves relative URLs against the document base URL
    // (`<base href>`) the same way `<img src>` / `<script src>` do.
    let pid = std::process::id();
    let key = format!("mem://api/data_{pid}.json");
    htui::resources::register(&key, br#"{"ok":true}"#.to_vec()).expect("register mem file");

    let html = format!(
        r#"<base href="mem://api/">
        <script>
          fetch('data_{pid}.json').then(function (res) {{ return res.text(); }})
            .then(function (t) {{ document.getElementById('out').textContent = t; }});
        </script>
        <div id="out"></div>"#
    );
    let mut engine =
        ScriptEngine::new(&html, EventBindings::default()).expect("engine initializes");
    engine.poll_fetches();
    engine.poll_fetches();

    assert_eq!(
        eval_str(&mut engine, "document.getElementById('out').textContent"),
        r#"{"ok":true}"#,
        "relative fetch must resolve against <base href>"
    );
    htui::resources::unregister(&key);
}

#[test]
fn base_uri_reflects_document_base_href() {
    // `Node.baseURI` reports the document base URL established by `<base>`;
    // without one it is an empty string.
    let pid = std::process::id();
    let html = format!(r#"<base href="mem://site/{pid}/"><div id="x"></div>"#);
    let mut engine =
        ScriptEngine::new(&html, EventBindings::default()).expect("engine initializes");
    let expected_uri = format!("mem://site/{pid}/");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('x').baseURI"),
        expected_uri
    );

    let mut no_base = ScriptEngine::new(r#"<div id="y"></div>"#, EventBindings::default())
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
    let engine = ScriptEngine::new(html, bindings).expect("engine initializes");
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
        arena,
        boxes,
        root_id,
    };
    (engine, geo)
}

fn spawn_script(
    html: &str,
    bindings: EventBindings,
    hit_map: Arc<Mutex<Arc<HitMap>>>,
) -> (Sender<ScriptMsg>, Receiver<ScriptResponse>, JoinHandle<()>) {
    let html: Arc<str> = Arc::from(html);
    let (tx, rx) = mpsc::channel::<ScriptMsg>();
    let (out_tx, out_rx) = mpsc::channel::<ScriptResponse>();
    let handle = std::thread::spawn(move || run_script_thread(html, bindings, rx, out_tx, hit_map));
    (tx, out_rx, handle)
}

/// Lay out `arena` and publish the resulting hit map for the script thread
/// (its region node ids come from the same arena the script thread owns).
fn publish_hit_map(hit_map: &Arc<Mutex<Arc<HitMap>>>, arena: &NodeArena, root_id: NodeId) {
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
        arena: arena.clone(),
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
