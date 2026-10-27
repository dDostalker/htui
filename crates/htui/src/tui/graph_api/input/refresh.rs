use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};

use crate::events::HitMap;
use crate::layout::layoutbox::geometry::select_item_indices;
use crate::script::ScriptMsg;
use crate::tdom::node::{NodeArena, NodeId};

use super::InputState;
use super::cmd::RenderCmd;
use super::edit::{
    apply_ctrl_shortcut, apply_input_key, apply_input_word_key, apply_shift_navigation,
    is_input_field_key, paste_into_field,
};
use super::field::{
    KeyGate, apply_range_key, apply_textarea_key, focused_form, is_disabled_control,
    is_number_input, is_readonly_input, number_input_allows_char, textarea_content_width,
};
use super::focus::{blur_input, push_input_render};
use super::key_info::key_event_info;
use super::mouse::{
    canvas_pos, handle_mouse_down, handle_mouse_move, handle_mouse_up, scroll_at_wheel,
};
use super::page::page_level_builtins;
use super::select::{OverlayKind, close_select, refresh_datalist};
use super::{SCROLL_LINES_PER_TICK, reapply_console_modes};

/// Whether keystrokes must not mutate the focused field: it is readonly, or
/// it is disabled. A disabled control can never legitimately hold keyboard
/// focus (mouse presses and `Tab` cycling both skip it); if JS `el.focus()`
/// targeted one anyway, navigation keys keep working but nothing mutates —
/// the same posture a readonly field takes.
fn is_edit_blocked(arena: Option<&NodeArena>, node_id: NodeId) -> bool {
    is_readonly_input(arena, node_id) || is_disabled_control(arena, node_id)
}

