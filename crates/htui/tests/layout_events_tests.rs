//! Tests for the script-driven event system: hit-region collection, hit
//! testing, JS listener dispatch (bubbling / stopPropagation), hover enter /
//! leave and host-function bindings. Pure in-memory — no terminal required.

use std::sync::{Arc, Mutex};

use boa_engine::JsValue;
use crossterm::style::Color;
use htui::events::{
    ElementRef, EventBindings, EventType, Geometry, HitMap, HitRegion,
    interaction::{HoverState, on_click, on_move},
};
use htui::html::css_parse::{
    Parser as CssParser, PseudoClass, Selector, css_into_node,
    css_into_node_with_hover, reset_css_attr,
};
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::layout::{BoxConstraint, Cursor, Rect, layout_tree};
use htui::script::ScriptEngine;
use htui::tdom::node::{NodeArena, NodeId};
use htui::tui::graph_api::{InputState, RenderCmd, apply_input_key, handle_click};

use crossterm::event::KeyCode;

// ---------------------------------------------------------------------------
// `#[htui::command]` / `#[derive(IntoJs)]` — tauri-style host bindings.
// ---------------------------------------------------------------------------

/// Command return type: struct → JS object with `#[js_name]` renames.
#[derive(htui::IntoJs)]
struct Greeting {
    name: String,
    #[js_name = "loudName"]
    loud_name: String,
    times: u64,
}

/// No-capture command: args `FromJs`, return `IntoJs`.
#[htui::command]
fn greet(name: String, times: u64) -> Greeting {
    Greeting {
        name: name.clone(),
        loud_name: format!("{name}!!"),
        times,
    }
}

/// Command returning `Result`: `Err` must surface as a JS throw.
#[htui::command]
fn divide(a: f64, b: f64) -> Result<f64, String> {
    if b == 0.0 {
        Err("division by zero".to_string())
    } else {
        Ok(a / b)
    }
}

/// Command argument type: struct read back from a JS object, with
/// `#[js_name]` mapping the JS property name to the Rust field.
#[derive(htui::FromJs)]
struct Threshold {
    #[js_name = "maxCpu"]
    max_cpu: f64,
    label: String,
}

/// Command taking a `#[derive(FromJs)]` struct argument.
#[htui::command]
fn clamp(value: f64, config: Threshold) -> Result<f64, String> {
    if config.max_cpu <= 0.0 {
        return Err(format!("{}: maxCpu must be positive", config.label));
    }
    Ok(value.min(config.max_cpu))
}

// ============================================================
// `<input>` field events (focus / blur / input / change) + `el.value`
// ============================================================

#[test]
fn input_events_dispatch_via_listeners_and_onstar_attributes() {
    // The four input events must fire along the DOM ancestor chain for both
    // `addEventListener` listeners and compiled `on*` attributes.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var seen = [];
          var el = document.getElementById('f');
          el.addEventListener('focus', function () { seen.push('a-focus'); });
          el.addEventListener('blur', function () { seen.push('a-blur'); });
          el.addEventListener('input', function () { seen.push('a-input'); });
          el.addEventListener('change', function () { seen.push('a-change'); });
          globalThis.__seen = seen;
        </script>
        <div onfocus="seen.push('p-focus')" onblur="seen.push('p-blur')">
          <input id="f" value="a" oninput="seen.push('i-input')" onchange="seen.push('i-change')">
        </div>"#,
        EventBindings::default(),
    );
    let arena = engine.arena();
    let input_id = find_node_by_id(&arena, "f");
    let region = region_of(&arena, input_id);

    engine.fire(&region, EventType::Focus, 0, 0);
    engine.fire(&region, EventType::Input, 0, 0);
    engine.fire(&region, EventType::Change, 0, 0);
    engine.fire(&region, EventType::Blur, 0, 0);

    let seen = eval_str(&mut engine, "String(globalThis.__seen.join(','))");
    assert_eq!(
        seen,
        "a-focus,p-focus,i-input,a-input,i-change,a-change,a-blur,p-blur",
        "on* attributes fire before addEventListener on the same element; target before ancestors"
    );
}

