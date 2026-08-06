//! Tests for the script-thread bridge: `apply_message` event semantics and
//! the `run_script_thread` loop, including the core decoupling property that
//! a blocking host function never stalls the input side's channel. Pure
//! in-memory — no terminal required.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use htui::events::{EventBindings, Geometry, HitMap, HoverState};
use htui::layout::layout::TermMap;
use htui::layout::{BoxConstraint, Cursor, layout_tree};
use htui::script::{ScriptEngine, ScriptMsg, ScriptResponse, apply_message, run_script_thread};
use htui::tdom::node::{NodeArena, NodeId};

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
    let ev = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Move { x: ax, y: ay });
    assert_eq!(ev.hovered, Some(a.node_id), "hover must land on `a`");
    assert!(ev.dirty.is_none(), "a plain pointer move mutates nothing");

    let ev = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Move { x: bx, y: by });
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
    assert_eq!(*calls.lock().unwrap(), 1, "click must call the host function");
    assert_eq!(ev.hovered, Some(region.node_id), "click syncs the hover target");
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
    let dirty = ev.dirty.expect("a select value change must request a repaint");
    let committed = dirty
        .arena
        .get(node_id)
        .and_then(|n| n.attributes.get("value"))
        .cloned();
    assert_eq!(committed.as_deref(), Some("b"), "dirty snapshot must carry the new value");
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
    tx.send(ScriptMsg::Quit).expect("send quit while script thread is busy");
    assert!(
        t0.elapsed() < Duration::from_millis(100),
        "sending to the script thread must never block on a busy engine"
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
    let handle = std::thread::spawn(move || {
        run_script_thread(html, bindings, rx, out_tx, hit_map)
    });
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
    mut f: impl FnMut(&mut htui::script::HostCall) -> boa_engine::JsValue + Send + 'static,
) -> EventBindings {
    let mut bindings = EventBindings::default();
    bindings.host.insert(
        name.to_string(),
        Box::new(move |call| Ok(f(call))),
    );
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
