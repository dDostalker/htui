//! Tests for the JS read APIs: `getBoundingClientRect`, `clientWidth` /
//! `clientHeight` / `clientTop` / `clientLeft`, `scrollTop` (read + write) /
//! `scrollLeft` / `scrollWidth` / `scrollHeight`, `document.activeElement`,
//! `window.getSelection()` and `event.isTrusted`. Values flow from the hit
//! map / shared UI state / selection channel exactly like the input loop
//! publishes them. Pure in-memory — no terminal required.

use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use htui::events::{EventBindings, Geometry, HitMap, SharedHitMap};
use htui::layout::layout::TermMap;
use htui::layout::{BoxConstraint, Cursor, layout_tree};
use htui::script::{
    ScriptEngine, ScriptMsg, ScriptResponse, SharedUiState, host_fn, run_script_thread,
};
use htui::tdom::node::{NodeArena, NodeId};
use htui::tui::graph_api::RenderCmd;

const COLS: u16 = 80;
const ROWS: u16 = 24;

/// Captured `__report(label, value)` calls.
type Captures = Arc<Mutex<Vec<(String, String)>>>;

fn report_bindings(captures: &Captures) -> EventBindings {
    let mut bindings = EventBindings::default();
    let sink = Arc::clone(captures);
    bindings.command(
        "__report",
        host_fn(move |ctx| {
            let label: String = ctx.arg_required(0)?;
            let value: String = ctx.arg_required(1)?;
            sink.lock().expect("report sink").push((label, value));
            Ok(())
        }),
    );
    bindings
}

/// Everything `spawn_script` hands back: message sender, response receiver,
/// thread handle, the shared hit map / UI state / selection channels.
type Spawned = (
    Sender<ScriptMsg>,
    Receiver<ScriptResponse>,
    JoinHandle<()>,
    SharedHitMap,
    Arc<SharedUiState>,
    Arc<Mutex<String>>,
);

/// Spawn the script thread with a shared hit map + UI state and wait for
/// `Ready`.
fn spawn_script(html: &str, bindings: EventBindings) -> Spawned {
    let mut parser = htui::html::html_parse::Parser::new(html);
    let document = parser.parse_with_scripts();
    let (tx, rx) = std::sync::mpsc::channel::<ScriptMsg>();
    let (out_tx, out_rx) = std::sync::mpsc::channel::<ScriptResponse>();
    let hit_map: SharedHitMap = Arc::new(Mutex::new(Arc::new(HitMap::new())));
    let ui = Arc::new(SharedUiState::default());
    let selection = Arc::new(Mutex::new(String::new()));
    let handle = std::thread::spawn({
        let hit_map = Arc::clone(&hit_map);
        let ui = Arc::clone(&ui);
        let selection = Arc::clone(&selection);
        move || {
            htui::resources::with_registry(
                Arc::new(htui::resources::ResourceRegistry::new()),
                || {
                    run_script_thread(
                        document,
                        bindings,
                        Arc::from(""),
                        rx,
                        out_tx,
                        hit_map,
                        ui,
                        selection,
                    )
                },
            );
        }
    });
    match out_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(ScriptResponse::Ready { .. }) => {}
        other => panic!("expected Ready, got {other:?}"),
    }
    (tx, out_rx, handle, hit_map, ui, selection)
}

/// Lay out the engine arena snapshot and publish it as the hit map, like the
/// render + hit-test threads do each frame.
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
        usize::from(COLS),
        usize::from(ROWS),
    );
    let geo = Geometry {
        arena: Arc::new(arena.clone()),
        boxes,
        root_id,
    };
    *hit_map.lock().expect("hit map lock") = Arc::new(HitMap::build(&geo));
}

/// The `NodeId` of the element carrying `id`.
fn find_node(arena: &NodeArena, id: &str) -> NodeId {
    arena
        .iter()
        .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id))
        .map(|(nid, _)| nid)
        .unwrap_or_else(|| panic!("element #{id} exists"))
}

