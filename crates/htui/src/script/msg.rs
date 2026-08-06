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

use crate::events::{ElementRef, EventBindings, EventType, HitMap, HitRegion, HoverState, interaction};
use crate::layout::Rect;
use crate::script::ScriptEngine;
use crate::tdom::node::{NodeArena, NodeId};
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
}

/// One-shot or per-message reply from the script thread.
#[derive(Debug)]
pub enum ScriptResponse {
    /// Engine constructed; initial arena for the first frame.
    Ready { arena: Arc<NodeArena>, root_id: NodeId },
    /// Engine construction failed (event handling is disabled afterwards).
    Failed(String),
    /// Result of processing one message / tick.
    Event(ScriptEvent),
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
    let cmds = match &msg {
        ScriptMsg::Move { x, y } => interaction::on_move(hover, hit_map, engine, *x, *y),
        ScriptMsg::Click { x, y } => interaction::on_click(hover, hit_map, engine, *x, *y),
        ScriptMsg::FocusInput { node_id } => {
            let region = region_for(&engine.arena(), *node_id);
            engine.fire(&region, EventType::Focus, 0, 0)
        }
        ScriptMsg::InputKeystroke { node_id } => {
            let region = region_for(&engine.arena(), *node_id);
            engine.fire(&region, EventType::Input, 0, 0)
        }
        ScriptMsg::BlurInput { node_id, value } => {
            let arena = engine.arena();
            let old = arena
                .get(*node_id)
                .and_then(|n| n.attributes.get("value"))
                .cloned()
                .unwrap_or_default();
            engine.set_value(*node_id, value);
            let region = region_for(&arena, *node_id);
            let mut cmds = engine.fire(&region, EventType::Blur, 0, 0);
            if old != *value {
                cmds.extend(engine.fire(&region, EventType::Change, 0, 0));
            }
            cmds
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
                engine.fire(&region, EventType::Change, 0, 0)
            } else {
                Vec::new()
            }
        }
        ScriptMsg::Quit => Vec::new(),
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
    ScriptEvent { cmds, dirty, hovered }
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
        // Block for the next message; an active timer (JS `setTimeout` /
        // `setInterval`) bounds the wait so a due callback can fire even
        // without user input.
        let msg = match engine.next_timer_timeout() {
            Some(d) => match rx.recv_timeout(d) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(msg) => Some(msg),
                Err(_) => break,
            },
        };

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
            },
        };

        // Fire timers that came due while waiting / processing the message.
        // Their callbacks may rewrite the DOM, so the dirty snapshot must
        // merge whatever they produced after the message's own batch.
        event.cmds.extend(engine.run_due_timers());
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
                    d.arena = snapshot.arena;
                    d.root_id = snapshot.root_id;
                    Some(d)
                }
                None => Some(snapshot),
            };
        }

        if tx.send(ScriptResponse::Event(event)).is_err() {
            log::debug!("Script response channel closed");
            break;
        }
    }
    log::debug!("Script thread exiting");
}

/// Build a minimal hit region for `node_id` (no geometry) so the script
/// thread can fire focus / blur / input / change events along the DOM
/// ancestor chain, exactly like a pointer-driven region from the hit map.
pub(crate) fn region_for(arena: &NodeArena, node_id: NodeId) -> HitRegion {
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
