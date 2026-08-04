use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};

use crate::{
    events::{EventBindings, HitMap, interaction},
    tdom::node::NodeId,
};

const SCROLL_LINES_PER_TICK: u32 = 3;
const ZOOM_STEP: f32 = 0.1;
const ZOOM_MIN: f32 = 0.2;
const ZOOM_MAX: f32 = 5.0;

#[derive(Debug, Clone, PartialEq)]
pub enum RenderCmd {
    Rerender { scroll_y: u32 },
    Redraw { scroll_y: u32, zoom: f32 },
    /// Re-style the cached document for the CSS `:hover` pseudo-class with the
    /// given session-local node (`None` clears any hover), then repaint. With
    /// no `:hover` rules in the stylesheet this repaints to the same pixels.
    Hover { node_id: Option<NodeId> },
    Quit,
}

/// Input-loop state that persists between events.
///
/// Groups the variables the input loop mutates (`scroll_y`, `zoom`) with the
/// mouse-tracking state (press position, hover target) so the loop only
/// passes one state value around.
#[derive(Debug)]
pub struct InputState {
    pub scroll_y: u32,
    pub zoom: f32,
    /// Screen coordinates of the last left-button press; a click is only
    /// recognised when the release lands on the same cell.
    pressed: Option<(u16, u16)>,
    /// Hover tracking state for mouse enter / leave detection.
    pub hover: interaction::HoverState,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            scroll_y: 0,
            zoom: 1.0,
            pressed: None,
            hover: interaction::HoverState::new(),
        }
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

