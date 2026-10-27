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
    DynamicState, Parser as CssParser, PseudoClass, Selector, css_into_node,
    css_into_node_with_hover, reset_css_attr,
};
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::layout::{BoxConstraint, Cursor, Rect, layout_tree};
use htui::script::{ScriptEngine, ScriptMsg, apply_message};
use htui::tdom::node::{Node, NodeArena, NodeId, NodeType};
use htui::tui::graph_api::{
    DirtyInfo, DrainedBatch, InputState, OverlayKind, RenderCmd, SelectOpenState, SelectionMode,
    TermLayout, apply_input_key, handle_click, handle_mouse_down, handle_mouse_move,
    handle_mouse_up,
};

use crossterm::event::{KeyCode, MouseButton};

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
    let (mut engine, _geo) = build_geometry(
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
        seen, "a-focus,p-focus,a-input,i-input,a-change,i-change,a-blur,p-blur",
        "addEventListener listeners fire before the element's on* attribute \
         handler (DOM inner-invoke order); target before ancestors"
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
    let cmds = click(&mut state, &mut engine, &map, ix, iy);
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
    let cmds = click(&mut state, &mut engine, &map, 0, 1000);
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
fn click_on_inline_anchor_resolves_to_the_anchor() {
    // An inline `<a>` owns no LayoutBox — its text folds into the parent
    // block's inline runs. Its runs still carry the anchor's node id (same
    // mechanism as `<label>`), so the hit map gives the link text its own
    // region: the click target is the anchor itself, and listeners on the
    // anchor fire.
    let (mut engine, geo) = build_geometry(
        r#"<p id="p">before <a id="link" href="https://www.baidu.com/s" onclick="globalThis.__clicked = (globalThis.__clicked || 0) + 1">Baidu</a> after</p>"#,
        EventBindings::default(),
    );
    let arena = engine.arena();
    let anchor = find_node_by_id(&arena, "link");

    let map = HitMap::build(&geo);
    assert!(
        map.iter().any(|r| r.node_id == anchor),
        "the inline anchor must own a hit region"
    );

    let region = map.resolve_by_id("link").expect("anchor region by id");
    let (x, y) = center(region.rect);

    // hit_test at the link text must resolve to the anchor, not the block.
    let hit = map.hit_test(x, y).expect("hit under the link text");
    assert_eq!(hit.node_id, anchor, "deepest hit must be the anchor");

    // Full input-loop path: the click dispatches a Click event whose target
    // is the anchor, so its listener runs.
    let mut state = InputState::new();
    click(&mut state, &mut engine, &map, x, y);
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__clicked || 0)"),
        "1",
        "clicking the link text must fire the anchor's onclick"
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
    click(&mut state, &mut engine, &map, nx, ny);
    // Type into the focused field.
    assert!(apply_input_key(
        &mut state.input_value,
        &mut state.input_cursor,
        KeyCode::Char('!'),
    ));

    // Click the other field.
    let cmds = click(&mut state, &mut engine, &map, cx, cy);
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
fn mouse_down_on_input_focuses_and_positions_caret() {
    // Pressing inside a field focuses it (blurring any previous field) and
    // enters drag-selection mode with the caret at the press point.
    let (mut engine, geo) = build_geometry(
        r#"<div><input id="name" value="hello" width="20"></div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("name").expect("input region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    let cmds = mouse_down(&mut state, &mut engine, &map, x, y, MouseButton::Left);
    assert!(
        state.drag_selecting,
        "press on a field starts drag-selection mode"
    );
    assert_eq!(state.focused_input, Some(region.node_id));
    assert_eq!(state.input_sel_anchor, Some(state.input_cursor));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            RenderCmd::RenderInput {
                node_id,
                focused: true,
                ..
            } if *node_id == region.node_id
        )),
        "the field must be repainted focused with the caret in place"
    );
}

#[test]
fn drag_on_input_selects_a_character_range() {
    // Press at the first content column (caret at char 0), drag right two
    // columns: the selection (0, 2) must be carried by the repaint command.
    let (mut engine, geo) = build_geometry(
        r#"<div><input id="name" value="hello" width="20"></div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("name").expect("input region");
    let rx = region.rect.x as u32;
    let ry = region.rect.y as u32;

    let mut state = InputState::new();
    mouse_down(&mut state, &mut engine, &map, rx, ry, MouseButton::Left);
    assert_eq!(state.input_cursor, 0, "press on the first column → char 0");

    // The input loop tracks the press position; a plain move is a drag.
    state.pressed = Some((0, 0));
    let cmds = mouse_move(&mut state, &mut engine, &map, rx + 2, ry);
    assert_eq!(state.input_cursor, 2, "drag to the third column → char 2");
    let sel = cmds.iter().find_map(|c| match c {
        RenderCmd::RenderInput {
            selection: Some(s), ..
        } => Some(*s),
        _ => None,
    });
    assert_eq!(sel, Some((0, 2)), "drag must select chars 0..2");
}

#[test]
fn drag_on_page_text_emits_text_selection() {
    // A press alone never creates a page text selection (browsers select only
    // on a real drag, so a jittery click leaves no residue); once the drag
    // passes the threshold, dragging updates its endpoint and queues a
    // `TextSelection` repaint.
    let (mut engine, geo) = build_geometry(
        r#"<div id="a">some text</div><div id="b">more text</div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);

    let mut state = InputState::new();
    mouse_down(&mut state, &mut engine, &map, 0, 0, MouseButton::Left);
    assert_eq!(
        state.text_sel_anchor, None,
        "a press alone defers the selection start"
    );
    assert_eq!(state.text_sel_end, None);
    assert_eq!(state.text_sel_mode, SelectionMode::Flow);

    state.pressed = Some((0, 0));
    let cmds = mouse_move(&mut state, &mut engine, &map, 4, 0);
    assert_eq!(
        state.text_sel_anchor,
        Some((0, 0)),
        "the anchor is the press point"
    );
    assert_eq!(state.text_sel_end, Some((4, 0)));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            RenderCmd::TextSelection {
                anchor: Some((0, 0)),
                end: Some((4, 0)),
                mode: SelectionMode::Flow
            }
        )),
        "a left-drag over page text must queue a flow TextSelection repaint"
    );
}

#[test]
fn pressing_elsewhere_replaces_the_page_selection() {
    // A second press clears the previous selection (rendering it away); the
    // new selection is anchored only once its drag begins — the render thread
    // needs the explicit clear.
    let (mut engine, geo) =
        build_geometry(r#"<div id="a">some text</div>"#, EventBindings::default());
    let map = HitMap::build(&geo);

    let mut state = InputState::new();
    mouse_down(&mut state, &mut engine, &map, 5, 5, MouseButton::Left);
    state.pressed = Some((0, 0));
    mouse_move(&mut state, &mut engine, &map, 8, 5);
    assert!(state.text_sel_anchor.is_some(), "a selection now exists");

    let cmds = mouse_down(&mut state, &mut engine, &map, 2, 2, MouseButton::Left);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            RenderCmd::TextSelection {
                anchor: None,
                end: None,
                ..
            }
        )),
        "the previous selection must be cleared on a new press"
    );
    assert_eq!(
        state.text_sel_anchor, None,
        "the new selection is deferred until its drag begins"
    );
}

#[test]
fn right_drag_emits_a_block_selection() {
    // Right-drag is the rectangle (block) selection: the command must carry
    // `SelectionMode::Block` so the render thread highlights a rectangle.
    // Like the left flow selection it only starts once the drag begins.
    let (mut engine, geo) = build_geometry(
        r#"<div id="a">some text</div><div id="b">more text</div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);

    let mut state = InputState::new();
    mouse_down(&mut state, &mut engine, &map, 2, 0, MouseButton::Right);
    assert_eq!(state.text_sel_mode, SelectionMode::Block);
    assert_eq!(
        state.text_sel_anchor, None,
        "a press alone defers the selection start"
    );

    state.pressed = Some((2, 0));
    let cmds = mouse_move(&mut state, &mut engine, &map, 5, 2);
    assert_eq!(
        state.text_sel_anchor,
        Some((2, 0)),
        "the anchor is the press point"
    );
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            RenderCmd::TextSelection {
                anchor: Some((2, 0)),
                end: Some((5, 2)),
                mode: SelectionMode::Block
            }
        )),
        "a right-drag must queue a block TextSelection repaint"
    );
    assert!(
        !state.drag_selecting,
        "right-drag never enters the field-internal drag mode"
    );
}