#[test]
fn input_value_property_reads_and_writes_the_dom() {
    // `el.value` reads the DOM attribute; writing it marks the node dirty so
    // the next frame re-renders (equal writes short-circuit).
    let (mut engine, _geo) = build_geometry(
        r#"<script>
          var el = document.getElementById('f');
          globalThis.__before = el.value;
          el.value = 'typed';
          globalThis.__after = el.value;
          el.value = 'typed';
        </script>
        <input id="f" value="init">"#,
        EventBindings::default(),
    );
    assert_eq!(eval_str(&mut engine, "globalThis.__before"), "init");
    assert_eq!(eval_str(&mut engine, "globalThis.__after"), "typed");
    let arena = engine.arena();
    let stored = arena
        .get(find_node_by_id(&arena, "f"))
        .and_then(|n| n.attributes.get("value"))
        .cloned();
    assert_eq!(stored.as_deref(), Some("typed"));
}

#[test]
fn click_input_focuses_and_click_elsewhere_blurs() {
    // The input loop's click rule: clicking an `<input>` focuses it, and any
    // other click (blank space here) blurs it — both repainting via
    // `RenderInput` so the field's caret / focus frame disappear.
    let (mut engine, geo) = build_geometry(
        r#"<div id="a"><input id="name" value="Ada"></div><div id="b">blank</div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let input_region = map.resolve_by_id("name").expect("input region");
    let (ix, iy) = center(input_region.rect);

    let mut state = InputState::new();
    let mut cmds = Vec::new();
    handle_click(&mut state, &map, &mut engine, ix, iy, &mut cmds);
    assert_eq!(state.focused_input, Some(input_region.node_id));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            RenderCmd::RenderInput {
                node_id,
                focused: true,
                ..
            } if *node_id == input_region.node_id
        )),
        "clicking the field must queue a focused repaint"
    );

    // Click far below the document (blank space) → the field must blur.
    let mut cmds = Vec::new();
    handle_click(&mut state, &map, &mut engine, 0, 1000, &mut cmds);
    assert!(state.focused_input.is_none(), "click outside must blur");
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            RenderCmd::RenderInput {
                node_id,
                focused: false,
                selection: None,
                ..
            } if *node_id == input_region.node_id
        )),
        "blur must queue an unfocused repaint with the selection cleared"
    );
}

#[test]
fn click_another_input_commits_the_previous_field() {
    // Clicking a second field switches focus: the previous field is repainted
    // unfocused and its edited value is committed into the DOM (single source
    // of truth), while the new field focuses with its own value.
    let (mut engine, geo) = build_geometry(
        r#"<div id="a"><input id="name" value="Ada"></div>
        <div id="b"><input id="city" value="Tokyo"></div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let name = map.resolve_by_id("name").expect("name region");
    let city = map.resolve_by_id("city").expect("city region");
    let (nx, ny) = center(name.rect);
    let (cx, cy) = center(city.rect);

    let mut state = InputState::new();
    let mut cmds = Vec::new();
    handle_click(&mut state, &map, &mut engine, nx, ny, &mut cmds);
    // Type into the focused field.
    assert!(apply_input_key(
        &mut state.input_value,
        &mut state.input_cursor,
        KeyCode::Char('!'),
    ));

    // Click the other field.
    let mut cmds = Vec::new();
    handle_click(&mut state, &map, &mut engine, cx, cy, &mut cmds);
    assert_eq!(state.focused_input, Some(city.node_id));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            RenderCmd::RenderInput {
                node_id,
                focused: false,
                ..
            } if *node_id == name.node_id
        )),
        "the previous field must be repainted unfocused"
    );
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            RenderCmd::RenderInput {
                node_id,
                focused: true,
                ..
            } if *node_id == city.node_id
        )),
        "the new field must be repainted focused"
    );

    // The edited value was committed to the DOM on blur.
    let arena = engine.arena();
    let committed = arena
        .get(name.node_id)
        .and_then(|n| n.attributes.get("value"))
        .cloned();
    assert_eq!(committed.as_deref(), Some("Ada!"));
}

