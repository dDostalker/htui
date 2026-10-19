//! Script-thread bridge: message protocol between the input loop and the
//! dedicated script thread that owns the [`ScriptEngine`].
//!
//! The input loop never runs JS directly. DOM reads happen against the latest
//! arena snapshot it keeps (`ScriptResponse::Event.dirty`), while writes and
//! event dispatch travel as [`ScriptMsg`]s to the script thread, which owns
//! the engine *and* the hover state machine. A blocking host function then
//! stalls the script thread only — scrolling, quitting and mouse tracking on
//! the input loop stay responsive, mirroring how a browser keeps its UI
//! thread free of user command code.
//!
//! [`apply_message`] is the single executor shared by the script thread and
//! the synchronous `handle_*` / `trigger_refresh` wrappers used by tests, so
//! both paths are guaranteed to behave identically.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::events::{
    ElementRef, EventBindings, EventType, HitMap, HitRegion, HoverState, KeyEventInfo, interaction,
};
use crate::layout::Rect;
use crate::script::ScriptEngine;
use crate::tdom::node::{NodeArena, NodeId, NodeType};
use crate::tui::graph_api::{DirtyInfo, RenderCmd};

/// An action the input loop hands to the script thread.
///
/// Each variant bundles a *complete* browser interaction so the events it
/// dispatches keep their relative order (e.g. `blur` before `change`).
#[derive(Debug)]
pub enum ScriptMsg {
    /// Pointer move with no button held: hover enter / leave + `MouseOver` /
    /// `MouseOut` dispatch.
    Move { x: u32, y: u32 },
    /// Completed click: hover sync + `Click` dispatch.
    Click { x: u32, y: u32 },
    /// Focus an `<input>` field (fire `focus`).
    FocusInput { node_id: NodeId },
    /// Blur `node_id`, committing the edited `value` into the DOM and firing
    /// `blur` (+ `change` when the value differs from the committed one).
    BlurInput { node_id: NodeId, value: String },
    /// Editing keystroke on a focused `<input>` (fire `input`).
    InputKeystroke { node_id: NodeId },
    /// A `<select>` option was picked: `set_value_dirty` + fire `change` when
    /// the value actually changed.
    SelectPick { node_id: NodeId, value: String },
    /// An `<input type=checkbox|radio>` was clicked: toggle its `checked`
    /// attribute (clearing same-`name` radio siblings), then fire `input` /
    /// `change`.
    ToggleCheck { node_id: NodeId },
    /// The focused text field's containing `<form>` was submitted via Enter
    /// (the browser's implicit submission): fire `submit` on the form. The
    /// engine performs no default navigation — pages handle it in JS.
    FormSubmit { form_id: NodeId },
    /// A key was pressed (`KeyDown`). `target` is the focused `<input>` node
    /// (the browser's key event target); `None` means the document `<body>`.
    /// `key_id` correlates the response back to the input loop's gate wait, so
    /// a JS `preventDefault()` can cancel the built-in shortcut for this key.
    KeyDown {
        key_id: u32,
        target: Option<NodeId>,
        key: KeyEventInfo,
    },
    /// A key was released (`KeyUp`); same target semantics as `KeyDown`.
    KeyUp {
        target: Option<NodeId>,
        key: KeyEventInfo,
    },
    /// Shut the script thread down.
    Quit,
}

/// A fresh DOM snapshot after JS mutated it, for the input loop's reads and
/// the render thread's `RenderArena`.
#[derive(Debug)]
pub struct ScriptDirty {
    /// Latest live arena (slotmap clone keeps the same node ids).
    pub arena: Arc<NodeArena>,
    pub root_id: NodeId,
    /// Nodes JS touched since the last reported batch.
    pub dirty: DirtyInfo,
}

/// Outcome of processing one [`ScriptMsg`] (or a due timer tick).
#[derive(Debug)]
pub struct ScriptEvent {
    /// Render commands pushed by host functions / hover repaints.
    pub cmds: Vec<RenderCmd>,
    /// Fresh DOM snapshot when JS mutated the arena (`None` = untouched).
    pub dirty: Option<ScriptDirty>,
    /// Current hover target after processing (feeds `RenderArena.hovered`).
    pub hovered: Option<NodeId>,
    /// Non-`None` only when the message was a gated `KeyDown`: the echoed
    /// `key_id` plus whether a JS listener called `preventDefault()` (the
    /// input loop uses the pair to decide whether the built-in shortcut runs).
    pub key_gate: Option<(u32, bool)>,
}