#[test]
fn jittery_click_on_a_button_leaves_no_selection() {
    // A button press with a one-cell wiggle (a sloppy but common click) must
    // not leave a highlighted character behind: the selection is deferred
    // until the drag clears the threshold, so no `TextSelection` is ever
    // queued and the anchor stays clear through the release.
    let (mut engine, geo) = build_geometry(
        r#"<div id="btn">▶ button text</div><div id="next">other</div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let btn = map.resolve_by_id("btn").expect("btn region");
    let (bx, by) = center(btn.rect);

    let mut state = InputState::new();
    mouse_down(&mut state, &mut engine, &map, bx, by, MouseButton::Left);
    assert_eq!(state.text_sel_anchor, None, "press alone: no selection yet");

    // The input loop tracks the press position; the pointer wiggles one cell
    // right while the button is held (still a click, not a drag).
    state.pressed = Some((bx as u16, by as u16));
    let cmds = mouse_move(&mut state, &mut engine, &map, bx + 1, by);
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, RenderCmd::TextSelection { .. })),
        "a sub-threshold wiggle must not queue a TextSelection"
    );
    assert_eq!(state.text_sel_anchor, None);

    // Release on the jittered cell: still no selection residue.
    mouse_up(&mut state, &mut engine, &map, (bx + 1) as u16, by as u16);
    assert_eq!(state.text_sel_anchor, None, "no residue after release");
    assert_eq!(state.text_sel_end, None);
}

#[test]
fn disabled_flow_copy_blocks_left_drag_selection() {
    // With `copy_flow` off, a left-drag over page content must never start a
    // selection: the press does not arm the flow mode and the drag queues no
    // `TextSelection` (so there is nothing to copy on `Ctrl+C`).
    let (mut engine, geo) = build_geometry(
        r#"<div id="a">some text</div><div id="b">more text</div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);

    let mut state = InputState::new();
    state.copy_flow = false;
    mouse_down(&mut state, &mut engine, &map, 0, 0, MouseButton::Left);
    assert_eq!(state.text_sel_anchor, None);
    assert_eq!(state.text_sel_end, None);

    state.pressed = Some((0, 0));
    let cmds = mouse_move(&mut state, &mut engine, &map, 4, 0);
    assert_eq!(
        state.text_sel_anchor, None,
        "disabled flow copy must never start a selection"
    );
    assert_eq!(state.text_sel_end, None);
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, RenderCmd::TextSelection { .. })),
        "a disabled flow copy must queue no TextSelection"
    );
}

#[test]
fn disabled_block_copy_blocks_right_drag_selection() {
    // With `copy_block` off, a right-drag must never start a selection — and
    // in particular must not fall back to the stale `Flow` mode from the
    // default state, which would otherwise highlight a wrong-shaped selection
    // on a disabled feature.
    let (mut engine, geo) = build_geometry(
        r#"<div id="a">some text</div><div id="b">more text</div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);

    let mut state = InputState::new();
    state.copy_block = false;
    mouse_down(&mut state, &mut engine, &map, 2, 0, MouseButton::Right);
    assert_eq!(state.text_sel_anchor, None);

    state.pressed = Some((2, 0));
    let cmds = mouse_move(&mut state, &mut engine, &map, 5, 2);
    assert_eq!(
        state.text_sel_anchor, None,
        "disabled block copy must never start a selection"
    );
    assert_eq!(state.text_sel_end, None);
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, RenderCmd::TextSelection { .. })),
        "a disabled block copy must queue no TextSelection"
    );
}

#[test]
fn arena_behind_a_pending_document_merges_without_losing_css() {
    // A reload reply can be followed by another script reply before the
    // render thread drains its channel. The newer arena must merge into the
    // document snapshot; replacing it would lose the new document CSS source.
    let mut id_arena = NodeArena::default();
    let first = id_arena.insert(Node::from(NodeType::Document));
    let second = id_arena.insert(Node::from(NodeType::Document));
    let mut batch = DrainedBatch::new();
    batch.absorb(RenderCmd::RenderDocument {
        arena: Arc::new(NodeArena::default()),
        root_id: first,
        document_css: Arc::from("p { color: red; }"),
        scroll_y: 0,
        zoom: 1.0,
        dirty: DirtyInfo {
            nodes: vec![first],
            ..DirtyInfo::default()
        },
    });

    let fresh_arena = Arc::new(NodeArena::default());
    batch.absorb(RenderCmd::RenderArena {
        arena: Arc::clone(&fresh_arena),
        root_id: second,
        scroll_y: 7,
        zoom: 2.0,
        dynamic: DynamicState::default(),
        dirty: DirtyInfo {
            nodes: vec![second],
            ..DirtyInfo::default()
        },
    });

    match batch.take_snapshot() {
        Some(RenderCmd::RenderDocument {
            arena,
            root_id,
            document_css,
            scroll_y,
            zoom,
            dirty,
        }) => {
            assert!(Arc::ptr_eq(&arena, &fresh_arena));
            assert_eq!(root_id, second);
            assert_eq!(&*document_css, "p { color: red; }");
            assert_eq!((scroll_y, zoom), (7, 2.0));
            assert!(dirty.nodes.contains(&first));
            assert!(dirty.nodes.contains(&second));
        }
        other => panic!("expected the document snapshot to survive, got {other:?}"),
    }
}

#[test]
fn reset_document_state_clears_replaced_document_interaction() {
    // Whole-document reload invalidates every input-side node id and canvas
    // coordinate. Scroll / zoom survive, but focus, edits, drags, page
    // selection and an open dropdown must not leak into the fresh arena.
    let mut id_arena = NodeArena::default();
    let focused = id_arena.insert(Node::from(NodeType::Input));
    let dropdown = id_arena.insert(Node::from(NodeType::Select));

    let mut state = InputState::new();
    state.pressed = Some((3, 4));
    state.focused_input = Some(focused);
    state.focused_is_textarea = true;
    state.input_value = "editing".into();
    state.input_cursor = 3;
    state.input_sel_anchor = Some(0);
    state.drag_selecting = true;
    state.text_sel_anchor = Some((0, 0));
    state.text_sel_end = Some((5, 1));
    state.open_select = Some(SelectOpenState {
        node_id: dropdown,
        kind: OverlayKind::Select,
        rect: Rect::new(0, 0, 4, 2),
        options: Vec::new(),
        rows: Vec::new(),
        selected: 0,
        highlight: 0,
    });

    let mut cmds = Vec::new();
    state.reset_document_state(&mut cmds);

    assert_eq!(state.pressed, None);
    assert_eq!(state.focused_input, None);
    assert!(!state.focused_is_textarea);
    assert_eq!(state.input_value, "");
    assert_eq!(state.input_cursor, 0);
    assert_eq!(state.input_sel_anchor, None);
    assert!(!state.drag_selecting);
    assert_eq!(state.text_sel_anchor, None);
    assert_eq!(state.text_sel_end, None);
    assert!(state.open_select.is_none());
    assert!(state.resize_pending);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, RenderCmd::SelectClose { node_id } if *node_id == dropdown))
    );
}

#[test]
fn focus_behind_a_pending_arena_merges_into_the_arena_dynamic() {
    // The render thread used to let a `Focus` queued behind a pending
    // `RenderArena` supersede it (dropping the DOM mutations the snapshot
    // carries), while the input-side pre-filter dropped the `Focus` instead
    // (freezing the `:focus` highlight). The drain now merges the focus into
    // the arena's interaction state, so one pass paints both.
    let new_focus = NodeId::default();
    let mut batch = DrainedBatch::new();
    batch.absorb(RenderCmd::RenderArena {
        arena: Arc::new(NodeArena::default()),
        root_id: NodeId::default(),
        scroll_y: 0,
        zoom: 1.0,
        dynamic: DynamicState {
            hovered: Some(NodeId::default()),
            focused: None,
        },
        dirty: DirtyInfo::default(),
    });
    batch.absorb(RenderCmd::Focus {
        node_id: Some(new_focus),
    });
    match batch.take_snapshot() {
        Some(RenderCmd::RenderArena { dynamic, .. }) => {
            assert_eq!(
                dynamic.focused,
                Some(new_focus),
                "the focus delta must merge into the arena's dynamic state"
            );
        }
        other => panic!("expected the arena snapshot to survive, got {other:?}"),
    }
    assert!(batch.take_hover().is_none(), "no hover was queued");
}