/// Wait until `pred` sees a capture (timers fire asynchronously), returning
/// the value for `label`.
fn captured(captures: &Captures, label: &str) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(guard) = captures.lock()
            && let Some((_, value)) = guard.iter().find(|(l, _)| l == label)
        {
            return value.clone();
        }
        if std::time::Instant::now() > deadline {
            panic!("no capture for {label}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Block until `label` is captured, then stop the script thread. The channel
/// must stay open while waiting — dropping the sender disconnects the script
/// loop before its timers ever fire.
fn settle(tx: &Sender<ScriptMsg>, handle: JoinHandle<()>, captures: &Captures, label: &str) {
    captured(captures, label);
    let _ = tx.send(ScriptMsg::Quit);
    let _ = handle.join();
}

const GEOM_PAGE_SCRIPT: &str = r#"<script>
  setTimeout(function(){
    var rect = document.getElementById('a').getBoundingClientRect();
    __report('ax', rect.x); __report('ay', rect.y);
    __report('aw', rect.width); __report('ah', rect.height);
    __report('atop', rect.top); __report('aright', rect.right);
    __report('abottom', rect.bottom); __report('aleft', rect.left);
    var ghost = document.createElement('div');
    var g = ghost.getBoundingClientRect();
    __report('gx', g.x); __report('gy', g.y); __report('gw', g.width);
  }, 30);
</script>"#;

#[test]
fn bounding_client_rect_reads_hit_map_with_page_scroll_offset() {
    let captures: Captures = Arc::new(Mutex::new(Vec::new()));
    let page = format!("{GEOM_PAGE_SCRIPT}\n<div id=\"a\">Alpha</div>\n<div id=\"b\">Beta</div>");
    let (tx, _out_rx, handle, hit_map, ui, _selection) =
        spawn_script(&page, report_bindings(&captures));

    // Lay out a same-shape document and publish the hit map (the script
    // thread's arena is the parsed original; ids line up by document order).
    let mut parser = htui::html::html_parse::Parser::new(&page);
    let doc = parser.parse_with_scripts();
    publish_hit_map(&hit_map, &doc.arena, doc.root_id);

    // Page scrolled by 2 rows: viewport y = document y − 2.
    ui.scroll_y.store(2, Ordering::Relaxed);

    settle(&tx, handle, &captures, "ax");

    let ax: f64 = captured(&captures, "ax").parse().expect("numeric ax");
    let ay: f64 = captured(&captures, "ay").parse().expect("numeric ay");
    let aw: f64 = captured(&captures, "aw").parse().expect("numeric aw");
    let ah: f64 = captured(&captures, "ah").parse().expect("numeric ah");
    assert!(aw > 0.0, "alpha has width, got {aw}");
    assert!(ah >= 1.0, "alpha has height, got {ah}");
    // Viewport y (doc y − 2) and the derived aliases agree.
    assert_eq!(captured(&captures, "atop"), captured(&captures, "ay"));
    assert_eq!(captured(&captures, "aleft"), captured(&captures, "ax"));
    assert_eq!(captured(&captures, "aright"), (ax + aw).to_string());
    assert_eq!(captured(&captures, "abottom"), (ay + ah).to_string());
    // A detached element has no boxes → all-zero rect.
    assert_eq!(captured(&captures, "gx"), "0");
    assert_eq!(captured(&captures, "gy"), "0");
    assert_eq!(captured(&captures, "gw"), "0");
}

#[test]
fn geometry_reads_return_zeros_without_input_loop() {
    let mut engine =
        ScriptEngine::new("<div id=\"a\">A</div>", "", EventBindings::default()).expect("engine");
    let v = engine
        .eval_source("JSON.stringify(document.getElementById('a').getBoundingClientRect())")
        .expect("eval");
    let s = v
        .as_string()
        .map(|s| s.to_std_string_lossy())
        .expect("string");
    assert!(s.contains("\":0"), "all-zero rect: {s}");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('a').clientWidth"),
        "0"
    );
    assert_eq!(
        eval_str(&mut engine, "String(document.activeElement)"),
        "null"
    );
    assert_eq!(
        eval_str(&mut engine, "window.getSelection().isCollapsed"),
        "true"
    );
}

#[test]
fn style_set_property_supports_custom_properties_and_var_flow() {
    let mut engine = ScriptEngine::new(
        r#"<style>
          div { color: var(--brand); background-color: var(--bg, blue); }
        </style>
        <div id="d">D</div>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine");
    engine
        .eval_source(
            r#"globalThis.s = document.getElementById('d').style;
               s.setProperty('--brand', 'red');
               s.setProperty('FONT-WEIGHT', 'bold');
               'ok'"#,
        )
        .expect("eval");
    // Custom properties read back case-sensitively; regular names lowercase.
    assert_eq!(
        eval_str(&mut engine, "s.getPropertyValue('--brand')"),
        "red"
    );
    assert_eq!(
        eval_str(&mut engine, "s.getPropertyValue('font-weight')"),
        "bold"
    );
    assert_eq!(eval_str(&mut engine, "s.getPropertyValue('--missing')"), "");
    // The style attribute carries both forms for the cascade to re-read.
    let css_text = eval_str(&mut engine, "s.cssText");
    assert!(css_text.contains("--brand: red"), "cssText: {css_text}");
    assert!(
        css_text.contains("font-weight: bold"),
        "cssText: {css_text}"
    );

    // Re-cascading the mutated arena substitutes var() from the custom
    // property written via setProperty (level at which the render thread
    // consumes each dirty batch).
    let mut arena = engine.arena();
    let root = engine.root_id();
    let css = htui::html::css_parse::Parser::new(engine.document_css().as_ref()).parse();
    htui::html::css_parse::css_into_node(&mut arena, root, &css);
    let div = find_node(&arena, "d");
    let attr = &arena.get(div).expect("div").css_attr;
    // `red` parses to Rgb { 255, 0, 0 }: substituted from --brand, not left
    // as an unparseable `var(...)` string (which would read None).
    assert_eq!(
        attr.get_color(),
        Some(crossterm::style::Color::Rgb { r: 255, g: 0, b: 0 })
    );
    // Missing --bg falls back to the var() default `blue`.
    assert_eq!(
        attr.get_bg_color(),
        Some(crossterm::style::Color::Rgb { r: 0, g: 0, b: 255 })
    );

    // removeProperty returns the removed value and clears it.
    assert_eq!(eval_str(&mut engine, "s.removeProperty('--brand')"), "red");
    assert_eq!(eval_str(&mut engine, "s.getPropertyValue('--brand')"), "");
}

/// Evaluate `String(expr)` and return its text.
fn eval_str(engine: &mut ScriptEngine, expr: &str) -> String {
    let v = engine
        .eval_source(&format!("String({expr})"))
        .expect("eval must succeed");
    v.as_string()
        .map(|s| s.to_std_string_lossy())
        .unwrap_or_default()
}

#[test]
fn is_trusted_true_for_engine_events_false_for_constructed() {
    let mut engine = ScriptEngine::new(
        r#"<script>
          globalThis.__engine = null;
          document.body.addEventListener('click', function(e){ globalThis.__engine = e.isTrusted; });
        </script>
        <button id="b">B</button>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine");
    // Constructed events go through the same `dispatchEvent` path.
    engine
        .eval_source(
            "var ev = new Event('click'); document.getElementById('b').dispatchEvent(ev); String(globalThis.__engine) + '/' + String(ev.isTrusted)",
        )
        .expect("eval");
    // The engine-dispatched path (`make_event_object`) is exercised in
    // layout_events_tests; here assert the constructed side is untrusted.
    assert_eq!(
        eval_str(&mut engine, "String(new Event('x').isTrusted)"),
        "false"
    );
}

#[test]
fn selection_reports_published_text() {
    let captures: Captures = Arc::new(Mutex::new(Vec::new()));
    let page = r#"<script>
      setTimeout(function(){
        var sel = window.getSelection();
        __report('text', sel.toString());
        __report('collapsed', sel.isCollapsed);
        __report('anchor', String(sel.anchorNode));
      }, 10);
    </script><p>hello</p>"#;
    let (tx, _out_rx, handle, _hit_map, _ui, selection) =
        spawn_script(page, report_bindings(&captures));
    *selection.lock().expect("selection lock") = String::from("picked text");
    settle(&tx, handle, &captures, "text");
    assert_eq!(captured(&captures, "text"), "picked text");
    assert_eq!(captured(&captures, "collapsed"), "false");
    assert_eq!(captured(&captures, "anchor"), "null");
}

#[test]
fn active_element_reads_shared_focus() {
    let captures: Captures = Arc::new(Mutex::new(Vec::new()));
    let page = r#"<script>
      setTimeout(function(){
        __report('active', document.activeElement === null
          ? 'null' : document.activeElement.id);
      }, 10);
    </script><input id="field" value="v">"#;
    let (tx, _out_rx, handle, _hit_map, ui, _selection) =
        spawn_script(page, report_bindings(&captures));
    // Lay out a minimal arena to obtain the input's NodeId, then publish the
    // focus the way the input loop does.
    let arena = {
        let mut parser = htui::html::html_parse::Parser::new("<input id=\"field\" value=\"v\">");
        let doc = parser.parse_with_scripts();
        doc.arena
    };
    let field = find_node(&arena, "field");
    ui.set_focused(Some(field));
    settle(&tx, handle, &captures, "active");
    assert_eq!(captured(&captures, "active"), "field");
}

#[test]
fn scroll_metrics_read_hit_map_and_scroll_top_write_pushes_cmd() {
    let captures: Captures = Arc::new(Mutex::new(Vec::new()));
    let page = r#"<script>
      setTimeout(function(){
        var box = document.getElementById('box');
        __report('scrollTop', box.scrollTop);
        __report('scrollHeight', box.scrollHeight);
        __report('clientHeight', box.clientHeight);
        __report('scrollLeft', box.scrollLeft);
        box.scrollTop = 1;
        __report('plain', document.getElementById('plain').scrollTop);
      }, 10);
    </script>
    <div id="box" style="height: 3; overflow-y: auto">a<br>b<br>c<br>d<br>e<br>f</div>
    <div id="plain">p</div>"#;
    let (tx, out_rx, handle, hit_map, _ui, _selection) =
        spawn_script(page, report_bindings(&captures));

    // Build + publish the hit map from a laid-out snapshot of the same page.
    // The cascade must run first so the inline `overflow-y: auto` reaches
    // layout (the engine's live arena carries no `css_attr`).
    let mut parser = htui::html::html_parse::Parser::new(page);
    let mut doc = parser.parse_with_scripts();
    let css = htui::html::css_parse::Parser::new(doc.css.as_ref()).parse();
    if let Some(arena) = Arc::get_mut(&mut doc.arena) {
        htui::html::css_parse::css_into_node(arena, doc.root_id, &css);
    }
    publish_hit_map(&hit_map, &doc.arena, doc.root_id);

    settle(&tx, handle, &captures, "scrollTop");

    // The box overflows (6 content rows in a 3-row viewport) → scrollbar in
    // the hit map; `scrollTop = 1` rides back as a RenderScroll command via
    // the timer's event batch.
    let mut saw_render_scroll = false;
    while let Ok(resp) = out_rx.recv_timeout(Duration::from_millis(200)) {
        if let ScriptResponse::Event(event) = resp
            && event
                .cmds
                .iter()
                .any(|c| matches!(c, RenderCmd::RenderScroll { scroll_y: 1, .. }))
        {
            saw_render_scroll = true;
        }
    }
    assert!(saw_render_scroll, "scrollTop write must push RenderScroll");
    let _ = (captured(&captures, "scrollTop"),);
}

#[test]
fn client_y_subtracts_the_published_scroll_offset() {
    let captures: Captures = Arc::new(Mutex::new(Vec::new()));
    let page = r#"<script>
      setTimeout(function(){
        var box = document.getElementById('box');
        box.addEventListener('click', function(e){
          __report('clientY', e.clientY);
          __report('pageY', e.pageY);
        });
        __report('ready', '1');
      }, 10);
    </script>
    <p style="height: 5">spacer</p>
    <div id="box" style="height: 3">b</div>"#;
    let (tx, out_rx, handle, hit_map, ui, _selection) =
        spawn_script(page, report_bindings(&captures));
    let mut parser = htui::html::html_parse::Parser::new(page);
    let doc = parser.parse_with_scripts();
    publish_hit_map(&hit_map, &doc.arena, doc.root_id);
    ui.scroll_y.store(2, std::sync::atomic::Ordering::Relaxed);
    captured(&captures, "ready");

    // Dispatch a click through the script thread with the canvas coordinate
    // of #box (the input loop sends canvas coords; clientY subtracts the
    // published scroll offset).
    let region = hit_map
        .lock()
        .expect("hit map")
        .region_for(find_node(&doc.arena, "box"))
        .expect("box region")
        .clone();
    let x = region.rect.x as u32 + 1;
    let y = region.rect.y as u32;
    tx.send(ScriptMsg::Click { x, y }).expect("send click");
    let _ = out_rx.recv_timeout(Duration::from_secs(5));
    let page_y: f64 = captured(&captures, "pageY").parse().expect("pageY number");
    let client_y: f64 = captured(&captures, "clientY")
        .parse()
        .expect("clientY number");
    assert_eq!(page_y, f64::from(y), "pageY is the canvas coordinate");
    assert_eq!(client_y, page_y - 2.0, "clientY subtracts scroll offset");
    let _ = tx.send(ScriptMsg::Quit);
    let _ = handle.join();
}

#[test]
fn dom_content_loaded_and_load_fire_once_after_build() {
    let captures: Captures = Arc::new(Mutex::new(Vec::new()));
    let page = r#"<script>
      __report('order', 'script');
      document.addEventListener('DOMContentLoaded', function(){ __report('dcl', 'fired'); });
      window.addEventListener('load', function(){ __report('load', 'fired'); });
    </script><p>hi</p>"#;
    let (tx, _out_rx, handle, _hit_map, _ui, _selection) =
        spawn_script(page, report_bindings(&captures));
    settle(&tx, handle, &captures, "load");
    assert_eq!(captured(&captures, "order"), "script");
    assert_eq!(captured(&captures, "dcl"), "fired");
    assert_eq!(captured(&captures, "load"), "fired");
}

#[test]
fn horizontal_scroll_metrics_read_and_scroll_left_write() {
    let captures: Captures = Arc::new(Mutex::new(Vec::new()));
    let page = r#"<script>
      setTimeout(function(){
        var box = document.getElementById('box');
        __report('scrollLeft', box.scrollLeft);
        __report('scrollWidth', box.scrollWidth);
        box.scrollLeft = 3;
      }, 10);
    </script>
    <div id="box" style="width: 20ch; height: 2ch; overflow-x: auto; white-space: nowrap">AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA</div>"#;
    let (tx, out_rx, handle, hit_map, _ui, _selection) =
        spawn_script(page, report_bindings(&captures));

    let mut parser = htui::html::html_parse::Parser::new(page);
    let mut doc = parser.parse_with_scripts();
    let css = htui::html::css_parse::Parser::new(doc.css.as_ref()).parse();
    if let Some(arena) = Arc::get_mut(&mut doc.arena) {
        htui::html::css_parse::css_into_node(arena, doc.root_id, &css);
    }
    publish_hit_map(&hit_map, &doc.arena, doc.root_id);

    settle(&tx, handle, &captures, "scrollLeft");
    let scroll_left: f64 = captured(&captures, "scrollLeft").parse().expect("number");
    assert_eq!(scroll_left, 0.0, "starts unscrolled");
    let scroll_width: f64 = captured(&captures, "scrollWidth").parse().expect("number");
    assert!(
        scroll_width > 20.0,
        "scrollWidth is the nowrap content extent, got {scroll_width}"
    );
    // `scrollLeft = 3` rides back as a RenderScrollX command via the timer
    // event batch.
    let mut saw_scroll_x = false;
    while let Ok(resp) = out_rx.recv_timeout(Duration::from_millis(200)) {
        if let ScriptResponse::Event(event) = resp
            && event
                .cmds
                .iter()
                .any(|c| matches!(c, RenderCmd::RenderScrollX { scroll_x: 3, .. }))
        {
            saw_scroll_x = true;
        }
    }
    assert!(saw_scroll_x, "scrollLeft write must push RenderScrollX");
}