/// One-shot or per-message reply from the script thread.
#[derive(Debug)]
pub enum ScriptResponse {
    /// Engine constructed; initial arena for the first frame.
    Ready {
        arena: Arc<NodeArena>,
        root_id: NodeId,
    },
    /// Engine construction failed (event handling is disabled afterwards).
    Failed(String),
    /// Result of processing one message / tick.
    Event(ScriptEvent),
}

/// The `<details>` a click on `node_id` should toggle, if any.
///
/// The browser's disclosure-widget default action: clicking a `<summary>`
/// (or one of its descendants) toggles the `open` attribute of the summary's
/// parent `<details>`. Anything else produces no default action.
fn summary_details(arena: &NodeArena, node_id: NodeId) -> Option<NodeId> {
    let mut current = Some(node_id);
    while let Some(id) = current {
        // `node_id` can be stale (a hit region referencing a node an
        // `innerHTML` sweep already freed) — treat it as "no target".
        let node = arena.get(id)?;
        if node.node_type == NodeType::Summary {
            return node
                .parent
                .filter(|p| arena[*p].node_type == NodeType::Details);
        }
        current = node.parent;
    }
    None
}

/// The `<dialog>` a click on `node_id` should close via the
/// `form[method=dialog]` default action, if any.
///
/// The browser's native dialog flow: a submit control (`<button type=submit>`
/// by default, or `<input type=submit>`) in a `<form method="dialog">` that
/// lives inside a `<dialog>` closes the dialog when clicked. `type=button` and
/// disabled controls are inert, exactly like the form submit rule in
/// [`button_form_action`].
fn dialog_form_action(arena: &NodeArena, node_id: NodeId) -> Option<NodeId> {
    let node = arena.get(node_id)?;
    if node.attributes.contains_key("disabled") {
        return None;
    }
    let is_submit = match node.node_type {
        NodeType::Button => !matches!(
            node.attributes
                .get("type")
                .map(|t| t.to_ascii_lowercase())
                .as_deref(),
            Some("button")
        ),
        NodeType::Input => matches!(
            node.attributes
                .get("type")
                .map(|t| t.to_ascii_lowercase())
                .as_deref(),
            Some("submit") | Some("image")
        ),
        _ => false,
    };
    if !is_submit {
        return None;
    }
    // Find the containing form; it must declare `method=dialog`.
    let mut current = node.parent;
    while let Some(pid) = current {
        if arena[pid].node_type == NodeType::Form {
            if !arena[pid]
                .attributes
                .get("method")
                .is_some_and(|m| m.eq_ignore_ascii_case("dialog"))
            {
                return None;
            }
            // The form itself must live inside a dialog.
            let mut up = arena[pid].parent;
            while let Some(uid) = up {
                if arena[uid].node_type == NodeType::Dialog {
                    return Some(uid);
                }
                up = arena[uid].parent;
            }
            return None;
        }
        current = arena[pid].parent;
    }
    None
}

/// The form event a submit / reset control click should produce:
/// `(form_id, EventType::Submit | EventType::Reset)`. `<button>` defaults to
/// `submit`; `<input type=submit|reset>` activates its form, while
/// `type=button` (both elements) and disabled controls produce nothing, and a
/// control outside any `<form>` has no form to fire on.
fn button_form_action(arena: &NodeArena, node_id: NodeId) -> Option<(NodeId, EventType)> {
    let node = arena.get(node_id)?;
    if node.attributes.contains_key("disabled") {
        return None;
    }
    let input_type = node.attributes.get("type").map(|t| t.to_ascii_lowercase());
    let action = match node.node_type {
        NodeType::Button => match input_type.as_deref() {
            Some("reset") => EventType::Reset,
            // `type=button` is inert: no submit, no reset.
            Some("button") => return None,
            _ => EventType::Submit,
        },
        // `<input type=submit|image|reset>` fire on the containing form; every
        // other input kind (text / checkbox / …) has no form default action.
        NodeType::Input => match input_type.as_deref() {
            Some("submit") | Some("image") => EventType::Submit,
            Some("reset") => EventType::Reset,
            _ => return None,
        },
        _ => return None,
    };
    let mut current = node.parent;
    while let Some(pid) = current {
        if arena[pid].node_type == NodeType::Form {
            return Some((pid, action));
        }
        current = arena[pid].parent;
    }
    None
}