/// Wait for the next input event and translate it into render commands.
///
/// `timeout` controls how long to block on `event::poll`:
/// - `None` blocks indefinitely (legacy behaviour, suitable for tests or
///   one-shot tools that don't need to react to the render thread dying).
/// - `Some(duration)` returns an empty vec when no event arrives within the
///   duration, giving the caller a chance to check other state (e.g.
///   whether the render thread is still alive).
///
/// `max_scroll` is the largest `scroll_y` the canvas currently allows
/// (i.e. `rendered_rows - viewport_rows`). It is supplied by the render
/// thread after each layout pass. Scroll events that would push past the
/// boundary in either direction are dropped here at the source instead of
/// being forwarded as a no-op `Rerender` — the render thread already had a
/// guard for that, but every dropped command here is one fewer channel
/// send and, more importantly, one fewer frame that risks a visible flash
/// at the scroll bounds.
///
/// `hit_map` and `bindings` activate mouse interaction (pass both together,
/// or neither):
/// - left `Down` records the press position; left `Up` on the same cell is
///   dispatched as a bubbling `Click` via
///   [`interaction::on_click`](crate::events::interaction::on_click);
/// - `Moved` detects pointer enter / leave and dispatches `MouseOver` /
///   `MouseOut` via [`interaction::on_move`](crate::events::interaction::on_move).
///   Like a browser without CSS `:hover`, moving the pointer never changes
///   what is painted.
pub fn trigger_refresh(
    state: &mut InputState,
    rows: u16,
    max_scroll: u32,
    hit_map: Option<&HitMap>,
    bindings: Option<&mut EventBindings>,
    timeout: Option<Duration>,
) -> Vec<RenderCmd> {
    let mut cmds = Vec::new();
    if let Some(t) = timeout {
        // `poll` returns `Ok(false)` on timeout and `Err` on a system error;
        // both cases should be surfaced as an empty vec so the outer loop can
        // re-check the render thread instead of blocking forever.
        if !event::poll(t).unwrap_or(false) {
            return cmds;
        }
    }
    match event::read() {
        Ok(Event::Key(key)) => {
            if key.kind == KeyEventKind::Release {
                return cmds;
            }

            if key.modifiers.contains(event::KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('=') | KeyCode::Char('+') => {
                        state.zoom = (state.zoom + ZOOM_STEP).min(ZOOM_MAX);
                        return vec![RenderCmd::Redraw {
                            scroll_y: state.scroll_y,
                            zoom: state.zoom,
                        }];
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') => {
                        state.zoom = (state.zoom - ZOOM_STEP).max(ZOOM_MIN);
                        return vec![RenderCmd::Redraw {
                            scroll_y: state.scroll_y,
                            zoom: state.zoom,
                        }];
                    }
                    KeyCode::Char('0') => {
                        state.zoom = 1.0;
                        return vec![RenderCmd::Redraw {
                            scroll_y: state.scroll_y,
                            zoom: state.zoom,
                        }];
                    }
                    _ => {}
                }
            }

            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => cmds.push(RenderCmd::Quit),

                KeyCode::Up | KeyCode::Char('k') => {
                    // Already at the top boundary — drop the event instead
                    // of forwarding a no-op scroll to the render thread.
                    if state.scroll_y == 0 {
                        return cmds;
                    }
                    state.scroll_y -= 1;
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    // Already at the bottom boundary — drop the event.
                    if state.scroll_y >= max_scroll {
                        return cmds;
                    }
                    state.scroll_y += 1;
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                KeyCode::PageUp => {
                    if state.scroll_y == 0 {
                        return cmds;
                    }
                    let step = (rows as u32).saturating_sub(1).max(1);
                    state.scroll_y = state.scroll_y.saturating_sub(step);
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                KeyCode::PageDown => {
                    if state.scroll_y >= max_scroll {
                        return cmds;
                    }
                    let step = (rows as u32).saturating_sub(1).max(1);
                    state.scroll_y = state.scroll_y.saturating_add(step);
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                KeyCode::Home => {
                    // No-op move — drop to avoid a wasted rerender.
                    if state.scroll_y == 0 {
                        return cmds;
                    }
                    state.scroll_y = 0;
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                KeyCode::End => {
                    // No-op move (already at the bottom) — drop.
                    if state.scroll_y >= max_scroll {
                        return cmds;
                    }
                    // Use the actual upper bound rather than u32::MAX so the
                    // render thread receives a value it doesn't have to clamp.
                    state.scroll_y = max_scroll;
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                _ => {}
            }
            cmds
        }

        Ok(Event::Mouse(mouse)) => {
            let ctrl = mouse.modifiers.contains(event::KeyModifiers::CONTROL);
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    if ctrl {
                        state.zoom = (state.zoom - ZOOM_STEP).max(ZOOM_MIN);
                        return vec![RenderCmd::Redraw {
                            scroll_y: state.scroll_y,
                            zoom: state.zoom,
                        }];
                    }
                    if state.scroll_y == 0 {
                        return cmds;
                    }
                    state.scroll_y = state.scroll_y.saturating_sub(SCROLL_LINES_PER_TICK);
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                MouseEventKind::ScrollDown => {
                    if ctrl {
                        state.zoom = (state.zoom + ZOOM_STEP).min(ZOOM_MAX);
                        return vec![RenderCmd::Redraw {
                            scroll_y: state.scroll_y,
                            zoom: state.zoom,
                        }];
                    }
                    if state.scroll_y >= max_scroll {
                        return cmds;
                    }
                    state.scroll_y = state.scroll_y.saturating_add(SCROLL_LINES_PER_TICK);
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if hit_map.is_some() {
                        state.pressed = Some((mouse.column, mouse.row));
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    if let (Some((px, py)), Some(hit_map), Some(bindings)) =
                        (state.pressed, hit_map, bindings)
                    {
                        state.pressed = None;
                        if px == mouse.column && py == mouse.row {
                            let (x, y) = canvas_pos(state, mouse.column, mouse.row);
                            cmds.extend(interaction::on_click(
                                &mut state.hover,
                                hit_map,
                                bindings,
                                x,
                                y,
                            ));
                        }
                    } else {
                        state.pressed = None;
                    }
                }
                MouseEventKind::Moved => {
                    if let (Some(hit_map), Some(bindings)) = (hit_map, bindings) {
                        let (x, y) = canvas_pos(state, mouse.column, mouse.row);
                        cmds.extend(interaction::on_move(
                            &mut state.hover,
                            hit_map,
                            bindings,
                            x,
                            y,
                        ));
                    }
                }
                _ => {}
            }
            cmds
        }

        Ok(Event::Resize(_, _new_rows)) => vec![RenderCmd::Redraw {
            scroll_y: state.scroll_y,
            zoom: state.zoom,
        }],

        _ => cmds,
    }
}

/// Convert a screen position to canvas coordinates by adding the scroll
/// offset. Zoom is not yet wired into layout, so no zoom scaling is applied.
fn canvas_pos(state: &InputState, column: u16, row: u16) -> (u32, u32) {
    (column as u32, row as u32 + state.scroll_y)
}