/// Wait for the next input event and translate it into render commands.
///
/// The input loop never executes JS: everything that needs the engine is
/// queued into `msgs` (forwarded to the dedicated script thread, so a blocking
/// host function never stalls input); DOM reads use `arena`, the input loop's
/// latest snapshot.
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
/// `hit_map` and `arena` activate mouse interaction (pass both together,
/// or neither):
/// - left `Down` records the press position and (browser-aligned) focuses an
///   `<input>` / blurs the focused field / starts a drag selection;
/// - `Moved` while the button is held drives the active drag (field-internal
///   character selection or the page text selection); without a press it
///   queues a [`ScriptMsg::Move`] for the script thread's hover machinery
///   (`MouseOver` / `MouseOut`, via
///   [`interaction::on_move`](crate::events::interaction::on_move)). Like a
///   browser without CSS `:hover`, moving the pointer never changes what is
///   painted;
/// - left `Up` on the same cell queues a bubbling `Click` (via
///   [`interaction::on_click`](crate::events::interaction::on_click)); a drag
///   that ends elsewhere keeps the selection.
pub fn trigger_refresh(
    state: &mut InputState,
    rows: u16,
    max_scroll: u32,
    hit_map: Option<&HitMap>,
    arena: Option<&NodeArena>,
    msgs: &mut Vec<ScriptMsg>,
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
                // Key releases only matter to the script layer (`keyup`); most
                // terminals never report them, so the built-in shortcuts stay
                // idle here.
                if arena.is_some() {
                    msgs.push(ScriptMsg::KeyUp {
                        target: state.focused_input,
                        key: key_event_info(&key),
                    });
                }
                return cmds;
            }

            // Every press / auto-repeat also reaches the script layer as
            // `keydown` (targeted at the focused `<input>`, or the document
            // `<body>` when nothing is focused). `key_id` correlates the
            // response back so a page `preventDefault()` can cancel the key's
            // built-in shortcut (scroll / zoom / quit, gated below).
            if arena.is_some() {
                state.next_key_id = state.next_key_id.wrapping_add(1);
                msgs.push(ScriptMsg::KeyDown {
                    key_id: state.next_key_id,
                    target: state.focused_input,
                    key: key_event_info(&key),
                });
            }

            // An open dropdown overlay consumes navigation keys: ↑ / ↓ move
            // the highlight (skipping `<optgroup>` headers), `Enter` picks the
            // highlighted option (a `<select>` commits it, an `<input list>`
            // fills the field), `Esc` closes. A plain `<select>` owns the
            // keyboard while open — any other key closes it and is swallowed.
            // A datalist autocomplete instead lets the key edit the field
            // (fall through to the input block, which re-filters).
            if let Some(open) = state.open_select.as_ref() {
                let Some(arena) = arena else {
                    return cmds;
                };
                let node_id = open.node_id;
                let kind = open.kind;
                match key.code {
                    KeyCode::Up | KeyCode::Down => {
                        let dir: i32 = if key.code == KeyCode::Up { -1 } else { 1 };
                        if let Some(open) = &mut state.open_select {
                            let items = select_item_indices(&open.rows);
                            if !items.is_empty() {
                                let pos =
                                    items.iter().position(|&i| i == open.highlight).unwrap_or(0);
                                let next = items[(((pos as i32) + dir)
                                    .rem_euclid(items.len() as i32))
                                    as usize];
                                open.highlight = next;
                                cmds.push(RenderCmd::SelectMove {
                                    node_id,
                                    highlight: next,
                                });
                            }
                        }
                        return cmds;
                    }
                    KeyCode::Enter => {
                        if kind == OverlayKind::Select {
                            let pick = state.open_select.as_ref().map(|o| o.highlight);
                            close_select(state, arena, pick, msgs, &mut cmds);
                        } else {
                            // Datalist: fill the field with the highlighted
                            // suggestion and keep focus (the value commits on
                            // blur / Enter as usual).
                            let pick_value = state.open_select.as_ref().and_then(|o| {
                                o.options.get(o.highlight).map(|opt| opt.value.clone())
                            });
                            if let Some(open) = state.open_select.take() {
                                cmds.push(RenderCmd::SelectClose {
                                    node_id: open.node_id,
                                });
                            }
                            if let Some(value) = pick_value {
                                state.input_value = value;
                                state.input_cursor = state.input_value.chars().count();
                                state.input_sel_anchor = None;
                                push_input_render(state, node_id, &mut cmds);
                                msgs.push(ScriptMsg::InputKeystroke { node_id });
                            }
                        }
                        return cmds;
                    }
                    KeyCode::Esc => {
                        close_select(state, arena, None, msgs, &mut cmds);
                        return cmds;
                    }
                    _ => {
                        close_select(state, arena, None, msgs, &mut cmds);
                        if kind == OverlayKind::Select {
                            return cmds;
                        }
                    }
                }
            }

            // A focused `<input>` consumes the keys it owns: text editing and
            // caret movement. `Esc` blurs it and `Enter` confirms (blur +
            // commit) — both write the value back to the DOM. A focused
            // `<textarea>` owns `Enter` (newline) and `↑`/`↓` (caret) too.
            // Keys the field does *not* own (page navigation such as
            // PageDown / PageUp) fall through to the page-level handler
            // below, matching a browser where an input only intercepts the
            // keys it edits.
            if state.focused_input.is_some() {
                let Some(focused) = state.focused_input else {
                    return cmds;
                };
                // `Esc` blurs; `Enter` on a single-line field commits and —
                // when the field lives in a `<form>` — submits it (the
                // browser's implicit submission). A focused `<textarea>`
                // owns `Enter` (newline) instead.
                if key.code == KeyCode::Esc
                    || (key.code == KeyCode::Enter && !state.focused_is_textarea)
                {
                    if key.code == KeyCode::Enter
                        && let Some(form_id) = focused_form(arena, focused)
                    {
                        msgs.push(ScriptMsg::FormSubmit { form_id });
                    }
                    blur_input(state, msgs, &mut cmds);
                    return cmds;
                }
                let content_width = textarea_content_width(arena, focused);

                // A focused range owns its discrete value changes. All other
                // field editing keys are swallowed so typing cannot corrupt a
                // numeric slider value; page navigation is also suppressed
                // because the slider has keyboard focus.
                if state.focused_is_range {
                    let Some(arena) = arena else {
                        return cmds;
                    };
                    if apply_range_key(state, arena, focused, key.code) {
                        state.input_sel_anchor = None;
                        push_input_render(state, focused, &mut cmds);
                        msgs.push(ScriptMsg::RangeInput {
                            node_id: focused,
                            value: state.input_value.clone(),
                        });
                    }
                    return cmds;
                }

                // Word-level editing: Ctrl / Alt + ←/→/Home/End/Backspace/
                // Delete, with optional Shift extending the selection. Must
                // run before the plain Ctrl shortcuts so Ctrl+Left is a word
                // move rather than an ignored combo. A readonly field keeps
                // the navigation keys but drops the deletion keys.
                if (key.modifiers.contains(event::KeyModifiers::CONTROL)
                    || key.modifiers.contains(event::KeyModifiers::ALT))
                    && matches!(
                        key.code,
                        KeyCode::Left
                            | KeyCode::Right
                            | KeyCode::Home
                            | KeyCode::End
                            | KeyCode::Backspace
                            | KeyCode::Delete
                    )
                {
                    let mut code = key.code;
                    if is_edit_blocked(arena, focused)
                        && matches!(code, KeyCode::Backspace | KeyCode::Delete)
                    {
                        code = KeyCode::Null;
                    }
                    if apply_input_word_key(
                        &mut state.input_value,
                        &mut state.input_cursor,
                        &mut state.input_sel_anchor,
                        code,
                        key.modifiers,
                    ) {
                        push_input_render(state, focused, &mut cmds);
                        if arena.is_some() {
                            msgs.push(ScriptMsg::InputKeystroke { node_id: focused });
                        }
                        refresh_datalist(state, hit_map, arena, &mut cmds);
                    }
                    return cmds;
                }

                // Ctrl shortcuts: select-all / copy / cut / paste. Other Ctrl
                // combos are ignored (not inserted as characters — browser
                // behaviour). A readonly field keeps select-all / copy but
                // drops cut / paste.
                if key.modifiers.contains(event::KeyModifiers::CONTROL) {
                    let mut code = key.code;
                    if is_edit_blocked(arena, focused)
                        && matches!(code, KeyCode::Char('x') | KeyCode::Char('v'))
                    {
                        code = KeyCode::Null;
                    }
                    if apply_ctrl_shortcut(state, code) {
                        push_input_render(state, focused, &mut cmds);
                        if arena.is_some() {
                            msgs.push(ScriptMsg::InputKeystroke { node_id: focused });
                        }
                        refresh_datalist(state, hit_map, arena, &mut cmds);
                    }
                    return cmds;
                }

                // Shift + navigation extends / shrinks the selection.
                if key.modifiers.contains(event::KeyModifiers::SHIFT) {
                    if apply_shift_navigation(state, key.code) {
                        push_input_render(state, focused, &mut cmds);
                    }
                    return cmds;
                }

                // `<textarea>`-owned keys: Enter inserts a newline, ↑ / ↓
                // move the caret between visual lines (keeping the column).
                if state.focused_is_textarea
                    && matches!(key.code, KeyCode::Enter | KeyCode::Up | KeyCode::Down)
                {
                    if apply_textarea_key(
                        &mut state.input_value,
                        &mut state.input_cursor,
                        content_width,
                        key.code,
                    ) {
                        state.input_sel_anchor = None;
                        push_input_render(state, focused, &mut cmds);
                        if arena.is_some() {
                            msgs.push(ScriptMsg::InputKeystroke { node_id: focused });
                        }
                    }
                    return cmds;
                }

                // Field-owned keys stay in the field even when they are
                // boundary no-ops (Home at the start must not scroll the
                // page). Everything else belongs to the page-level handler.
                if is_input_field_key(key.code) {
                    // `<input type=number>` fields only accept the numeric
                    // character set (digit, sign, decimal point, exponent) —
                    // mirroring the browser's number editor, which rejects
                    // other characters at the keystroke level.
                    let mut code = key.code;
                    if !state.focused_is_textarea
                        && is_number_input(arena, focused)
                        && let KeyCode::Char(c) = code
                        && !number_input_allows_char(c)
                    {
                        code = KeyCode::Null;
                    }
                    // A readonly field keeps caret navigation but rejects the
                    // editing keys (insert / delete) — its value is fixed.
                    if is_edit_blocked(arena, focused)
                        && matches!(
                            code,
                            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete
                        )
                    {
                        code = KeyCode::Null;
                    }
                    // Plain editing keys clear the selection.
                    let changed = if state.focused_is_textarea {
                        apply_textarea_key(
                            &mut state.input_value,
                            &mut state.input_cursor,
                            content_width,
                            code,
                        )
                    } else {
                        apply_input_key(&mut state.input_value, &mut state.input_cursor, code)
                    };
                    if changed {
                        state.input_sel_anchor = None;
                        push_input_render(state, focused, &mut cmds);
                        if arena.is_some() {
                            msgs.push(ScriptMsg::InputKeystroke { node_id: focused });
                        }
                        refresh_datalist(state, hit_map, arena, &mut cmds);
                    }
                    return cmds;
                }
                // Fall through to the page-level key handling below.
            }

            // Page-level built-ins: no dropdown / field consumed the key. With
            // the script layer active, scroll / zoom / quit keys are deferred
            // pending the `keydown` gate — a page `preventDefault()` cancels
            // them, a gate timeout (busy or blocked script thread) falls back
            // to running them so input stays responsive. `Tab` focus cycling
            // is input-control behaviour and always runs immediately.
            let builtin = page_level_builtins(state, key, rows, max_scroll, arena, msgs);
            if arena.is_some()
                && !builtin.is_empty()
                && !matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
            {
                state.key_gate = Some(KeyGate {
                    id: state.next_key_id,
                    cmds: builtin,
                });
                return cmds;
            }
            cmds.extend(builtin);
            cmds
        }

        Ok(Event::Mouse(mouse)) => {
            let ctrl = mouse.modifiers.contains(event::KeyModifiers::CONTROL);
            state.last_mouse = Some((mouse.column, mouse.row));
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    // Scrolling dismisses an open dropdown (focus loss).
                    if state.open_select.is_some()
                        && let Some(arena) = arena
                    {
                        close_select(state, arena, None, msgs, &mut cmds);
                    }
                    if ctrl {
                        return cmds;
                    }
                    if let (Some(hit_map), Some(arena)) = (hit_map, arena)
                        && let Some(cmd) = scroll_at_wheel(
                            state,
                            hit_map,
                            arena,
                            mouse.column as u32,
                            mouse.row as u32 + state.scroll_y,
                            true,
                        )
                    {
                        cmds.push(cmd);
                        return cmds;
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
                    // Scrolling dismisses an open dropdown (focus loss).
                    if state.open_select.is_some()
                        && let Some(arena) = arena
                    {
                        close_select(state, arena, None, msgs, &mut cmds);
                    }
                    if ctrl {
                        return cmds;
                    }
                    if let (Some(hit_map), Some(arena)) = (hit_map, arena)
                        && let Some(cmd) = scroll_at_wheel(
                            state,
                            hit_map,
                            arena,
                            mouse.column as u32,
                            mouse.row as u32 + state.scroll_y,
                            false,
                        )
                    {
                        cmds.push(cmd);
                        return cmds;
                    }
                    if state.scroll_y >= max_scroll {
                        return cmds;
                    }
                    state.scroll_y = state.scroll_y.saturating_add(SCROLL_LINES_PER_TICK);
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                MouseEventKind::Down(button) => {
                    if let (Some(hit_map), Some(arena)) = (hit_map, arena) {
                        state.pressed = Some((mouse.column, mouse.row));
                        let (x, y) = canvas_pos(state, mouse.column, mouse.row);
                        handle_mouse_down(state, hit_map, arena, x, y, button, msgs, &mut cmds);
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    if let (Some(hit_map), Some(arena)) = (hit_map, arena) {
                        // Clears the press, re-validates the hover at the
                        // release position (so a press-and-drag-away button
                        // does not stay `:hover`-highlighted), and dispatches
                        // the click / dropdown pick for a left release.
                        handle_mouse_up(
                            state,
                            hit_map,
                            arena,
                            (mouse.column, mouse.row),
                            MouseButton::Left,
                            msgs,
                            &mut cmds,
                        );
                    } else {
                        state.pressed = None;
                        state.drag_selecting = false;
                    }
                }
                MouseEventKind::Up(btn @ (MouseButton::Right | MouseButton::Middle)) => {
                    // Block selection (right button) ends here; the selection
                    // itself is kept. No click is dispatched for these
                    // buttons, but the hover still re-validates (it froze
                    // during the drag).
                    if let (Some(hit_map), Some(arena)) = (hit_map, arena) {
                        handle_mouse_up(
                            state,
                            hit_map,
                            arena,
                            (mouse.column, mouse.row),
                            btn,
                            msgs,
                            &mut cmds,
                        );
                    } else {
                        state.pressed = None;
                        state.drag_selecting = false;
                    }
                }
                // crossterm reports button-held motion as `Drag` (SGR code
                // 32..34) and button-free motion as `Moved` (SGR 64) — both
                // must reach the drag / hover handler. A `Moved` event also
                // proves no button is held: a press whose release was lost
                // (the console can drop the `Up` during a resize) is dropped
                // here, otherwise every later move is misread as a drag and
                // hover / click events stay silent forever.
                MouseEventKind::Drag(_) | MouseEventKind::Moved => {
                    if let (Some(hit_map), Some(arena)) = (hit_map, arena) {
                        if !matches!(mouse.kind, MouseEventKind::Drag(_)) {
                            state.drop_stale_press();
                        }
                        let (x, y) = canvas_pos(state, mouse.column, mouse.row);
                        handle_mouse_move(state, hit_map, arena, x, y, msgs, &mut cmds);
                    }
                }
                _ => {}
            }
            cmds
        }

        Ok(Event::Resize(_, _new_rows)) => {
            // A terminal resize on Windows can reset the console input mode
            // (mouse capture off, keyboard line-buffered); re-apply it right
            // away. The run loop also polls the size every iteration as a
            // fallback for resize paths that emit no event.
            reapply_console_modes();
            // A resize invalidates every canvas coordinate the input loop
            // caches (a stuck press would kill hover / click, a stale
            // selection or dropdown would paint wrong cells), and the hover
            // target must be re-validated against the rebuilt hit map.
            state.invalidate_viewport(&mut cmds);
            state.request_resize_revalidation();
            // The render thread re-lays-out against the new size and clamps
            // its own scroll position; mirror the clamp here (to the last
            // known ceiling) so the scroll sent with the redraw matches, and
            // the run loop re-syncs to the freshly computed ceiling on the
            // next iteration.
            state.clamp_scroll_to(max_scroll);
            cmds.push(RenderCmd::Redraw {
                scroll_y: state.scroll_y,
                zoom: state.zoom,
            });
            cmds
        }

        Ok(Event::Paste(pasted)) => {
            // Bracketed paste (enabled at startup): the terminal delivers the
            // system clipboard verbatim. Insert it at the caret of the focused
            // field — `<textarea>` keeps newlines, single-line fields collapse
            // them — then fire `input` and repaint. Nothing to do when no
            // field is focused (the terminal already showed the pasted text)
            // or the field is readonly.
            if let Some(focused) = state.focused_input
                && !is_edit_blocked(arena, focused)
                && paste_into_field(state, &pasted, focused, &mut cmds)
                && arena.is_some()
            {
                msgs.push(ScriptMsg::InputKeystroke { node_id: focused });
            }
            cmds
        }

        _ => cmds,
    }
}