/// Execute one [`ScriptMsg`] against an engine and the hover state machine.
///
/// Used by [`run_script_thread`] for production and by the synchronous
/// `handle_*` / `trigger_refresh` wrappers (tests / single-threaded callers),
/// so both paths share identical event semantics.
pub fn apply_message(
    engine: &mut ScriptEngine,
    hover: &mut HoverState,
    hit_map: &HitMap,
    msg: ScriptMsg,
) -> ScriptEvent {
    let (cmds, key_gate) = match &msg {
        ScriptMsg::Move { x, y } => (interaction::on_move(hover, hit_map, engine, *x, *y), None),
        ScriptMsg::Click { x, y } => {
            let mut cmds = interaction::on_click(hover, hit_map, engine, *x, *y);
            // `<summary>` default action: after the click handlers, clicking
            // a summary (or its content) toggles the owning `<details>`'s
            // `open` attribute and fires a `toggle` event on it — the
            // browser's disclosure widget.
            if let Some(details_id) = hit_map
                .hit_test(*x, *y)
                .and_then(|r| summary_details(&engine.arena(), r.node_id))
            {
                engine.toggle_details(details_id);
                let region = region_for(&engine.arena(), details_id);
                cmds.extend(engine.fire(&region, EventType::Toggle, 0, 0));
            }
            // `<button type=submit|reset>` default action: after the click
            // handlers, fire the matching event on the containing `<form>`.
            // The engine performs no default navigation / control reset —
            // pages handle it in JS.
            if let Some((form_id, et)) = hit_map
                .hit_test(*x, *y)
                .and_then(|r| button_form_action(&engine.arena(), r.node_id))
            {
                let region = region_for(&engine.arena(), form_id);
                cmds.extend(engine.fire(&region, et, 0, 0));
            }
            // `form[method=dialog]` default action: after the submit handlers,
            // a submit inside a dialog closes it (removes `open`) and fires a
            // `close` event on the dialog — the browser's native dialog flow.
            if let Some(dialog_id) = hit_map
                .hit_test(*x, *y)
                .and_then(|r| dialog_form_action(&engine.arena(), r.node_id))
                && engine.close_dialog(dialog_id)
            {
                let region = region_for(&engine.arena(), dialog_id);
                cmds.extend(engine.fire(&region, EventType::Close, 0, 0));
            }
            (cmds, None)
        }
        ScriptMsg::FocusInput { node_id } => {
            let region = region_for(&engine.arena(), *node_id);
            (engine.fire(&region, EventType::Focus, 0, 0), None)
        }
        ScriptMsg::InputKeystroke { node_id } => {
            let region = region_for(&engine.arena(), *node_id);
            (engine.fire(&region, EventType::Input, 0, 0), None)
        }
        ScriptMsg::BlurInput { node_id, value } => {
            let arena = engine.arena();
            let is_textarea = arena
                .get(*node_id)
                .is_some_and(|n| n.node_type == NodeType::Textarea);
            // `<input>` fields commit to the `value` attribute; a textarea's
            // DOM value is its child text content.
            let old = if is_textarea {
                arena
                    .get(*node_id)
                    .map(|n| n.text_content(&arena))
                    .unwrap_or_default()
            } else {
                arena
                    .get(*node_id)
                    .and_then(|n| n.attributes.get("value"))
                    .cloned()
                    .unwrap_or_default()
            };
            if is_textarea {
                engine.set_textarea_value(*node_id, value);
            } else {
                engine.set_value(*node_id, value);
            }
            let region = region_for(&engine.arena(), *node_id);
            let mut cmds = engine.fire(&region, EventType::Blur, 0, 0);
            if old != *value {
                cmds.extend(engine.fire(&region, EventType::Change, 0, 0));
            }
            (cmds, None)
        }
        ScriptMsg::SelectPick { node_id, value } => {
            let arena = engine.arena();
            let old = arena
                .get(*node_id)
                .and_then(|n| n.attributes.get("value"))
                .cloned()
                .unwrap_or_default();
            if old != *value {
                engine.set_value_dirty(*node_id, value);
                let region = region_for(&engine.arena(), *node_id);
                (engine.fire(&region, EventType::Change, 0, 0), None)
            } else {
                (Vec::new(), None)
            }
        }
        ScriptMsg::ToggleCheck { node_id } => {
            // Checkbox toggles every click; a radio always checks (clicking an
            // already-selected radio is a no-op). Only actual state flips fire
            // `input` / `change`, matching browser default-action timing (the
            // `Click` dispatch ran first).
            let node_id = *node_id;
            let is_radio = engine.arena().get(node_id).is_some_and(|n| {
                n.get_type()
                    .is_some_and(|t| t.eq_ignore_ascii_case("radio"))
            });
            let should_check = if is_radio {
                true
            } else {
                !engine
                    .arena()
                    .get(node_id)
                    .is_some_and(|n| n.attributes.contains_key("checked"))
            };
            let changed = engine.set_checked(node_id, should_check);
            if changed {
                let region = region_for(&engine.arena(), node_id);
                let mut cmds = engine.fire(&region, EventType::Input, 0, 0);
                cmds.extend(engine.fire(&region, EventType::Change, 0, 0));
                (cmds, None)
            } else {
                (Vec::new(), None)
            }
        }
        ScriptMsg::FormSubmit { form_id } => {
            // Enter in a focused text field submits its containing form (the
            // browser's implicit submission). No default navigation — pages
            // handle the `submit` event in JS.
            let region = region_for(&engine.arena(), *form_id);
            (engine.fire(&region, EventType::Submit, 0, 0), None)
        }
        ScriptMsg::KeyDown {
            key_id,
            target,
            key,
        } => {
            let region = key_region(engine, *target);
            let (cmds, prevented) = engine.fire_key_gated(&region, EventType::KeyDown, key);
            (cmds, Some((*key_id, prevented)))
        }
        ScriptMsg::KeyUp { target, key } => {
            let region = key_region(engine, *target);
            (engine.fire_key(&region, EventType::KeyUp, key), None)
        }
        ScriptMsg::Quit => (Vec::new(), None),
    };

    let dirty = {
        let dirty = engine.take_dirty();
        if dirty.is_empty() {
            None
        } else {
            Some(ScriptDirty {
                arena: Arc::new(engine.arena()),
                root_id: engine.root_id(),
                dirty,
            })
        }
    };
    let hovered = hover.current().map(|r| r.node_id);
    ScriptEvent {
        cmds,
        dirty,
        hovered,
        key_gate,
    }
}

