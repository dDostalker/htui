//! Tests for the event system: hit-region collection, hit testing, bubbling
//! dispatch and hover enter / leave. Pure in-memory — no terminal required.

use std::sync::{Arc, Mutex};

use htui::events::eval::{EventContext, eval_handler};
use htui::events::{
    EventBindings, EventType, Geometry, HitMap, HtmlEvent,
    interaction::{HoverState, on_click, on_move},
};
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::layout::{BoxConstraint, Cursor, Rect, layout_tree};
use htui::tdom::node::{NodeArena, NodeId};
use htui::tui::graph_api::RenderCmd;

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

/// Layout the document without painting, mirroring `TermLayout::draw`'s
/// internal pipeline so the test only exercises the event side channel.
fn build_geometry(html: &str) -> Geometry {
    let (root_id, arena) = setup(html);
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
    Geometry { arena, boxes }
}

/// A canvas point inside `rect` (its centre).
fn center(rect: Rect) -> (u32, u32) {
    (
        rect.x as u32 + rect.width.max(1) as u32 / 2,
        rect.y as u32 + rect.height.max(1) as u32 / 2,
    )
}

#[test]
fn interactive_elements_become_hit_regions() {
    let geo = build_geometry(
        r#"<div id="outer" onclick="o"><p id="inner" onclick="i">inner</p><p>plain</p></div>"#,
    );
    let map = HitMap::build(&geo);
    assert_eq!(map.len(), 2, "only elements with on* attributes become regions");
    assert!(map.resolve_by_id("outer").is_some());
    assert!(map.resolve_by_id("inner").is_some());
    assert!(map.resolve_by_id("missing").is_none());
}

#[test]
fn plain_document_produces_no_regions() {
    let geo = build_geometry("<p>no events here</p>");
    let map = HitMap::build(&geo);
    assert!(map.is_empty());
}

#[test]
fn hit_test_prefers_the_deepest_interactive_element() {
    let geo = build_geometry(
        r#"<div id="outer" onclick="o"><p id="inner" onclick="i">inner</p><p>plain</p></div>"#,
    );
    let map = HitMap::build(&geo);
    let inner_rect = map.resolve_by_id("inner").expect("inner region").rect;
    let outer_rect = map.resolve_by_id("outer").expect("outer region").rect;

    // A point inside the inner element resolves to the inner element.
    let (x, y) = center(inner_rect);
    assert_eq!(
        map.hit_test(x, y).expect("inner cell must hit").id(),
        Some("inner")
    );

    // A point inside the outer element but outside the inner one (the plain
    // <p> row) resolves to the outer element.
    let outer_only = (outer_rect.y..outer_rect.y + outer_rect.height)
        .find(|y| *y < inner_rect.y || *y >= inner_rect.y + inner_rect.height)
        .expect("outer must extend beyond inner");
    let hit = map
        .hit_test(outer_rect.x as u32, outer_only as u32)
        .expect("outer cell must hit");
    assert_eq!(hit.id(), Some("outer"));
}

#[test]
fn click_on_empty_space_dispatches_nothing() {
    let geo = build_geometry(r#"<div id="btn" onclick="go">click me</div>"#);
    let map = HitMap::build(&geo);
    let mut bindings = EventBindings::default();
    let mut hover = HoverState::new();
    // Below the document: no region, no event, no commands.
    let cmds = on_click(&mut hover, &map, &mut bindings, 0, 1000);
    assert!(cmds.is_empty());
    assert!(hover.current().is_none());
}

#[test]
fn click_dispatches_the_matching_action() {
    let geo = build_geometry(r#"<div id="btn" onclick="go()">click me</div>"#);
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("btn").expect("region");
    let (x, y) = center(region.rect);

    let mut bindings = EventBindings::default();
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    bindings.actions.insert(
        "go".to_string(),
        Box::new(move |ev: &HtmlEvent| {
            log.lock().unwrap().push(format!(
                "{:?}|{:?}|{}|{}|{}",
                ev.event_type,
                ev.target,
                ev.current_target.unwrap_or("?"),
                ev.action.unwrap_or("?"),
                ev.bubbles
            ));
            None
        }),
    );

    let mut hover = HoverState::new();
    let cmds = on_click(&mut hover, &map, &mut bindings, x, y);

    let log = seen.lock().unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0], "Click|Some(\"btn\")|btn|go|true");
    // A click produces no render command (browser-like: no visual feedback
    // without CSS), only the dispatched event.
    assert!(cmds.is_empty());
}

