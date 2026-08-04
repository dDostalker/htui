//! Tests for the event system: hit-region collection, hit testing, bubbling
//! dispatch and hover enter / leave. Pure in-memory — no terminal required.

use std::sync::{Arc, Mutex};

use crossterm::style::Color;
use htui::events::eval::{EventContext, eval_handler};
use htui::events::{
    EventBindings, EventType, Geometry, HitMap, HtmlEvent,
    interaction::{HoverState, on_click, on_move},
};
use htui::html::css_parse::{
    CssSelector, Parser as CssParser, PseudoClass, Selector, css_into_node,
    css_into_node_with_hover, reset_css_attr,
};
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::layout::{BoxConstraint, Cursor, Rect, layout_tree};
use htui::tdom::node::{NodeArena, NodeId};
use htui::tui::graph_api::{RenderCmd, TermLayout};

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
    Geometry {
        arena,
        boxes,
        root_id,
    }
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

#[test]
fn every_element_becomes_a_hit_region() {
    let geo = build_geometry(
        r#"<div id="outer" onclick="o()"><p id="inner" onclick="i()">inner</p><p>plain</p></div>"#,
    );
    let map = HitMap::build(&geo);
    // Elements without on* attributes (the plain <p>) are regions too, so CSS
    // `:hover` rules can target any element.
    assert!(map.iter().any(|r| r.id().is_none()), "id-less elements must be regions");
    assert!(map.resolve_by_id("outer").is_some());
    assert!(map.resolve_by_id("inner").is_some());
    assert!(map.resolve_by_id("missing").is_none());
}

#[test]
fn plain_elements_are_regions_but_dispatch_nothing() {
    // A document with no on* attributes anywhere still produces regions
    // (hover / `:hover` need them), but clicking them dispatches no handlers.
    let geo = build_geometry(r#"<div id="plain"><p>no events here</p></div>"#);
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("plain").expect("plain div is a region");
    let (x, y) = center(region.rect);

    let mut bindings = EventBindings::default();
    let seen = Arc::new(Mutex::new(0usize));
    let counter = Arc::clone(&seen);
    bindings.actions.insert(
        "go".to_string(),
        Box::new(move |_| {
            *counter.lock().unwrap() += 1;
            None
        }),
    );
    let mut hover = HoverState::new();
    let cmds = on_click(&mut hover, &map, &mut bindings, x, y);

    // No handler may fire — no element on the bubbling path has on* attrs.
    assert_eq!(*seen.lock().unwrap(), 0);
    // The click still synchronises the hover target to the deepest element
    // under the pointer (so CSS `:hover` can highlight it), but produces no
    // event result.
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        RenderCmd::Hover { node_id: Some(_) }
    ));
}