#[test]
fn hover_behind_a_pending_arena_is_deferred_not_dropped() {
    // A newer `Hover` behind a pending `RenderArena` must not supersede the
    // DOM mutation; the drain defers it so the freshest highlight is applied
    // on top of the arena render.
    let mut batch = DrainedBatch::new();
    batch.absorb(RenderCmd::RenderArena {
        arena: Arc::new(NodeArena::default()),
        root_id: NodeId::default(),
        scroll_y: 0,
        zoom: 1.0,
        dynamic: DynamicState::default(),
        dirty: DirtyInfo::default(),
    });
    batch.absorb(RenderCmd::Hover { node_id: None });
    assert!(
        matches!(batch.take_snapshot(), Some(RenderCmd::RenderArena { .. })),
        "the arena snapshot must survive the deferred hover"
    );
    assert!(
        matches!(batch.take_hover(), Some(RenderCmd::Hover { node_id: None })),
        "the newer hover must be deferred, not dropped"
    );
}

#[test]
fn hover_behind_a_pending_redraw_is_deferred_not_dropped() {
    // A resize pushes a `Redraw`, and pointer motion in the same batch queues
    // a `Hover` behind it. The drain used to let the hover supersede the
    // redraw, permanently losing the resize re-layout; it must defer instead.
    let mut batch = DrainedBatch::new();
    batch.absorb(RenderCmd::Redraw {
        scroll_y: 0,
        zoom: 1.0,
    });
    batch.absorb(RenderCmd::Hover { node_id: None });
    assert!(
        matches!(batch.take_snapshot(), Some(RenderCmd::Redraw { .. })),
        "the resize re-layout must survive the deferred hover"
    );
    assert!(
        matches!(batch.take_hover(), Some(RenderCmd::Hover { node_id: None })),
        "the newer hover must be deferred, not dropped"
    );
}

#[test]
fn rerender_behind_a_pending_arena_folds_the_scroll() {
    // A scroll queued behind a pending `RenderArena` must fold into the arena
    // instead of replacing it: the arena carries DOM mutations a pure
    // viewport command must not drop.
    let mut batch = DrainedBatch::new();
    batch.absorb(RenderCmd::RenderArena {
        arena: Arc::new(NodeArena::default()),
        root_id: NodeId::default(),
        scroll_y: 0,
        zoom: 1.0,
        dynamic: DynamicState::default(),
        dirty: DirtyInfo::default(),
    });
    batch.absorb(RenderCmd::Rerender { scroll_y: 9 });
    match batch.take_snapshot() {
        Some(RenderCmd::RenderArena { scroll_y, .. }) => {
            assert_eq!(scroll_y, 9, "the newer scroll must fold into the arena");
        }
        other => panic!("expected the arena snapshot to survive, got {other:?}"),
    }
}

#[test]
fn redraw_behind_a_pending_arena_folds_scroll_and_zoom() {
    // Same fold rule for `Redraw` (resize / zoom): the DOM mutations win,
    // the viewport state updates in place.
    let mut batch = DrainedBatch::new();
    batch.absorb(RenderCmd::RenderArena {
        arena: Arc::new(NodeArena::default()),
        root_id: NodeId::default(),
        scroll_y: 0,
        zoom: 1.0,
        dynamic: DynamicState::default(),
        dirty: DirtyInfo::default(),
    });
    batch.absorb(RenderCmd::Redraw {
        scroll_y: 5,
        zoom: 2.0,
    });
    match batch.take_snapshot() {
        Some(RenderCmd::RenderArena { scroll_y, zoom, .. }) => {
            assert_eq!(scroll_y, 5);
            assert_eq!(zoom, 2.0);
        }
        other => panic!("expected the arena snapshot to survive, got {other:?}"),
    }
}

#[test]
fn focus_behind_a_pending_redraw_is_deferred_not_dropped() {
    // Focus moving while a resize `Redraw` is pending must defer like a
    // hover, not supersede the re-layout.
    let mut batch = DrainedBatch::new();
    batch.absorb(RenderCmd::Redraw {
        scroll_y: 0,
        zoom: 1.0,
    });
    batch.absorb(RenderCmd::Focus {
        node_id: Some(NodeId::default()),
    });
    assert!(
        matches!(batch.take_snapshot(), Some(RenderCmd::Redraw { .. })),
        "the resize re-layout must survive the deferred focus"
    );
    assert!(
        matches!(batch.take_focus(), Some(RenderCmd::Focus { .. })),
        "the focus delta must be deferred, not dropped"
    );
}

#[test]
fn rerender_still_supersedes_a_pending_redraw() {
    // Two viewport-only snapshots: the later one carries the newest complete
    // state and wins, as before.
    let mut batch = DrainedBatch::new();
    batch.absorb(RenderCmd::Redraw {
        scroll_y: 0,
        zoom: 1.0,
    });
    batch.absorb(RenderCmd::Rerender { scroll_y: 7 });
    assert!(
        matches!(
            batch.take_snapshot(),
            Some(RenderCmd::Rerender { scroll_y: 7 })
        ),
        "the latest viewport snapshot must win"
    );
}

#[test]
fn focus_without_a_pending_arena_supersedes_as_a_snapshot() {
    // With no arena pending, a later `Focus` fully supersedes an earlier one
    // (both are complete interaction-state deltas).
    let mut batch = DrainedBatch::new();
    batch.absorb(RenderCmd::Focus {
        node_id: Some(NodeId::default()),
    });
    batch.absorb(RenderCmd::Focus { node_id: None });
    match batch.take_snapshot() {
        Some(RenderCmd::Focus { node_id }) => assert_eq!(node_id, None),
        other => panic!("expected the latest focus, got {other:?}"),
    }
}

#[test]
fn stateful_commands_run_in_order_and_selection_keeps_the_latest() {
    // `RenderInput` / `Select*` must survive in arrival order (a focus switch
    // queues blur + focus; a `SelectClose` must not be swallowed), while the
    // incremental `TextSelection` keeps only the latest of the batch.
    let mut batch = DrainedBatch::new();
    batch.absorb(RenderCmd::RenderInput {
        node_id: NodeId::default(),
        value: "a".into(),
        cursor: 1,
        focused: true,
        selection: None,
    });
    batch.absorb(RenderCmd::SelectOpen {
        node_id: NodeId::default(),
        filter: None,
    });
    batch.absorb(RenderCmd::TextSelection {
        anchor: Some((0, 0)),
        end: Some((1, 0)),
        mode: SelectionMode::Flow,
    });
    batch.absorb(RenderCmd::SelectClose {
        node_id: NodeId::default(),
    });
    batch.absorb(RenderCmd::TextSelection {
        anchor: None,
        end: None,
        mode: SelectionMode::Block,
    });
    assert_eq!(batch.drain_inputs().count(), 1);
    let selects: Vec<RenderCmd> = batch.drain_selects().collect();
    assert_eq!(selects.len(), 2, "both select commands must survive");
    assert!(matches!(selects[0], RenderCmd::SelectOpen { .. }));
    assert!(matches!(selects[1], RenderCmd::SelectClose { .. }));
    match batch.take_text_selection() {
        Some(RenderCmd::TextSelection { anchor, mode, .. }) => {
            assert_eq!(anchor, None, "only the latest selection matters");
            assert_eq!(mode, SelectionMode::Block);
        }
        other => panic!("expected the latest selection, got {other:?}"),
    }
}

#[test]
fn quit_supersedes_the_batch_and_flags_the_drain() {
    let mut batch = DrainedBatch::new();
    batch.absorb(RenderCmd::Redraw {
        scroll_y: 0,
        zoom: 1.0,
    });
    batch.absorb(RenderCmd::Quit);
    assert!(batch.quit_requested(), "the drain must stop after a Quit");
    assert!(matches!(batch.take_snapshot(), Some(RenderCmd::Quit)));
}