/// Drive a [`ScriptEngine`] on a dedicated thread.
///
/// Constructs the engine (running the document's `<script>` blocks), answers
/// with `ScriptResponse::Ready` (or `Failed`), then processes [`ScriptMsg`]s
/// from the input loop. When `EventBindings::auto` is set, ticks the periodic
/// action between messages via `recv_timeout`. Exits on `ScriptMsg::Quit` or
/// when the input loop drops its sender.
pub fn run_script_thread(
    html: Arc<str>,
    bindings: EventBindings,
    rx: Receiver<ScriptMsg>,
    tx: Sender<ScriptResponse>,
    hit_map: Arc<Mutex<Arc<HitMap>>>,
) {
    let mut engine = match ScriptEngine::new(html.as_ref(), bindings) {
        Ok(engine) => engine,
        Err(err) => {
            log::error!("Failed to initialize script engine: {err}");
            if tx.send(ScriptResponse::Failed(err.to_string())).is_err() {
                log::debug!("Script response channel closed during init: {err}");
            }
            return;
        }
    };
    let mut hover = HoverState::new();

    if tx
        .send(ScriptResponse::Ready {
            arena: Arc::new(engine.arena()),
            root_id: engine.root_id(),
        })
        .is_err()
    {
        log::debug!("Script response channel closed before first frame");
        return;
    }

    loop {
        // Block for the next message. The wait is bounded only by real wake
        // sources:
        // - the earliest due timer (JS `setTimeout` / `setInterval`) bounds
        //   it to its deadline;
        // - a pending `fetch` completion needs a zero wait: loads resolve
        //   synchronously into the fetch channel at `fetch()` call time, so
        //   an outstanding fetch id means the result is already queued and
        //   the loop must drain it now;
        // - a quiet page (no timers, no fetches) blocks indefinitely — the
        //   input loop's messages and its drop are the only wake-ups, so an
        //   idle session costs zero script-thread wake-ups.
        let wait = match engine.next_timer_timeout() {
            Some(deadline) => Some(deadline),
            None if engine.has_pending_fetch() => Some(Duration::ZERO),
            None => None,
        };
        let msg = match wait {
            Some(wait) => match rx.recv_timeout(wait) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(msg) => Some(msg),
                Err(_) => break,
            },
        };

        // Whether this iteration woke on a real message (vs a timer / fetch
        // wake). Real messages always get a reply — the input loop's key gate
        // correlates its verdict on them.
        let has_msg = msg.is_some();
        let mut event = match msg {
            Some(ScriptMsg::Quit) => break,
            Some(msg) => {
                let map = match hit_map.lock() {
                    Ok(guard) => Arc::clone(&guard),
                    Err(e) => {
                        log::error!("Hit map lock poisoned: {}", e);
                        Arc::new(HitMap::new())
                    }
                };
                apply_message(&mut engine, &mut hover, &map, msg)
            }
            // No message arrived in time — a timer must be due.
            None => ScriptEvent {
                cmds: Vec::new(),
                dirty: None,
                hovered: hover.current().map(|r| r.node_id),
                key_gate: None,
            },
        };

        // Fire timers that came due while waiting / processing the message.
        // Their callbacks may rewrite the DOM, so the dirty snapshot must
        // merge whatever they produced after the message's own batch.
        event.cmds.extend(engine.run_due_timers());

        // Settle completed `fetch` loads and run the microtask queue. This
        // must happen after timers so a timer callback that starts a fetch and
        // the fetch's `.then` chain both drain in the same tick.
        engine.poll_fetches();
        // Flush render commands queued outside a dispatch (e.g. `el.focus()`
        // from a top-level `<script>`): `dispatch` only drains the queue for
        // its own calls, so without this a load-time command would wait for
        // the first user event.
        event.cmds.extend(engine.take_pending_cmds());
        let extra = engine.take_dirty();
        if !extra.is_empty() {
            let snapshot = ScriptDirty {
                arena: Arc::new(engine.arena()),
                root_id: engine.root_id(),
                dirty: extra,
            };
            event.dirty = match event.dirty {
                Some(mut d) => {
                    d.dirty.nodes.extend(snapshot.dirty.nodes);
                    // Later writes win per host (IndexMap keeps the first
                    // insertion position, matching document order).
                    d.dirty.injected_css.extend(snapshot.dirty.injected_css);
                    d.arena = snapshot.arena;
                    d.root_id = snapshot.root_id;
                    Some(d)
                }
                None => Some(snapshot),
            };
        }

        // A timeout wake that produced nothing (a timer fired without visible
        // effect, or a fetch settled nothing) stays silent: the empty event
        // carries no information for the input loop, and sending it would
        // wake that loop for nothing.
        if !has_msg && event.cmds.is_empty() && event.dirty.is_none() && event.key_gate.is_none() {
            continue;
        }
        if tx.send(ScriptResponse::Event(event)).is_err() {
            log::debug!("Script response channel closed");
            break;
        }
    }
    log::debug!("Script thread exiting");
}

