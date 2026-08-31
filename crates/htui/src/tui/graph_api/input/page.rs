use crossterm::event::{self, KeyCode, KeyEvent};

use crate::script::ScriptMsg;
use crate::tdom::node::NodeArena;

use super::InputState;
use super::cmd::{RenderCmd, SelectionMode};
use super::field::{clear_text_selection, collect_input_ids};
use super::focus::focus_input;
use super::{ZOOM_MAX, ZOOM_MIN, ZOOM_STEP};

/// The page-level built-in action for a key that no `<select>` dropdown or
/// focused `<input>` consumed: `Ctrl` zoom / document jumps, page text-
/// selection clearing, PageUp / PageDown / arrows / `g` / `G` scrolling,
/// `Home` / `End`, `Esc` quit and `Tab` / `Shift`+`Tab` focus cycling.
///
/// The browser-style keys are deliberately **not** bound: `j` / `k` / `q` /
/// `Space` type into focused fields and match a browser, which has no vim
/// shortcuts, terminal quit key or page-scroll shortcut. `Esc` remains the
/// engine's quit key.
///
/// Returns the render commands the key produces (empty = no built-in action,
/// e.g. a scroll key already at the page boundary). With the script layer
/// active the caller defers scroll / zoom / quit commands until the keydown
/// gate verdict; `Tab` focus cycling pushes script messages, so it is excluded
/// from the gate and always runs immediately.
pub(super) fn page_level_builtins(
    state: &mut InputState,
    key: KeyEvent,
    rows: u16,
    max_scroll: u32,
    arena: Option<&NodeArena>,
    msgs: &mut Vec<ScriptMsg>,
) -> Vec<RenderCmd> {
    let mut cmds = Vec::new();
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
            // Copy a page text selection (inside an `<input>` the Ctrl
            // shortcuts are handled earlier). `Ctrl+X` on read-only page text
            // degrades to a copy, like a browser. The selection can only exist
            // when its copy feature is enabled, but the mode is also checked
            // here so a disabled feature never copies.
            KeyCode::Char('c') | KeyCode::Char('x') => {
                if state.focused_input.is_none()
                    && state.text_sel_anchor.is_some()
                    && match state.text_sel_mode {
                        SelectionMode::Flow => state.copy_flow,
                        SelectionMode::Block => state.copy_block,
                    }
                    && let Ok(guard) = state.selection_text.lock()
                {
                    state.input_clipboard = guard.clone();
                }
            }
            // `Ctrl` + Home / End jump to the top / bottom of the page
            // (matching most editors' document navigation).
            KeyCode::Home => {
                if state.scroll_y == 0 {
                    return cmds;
                }
                state.scroll_y = 0;
                return vec![RenderCmd::Rerender {
                    scroll_y: state.scroll_y,
                }];
            }
            KeyCode::End => {
                if state.scroll_y >= max_scroll {
                    return cmds;
                }
                state.scroll_y = max_scroll;
                return vec![RenderCmd::Rerender {
                    scroll_y: state.scroll_y,
                }];
            }
            _ => {}
        }
    } else if state.text_sel_anchor.is_some() {
        // A non-Ctrl key abandons the page text selection (the selection is
        // a pointer-driven, transient visual state).
        clear_text_selection(state, &mut cmds);
    }

    match key.code {
        // `Esc` quits the engine; plain `q` is deliberately not bound (it
        // types into focused fields and matches the browser, which has no
        // terminal quit key).
        KeyCode::Esc => cmds.push(RenderCmd::Quit),

        // vim-style document jumps: `g` to the top, `G` to the bottom
        // (shifted `g` — the shift modifier is read via the `Char('G')` code
        // crossterm reports).
        KeyCode::Char('g') => {
            if state.scroll_y == 0 {
                return cmds;
            }
            state.scroll_y = 0;
            cmds.push(RenderCmd::Rerender {
                scroll_y: state.scroll_y,
            });
        }
        KeyCode::Char('G') => {
            if state.scroll_y >= max_scroll {
                return cmds;
            }
            state.scroll_y = max_scroll;
            cmds.push(RenderCmd::Rerender {
                scroll_y: state.scroll_y,
            });
        }
        // `Tab` / `Shift`+`Tab` cycle keyboard focus between the page's
        // `<input>` fields (browser tab order). With no field focused, `Tab`
        // focuses the first one.
        KeyCode::Tab => {
            if let Some(arena) = arena {
                let ids = collect_input_ids(arena);
                if !ids.is_empty() {
                    let next = match state
                        .focused_input
                        .and_then(|cur| ids.iter().position(|&id| id == cur))
                    {
                        Some(p) => ids[(p + 1) % ids.len()],
                        None => ids[0],
                    };
                    focus_input(state, next, arena, msgs, &mut cmds);
                }
            }
        }
        KeyCode::BackTab => {
            if let Some(arena) = arena {
                let ids = collect_input_ids(arena);
                if !ids.is_empty() {
                    let next = match state
                        .focused_input
                        .and_then(|cur| ids.iter().position(|&id| id == cur))
                    {
                        Some(p) => ids[(p + ids.len() - 1) % ids.len()],
                        None => ids[ids.len() - 1],
                    };
                    focus_input(state, next, arena, msgs, &mut cmds);
                }
            }
        }

        KeyCode::Up => {
            // Already at the top boundary — drop the event instead of
            // forwarding a no-op scroll to the render thread.
            if state.scroll_y == 0 {
                return cmds;
            }
            state.scroll_y -= 1;
            cmds.push(RenderCmd::Rerender {
                scroll_y: state.scroll_y,
            });
        }
        KeyCode::Down => {
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
            // Use the actual upper bound rather than u32::MAX so the render
            // thread receives a value it doesn't have to clamp.
            state.scroll_y = max_scroll;
            cmds.push(RenderCmd::Rerender {
                scroll_y: state.scroll_y,
            });
        }
        _ => {}
    }
    cmds
}