#[test]
fn click_bubbles_to_an_interactive_ancestor() {
    let geo = build_geometry(
        r#"<div id="wrap" onclick="buy()"><p id="child" onclick="inner()">text</p></div>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("child").expect("child region");
    let (x, y) = center(region.rect);

    // Only "buy" is registered — "inner" has no handler, so the event must
    // bubble up to the ancestor div.
    let mut bindings = EventBindings::default();
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    bindings.actions.insert(
        "buy".to_string(),
        Box::new(move |ev: &HtmlEvent| {
            log.lock().unwrap()
                .push(format!("{:?}|{:?}", ev.target, ev.current_target));
            Some(RenderCmd::Redraw {
                scroll_y: 0,
                zoom: 1.0,
            })
        }),
    );

    let mut hover = HoverState::new();
    let cmds = on_click(&mut hover, &map, &mut bindings, x, y);

    let log = seen.lock().unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0], "Some(\"child\")|Some(\"wrap\")");
    assert_eq!(
        cmds,
        vec![RenderCmd::Redraw {
            scroll_y: 0,
            zoom: 1.0
        }]
    );

    // A second click on the same element produces the same single command.
    drop(log);
    let cmds = on_click(&mut hover, &map, &mut bindings, x, y);
    assert_eq!(
        cmds,
        vec![RenderCmd::Redraw {
            scroll_y: 0,
            zoom: 1.0
        }]
    );
}

#[test]
fn hover_dispatches_enter_and_leave_events_without_visual_highlight() {
    // Like a browser without CSS `:hover` rules, moving the pointer only
    // fires the `onmouseover` / `onmouseout` events — never a highlight.
    let geo = build_geometry(
        r#"<div id="a" onmouseover="va()" onmouseout="la()">a</div><div id="b" onmouseover="vb()" onmouseout="lb()">b</div>"#,
    );
    let map = HitMap::build(&geo);
    let a_rect = map.resolve_by_id("a").expect("a region").rect;
    let b_rect = map.resolve_by_id("b").expect("b region").rect;

    let mut bindings = EventBindings::default();
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    for (action, name) in [("va", "a"), ("vb", "b"), ("la", "a"), ("lb", "b")] {
        let name = name.to_string();
        let log = Arc::clone(&seen);
        bindings.actions.insert(
            action.to_string(),
            Box::new(move |ev: &HtmlEvent| {
                log.lock()
                    .unwrap()
                    .push(format!("{}:{}", ev.action.unwrap_or("?"), name));
                None
            }),
        );
    }

    let mut hover = HoverState::new();
    let (ax, ay) = center(a_rect);
    let (bx, by) = center(b_rect);

    // Enter element a: MouseOver("va") fires; no render command.
    let cmds = on_move(&mut hover, &map, &mut bindings, ax, ay);
    assert!(cmds.is_empty());

    // Move to element b: leave a ("la"), then enter b ("vb"); still no
    // render command.
    let cmds = on_move(&mut hover, &map, &mut bindings, bx, by);
    assert!(cmds.is_empty());

    // Leave all elements: MouseOut("lb") fires and the hover target clears.
    let empty_y = (b_rect.y + b_rect.height + 1) as u32;
    let cmds = on_move(&mut hover, &map, &mut bindings, bx, empty_y);
    assert!(cmds.is_empty());
    assert!(hover.current().is_none());

    let log = seen.lock().unwrap();
    assert_eq!(log.clone(), vec!["va:a", "la:a", "vb:b", "lb:b"]);
}

// ============================================================
// Handler-body evaluator (browser-aligned `on*` semantics)
// ============================================================

fn test_ctx(event_type: EventType, target: Option<&str>) -> EventContext<'_> {
    EventContext {
        event_type,
        target,
        current_target: target,
        path: &[],
        x: 0,
        y: 0,
    }
}