#[test]
fn command_macro_exposes_rust_functions_to_js() {
    let mut bindings = EventBindings::default();
    bindings.command("greet", greet_command());
    bindings.command("divide", divide_command());
    bindings.command("clamp", clamp_command());

    let mut engine = ScriptEngine::new(
        r#"
        <script>
          var g = greet("Ada", 3);
          globalThis.__greetName = g.name;
          globalThis.__greetLoud = g.loudName;
          globalThis.__greetTimes = g.times;
          try {
            globalThis.__bad = divide(1, 0);
          } catch (e) {
            globalThis.__bad = 'threw: ' + e.message;
          }
          globalThis.__good = divide(10, 4);
          globalThis.__clamped = clamp(95, { maxCpu: 80, label: "cpu" });
          globalThis.__clamped2 = clamp(95, { maxCpu: 99, label: "cpu" });
          try {
            globalThis.__badClamp = clamp(95, { maxCpu: -1, label: "cpu" });
          } catch (e) {
            globalThis.__badClamp = 'threw: ' + e.message;
          }
        </script>
        <p>ok</p>
        "#,
        bindings,
    )
    .expect("engine");

    assert_eq!(
        eval_str(&mut engine, "globalThis.__greetName"),
        "Ada",
        "struct field must map to the JS property"
    );
    assert_eq!(
        eval_str(&mut engine, "globalThis.__greetLoud"),
        "Ada!!",
        "#[js_name] must rename the property"
    );
    assert_eq!(eval_str(&mut engine, "globalThis.__greetTimes"), "3");
    assert_eq!(
        eval_str(&mut engine, "globalThis.__bad"),
        "threw: division by zero",
        "Result::Err must throw a catchable JS error"
    );
    assert_eq!(eval_str(&mut engine, "globalThis.__good"), "2.5");
    assert_eq!(
        eval_str(&mut engine, "globalThis.__clamped"),
        "80",
        "#[derive(FromJs)] must read a struct arg, #[js_name] maps maxCpu → max_cpu"
    );
    assert_eq!(
        eval_str(&mut engine, "globalThis.__clamped2"),
        "95",
        "struct arg with a value below the clamp must pass through"
    );
    assert_eq!(
        eval_str(&mut engine, "globalThis.__badClamp"),
        "threw: cpu: maxCpu must be positive",
        "command Err must throw a catchable JS error"
    );
}

/// Evaluate `String(<expr>)` and return the stringified result.
fn eval_str(engine: &mut ScriptEngine, expr: &str) -> String {
    let v = engine
        .eval_source(&format!("String({expr})"))
        .expect("eval must succeed");
    v.as_string()
        .map(|s| s.to_std_string_lossy())
        .unwrap_or_default()
}

const COLS: u16 = 80;
const ROWS: u16 = 24;

fn setup(html: &str) -> (NodeId, NodeArena) {
    let mut parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = parser.parse();
    if !embedded.trim().is_empty() {
        let mut css_parser = css_parse::Parser::new(&embedded);
        let rules = css_parser.parse();
        css_parse::css_into_node(&mut arena, root_id, &rules);
    }
    (root_id, arena)
}

/// Build the engine + geometry for a document.
///
/// Mirrors the runtime path: the engine owns the live arena (scripts run at
/// construction), and the hit map is built from that arena's layout.
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

/// `NodeId` of the element whose `id` attribute equals `id`.
fn find_node_by_id(arena: &NodeArena, id: &str) -> NodeId {
    arena
        .iter()
        .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id))
        .map(|(nid, _)| nid)
        .expect("element with id exists")
}

/// A canvas point inside `rect` (its centre).
fn center(rect: Rect) -> (u32, u32) {
    (
        rect.x as u32 + rect.width.max(1) as u32 / 2,
        rect.y as u32 + rect.height.max(1) as u32 / 2,
    )
}

/// Build bindings with a single host function.
fn host_bindings(
    name: &str,
    mut f: impl FnMut(&mut htui::script::HostCall) -> JsValue + Send + 'static,
) -> EventBindings {
    let mut bindings = EventBindings::default();
    bindings.host.insert(name.to_string(), Box::new(move |call| Ok(f(call))));
    bindings
}

/// A host function that increments a shared counter and returns `undefined`.
fn counting_host(counter: Arc<Mutex<usize>>) -> EventBindings {
    host_bindings("tick", move |_| {
        *counter.lock().unwrap() += 1;
        JsValue::undefined()
    })
}

/// Minimal hit region for `node_id` (no geometry) to fire non-pointer events
/// (focus / blur / input / change) along the DOM ancestor chain.
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
        path,
    }
}