#[test]
fn click_select_opens_the_dropdown() {
    // Clicking a `<select>` opens its dropdown: the input loop remembers the
    // options and overlay geometry, and queues a `SelectOpen` repaint.
    let (mut engine, geo) = build_geometry(
        r#"<div><select id="s"><option value="a">Alpha</option><option value="b">Beta</option></select></div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("s").expect("select region");
    let (sx, sy) = center(region.rect);

    let mut state = InputState::new();
    let cmds = mouse_down(&mut state, &mut engine, &map, sx, sy, MouseButton::Left);
    assert!(
        state.open_select.is_some(),
        "clicking a select must open its dropdown"
    );
    let open = state.open_select.clone().unwrap();
    assert_eq!(open.node_id, region.node_id);
    assert_eq!(open.options.len(), 2);
    assert_eq!(
        open.highlight, 0,
        "first option highlighted when nothing selected"
    );
    assert!(
        cmds.iter()
            .any(|c| matches!(c, RenderCmd::SelectOpen { .. })),
        "opening must queue a SelectOpen repaint"
    );
}

#[test]
fn picking_a_select_option_commits_the_value_and_fires_change() {
    // Clicking an option row picks it: the dropdown closes, the value is
    // committed to the DOM, and a `change` event fires along the DOM path.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var seen = [];
          var s = document.getElementById('s');
          s.addEventListener('change', function () { seen.push(s.value); });
          globalThis.__seen = seen;
        </script>
        <div><select id="s"><option value="a">Alpha</option><option value="b">Beta</option></select></div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("s").expect("select region");
    let (sx, sy) = center(region.rect);

    let mut state = InputState::new();
    mouse_down(&mut state, &mut engine, &map, sx, sy, MouseButton::Left);
    let open = state.open_select.clone().unwrap();
    // Click the second option row (one row below the field).
    let cmds = mouse_down(
        &mut state,
        &mut engine,
        &map,
        open.rect.x as u32,
        open.rect.y as u32 + 1,
        MouseButton::Left,
    );
    assert!(
        state.open_select.is_none(),
        "picking an option closes the dropdown"
    );
    assert!(
        cmds.iter()
            .any(|c| matches!(c, RenderCmd::SelectClose { .. })),
        "closing must queue a SelectClose repaint"
    );
    let arena = engine.arena();
    let value = arena
        .get(region.node_id)
        .and_then(|n| n.attributes.get("value"))
        .cloned();
    assert_eq!(
        value.as_deref(),
        Some("b"),
        "chosen option value must be committed"
    );
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))"),
        "b",
        "change must fire with the new value"
    );
}