#[test]
fn hit_test_prefers_the_deepest_element() {
    let geo = build_geometry(
        r#"<div id="outer" onclick="o()"><p id="inner" onclick="i()">inner</p><p>plain</p></div>"#,
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
    // <p> row) resolves to the plain <p> — the deepest element, not the
    // interactive ancestor.
    let outer_only = (outer_rect.y..outer_rect.y + outer_rect.height)
        .find(|y| *y < inner_rect.y || *y >= inner_rect.y + inner_rect.height)
        .expect("outer must extend beyond inner");
    let hit = map
        .hit_test(outer_rect.x as u32, outer_only as u32)
        .expect("plain p cell must hit");
    assert!(hit.id().is_none(), "deepest element is the id-less plain <p>");
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
    // The first click synchronises the hover state and emits a `Hover`
    // restyle command so CSS `:hover` rules can apply.
    assert_eq!(
        cmds,
        vec![RenderCmd::Hover {
            node_id: Some(region.node_id)
        }]
    );
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
        vec![
            RenderCmd::Hover {
                node_id: Some(region.node_id)
            },
            RenderCmd::Redraw {
                scroll_y: 0,
                zoom: 1.0
            },
        ]
    );

    // A second click on the same element skips the hover sync.
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
fn hover_emits_restyle_command_on_enter_and_leave() {
    let geo = build_geometry(
        r#"<div id="a" onmouseover="va()" onmouseout="la()">a</div><div id="b" onmouseover="vb()" onmouseout="lb()">b</div>"#,
    );
    let map = HitMap::build(&geo);
    let a_region = map.resolve_by_id("a").expect("a region");
    let b_region = map.resolve_by_id("b").expect("b region");
    let a_rect = a_region.rect;
    let b_rect = b_region.rect;

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

    // Enter element a: MouseOver("va") fires, plus a `Hover` restyle command
    // so the render thread can re-cascade for CSS `:hover`.
    let cmds = on_move(&mut hover, &map, &mut bindings, ax, ay);
    assert_eq!(
        cmds,
        vec![RenderCmd::Hover {
            node_id: Some(a_region.node_id)
        }]
    );

    // Move to element b: leave a ("la"), enter b ("vb"), restyle for b.
    let cmds = on_move(&mut hover, &map, &mut bindings, bx, by);
    assert_eq!(
        cmds,
        vec![RenderCmd::Hover {
            node_id: Some(b_region.node_id)
        }]
    );

    // Leave all elements: MouseOut("lb") fires and a clear command follows.
    let empty_y = (b_rect.y + b_rect.height + 1) as u32;
    let cmds = on_move(&mut hover, &map, &mut bindings, bx, empty_y);
    assert_eq!(cmds, vec![RenderCmd::Hover { node_id: None }]);
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

// ============================================================
// CSS `:hover` pseudo-class (browser-style hover highlight)
// ============================================================

#[test]
fn hover_pseudo_class_parses() {
    let sel: CssSelector = "#panel:hover".parse().expect("selector parses");
    assert!(matches!(
        &sel.compounds[0][..],
        [Selector::Id(_), Selector::PseudoClass(PseudoClass::Hover)]
    ));
    assert!("button:hover".parse::<CssSelector>().is_ok());
    assert!(":hover()".parse::<CssSelector>().is_err());
}

#[test]
fn hover_pseudo_class_applies_only_to_the_hovered_node() {
    let (root_id, mut arena) = setup(r#"<div id="a">a</div><div id="b">b</div>"#);
    let rules = CssParser::new("#a:hover { background-color: #112233 }").parse();
    let a_id = find_node_by_id(&arena, "a");
    let b_id = find_node_by_id(&arena, "b");

    // Without a hovered node, `:hover` never matches.
    css_into_node(&mut arena, root_id, &rules);
    assert_eq!(arena[a_id].css_attr.get_bg_color(), None);
    assert_eq!(arena[b_id].css_attr.get_bg_color(), None);

    // With `a` hovered, only `a` picks up the `:hover` background.
    reset_css_attr(&mut arena, root_id);
    css_into_node_with_hover(&mut arena, root_id, &rules, Some(a_id));
    assert_eq!(
        arena[a_id].css_attr.get_bg_color(),
        Some(Color::Rgb {
            r: 0x11,
            g: 0x22,
            b: 0x33
        })
    );
    assert_eq!(arena[b_id].css_attr.get_bg_color(), None);
}

#[test]
fn hover_pseudo_class_matches_ancestors_of_the_hovered_node() {
    // CSS `:hover` matches an element when the pointer is over it *or* one of
    // its descendants: hovering the inner <p> must style the outer #panel.
    let (root_id, mut arena) = setup(r#"<div id="panel"><p id="child">text</p></div>"#);
    let rules = CssParser::new("#panel:hover { background-color: #445566 }").parse();
    let child_id = find_node_by_id(&arena, "child");

    css_into_node_with_hover(&mut arena, root_id, &rules, Some(child_id));
    assert_eq!(
        arena[find_node_by_id(&arena, "panel")].css_attr.get_bg_color(),
        Some(Color::Rgb {
            r: 0x44,
            g: 0x55,
            b: 0x66
        })
    );
}

#[test]
fn hover_cascade_paints_the_highlight_onto_the_canvas() {
    // Mirrors the render thread's `RenderCmd::Hover` path: clone the cascaded
    // arena, re-cascade with the hovered node, repaint, and the `:hover`
    // background must land on the canvas behind the target box.
    let html = r#"<div id="btn" onclick="go()">click me</div>"#;
    let css = "#btn:hover { background-color: #112233 }";
    let (root_id, mut arena) = setup(html);
    let rules = CssParser::new(css).parse();

    css_into_node(&mut arena, root_id, &rules);
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
    let btn_id = find_node_by_id(&arena, "btn");

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let geo = Geometry {
        arena: arena.clone(),
        boxes: boxes.clone(),
        root_id,
    };
    let region = HitMap::build(&geo).resolve_by_id("btn").expect("region").rect;
    let cell = region.y * COLS as usize + region.x;

    // Baseline: no hovered node, so the `:hover` background is absent.
    assert_eq!(tl.layout[cell].bcolor, None);

    // Hover: re-cascade with the hovered node and repaint.
    let mut arena_hover = arena.clone();
    reset_css_attr(&mut arena_hover, root_id);
    css_into_node_with_hover(&mut arena_hover, root_id, &rules, Some(btn_id));
    tl.repaint_from(&arena_hover, &boxes);
    assert_eq!(
        tl.layout[cell].bcolor,
        Some(Color::Rgb {
            r: 0x11,
            g: 0x22,
            b: 0x33
        })
    );

    // Clearing the hover (re-cascade with `None`) removes the highlight.
    let mut arena_clear = arena.clone();
    reset_css_attr(&mut arena_clear, root_id);
    css_into_node_with_hover(&mut arena_clear, root_id, &rules, None);
    tl.repaint_from(&arena_clear, &boxes);
    assert_eq!(tl.layout[cell].bcolor, None);
}

#[test]
fn hover_works_for_elements_without_on_attributes() {
    // Previously only elements with on* attributes were hit regions, so
    // `:hover` rules on plain elements never matched. Now every box is a
    // region: hovering the plain <p> targets it and `#plain:hover` applies.
    let html = r#"<div id="panel"><p id="plain">plain text</p></div>"#;
    let css = "#plain:hover { background-color: #223344 }";
    let (root_id, mut arena) = setup(html);
    let rules = CssParser::new(css).parse();

    css_into_node(&mut arena, root_id, &rules);
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
        arena: arena.clone(),
        boxes: boxes.clone(),
        root_id,
    };

    // The plain <p> (no on* attributes) is a region and hit-testable.
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("plain").expect("plain p is a region");
    let (x, y) = center(region.rect);
    let hovered = map.hit_test(x, y).expect("hover hits the plain p");
    assert_eq!(hovered.node_id, region.node_id);

    // Cascading with that node applies the `:hover` rule to it.
    let mut arena_hover = arena.clone();
    reset_css_attr(&mut arena_hover, root_id);
    css_into_node_with_hover(&mut arena_hover, root_id, &rules, Some(hovered.node_id));
    assert_eq!(
        arena_hover[hovered.node_id].css_attr.get_bg_color(),
        Some(Color::Rgb {
            r: 0x22,
            g: 0x33,
            b: 0x44
        })
    );
}