fn counting_binding(counter: Arc<Mutex<usize>>) -> EventBindings {
    let mut bindings = EventBindings::default();
    bindings.actions.insert(
        "buy".to_string(),
        Box::new(move |_| {
            *counter.lock().unwrap() += 1;
            None
        }),
    );
    bindings
}

#[test]
fn bare_identifier_never_invokes_like_the_browser() {
    // `onclick="buy"` compiles to `function (event) { buy }` in a browser:
    // the identifier resolves but is never called. The evaluator must not
    // invoke the registered handler either.
    let seen = Arc::new(Mutex::new(0usize));
    let mut bindings = counting_binding(Arc::clone(&seen));
    let ctx = test_ctx(EventType::Click, Some("btn"));
    assert_eq!(eval_handler("buy", &ctx, &mut bindings), None);
    assert_eq!(*seen.lock().unwrap(), 0);
}

#[test]
fn call_expression_invokes_registered_function() {
    let mut bindings = EventBindings::default();
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    bindings.actions.insert(
        "buy".to_string(),
        Box::new(move |ev: &HtmlEvent| {
            log.lock()
                .unwrap()
                .push(format!("{:?}|{:?}", ev.action, ev.current_target));
            Some(RenderCmd::Quit)
        }),
    );
    let ctx = test_ctx(EventType::Click, Some("btn"));
    assert_eq!(eval_handler("buy()", &ctx, &mut bindings), Some(RenderCmd::Quit));
    assert_eq!(seen.lock().unwrap().clone(), vec!["Some(\"buy\")|Some(\"btn\")"]);
}

#[test]
fn unknown_function_is_skipped_without_error() {
    // Unregistered call → the browser's `ReferenceError` equivalent: warn and
    // continue, never panic.
    let mut bindings = EventBindings::default();
    let ctx = test_ctx(EventType::Click, Some("btn"));
    assert_eq!(eval_handler("nope()", &ctx, &mut bindings), None);
    assert_eq!(eval_handler("nope", &ctx, &mut bindings), None);
}

#[test]
fn parse_errors_are_skipped_quietly() {
    let seen = Arc::new(Mutex::new(0usize));
    let mut bindings = counting_binding(Arc::clone(&seen));
    let ctx = test_ctx(EventType::Click, Some("btn"));
    assert_eq!(eval_handler("buy(", &ctx, &mut bindings), None);
    assert_eq!(eval_handler("buy(1,", &ctx, &mut bindings), None);
    assert_eq!(eval_handler("alert('unterminated", &ctx, &mut bindings), None);
    assert_eq!(eval_handler("1 + 2", &ctx, &mut bindings), None);
    assert_eq!(*seen.lock().unwrap(), 0);
}

#[test]
fn multiple_statements_run_in_order() {
    let seen = Arc::new(Mutex::new(0usize));
    let mut bindings = counting_binding(Arc::clone(&seen));
    let ctx = test_ctx(EventType::Click, Some("btn"));
    assert_eq!(
        eval_handler("console.log('x'); buy(); console.warn('y')", &ctx, &mut bindings),
        None
    );
    assert_eq!(*seen.lock().unwrap(), 1);
}

#[test]
fn call_arguments_parse_but_are_not_forwarded() {
    // The handler signature is fixed to `&HtmlEvent`; arguments are accepted
    // syntactically (so the same body runs in both browser and htui) but not
    // forwarded. Documented v1 limitation.
    let seen = Arc::new(Mutex::new(0usize));
    let mut bindings = counting_binding(Arc::clone(&seen));
    let ctx = test_ctx(EventType::Click, Some("btn"));
    assert_eq!(eval_handler("buy('sku', 42)", &ctx, &mut bindings), None);
    assert_eq!(*seen.lock().unwrap(), 1);
}

#[test]
fn builtin_alert_and_quit_behave_like_functions() {
    let mut bindings = EventBindings::default();
    let ctx = test_ctx(EventType::Click, Some("btn"));
    assert_eq!(eval_handler("alert('hi')", &ctx, &mut bindings), None);
    assert_eq!(eval_handler("console.log('hi')", &ctx, &mut bindings), None);
    assert_eq!(eval_handler("quit()", &ctx, &mut bindings), Some(RenderCmd::Quit));
}