/// Build the hit region a keyboard event targets: the focused `<input>` when
/// it still exists in the live arena (JS may have removed it since the input
/// loop's snapshot), otherwise the document `<body>` — the browser's fallback
/// for key events with no focused element, so a `<body onkeydown>` handler
/// catches every key the page does not intercept.
pub(crate) fn key_region(engine: &ScriptEngine, target: Option<NodeId>) -> HitRegion {
    let arena = engine.arena();
    let id = match target {
        Some(id) if arena.contains_key(id) => id,
        _ => arena
            .iter()
            .find(|(_, n)| n.node_type == NodeType::Body)
            .map(|(id, _)| id)
            .unwrap_or_else(|| engine.root_id()),
    };
    region_for(&arena, id)
}

/// Build a minimal hit region for `node_id` (no geometry) so the script
/// thread can fire focus / blur / input / change events along the DOM
/// ancestor chain, exactly like a pointer-driven region from the hit map.
pub(crate) fn region_for(arena: &NodeArena, node_id: NodeId) -> HitRegion {
    let mut path = Vec::new();
    let mut current = Some(node_id);
    while let Some(id) = current {
        // A stale id (the node was freed by an `innerHTML` sweep between the
        // hit test and this call) yields a truncated/empty path, which fires
        // no listeners — the same outcome as a removed browser node.
        let Some(node) = arena.get(id) else {
            break;
        };
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