#[test]
fn hovering_a_select_option_moves_the_highlight() {
    // While the dropdown is open, a button-free pointer move over an option
    // row moves the highlight (browser hover) and queues a `SelectMove`.
    let (mut engine, geo) = build_geometry(
        r#"<div><select id="s"><option value="a">Alpha</option><option value="b">Beta</option></select></div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("s").expect("select region");
    let (sx, sy) = center(region.rect);

    let mut state = InputState::new();
    mouse_down(&mut state, &mut engine, &map, sx, sy, MouseButton::Left);
    let open = state.open_select.clone().unwrap();

    let cmds = mouse_move(
        &mut state,
        &mut engine,
        &map,
        open.rect.x as u32,
        open.rect.y as u32 + 1,
    );
    assert_eq!(
        state.open_select.as_ref().unwrap().highlight,
        1,
        "hovering the second row must move the highlight"
    );
    assert!(
        cmds.iter()
            .any(|c| matches!(c, RenderCmd::SelectMove { highlight: 1, .. })),
        "a hover move must queue a SelectMove repaint"
    );

    // Hovering the same row again emits nothing (highlight unchanged).
    let cmds = mouse_move(
        &mut state,
        &mut engine,
        &map,
        open.rect.x as u32,
        open.rect.y as u32 + 1,
    );
    assert!(
        cmds.iter()
            .all(|c| !matches!(c, RenderCmd::SelectMove { .. })),
        "an unchanged highlight must not repaint"
    );
}

#[test]
fn dragging_over_select_options_moves_the_highlight() {
    // The browser's native interaction: press the select, drag over the
    // options (button held) — the highlight must follow the pointer, even on
    // the bottommost option row.
    let (mut engine, geo) = build_geometry(
        r#"<div><select id="s"><option value="a">Alpha</option><option value="b">Beta</option><option value="c">Gamma</option></select></div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("s").expect("select region");
    let (sx, sy) = center(region.rect);

    let mut state = InputState::new();
    mouse_down(&mut state, &mut engine, &map, sx, sy, MouseButton::Left);
    let open = state.open_select.clone().unwrap();

    // Button held while dragging to the bottommost row.
    state.pressed = Some((0, 0));
    let cmds = mouse_move(
        &mut state,
        &mut engine,
        &map,
        open.rect.x as u32,
        open.rect.y as u32 + 2,
    );
    assert_eq!(
        state.open_select.as_ref().unwrap().highlight,
        2,
        "dragging must highlight the bottommost option too"
    );
    assert!(
        cmds.iter()
            .any(|c| matches!(c, RenderCmd::SelectMove { highlight: 2, .. })),
        "drag hover must queue a SelectMove"
    );
}

#[test]
fn clicking_outside_a_select_closes_it_without_changing_the_value() {
    let (mut engine, geo) = build_geometry(
        r#"<div><select id="s" value="a"><option value="a">A</option><option value="b">B</option></select><div id="other">text</div></div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("s").expect("select region");
    let (sx, sy) = center(region.rect);

    let mut state = InputState::new();
    mouse_down(&mut state, &mut engine, &map, sx, sy, MouseButton::Left);
    assert!(state.open_select.is_some());

    // Click far away (below the document): closes, value untouched.
    mouse_down(&mut state, &mut engine, &map, 0, 1000, MouseButton::Left);
    assert!(state.open_select.is_none());
    let arena = engine.arena();
    let value = arena
        .get(region.node_id)
        .and_then(|n| n.attributes.get("value"))
        .cloned();
    assert_eq!(
        value.as_deref(),
        Some("a"),
        "dismissing must not change the value"
    );
}

// ============================================================
// Flow-selection text extraction (`layout::extract_flow_text`)
// ============================================================

#[test]
fn flow_extraction_joins_wrapped_lines_within_a_block() {
    // A long paragraph wraps across several visual rows; copying the whole
    // block must yield one continuous string — no newline at the wrap points
    // — exactly like a browser. Inline spaces are preserved by the wrap phase
    // (only the hanging space at a break is dropped), so the concatenation
    // restores the original text.
    let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
    let (_, geo) = build_geometry(&format!("<div>{text}</div>"), EventBindings::default());
    let lines: Vec<(u32, u32, u32)> = (0..10).map(|y| (y, 0, 200)).collect();
    let copied = htui::layout::extract_flow_text(&geo.arena, &geo.boxes, &lines);
    assert_eq!(copied, text, "wrapped lines of one block must concatenate");
}

#[test]
fn flow_extraction_breaks_between_blocks() {
    // Two sibling blocks: the selection crosses a block boundary, so a
    // newline must be inserted between them.
    let (_, geo) = build_geometry(
        r#"<div>alpha</div><div>beta</div>"#,
        EventBindings::default(),
    );
    let lines = vec![(0u32, 0u32, 200u32), (1u32, 0u32, 200u32)];
    let copied = htui::layout::extract_flow_text(&geo.arena, &geo.boxes, &lines);
    assert_eq!(
        copied, "alpha\nbeta",
        "blocks must be separated by a newline"
    );
}

#[test]
fn flow_extraction_breaks_between_nested_blocks() {
    // Blocks nested inside a container still count as distinct blocks: the
    // paragraphs are separated even though a common ancestor wraps them.
    let (_, geo) = build_geometry(
        r#"<div><p>one</p><p>two</p></div>"#,
        EventBindings::default(),
    );
    // `<p>`'s 1-row UA margins: "one" sits on row 1, "two" on row 3 (the
    // adjacent margins collapse to one blank row between them). Select the
    // whole band; empty margin rows must not leak into the copied text.
    let lines = vec![
        (0u32, 0u32, 200u32),
        (1u32, 0u32, 200u32),
        (2u32, 0u32, 200u32),
        (3u32, 0u32, 200u32),
    ];
    let copied = htui::layout::extract_flow_text(&geo.arena, &geo.boxes, &lines);
    assert_eq!(copied, "one\ntwo");
}

#[test]
fn flow_extraction_slices_selected_columns() {
    // A partial-column selection takes only the covered characters of a line.
    let (_, geo) = build_geometry(r#"<div>alpha</div>"#, EventBindings::default());
    let copied = htui::layout::extract_flow_text(&geo.arena, &geo.boxes, &[(0, 0, 3)]);
    assert_eq!(copied, "alp");
}

#[test]
fn selection_text_lines_clip_to_actual_text() {
    // Full-row bands must be clipped down to the columns the text actually
    // covers, so the highlight never spills over borders / padding / trailing
    // whitespace.
    let (_, geo) = build_geometry(r#"<div>alpha</div>"#, EventBindings::default());
    let lines = htui::layout::selection_text_lines(&geo.arena, &geo.boxes, &[(0, 0, 200)]);
    assert_eq!(
        lines,
        vec![(0, 0, 5)],
        "highlight must stop at the last glyph"
    );
}

#[test]
fn selection_text_lines_keep_partial_columns() {
    // A selection that starts inside the text keeps the intersection, and a
    // band past the text yields nothing.
    let (_, geo) = build_geometry(r#"<div>alpha</div>"#, EventBindings::default());
    let lines = htui::layout::selection_text_lines(&geo.arena, &geo.boxes, &[(0, 2, 200)]);
    assert_eq!(lines, vec![(0, 2, 5)]);
    let past = htui::layout::selection_text_lines(&geo.arena, &geo.boxes, &[(0, 10, 200)]);
    assert!(
        past.is_empty(),
        "a band past the text must highlight nothing"
    );
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
        "",
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
        arena,
        boxes,
        root_id,
    };
    (engine, geo)
}

/// Geometry for a document with its `<style>` CSS cascaded — the styled
/// counterpart of `build_geometry` (`ScriptEngine::new` returns a raw
/// uncascaded arena, so inline-only or `<style>` styling needs this path).
fn build_styled_geometry(html: &str) -> Geometry {
    let mut parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = parser.parse();
    if !embedded.trim().is_empty() {
        let mut css_parser = CssParser::new(&embedded);
        let rules = css_parser.parse();
        css_into_node(&mut arena, root_id, &rules);
    }
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

/// Depth-first search for the first box belonging to `node_id`.
fn find_box(
    boxes: &[htui::layout::LayoutBox],
    node_id: NodeId,
) -> Option<&htui::layout::LayoutBox> {
    for b in boxes {
        if b.node_id == node_id {
            return Some(b);
        }
        if let Some(found) = find_box(&b.children, node_id) {
            return Some(found);
        }
    }
    None
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
    mut f: impl FnMut(&mut htui::script::HostCtx) -> JsValue + Send + 'static,
) -> EventBindings {
    let mut bindings = EventBindings::default();
    bindings
        .host
        .insert(name.to_string(), Box::new(move |call| Ok(f(call))));
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
        path: path.into(),
    }
}

// ============================================================
// Input-loop drivers (test-side script thread)
// ============================================================

/// Drive the collected script messages against a real engine — the test-side
/// counterpart of the production script thread — and fold the replies into
/// the returned commands (hover updates + host-pushed cmds + dirty snapshots).
fn drive_msgs(
    state: &mut InputState,
    engine: &mut ScriptEngine,
    map: &HitMap,
    msgs: Vec<ScriptMsg>,
) -> Vec<RenderCmd> {
    let mut cmds = Vec::new();
    let mut hover = HoverState::new();
    for msg in msgs {
        let ev = apply_message(engine, &mut hover, map, msg);
        cmds.extend(ev.cmds);
        if let Some(d) = ev.dirty {
            cmds.push(RenderCmd::RenderArena {
                arena: d.arena,
                root_id: d.root_id,
                scroll_y: state.scroll_y,
                zoom: state.zoom,
                dynamic: DynamicState {
                    hovered: ev.hovered,
                    focused: None,
                },
                dirty: d.dirty,
            });
        }
    }
    cmds
}

/// Click at canvas coordinates, executing the queued messages like the script
/// thread would.
fn click(
    state: &mut InputState,
    engine: &mut ScriptEngine,
    map: &HitMap,
    x: u32,
    y: u32,
) -> Vec<RenderCmd> {
    let arena = engine.arena();
    let mut msgs = Vec::new();
    let mut cmds = Vec::new();
    handle_click(state, map, &arena, x, y, &mut msgs, &mut cmds);
    cmds.extend(drive_msgs(state, engine, map, msgs));
    cmds
}

/// Mouse-down at canvas coordinates, executing the queued messages like the
/// script thread would.
fn mouse_down(
    state: &mut InputState,
    engine: &mut ScriptEngine,
    map: &HitMap,
    x: u32,
    y: u32,
    button: MouseButton,
) -> Vec<RenderCmd> {
    let arena = engine.arena();
    let mut msgs = Vec::new();
    let mut cmds = Vec::new();
    handle_mouse_down(state, map, &arena, x, y, button, &mut msgs, &mut cmds);
    cmds.extend(drive_msgs(state, engine, map, msgs));
    cmds
}

/// Pointer move at canvas coordinates, executing the queued messages like the
/// script thread would.
fn mouse_move(
    state: &mut InputState,
    engine: &mut ScriptEngine,
    map: &HitMap,
    x: u32,
    y: u32,
) -> Vec<RenderCmd> {
    let mut msgs = Vec::new();
    let mut cmds = Vec::new();
    handle_mouse_move(state, map, x, y, &mut msgs, &mut cmds);
    cmds.extend(drive_msgs(state, engine, map, msgs));
    cmds
}

/// Left mouse-up at screen coordinates, executing the queued messages like
/// the script thread would.
fn mouse_up(
    state: &mut InputState,
    engine: &mut ScriptEngine,
    map: &HitMap,
    sx: u16,
    sy: u16,
) -> Vec<RenderCmd> {
    let arena = engine.arena();
    let mut msgs = Vec::new();
    let mut cmds = Vec::new();
    handle_mouse_up(
        state,
        map,
        &arena,
        (sx, sy),
        MouseButton::Left,
        &mut msgs,
        &mut cmds,
    );
    cmds.extend(drive_msgs(state, engine, map, msgs));
    cmds
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
        r#"<div id="outer"><div id="inner" style="width: 10ch">inner</div></div>"#,
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
fn hit_test_resolves_overlaps_by_paint_order() {
    // Browser rule: the hit target is the element painted on top. A
    // positioned box earlier in the DOM paints above the static content
    // beneath it, so it — not the later static sibling — wins the overlap
    // (the previous smallest-area rule could pick either).
    let geo = build_styled_geometry(
        r#"<style>
            #overlay { position: absolute; top: 0ch; left: 0ch; width: 10ch; height: 2ch; }
            #static { width: 10ch; }
        </style>
        <div id="overlay">ov</div><div id="static">static text</div>"#,
    );
    let map = HitMap::build(&geo);
    let overlay = map.resolve_by_id("overlay").expect("overlay region").rect;
    let stat = map.resolve_by_id("static").expect("static region").rect;
    // The fixture must actually overlap; otherwise the test proves nothing.
    let (x, y) = (overlay.x.max(stat.x), overlay.y.max(stat.y));
    assert!(
        overlay.contains(x as u32, y as u32) && stat.contains(x as u32, y as u32),
        "fixture boxes must overlap: overlay={overlay:?} static={stat:?}"
    );
    assert_eq!(
        map.hit_test(x as u32, y as u32)
            .expect("overlap must hit")
            .id(),
        Some("overlay"),
        "the positioned box painted above the static content must win"
    );
}

#[test]
fn hit_test_orders_overlaps_by_z_index() {
    // Higher z-index paints on top even when it comes first in the DOM —
    // the previous smallest-area rule would have picked the later sibling
    // on an area tie.
    let geo = build_styled_geometry(
        r#"<style>
            #high { position: absolute; top: 0ch; left: 0ch; width: 10ch; height: 2ch; z-index: 2; }
            #low { position: absolute; top: 0ch; left: 0ch; width: 10ch; height: 2ch; z-index: 1; }
        </style>
        <div id="high">h</div><div id="low">l</div>"#,
    );
    let map = HitMap::build(&geo);
    let high = map.resolve_by_id("high").expect("high region").rect;
    let low = map.resolve_by_id("low").expect("low region").rect;
    let (x, y) = (high.x.max(low.x), high.y.max(low.y));
    assert!(
        high.contains(x as u32, y as u32) && low.contains(x as u32, y as u32),
        "fixture boxes must overlap: high={high:?} low={low:?}"
    );
    assert_eq!(
        map.hit_test(x as u32, y as u32)
            .expect("overlap must hit")
            .id(),
        Some("high"),
        "the higher z-index box must win the overlap"
    );
}

#[test]
fn click_on_empty_space_dispatches_nothing() {
    let (mut engine, geo) =
        build_geometry(r#"<div id="btn">click me</div>"#, EventBindings::default());
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
fn add_event_listener_is_idempotent_for_identical_listener() {
    // Browser semantics: re-registering the same (type, callback) pair is a
    // no-op — the listener must fire once per event, not once per add call.
    let seen = Arc::new(Mutex::new(0usize));
    let seen2 = Arc::clone(&seen);
    let html = r#"
      <script>
        var h = function () { tick(); };
        var btn = document.getElementById('btn');
        btn.addEventListener('click', h);
        btn.addEventListener('click', h);
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
fn remove_child_drops_listeners_of_the_detached_subtree() {
    // A node leaving the document must not retain its listeners in the
    // GC-rooted registry — otherwise removeChild churn grows the registry
    // (and every closure the listeners captured) without bound.
    let html = r#"
      <script>
        var wrap = document.getElementById('wrap');
        wrap.addEventListener('click', function () {});
        document.getElementById('btn').addEventListener('click', function () {});
        document.body.removeChild(wrap);
      </script>
      <div id="wrap"><div id="btn">b</div></div>
    "#;
    let mut engine = ScriptEngine::new(html, "", EventBindings::default()).expect("engine");
    assert_eq!(
        eval_str(
            &mut engine,
            "Object.keys(globalThis.__htuiListeners).length"
        ),
        "0",
        "the removed node and its subtree must leave no listener entries"
    );
}

#[test]
fn replace_child_drops_listeners_of_the_replaced_subtree() {
    let html = r#"
      <script>
        var wrap = document.getElementById('wrap');
        wrap.addEventListener('click', function () {});
        document.getElementById('btn').addEventListener('click', function () {});
        document.body.replaceChild(document.createElement('div'), wrap);
      </script>
      <div id="wrap"><div id="btn">b</div></div>
    "#;
    let mut engine = ScriptEngine::new(html, "", EventBindings::default()).expect("engine");
    assert_eq!(
        eval_str(
            &mut engine,
            "Object.keys(globalThis.__htuiListeners).length"
        ),
        "0",
        "the replaced node and its subtree must leave no listener entries"
    );
}

#[test]
fn removing_a_sibling_keeps_the_survivor_listener() {
    // Clearing is scoped to the detached subtree: a listener on a node that
    // stays in the document must survive a sibling's removal.
    let html = r#"
      <script>
        document.getElementById('live').addEventListener('click', function () {});
        document.body.removeChild(document.getElementById('gone'));
      </script>
      <div id="gone">g</div>
      <div id="live">l</div>
    "#;
    let mut engine = ScriptEngine::new(html, "", EventBindings::default()).expect("engine");
    assert_eq!(
        eval_str(
            &mut engine,
            "Object.keys(globalThis.__htuiListeners).length"
        ),
        "1",
        "only the removed node's entry must be cleared"
    );
}

#[test]
fn js_can_mutate_the_dom_and_marks_dirty() {
    let html = r#"
      <script>
        document.getElementById('out').textContent = 'hello world';
      </script>
      <p id="out">before</p>
    "#;
    let mut engine = ScriptEngine::new(html, "", EventBindings::default()).expect("engine");
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
            htui::tdom::node::NodeType::Text => arena[c].data.as_deref(),
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
            if let Some(s) = call.arg::<String>(0).ok().flatten() {
                hits2.lock().unwrap().push(s);
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
            if let Some(s) = call.arg::<String>(0).ok().flatten() {
                hits2.lock().unwrap().push(s);
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

    assert_eq!(
        *hits.lock().unwrap(),
        vec!["enter".to_string(), "leave".to_string()]
    );
}

// ============================================================
// Viewport change (terminal resize) resilience
// ============================================================

/// Re-lay-out the engine's live arena at `(columns, rows)` — the test-side
/// counterpart of the render thread re-laying out after a terminal resize.
/// The arena clone keeps the same node ids, exactly like the runtime's
/// `RenderArena` path, so listeners / hover tracking survive the reflow.
fn relayout(engine: &ScriptEngine, columns: u16, rows: u16) -> Geometry {
    let arena = engine.arena();
    let root_id = engine.root_id();
    let term_map = TermMap { columns, rows };
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

#[test]
fn resize_relayout_keeps_node_ids_and_events_fire() {
    let hits: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let hits2 = Arc::clone(&hits);
    let html = r#"
      <script>
        document.getElementById('btn')
          .addEventListener('click', function () { report('click'); });
        document.getElementById('btn')
          .addEventListener('mouseover', function () { report('over'); });
      </script>
      <div id="btn" style="padding: 1ch 10ch;">button</div>
    "#;
    let (mut engine, geo) = build_geometry(
        html,
        host_bindings("report", move |call| {
            if let Some(s) = call.arg::<String>(0).ok().flatten() {
                hits2.lock().unwrap().push(s);
            }
            JsValue::undefined()
        }),
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("btn").expect("btn region");
    let (x, y) = center(region.rect);

    let mut hover = HoverState::new();
    on_move(&mut hover, &map, &mut engine, x, y);
    assert_eq!(*hits.lock().unwrap(), vec!["over".to_string()]);

    // A terminal resize re-lays-out the same live arena and rebuilds the hit
    // map. Node ids are slotmap keys and survive the arena clone, so the
    // listeners and hover tracking keep working against the fresh geometry.
    let geo2 = relayout(&engine, 40, 10);
    let map2 = HitMap::build(&geo2);
    let region2 = map2.resolve_by_id("btn").expect("btn survives resize");
    let (x2, y2) = center(region2.rect);
    on_click(&mut hover, &map2, &mut engine, x2, y2);

    assert_eq!(
        *hits.lock().unwrap(),
        vec!["over".to_string(), "click".to_string()]
    );
}

#[test]
fn resize_reflow_revalidates_hover_under_the_stationary_pointer() {
    let hits: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let hits2 = Arc::clone(&hits);
    let html = r#"
      <script>
        document.getElementById('a')
          .addEventListener('mouseover', function () { report('over-a'); });
        document.getElementById('a')
          .addEventListener('mouseout', function () { report('out-a'); });
        document.getElementById('b')
          .addEventListener('mouseover', function () { report('over-b'); });
        document.getElementById('b')
          .addEventListener('mouseout', function () { report('out-b'); });
      </script>
      <div id="a" style="padding: 1ch;">AAAA AAAA AAAA AAAA AAAA AAAA AAAA AAAA AAAA AAAA AAAA AAAA</div>
      <div id="b" style="padding: 1ch;">bbbb</div>
    "#;
    let (mut engine, geo) = build_geometry(
        html,
        host_bindings("report", move |call| {
            if let Some(s) = call.arg::<String>(0).ok().flatten() {
                hits2.lock().unwrap().push(s);
            }
            JsValue::undefined()
        }),
    );
    let map = HitMap::build(&geo);
    let b_region = map.resolve_by_id("b").expect("b region");
    let (bx, by) = center(b_region.rect);

    let mut hover = HoverState::new();
    on_move(&mut hover, &map, &mut engine, bx, by); // enter b
    assert_eq!(*hits.lock().unwrap(), vec!["over-b".to_string()]);

    // Narrow reflow: `a` wraps taller and pushes `b` down, so the cell under
    // the stationary pointer now belongs to `a`. The synthetic `Move` the run
    // loop queues after a resize (once the rebuilt hit map is published)
    // re-validates hover: b must receive `mouseout`, a `mouseover`.
    let geo2 = relayout(&engine, 20, 24);
    let map2 = HitMap::build(&geo2);
    // `b`'s top row in the wide layout falls inside the (taller) `a` once the
    // text wraps; assert the reflow actually moved b away from that cell.
    let px = 10;
    let py = b_region.rect.y as u32 + 1;
    let under = map2.hit_test(px, py).expect("cell under pointer");
    assert_ne!(
        under.node_id, b_region.node_id,
        "the narrow reflow must push b below the stationary pointer"
    );

    on_move(&mut hover, &map2, &mut engine, px, py);

    assert_eq!(
        *hits.lock().unwrap(),
        vec![
            "over-b".to_string(),
            "out-b".to_string(),
            "over-a".to_string()
        ]
    );
}

#[test]
fn stale_press_from_lost_release_does_not_kill_hover() {
    // Simulate a resize that dropped the button release: `pressed` stays set
    // (the console delivered the `Down` but lost the `Up`). The input loop
    // clears it on the next button-free `Moved` event, so hover tracking
    // (`ScriptMsg::Move`) resumes instead of every move being read as a drag.
    let (_, geo) = build_geometry("<div id='a'>a</div>", EventBindings::default());
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("a").expect("a region");
    let (x, y) = center(region.rect);
    let mut state = InputState::new();
    state.pressed = Some((x as u16, y as u16));
    state.drag_selecting = true;

    assert!(state.drop_stale_press(), "stale press must be dropped");
    assert!(state.pressed.is_none());
    assert!(!state.drag_selecting);

    let mut msgs = Vec::new();
    let mut cmds = Vec::new();
    handle_mouse_move(&mut state, &map, x, y, &mut msgs, &mut cmds);
    assert!(
        msgs.iter().any(|m| matches!(m, ScriptMsg::Move { .. })),
        "hover tracking must resume after a stale press is cleared"
    );
}

// ============================================================
// invoke_action (manual JS global invocation)
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
        "",
        host_bindings("report", move |call| {
            if let Some(n) = call.arg::<f64>(0).ok().flatten() {
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
    let mut engine = ScriptEngine::new("<p>x</p>", "", EventBindings::default()).expect("engine");
    // Must not panic, must not produce render commands.
    assert!(engine.invoke_action("nope").is_empty());
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
        htui::script::host_fn(|ctx| {
            let mut obj = ctx.object();
            obj.set("cpu", "23%");
            Ok(obj)
        }),
    );
    let mut engine = ScriptEngine::new(html, "", bindings).expect("engine");

    // Before refresh: initial script didn't call sample, so cpu is untouched.
    assert!(engine.take_dirty().is_empty());
    engine.invoke_action("refresh");
    assert!(
        !engine.take_dirty().is_empty(),
        "refresh must mark the DOM dirty"
    );

    let arena = engine.arena();
    let cpu = find_node_by_id(&arena, "cpu");
    let text: String = arena[cpu]
        .children
        .iter()
        .filter_map(|&c| match &arena[c].node_type {
            htui::tdom::node::NodeType::Text => arena[c].data.as_deref(),
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
        htui::script::host_fn(|ctx| {
            let mut obj = ctx.object();
            obj.set("cpu", "[########--------] 45%");
            Ok(obj)
        }),
    );
    let mut engine = ScriptEngine::new(html, "", bindings).expect("engine");
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
    tl.draw(&arena, root_id, &map);
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
    let entry = rules.0.first().expect("one rule");
    let selector = &entry.selector;
    // `#panel:hover` → one compound `[Id, PseudoClass]`, no combinators.
    assert!(selector.combinators.is_empty());
    assert!(selector.compounds.iter().any(|c| {
        c.iter()
            .any(|s| matches!(s, Selector::PseudoClass(PseudoClass::Hover)))
    }));
}

#[test]
fn hover_pseudo_class_applies_only_to_the_hovered_node() {
    let (root_id, mut arena) =
        setup(r#"<div id="a" class="box">a</div><div id="b" class="box">b</div>"#);
    let mut css_parser = CssParser::new(".box:hover { background-color: #00ff00; }");
    let rules = css_parser.parse();
    let a = find_node_by_id(&arena, "a");
    let b = find_node_by_id(&arena, "b");

    reset_css_attr(&mut arena, root_id);
    css_into_node_with_hover(&mut arena, root_id, &rules, Some(a));
    assert_eq!(
        arena[a].css_attr.get_bg_color(),
        Some(Color::Rgb { r: 0, g: 255, b: 0 })
    );
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
        Some(Color::Rgb {
            r: 58,
            g: 58,
            b: 58
        })
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

#[test]
fn hover_background_fills_padding_cells_not_just_the_text_run() {
    // A `:hover` background introduced by the render thread's re-cascade (the
    // element had no background when its boxes were laid out) must fill the
    // whole padding-box, not only the text columns. Without this, the padding
    // cells of a bordered/padded `.btn:hover` keep the page background and the
    // highlight reads as a hollow frame (the configurator save/reset bug).
    let html = r#"<div id="save" class="btn">Save</div>"#;
    let css = ".btn { padding-left: 1ch; padding-right: 1ch; border: 1ch solid #7aa2f7; }\n\
               .btn:hover { background-color: #28304d; }";
    let (root_id, mut arena) = setup(html);
    let mut css_parser = CssParser::new(css);
    let rules = css_parser.parse();
    let save = find_node_by_id(&arena, "save");

    // Layout with the base cascade: `.btn` carries no background, so the
    // box's layout-time `fill_bg` is false — exactly what the `Hover` render
    // command sees when it re-paints without re-laying-out.
    reset_css_attr(&mut arena, root_id);
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
        COLS as usize,
        ROWS as usize,
    );
    let r = find_box(&boxes, save).expect("btn box").rect;
    assert_eq!(r.height, 3, "border-top + content + border-bottom");

    // Re-cascade with the button hovered (no re-layout) and paint from the
    // cached layout, like the render thread's `Hover` handler.
    reset_css_attr(&mut arena, root_id);
    css_into_node_with_hover(&mut arena, root_id, &rules, Some(save));
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.repaint_from(&arena, &boxes);

    let hover_bg = Some(Color::Rgb {
        r: 0x28,
        g: 0x30,
        b: 0x4d,
    });
    let cols = COLS as usize;
    let content_row = (r.y + 1) as usize; // directly below the top border
    let left_padding = (r.x + 1) as usize; // after the left border
    let right_padding = (r.x + r.width - 2) as usize; // before the right border
    assert_eq!(
        tl.layout[content_row * cols + left_padding].bcolor,
        hover_bg,
        "left padding cell must be filled by the hover background"
    );
    assert_eq!(
        tl.layout[content_row * cols + right_padding].bcolor,
        hover_bg,
        "right padding cell must be filled by the hover background"
    );
    assert_eq!(
        tl.layout[content_row * cols + left_padding + 1].bcolor,
        hover_bg,
        "text run cell must keep the hover background"
    );
}

#[test]
fn hover_clear_reuses_cached_boxes_without_stale_run_background() {
    // The inverse of the hover-on fill: when the geometry was laid out from
    // the *hovered* cascade (a DOM mutation that re-laid-out under the
    // pointer — e.g. a button click that toggled its label), the `Hover`
    // clear repaint reuses those cached boxes with a fresh un-hovered
    // cascade. The block's own background must come from the fresh cascade
    // (`effective_bg`), not from the runs baked at layout time — otherwise
    // the glyph cell keeps the stale `:hover` background, the single
    // highlighted character left behind after a button click.
    let html = r#"<div id="save" class="btn">Save</div>"#;
    let css = ".btn { padding-left: 1ch; padding-right: 1ch; border: 1ch solid #7aa2f7; }\n\
               .btn:hover { background-color: #28304d; }";
    let (root_id, mut arena) = setup(html);
    let mut css_parser = CssParser::new(css);
    let rules = css_parser.parse();
    let save = find_node_by_id(&arena, "save");

    // Layout from the *hovered* cascade — the geometry the render thread
    // caches after a DOM mutation that happened while the pointer sat over
    // the element.
    reset_css_attr(&mut arena, root_id);
    css_into_node_with_hover(&mut arena, root_id, &rules, Some(save));
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
        COLS as usize,
        ROWS as usize,
    );
    let r = find_box(&boxes, save).expect("btn box").rect;

    // Hover clear: re-cascade without hover and repaint the element's
    // region from the cached boxes, like the render thread's `Hover { None }`.
    reset_css_attr(&mut arena, root_id);
    css_into_node(&mut arena, root_id, &rules);
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.repaint_region(&arena, &boxes, r);

    let hover_bg = Some(Color::Rgb {
        r: 0x28,
        g: 0x30,
        b: 0x4d,
    });
    let cols = COLS as usize;
    let content_row = (r.y + 1) as usize; // directly below the top border
    for x in (r.x as usize)..(r.x + r.width) as usize {
        assert_ne!(
            tl.layout[content_row * cols + x].bcolor,
            hover_bg,
            "no cell of the un-hovered button may keep the :hover background"
        );
    }
}

/// A press-and-drag-away release must clear the pressed element's `:hover`
/// highlight.
///
/// The hover state freezes while a button is held (button-free moves are the
/// only thing that re-validate it), so releasing away from the press cell
/// without re-validating leaves the pressed button highlighted until the next
/// pointer move — the highlight residue after a button press. `handle_mouse_up`
/// re-validates the hover at the release position.
#[test]
fn press_and_drag_away_clears_the_button_hover_on_release() {
    let (mut engine, geo) = build_geometry(
        r#"<div id="btn">click me</div><div id="other">other</div>"#,
        EventBindings::default(),
    );
    let map = HitMap::build(&geo);
    let btn = map.resolve_by_id("btn").expect("btn region");
    let other = map.resolve_by_id("other").expect("other region");
    let (bx, by) = center(btn.rect);
    let (ox, oy) = center(other.rect);

    let mut state = InputState::new();
    // One persistent hover state, like the script thread's.
    let mut hover = HoverState::new();
    let arena = engine.arena();

    // Move over the button → hover = button (the `:hover` highlight appears).
    let ev = apply_message(
        &mut engine,
        &mut hover,
        &map,
        ScriptMsg::Move { x: bx, y: by },
    );
    assert_eq!(ev.hovered, Some(btn.node_id), "hover enters the button");

    // Press on the button.
    let mut down_msgs = Vec::new();
    let mut cmds = Vec::new();
    handle_mouse_down(
        &mut state,
        &map,
        &arena,
        bx,
        by,
        MouseButton::Left,
        &mut down_msgs,
        &mut cmds,
    );
    state.pressed = Some((bx as u16, by as u16));

    // Drag away while the button is held: hover must stay frozen on it.
    let mut drag_msgs = Vec::new();
    handle_mouse_move(&mut state, &map, ox, oy, &mut drag_msgs, &mut cmds);
    assert!(
        drag_msgs.is_empty(),
        "no hover updates may be queued while a button is held"
    );
    assert_eq!(
        hover.current().map(|r| r.node_id),
        Some(btn.node_id),
        "hover stays on the pressed button during the drag"
    );

    // Release on the other element: the release re-validates the hover.
    let mut up_msgs = Vec::new();
    let mut up_cmds = Vec::new();
    handle_mouse_up(
        &mut state,
        &map,
        &arena,
        (ox as u16, oy as u16),
        MouseButton::Left,
        &mut up_msgs,
        &mut up_cmds,
    );
    assert!(state.pressed.is_none(), "the press is cleared on release");
    assert!(
        up_msgs.iter().any(|m| matches!(
            m,
            ScriptMsg::Move { x, y } if *x == ox && *y == oy
        )),
        "the release must re-validate the hover at the release position"
    );
    for msg in up_msgs {
        let ev = apply_message(&mut engine, &mut hover, &map, msg);
        cmds.extend(ev.cmds);
    }
    assert_eq!(
        hover.current().map(|r| r.node_id),
        Some(other.node_id),
        "hover moves to the element under the release point"
    );
    assert!(
        !cmds.iter().any(|c| matches!(
            c,
            RenderCmd::Hover { node_id: Some(n) } if *n == btn.node_id
        )),
        "no Hover command may keep the button highlighted after the release"
    );
}

// ---------------------------------------------------------------------------
// Dynamic `on*` wiring — attribute handlers stay live on JS-inserted DOM.
// Every test dispatches through `ScriptEngine::fire` with a hand-built hit
// region, so the attribute-handler slot (not just the listener list) is
// exercised end-to-end.
// ---------------------------------------------------------------------------

const DYNAMIC_WIRING_HTML: &str = r#"
  <script>
    var n = 0;
    function bump() {
      n += 1;
      document.getElementById('status').textContent = String(n);
    }
    function alt() {
      n += 10;
      document.getElementById('status').textContent = String(n);
    }
  </script>
  <div id="host"></div>
  <button id="b" onclick="bump()">b</button>
  <p id="status">0</p>
"#;

fn click_region(node_id: NodeId, id: &str) -> HitRegion {
    HitRegion {
        node_id,
        rect: Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 1,
        },
        path: vec![ElementRef {
            id: Some(id.to_string()),
            node_id,
        }]
        .into(),
    }
}

#[test]
fn inner_html_onclick_wires_after_mutation() {
    let mut engine =
        ScriptEngine::new(DYNAMIC_WIRING_HTML, "", EventBindings::default()).expect("engine");
    engine
        .eval_source(
            r#"document.getElementById('host').innerHTML =
                   '<div id="dyn" onclick="bump()"></div>'"#,
        )
        .expect("innerHTML write");

    let dyn_id = find_node_by_id(&engine.arena(), "dyn");
    let region = click_region(dyn_id, "dyn");
    engine.fire(&region, EventType::Click, 0, 0);
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('status').textContent"),
        "1",
        "the injected onclick must fire exactly once per click"
    );
    engine.fire(&region, EventType::Click, 0, 0);
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('status').textContent"),
        "2",
        "the injected handler must not double-fire"
    );
}

#[test]
fn set_attribute_onclick_replaces_handler() {
    let mut engine =
        ScriptEngine::new(DYNAMIC_WIRING_HTML, "", EventBindings::default()).expect("engine");
    let region = click_region(find_node_by_id(&engine.arena(), "b"), "b");
    engine.fire(&region, EventType::Click, 0, 0);
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('status').textContent"),
        "1",
        "boot"
    );

    engine
        .eval_source(r#"document.getElementById('b').setAttribute('onclick', 'alt()')"#)
        .expect("setAttribute");

    engine.fire(&region, EventType::Click, 0, 0);
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('status').textContent"),
        "11",
        "the new handler must replace the old one (1 + 10), not add to it"
    );
    engine.fire(&region, EventType::Click, 0, 0);
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('status').textContent"),
        "21",
        "the replaced handler stays single-slot across clicks"
    );
}

#[test]
fn remove_attribute_onclick_unwires_handler() {
    let mut engine =
        ScriptEngine::new(DYNAMIC_WIRING_HTML, "", EventBindings::default()).expect("engine");
    let region = click_region(find_node_by_id(&engine.arena(), "b"), "b");
    engine.fire(&region, EventType::Click, 0, 0);
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('status').textContent"),
        "1",
        "boot"
    );

    engine
        .eval_source(r#"document.getElementById('b').removeAttribute('onclick')"#)
        .expect("removeAttribute");
    engine.fire(&region, EventType::Click, 0, 0);
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('status').textContent"),
        "1",
        "the unwired handler must not fire after removeAttribute"
    );
}

#[test]
fn appended_child_onclick_becomes_clickable() {
    let mut engine =
        ScriptEngine::new(DYNAMIC_WIRING_HTML, "", EventBindings::default()).expect("engine");
    engine
        .eval_source(
            r#"var d = document.createElement('div');
               d.id = 'dyn';
               d.setAttribute('onclick', 'bump()');
               document.getElementById('host').appendChild(d);"#,
        )
        .expect("createElement + setAttribute + appendChild");

    let dyn_id = find_node_by_id(&engine.arena(), "dyn");
    let region = click_region(dyn_id, "dyn");
    engine.fire(&region, EventType::Click, 0, 0);
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('status').textContent"),
        "1",
        "an appended element whose onclick was set before insertion must fire"
    );
}
