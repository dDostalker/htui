//! Pure mouse-interaction logic (hit testing + dispatch), independent of the
//! terminal event source.
//!
//! `input.rs` feeds crossterm mouse events into [`on_click`] / [`on_move`]
//! after converting screen coordinates to canvas coordinates; the tests drive
//! these functions directly with canvas coordinates, so no terminal is needed.

use crate::{
    script::ScriptEngine,
    tui::graph_api::RenderCmd,
};

use super::{EventType, HitMap, HitRegion};

/// Tracks the element currently under the pointer for enter / leave
/// detection.
#[derive(Debug, Default)]
pub struct HoverState {
    current: Option<HitRegion>,
}

impl HoverState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Region currently hovered, if any.
    pub fn current(&self) -> Option<&HitRegion> {
        self.current.as_ref()
    }
}

/// Process a pointer move at canvas coordinates `(x, y)`.
///
/// Detects pointer enter / leave against the previous hover target and
/// dispatches the matching [`EventType::MouseOver`] / [`EventType::MouseOut`]
/// events (leave before enter, matching the DOM). When the hover target
/// changes, a `Hover` command is emitted so the render thread can re-style
/// the document for CSS `:hover` rules — with no `:hover` CSS, the command
/// changes nothing visually, exactly like a browser. Returns the render
/// commands to run, in order.
pub fn on_move(
    state: &mut HoverState,
    hit_map: &HitMap,
    engine: &mut ScriptEngine,
    x: u32,
    y: u32,
) -> Vec<RenderCmd> {
    let mut cmds = Vec::new();
    let new_hover = hit_map.hit_test(x, y);
    let hover_changed = new_hover.map(|r| r.node_id) != state.current.as_ref().map(|r| r.node_id);
    if !hover_changed {
        return cmds;
    }
    if let Some(old) = state.current.take() {
        cmds.extend(engine.fire(&old, EventType::MouseOut, x, y));
    }
    state.current = new_hover.cloned();
    if let Some(region) = new_hover {
        cmds.extend(engine.fire(region, EventType::MouseOver, x, y));
    }
    cmds.push(RenderCmd::Hover {
        node_id: new_hover.map(|r| r.node_id),
    });
    cmds
}

/// Process a click at canvas coordinates `(x, y)`.
///
/// A click implies the pointer is over the target, so the hover state is
/// synchronised first (emitting a `Hover` command when the target changed, so
/// CSS `:hover` styling stays correct), then the [`EventType::Click`] event is
/// dispatched with bubbling through the script engine.
pub fn on_click(
    state: &mut HoverState,
    hit_map: &HitMap,
    engine: &mut ScriptEngine,
    x: u32,
    y: u32,
) -> Vec<RenderCmd> {
    let mut cmds = Vec::new();
    let Some(region) = hit_map.hit_test(x, y) else {
        return cmds;
    };
    if state.current.as_ref().map(|r| r.node_id) != Some(region.node_id) {
        state.current = Some(region.clone());
        cmds.push(RenderCmd::Hover {
            node_id: Some(region.node_id),
        });
    }
    cmds.extend(engine.fire(region, EventType::Click, x, y));
    cmds
}