// ============================================================
// Hit regions
// ============================================================

#[test]
fn every_element_becomes_a_hit_region() {
    let (_, geo) = build_geometry(
        r#"<div id="outer"><p id="inner">inner</p><p>plain</p></div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    // Elements without listeners are regions too, so CSS `:hover` rules can
    // target any element.
    assert!(
        map.iter().any(|r| r.id().is_none()),
        "id-less elements must be regions"
    );
    assert!(map.resolve_by_id("outer").is_some());
    assert!(map.resolve_by_id("inner").is_some());
    assert!(map.resolve_by_id("missing").is_none());
}

#[test]
fn hit_test_prefers_the_deepest_element() {
    let (_, geo) = build_geometry(
        r#"<div id="outer"><div id="inner" style="width: 10px">inner</div></div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let inner_rect = map.resolve_by_id("inner").expect("inner region").rect;

    // The centre of the inner element resolves to the inner element (deepest),
    // not its ancestor.
    let (x, y) = center(inner_rect);
    assert_eq!(
        map.hit_test(x, y).expect("inner cell must hit").id(),
        Some("inner")
    );
    // A point outside every element hits nothing.
    assert!(map.hit_test(0, 1000).is_none());
}

#[test]
fn click_on_empty_space_dispatches_nothing() {
    let (mut engine, geo) = build_geometry(r#"<div id="btn">click me</div>"#, EventBindings::default());
    let map = HitMap::build(&geo);
    let mut hover = HoverState::new();
    // Below the document: no region, no event, no commands.
    let cmds = on_click(&mut hover, &map, &mut engine, 0, 1000);
    assert!(cmds.is_empty());
    assert!(hover.current().is_none());
}

// ============================================================
// Attribute handlers (`on*` compiled to `function (event) { … }`)
// ============================================================

#[test]
fn click_dispatches_the_matching_listener() {
    // The attribute handler compiles to `function (event) { go() }`; `go`
    // resolves to the registered host function.
    let (mut engine, geo) = build_geometry(
        r#"<div id="btn" onclick="go()">click me</div>"#,
        host_bindings("go", |_| JsValue::undefined()),
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("btn").expect("region");
    let (x, y) = center(region.rect);

    let mut hover = HoverState::new();
    let cmds = on_click(&mut hover, &map, &mut engine, x, y);
    // The click fires `go` and returns no render command, but hover sync emits
    // a Hover command.
    assert!(cmds.iter().any(|c| matches!(c, RenderCmd::Hover { .. })));
}

#[test]
fn bare_identifier_never_invokes_like_the_browser() {
    // `onclick="go"` compiles to `function (event) { go }`: the identifier
    // resolves but is never called.
    let seen = Arc::new(Mutex::new(0usize));
    let seen2 = Arc::clone(&seen);
    let (mut engine, geo) = build_geometry(
        r#"<div id="btn" onclick="go">click me</div>"#,
        host_bindings("go", move |_| {
            *seen2.lock().unwrap() += 1;
            JsValue::undefined()
        }),
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("btn").expect("region");
    let (x, y) = center(region.rect);
    let mut hover = HoverState::new();
    on_click(&mut hover, &map, &mut engine, x, y);
    assert_eq!(*seen.lock().unwrap(), 0);
}

#[test]
fn click_bubbles_to_an_interactive_ancestor() {
    // Child has no listener, so the click bubbles up to the ancestor's `onclick`.
    let seen = Arc::new(Mutex::new(0usize));
    let seen2 = Arc::clone(&seen);
    let (mut engine, geo) = build_geometry(
        r#"<div id="wrap" onclick="buy()"><p id="child">text</p></div>"#,
        host_bindings("buy", move |_| {
            *seen2.lock().unwrap() += 1;
            JsValue::undefined()
        }),
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("child").expect("child region");
    let (x, y) = center(region.rect);

    let mut hover = HoverState::new();
    on_click(&mut hover, &map, &mut engine, x, y);
    assert_eq!(*seen.lock().unwrap(), 1);
}

// ============================================================
// addEventListener + DOM mutation
// ============================================================

#[test]
fn add_event_listener_registers_a_js_listener() {
    // A `<script>` registers a click listener; clicking the button runs the
    // JS function which calls back into a host function.
    let seen = Arc::new(Mutex::new(0usize));
    let seen2 = Arc::clone(&seen);
    let html = r#"
      <script>
        document.getElementById('btn').addEventListener('click', function () { tick(); });
      </script>
      <div id="btn">click me</div>
    "#;
    let (mut engine, geo) = build_geometry(html, counting_host(seen2));
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("btn").expect("region");
    let (x, y) = center(region.rect);
    let mut hover = HoverState::new();
    on_click(&mut hover, &map, &mut engine, x, y);
    assert_eq!(*seen.lock().unwrap(), 1);
}

#[test]
fn js_can_mutate_the_dom_and_marks_dirty() {
    let html = r#"
      <script>
        document.getElementById('out').textContent = 'hello world';
      </script>
      <p id="out">before</p>
    "#;
    let mut engine = ScriptEngine::new(html, EventBindings::default()).expect("engine");
    assert!(
        !engine.take_dirty().is_empty(),
        "script textContent mutation must mark dirty"
    );
    let arena = engine.arena();
    let node = find_node_by_id(&arena, "out");
    let text: String = arena[node]
        .children
        .iter()
        .filter_map(|&c| match &arena[c].node_type {
            htui::tdom::node::NodeType::Text(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "hello world");
}

#[test]
fn stop_propagation_stops_the_ancestor_listener() {
    // Child stops propagation, so the ancestor listener never fires.
    let hits: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let hits2 = Arc::clone(&hits);
    let html = r#"
      <script>
        document.getElementById('child')
          .addEventListener('click', function (e) { report('child'); e.stopPropagation(); });
        document.getElementById('wrap')
          .addEventListener('click', function () { report('wrap'); });
      </script>
      <div id="wrap"><p id="child">text</p></div>
    "#;
    let (mut engine, geo) = build_geometry(
        html,
        host_bindings("report", move |call| {
            if let Some(s) = call.args.first().and_then(|v| v.as_string()) {
                hits2.lock().unwrap().push(s.to_std_string_lossy());
            }
            JsValue::undefined()
        }),
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("child").expect("child region");
    let (x, y) = center(region.rect);
    let mut hover = HoverState::new();
    on_click(&mut hover, &map, &mut engine, x, y);

    assert_eq!(*hits.lock().unwrap(), vec!["child".to_string()]);
}

// ============================================================
// Hover
// ============================================================

#[test]
fn hover_emits_restyle_command_on_enter_and_leave() {
    let (mut engine, geo) = build_geometry(
        r#"<div id="a">a</div><div id="b">b</div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let a_region = map.resolve_by_id("a").expect("a region");
    let b_region = map.resolve_by_id("b").expect("b region");
    let (ax, ay) = center(a_region.rect);
    let (bx, by) = center(b_region.rect);

    let mut hover = HoverState::new();
    let cmds1 = on_move(&mut hover, &map, &mut engine, ax, ay);
    assert!(cmds1.iter().any(|c| matches!(
        c,
        RenderCmd::Hover { node_id: Some(n) } if *n == a_region.node_id
    )));
    let cmds2 = on_move(&mut hover, &map, &mut engine, bx, by);
    // Leaving a must fire mouseout + hover with b's node.
    assert!(cmds2.iter().any(|c| matches!(
        c,
        RenderCmd::Hover { node_id: Some(n) } if *n == b_region.node_id
    )));
}

#[test]
fn hover_fires_mouseover_and_mouseout_listeners() {
    let hits: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let hits2 = Arc::clone(&hits);
    let html = r#"
      <script>
        document.getElementById('a')
          .addEventListener('mouseover', function () { report('enter'); });
        document.getElementById('a')
          .addEventListener('mouseout', function () { report('leave'); });
      </script>
      <div id="a">a</div>
    "#;
    let (mut engine, geo) = build_geometry(
        html,
        host_bindings("report", move |call| {
            if let Some(s) = call.args.first().and_then(|v| v.as_string()) {
                hits2.lock().unwrap().push(s.to_std_string_lossy());
            }
            JsValue::undefined()
        }),
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("a").expect("a region");
    let (x, y) = center(region.rect);

    let mut hover = HoverState::new();
    on_move(&mut hover, &map, &mut engine, x, y); // enter
    // Move far away to leave.
    on_move(&mut hover, &map, &mut engine, 0, 1000);

    assert_eq!(*hits.lock().unwrap(), vec!["enter".to_string(), "leave".to_string()]);
}

// ============================================================
// Auto-refresh (invoke_action)
// ============================================================

#[test]
fn invoke_action_calls_a_js_global_function() {
    let sampled: Arc<Mutex<f64>> = Arc::new(Mutex::new(0.0));
    let sampled2 = Arc::clone(&sampled);
    let html = r#"
      <script>
        function refresh() { report(42); }
      </script>
      <p>monitor</p>
    "#;
    let mut engine = ScriptEngine::new(
        html,
        host_bindings("report", move |call| {
            if let Some(n) = call.args.first().and_then(|v| v.as_number()) {
                *sampled2.lock().unwrap() = n;
            }
            JsValue::undefined()
        }),
    )
    .expect("engine");
    engine.invoke_action("refresh");
    assert_eq!(*sampled.lock().unwrap(), 42.0);
}

#[test]
fn unknown_action_is_ignored_without_error() {
    let mut engine =
        ScriptEngine::new("<p>x</p>", EventBindings::default()).expect("engine");
    // Must not panic, must not produce render commands.
    assert!(engine.invoke_action("nope").is_empty());
}

#[test]
fn visual_test_events_document_initializes() {
    // Smoke test: the events demo page's `<script>` block must initialize in
    // the engine (functions defined, listeners registered) without error.
    let html = include_str!("../../../visual_test/src/events/index.html");
    let engine = ScriptEngine::new(html, EventBindings::default()).expect("engine");
    let arena = engine.arena();
    let ids: Vec<&str> = arena
        .iter()
        .filter_map(|(_, n)| n.attributes.get("id").map(String::as_str))
        .collect();
    assert!(
        ids.contains(&"btn-add"),
        "demo element btn-add must exist; got {ids:?}"
    );
}

#[test]
fn visual_test_sysmon_document_initializes() {
    // Smoke test: sysmon page initializes; its script references the host
    // `sample` function which is only defined at runtime, so init must not
    // fail on the unresolved reference (the refresh action resolves it later).
    let html = include_str!("../../../visual_test/src/sysmon/index.html");
    let mut bindings = EventBindings::default();
    bindings.host.insert(
        "sample".to_string(),
        Box::new(|_| Ok(JsValue::undefined())),
    );
    let engine = ScriptEngine::new(html, bindings).expect("engine");
    let arena = engine.arena();
    assert!(
        arena
            .iter()
            .any(|(_, n)| n.attributes.get("id").map(String::as_str) == Some("cpuBar")),
        "sysmon cpuBar cell must exist"
    );
    assert!(
        arena
            .iter()
            .any(|(_, n)| n.attributes.get("id").map(String::as_str) == Some("disks")),
        "sysmon disks cell must exist"
    );
}

/// Text content of the first text child of `id="target"`.
fn text_of(arena: &NodeArena, id: &str) -> String {
    let node = find_node_by_id(arena, id);
    arena[node]
        .children
        .iter()
        .filter_map(|&c| match &arena[c].node_type {
            htui::tdom::node::NodeType::Text(s) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn sysmon_page_refresh_populates_all_cards() {
    // End-to-end over the real demo page: the host `sample()` returns the full
    // field set and the page's own `refresh()` writes every card. Any typo in
    // the page's element ids fails here.
    let html = include_str!("../../../visual_test/src/sysmon/index.html");
    let mut bindings = EventBindings::default();
    bindings.host.insert(
        "sample".to_string(),
        Box::new(|call: &mut htui::script::HostCall| {
            let mut init = boa_engine::object::ObjectInitializer::new(call.context);
            init.property(boa_engine::js_string!("host"), boa_engine::js_string!("testhost"), boa_engine::property::Attribute::all())
                .property(boa_engine::js_string!("os"), boa_engine::js_string!("TestOS"), boa_engine::property::Attribute::all())
                .property(boa_engine::js_string!("cores"), boa_engine::js_string!("8"), boa_engine::property::Attribute::all())
                .property(boa_engine::js_string!("cpuBar"), boa_engine::js_string!("[########------]"), boa_engine::property::Attribute::all())
                .property(boa_engine::js_string!("cpuPct"), boa_engine::js_string!("45%"), boa_engine::property::Attribute::all())
                .property(boa_engine::js_string!("cpuName"), boa_engine::js_string!("Test CPU"), boa_engine::property::Attribute::all())
                .property(boa_engine::js_string!("memBar"), boa_engine::js_string!("[####----------]"), boa_engine::property::Attribute::all())
                .property(boa_engine::js_string!("memText"), boa_engine::js_string!("8.0 / 16.0 GB"), boa_engine::property::Attribute::all())
                .property(boa_engine::js_string!("disks"), boa_engine::js_string!("C: [####] 10.0 / 100.0 GB\nD: [--] 5.0 / 50.0 GB"), boa_engine::property::Attribute::all())
                .property(boa_engine::js_string!("proc"), boa_engine::js_string!("42"), boa_engine::property::Attribute::all())
                .property(boa_engine::js_string!("now"), boa_engine::js_string!("2026-01-01 00:00:00"), boa_engine::property::Attribute::all())
                .property(boa_engine::js_string!("session"), boa_engine::js_string!("00:01"), boa_engine::property::Attribute::all());
            Ok(init.build().into())
        }),
    );
    let mut engine = ScriptEngine::new(html, bindings).expect("engine");
    engine.invoke_action("refresh");
    assert!(
        !engine.take_dirty().is_empty(),
        "refresh must mark the DOM dirty"
    );

    let arena = engine.arena();
    assert_eq!(text_of(&arena, "meta"), "testhost · TestOS · cores 8");
    assert_eq!(text_of(&arena, "cpuBar"), "[########------]");
    assert_eq!(text_of(&arena, "cpuPct"), "45%");
    assert_eq!(text_of(&arena, "cpuName"), "Test CPU");
    assert_eq!(text_of(&arena, "memBar"), "[####----------]");
    assert_eq!(text_of(&arena, "memText"), "8.0 / 16.0 GB");
    assert_eq!(text_of(&arena, "proc"), "processes: 42");
    assert_eq!(text_of(&arena, "now"), "2026-01-01 00:00:00 · session 00:01");
}

#[test]
fn sysmon_refresh_pipeline_updates_the_dom() {
    // Full demo pipeline: JS `refresh()` calls the host `sample()` (returns a
    // JS object) and writes textContent into the DOM, marking the engine dirty
    // so the input loop would emit `RenderArena`.
    let html = r#"
      <script>
        function refresh() {
          var d = sample();
          document.getElementById('cpu').textContent = d.cpu;
        }
      </script>
      <p id="cpu">n/a</p>
    "#;
    let mut bindings = EventBindings::default();
    bindings.host.insert(
        "sample".to_string(),
        Box::new(|call: &mut htui::script::HostCall| {
            let obj = boa_engine::object::ObjectInitializer::new(call.context)
                .property(
                    boa_engine::js_string!("cpu"),
                    boa_engine::js_string!("23%"),
                    boa_engine::property::Attribute::all(),
                )
                .build();
            Ok(obj.into())
        }),
    );
    let mut engine = ScriptEngine::new(html, bindings).expect("engine");

    // Before refresh: initial script didn't call sample, so cpu is untouched.
    assert!(engine.take_dirty().is_empty());
    engine.invoke_action("refresh");
    assert!(!engine.take_dirty().is_empty(), "refresh must mark the DOM dirty");

    let arena = engine.arena();
    let cpu = find_node_by_id(&arena, "cpu");
    let text: String = arena[cpu]
        .children
        .iter()
        .filter_map(|&c| match &arena[c].node_type {
            htui::tdom::node::NodeType::Text(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "23%");
}

#[test]
fn sysmon_refresh_renders_the_progress_bar() {
    // End-to-end over the interactive path: JS `refresh()` calls the host
    // `sample()` and writes a progress-bar string into `<p id=cpu>`, then the
    // engine's arena is laid out the way the render thread does it. The bar
    // glyphs (fill + track) must appear in the canvas.
    let html = r#"
      <script>
        function refresh() {
          var d = sample();
          document.getElementById('cpu').textContent = d.cpu;
        }
      </script>
      <p id="cpu">n/a</p>
    "#;
    let mut bindings = EventBindings::default();
    bindings.host.insert(
        "sample".to_string(),
        Box::new(|call: &mut htui::script::HostCall| {
            Ok(boa_engine::object::ObjectInitializer::new(call.context)
                .property(
                    boa_engine::js_string!("cpu"),
                    boa_engine::js_string!("[########--------] 45%"),
                    boa_engine::property::Attribute::all(),
                )
                .build()
                .into())
        }),
    );
    let mut engine = ScriptEngine::new(html, bindings).expect("engine");
    engine.invoke_action("refresh");
    assert!(
        !engine.take_dirty().is_empty(),
        "refresh must mark the DOM dirty"
    );

    let arena = engine.arena();
    let root_id = engine.root_id();
    let map = TermMap {
        columns: COLS,
        rows: ROWS,
    };
    let mut tl = htui::tui::graph_api::TermLayout::new(&map, 1.0);
    tl.draw(&arena, root_id, &map).unwrap();
    let frame = tl.render_to_bytes();
    let text = String::from_utf8_lossy(&frame);
    assert!(text.contains("#"), "progress-bar fill must render");
    assert!(text.contains("-"), "progress-bar track must render");
    assert!(text.contains("45%"), "percentage must render");
}

// ============================================================
// CSS `:hover` (unchanged side-channel behaviour)
// ============================================================

#[test]
fn hover_pseudo_class_parses() {
    let mut css_parser = CssParser::new("#panel:hover { background-color: #3a3a3a; }");
    let rules = css_parser.parse();
    let (selector, _) = rules.0.iter().next().expect("one rule");
    // `#panel:hover` → one compound `[Id, PseudoClass]`, no combinators.
    assert!(selector.combinators.is_empty());
    assert!(selector.compounds.iter().any(|c| c.iter().any(
        |s| matches!(s, Selector::PseudoClass(PseudoClass::Hover))
    )));
}

#[test]
fn hover_pseudo_class_applies_only_to_the_hovered_node() {
    let (root_id, mut arena) = setup(
        r#"<div id="a" class="box">a</div><div id="b" class="box">b</div>"#,
    );
    let mut css_parser = CssParser::new(".box:hover { background-color: #00ff00; }");
    let rules = css_parser.parse();
    let a = find_node_by_id(&arena, "a");
    let b = find_node_by_id(&arena, "b");

    reset_css_attr(&mut arena, root_id);
    css_into_node_with_hover(&mut arena, root_id, &rules, Some(a));
    assert_eq!(arena[a].css_attr.get_bg_color(), Some(Color::Rgb { r: 0, g: 255, b: 0 }));
    assert_eq!(arena[b].css_attr.get_bg_color(), None);
}

#[test]
fn hover_pseudo_class_matches_ancestors_of_the_hovered_node() {
    let (root_id, mut arena) = setup(r#"<div id="panel"><p id="inner">x</p></div>"#);
    let mut css_parser = CssParser::new("#panel:hover { background-color: #3a3a3a; }");
    let rules = css_parser.parse();
    let inner = find_node_by_id(&arena, "inner");

    reset_css_attr(&mut arena, root_id);
    css_into_node_with_hover(&mut arena, root_id, &rules, Some(inner));
    let panel = find_node_by_id(&arena, "panel");
    assert_eq!(
        arena[panel].css_attr.get_bg_color(),
        Some(Color::Rgb { r: 58, g: 58, b: 58 })
    );
}

#[test]
fn hover_cascade_paints_the_highlight_onto_the_canvas() {
    let html = r#"<div id="panel" class="box">panel</div>"#;
    let mut css_parser = CssParser::new(".box:hover { background-color: #00ff00; }");
    let rules = css_parser.parse();

    let (root_id, mut arena) = setup(html);
    reset_css_attr(&mut arena, root_id);
    css_into_node(&mut arena, root_id, &rules);
    let hovered = find_node_by_id(&arena, "panel");
    assert_eq!(arena[hovered].css_attr.get_bg_color(), None);

    // Re-cascade with the hovered node: the rule now applies.
    reset_css_attr(&mut arena, root_id);
    css_into_node_with_hover(&mut arena, root_id, &rules, Some(hovered));
    assert_eq!(
        arena[hovered].css_attr.get_bg_color(),
        Some(Color::Rgb { r: 0, g: 255, b: 0 })
    );
}
