use crossterm::event::MouseButton;

use crate::events::HitMap;
use crate::script::ScriptMsg;
use crate::tdom::node::{NodeArena, NodeId, NodeType};

use super::InputState;
use super::TEXT_SEL_DRAG_THRESHOLD;
use super::cmd::{RenderCmd, SelectionMode};
use super::field::{
    blocks_page_selection, char_index_at_x, clear_text_selection, is_button_input,
    is_disabled_control, is_range_input, is_text_field, is_textarea_node, is_toggle_input,
    set_range_at_pointer, textarea_index_at,
};
use super::focus::{blur_input, focus_input, push_input_render};
use super::scroll_anim::{ScrollTarget, active_scroll_target, step_target};
use super::select::{close_select, hit_select_option, open_select, refresh_datalist, row_option};

/// Handle a completed left-button click at canvas coordinates `(x, y)`.
///
/// Queues the `Click` dispatch (the script thread runs it via
/// [`interaction::on_click`](crate::events::interaction::on_click), which also syncs the hover state) and then
/// applies the field-focus rule used by the input loop: a click on an
/// `<input>` focuses it (blurring the previously focused field), any other
/// click blurs the focused field. `arena` is the input loop's latest DOM
/// snapshot for the read-only node-type / value lookups; everything that
/// needs the engine is collected into `msgs` for the script thread. Local
/// render commands are appended to `cmds` in dispatch order.
pub fn handle_click(
    state: &mut InputState,
    hit_map: &HitMap,
    arena: &NodeArena,
    x: u32,
    y: u32,
    msgs: &mut Vec<ScriptMsg>,
    cmds: &mut Vec<RenderCmd>,
) {
    let clicked = hit_map.hit_test(x, y);
    // A disabled form control swallows the click entirely (no Click
    // dispatch, no focus change) — matching browser behaviour.
    if clicked.is_some_and(|r| is_disabled_control(Some(arena), r.node_id)) {
        return;
    }
    msgs.push(ScriptMsg::Click { x, y });
    // Clicking an `<input>` / `<textarea>` focuses it; a click on a `<label>`
    // delegates to its associated control (`for` or a wrapped element). Any
    // other click blurs the previously focused field.
    let click_node = resolve_click_node(arena, clicked.map(|r| r.node_id));
    let clicked_input = click_node.is_some_and(|id| is_text_field(arena, id));
    if clicked_input {
        if let Some(node_id) = click_node {
            if is_toggle_input(arena, node_id) {
                // A checkbox / radio toggle: no text focus, but clicking it
                // still blurs any previously focused text field.
                if state.focused_input.is_some() {
                    blur_input(state, msgs, cmds);
                }
                msgs.push(ScriptMsg::ToggleCheck { node_id });
            } else if is_button_input(arena, node_id) {
                // A button input (`button` / `submit` / `reset`) activates on
                // click — the `Click` dispatch already queued above, and the
                // script thread runs the form submit / reset default action.
                // It never takes text focus, but blurs any focused field.
                if state.focused_input.is_some() {
                    blur_input(state, msgs, cmds);
                }
            } else if is_range_input(arena, node_id) {
                focus_input(state, node_id, arena, msgs, cmds);
                if let Some(region) = clicked
                    && set_range_at_pointer(state, arena, node_id, region.rect, x, y)
                {
                    push_input_render(state, node_id, cmds);
                    msgs.push(ScriptMsg::RangeInput {
                        node_id,
                        value: state.input_value.clone(),
                    });
                }
            } else {
                focus_input(state, node_id, arena, msgs, cmds);
                // Clicking an `<input list>` opens its autocomplete dropdown
                // (browser behavior: focus shows the suggestions).
                refresh_datalist(state, Some(hit_map), Some(arena), cmds);
            }
        }
    } else if state.focused_input.is_some() {
        blur_input(state, msgs, cmds);
    }
}

/// Apply a wheel tick to the nearest native scroll container under `x/y`.
///
/// With smooth scrolling enabled the tick returns a `ScrollAnimate` marker
/// for the run loop's eased animation instead of moving the container here.
pub(super) fn scroll_at_wheel(
    state: &mut InputState,
    hit_map: &HitMap,
    arena: &NodeArena,
    x: u32,
    y: u32,
    up: bool,
) -> Option<RenderCmd> {
    let hit = hit_map.hit_test(x, y)?;
    let scroll = hit_map.scroll_container(Some(hit.node_id), arena)?;
    let current = state
        .scroll_offsets
        .get(&scroll.node_id)
        .copied()
        .unwrap_or(scroll.scroll_y);
    if state.smooth_scroll {
        let base =
            active_scroll_target(state, ScrollTarget::Container(scroll.node_id)).unwrap_or(current);
        let delta = if up {
            -(super::SCROLL_LINES_PER_TICK as i32)
        } else {
            super::SCROLL_LINES_PER_TICK as i32
        };
        return step_target(base, delta, 0, scroll.max_scroll).map(|to| {
            state.show_scrollbar(scroll.node_id);
            RenderCmd::ScrollAnimate {
                target: ScrollTarget::Container(scroll.node_id),
                to,
            }
        });
    }
    let next = if up {
        current.saturating_sub(super::SCROLL_LINES_PER_TICK)
    } else {
        current
            .saturating_add(super::SCROLL_LINES_PER_TICK)
            .min(scroll.max_scroll)
    };
    if next == current {
        return None;
    }
    state.show_scrollbar(scroll.node_id);
    state.scroll_offsets.insert(scroll.node_id, next);
    Some(RenderCmd::RenderScroll {
        node_id: scroll.node_id,
        scroll_y: next,
    })
}

/// Reveal a scrollbar when the pointer is over its track, even while idle.
///
/// The layout geometry remains in the hit map while chrome is hidden, so the
/// track is the hover affordance that brings it back. A same-offset scroll
/// update is harmless when the chrome is already visible and performs the
/// reveal repaint when it is hidden.
pub(super) fn reveal_scrollbar_at(
    state: &mut InputState,
    hit_map: &HitMap,
    x: u32,
    y: u32,
    cmds: &mut Vec<RenderCmd>,
) {
    if let Some(scroll) = hit_map
        .scrollbars
        .values()
        .find(|scroll| scroll.track.contains(x, y))
    {
        state.show_scrollbar(scroll.node_id);
        let offset = state
            .scroll_offsets
            .get(&scroll.node_id)
            .copied()
            .unwrap_or(scroll.scroll_y);
        cmds.push(RenderCmd::RenderScroll {
            node_id: scroll.node_id,
            scroll_y: offset,
        });
    }
}

/// End a button press at the release position (screen coordinates in
/// `release_screen`).
///
/// Clears the pressed / drag state, then re-validates the hover at the
/// release position: hover freezes while a button is held (button-free moves
/// are the only thing that update it), so a press-and-drag-away release must
/// not leave the pressed element `:hover`-highlighted — the highlight residue
/// after a button press. A left release within the shared drag threshold on
/// the same resolved element dispatches the click (and the dropdown pick, when
/// the release lands on an open `<select>` overlay row). A larger move is a
/// drag: it may leave a selection, but it never turns into a click.
pub fn handle_mouse_up(
    state: &mut InputState,
    hit_map: &HitMap,
    arena: &NodeArena,
    release_screen: (u16, u16),
    button: MouseButton,
    msgs: &mut Vec<ScriptMsg>,
    cmds: &mut Vec<RenderCmd>,
) {
    let range_drag = state.drag_selecting && state.field_drag_started && state.focused_is_range;
    let scroll_drag = state.scroll_drag.take();
    let pressed = state.pressed.take();
    let press_target = state.pressed_target.take();
    let press_origin = state.press_screen.take();
    state.drag_origin = None;
    state.drag_selecting = false;
    state.field_drag_started = false;
    let (x, y) = canvas_pos(state, release_screen.0, release_screen.1);
    // Re-validate the hover at the release position (see the doc comment).
    msgs.push(ScriptMsg::Move { x, y });
    if button != MouseButton::Left {
        return;
    }
    if scroll_drag.is_some() {
        return;
    }
    if pressed.is_none() {
        return;
    }
    // A range drag commits at the release coordinate as well as during moves;
    // this covers consoles that coalesce or drop the final drag event.
    if range_drag
        && let Some(focused) = state.focused_input
        && let Some(region) = hit_map.iter().find(|r| r.node_id == focused)
        && set_range_at_pointer(state, arena, focused, region.rect, x, y)
    {
        push_input_render(state, focused, cmds);
        msgs.push(ScriptMsg::RangeInput {
            node_id: focused,
            value: state.input_value.clone(),
        });
    }
    // Press-drag-release over a dropdown row picks it (the browser's native
    // `<select>` interaction), even when the release lands off the press
    // cell. Rows map to option indices like the mousedown pick above:
    // `<optgroup>` headers pick their first option.
    if let Some(open) = state.open_select.clone()
        && let Some(row) = hit_select_option(&open, x, y)
        && let Some(option_idx) = row_option(&open.rows, row)
    {
        close_select(state, arena, Some(option_idx), msgs, cmds);
        return;
    }
    // A click is a small jitter, not necessarily on the exact terminal cell.
    // The queued `Click` dispatch resolves its bubbling target from the
    // release hit path, so a tiny move onto an inline sibling reaches their
    // common ancestor like a browser. Once the shared threshold has been
    // crossed, the gesture is a drag: a selection may survive, but the release
    // must not also activate a button / field.
    let within_click_threshold = press_origin.is_some_and(|(px, py)| {
        u32::from(
            release_screen
                .0
                .abs_diff(px)
                .max(release_screen.1.abs_diff(py)),
        ) < TEXT_SEL_DRAG_THRESHOLD
    });
    if within_click_threshold && press_target.is_some() {
        handle_click(state, hit_map, arena, x, y, msgs, cmds);
    }
    // A real drag ended elsewhere: the selection (field-internal or page
    // text) is kept as-is.
}

/// Resolve the element a click should act on: a `<label>` hit resolves to
/// its associated control (`for` attribute, or the first wrapped labelable
/// element); every other hit resolves to itself. `None` = no element.
fn resolve_click_node(arena: &NodeArena, clicked: Option<NodeId>) -> Option<NodeId> {
    let id = clicked?;
    if arena
        .get(id)
        .is_some_and(|n| n.node_type == NodeType::Label)
    {
        Some(label_control(arena, id).unwrap_or(id))
    } else {
        Some(id)
    }
}

/// The document `<body>` (or root fallback), used when no box was hit.
fn find_body(arena: &NodeArena) -> Option<NodeId> {
    arena
        .iter()
        .find(|(_, node)| node.node_type == NodeType::Body)
        .map(|(id, _)| id)
        .or_else(|| {
            arena
                .iter()
                .find(|(_, node)| node.parent.is_none())
                .map(|(id, _)| id)
        })
}

/// The control a `<label>` activates: the element named by `for` (when it is
/// labelable), else the first labelable descendant (input / select /
/// textarea / button) in document order.
fn label_control(arena: &NodeArena, label_id: NodeId) -> Option<NodeId> {
    let label = &arena[label_id];
    if let Some(for_id) = label.attributes.get("for") {
        return arena.iter().find_map(|(id, n)| {
            if n.get_id().is_some_and(|i| i == for_id) && is_labelable(&n.node_type) {
                Some(id)
            } else {
                None
            }
        });
    }
    let mut stack: Vec<NodeId> = label.children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        let node = &arena[id];
        if is_labelable(&node.node_type) {
            return Some(id);
        }
        stack.extend(node.children.iter().rev().copied());
    }
    None
}

/// `true` for elements a `<label>` can activate.
fn is_labelable(node_type: &NodeType) -> bool {
    matches!(
        node_type,
        NodeType::Input
            | NodeType::Select
            | NodeType::Textarea
            | NodeType::Button
            | NodeType::Output
            | NodeType::Progress
            | NodeType::Meter
    )
}

/// Handle a left-button press at canvas coordinates `(x, y)`.
///
/// Browser-aligned mousedown behaviour: a press on an `<input>` focuses it
/// (blurring any previously focused field) and positions the caret at the
/// press point, entering drag-selection mode; a press anywhere else blurs the
/// focused field and defers the page text selection start until the pointer
/// actually drags (see [`handle_mouse_move`]) — a press alone never creates
/// a selection. Any prior page selection is cleared. `arena` is the input
/// loop's latest DOM snapshot; everything that needs the engine is collected
/// into `msgs` for the script thread. Local render commands are appended to
/// `cmds` in dispatch order.
pub fn handle_mouse_down(
    state: &mut InputState,
    hit_map: &HitMap,
    arena: &NodeArena,
    x: u32,
    y: u32,
    button: MouseButton,
    msgs: &mut Vec<ScriptMsg>,
    cmds: &mut Vec<RenderCmd>,
) {
    let clicked = hit_map.hit_test(x, y);
    // Record the stable press origin once: drag thresholds and click-vs-drag
    // decisions must use the original pointer cell, not an intermediate drag
    // position. The target starts clear so swallowed presses cannot click.
    state.press_screen = Some((
        x.min(u16::MAX as u32) as u16,
        y.saturating_sub(state.scroll_y).min(u16::MAX as u32) as u16,
    ));
    state.drag_origin = Some((x, y));
    state.pressed_target = None;
    state.field_drag_started = false;
    // A native scrollbar owns its press before text selection / controls.
    let scrollbar_hit = (button == MouseButton::Left)
        .then(|| {
            hit_map
                .scrollbars
                .values()
                .find(|scroll| scroll.track.contains(x, y))
                .map(|scroll| {
                    let grab_offset = y
                        .saturating_sub(scroll.thumb.y as u32)
                        .min(scroll.thumb.height as u32);
                    let offset = scroll.offset_at(y, grab_offset);
                    (scroll.node_id, grab_offset, offset)
                })
        })
        .flatten();
    if let Some((node_id, grab_offset, offset)) = scrollbar_hit {
        // Grabbing the thumb takes over from any glide in progress. A same-
        // offset command still reveals chrome that a previous timeout hid.
        state.scroll_anim = None;
        state.show_scrollbar(node_id);
        state.scroll_drag = Some((node_id, grab_offset));
        state.scroll_offsets.insert(node_id, offset);
        cmds.push(RenderCmd::RenderScroll {
            node_id,
            scroll_y: offset,
        });
        return;
    }

    // An open dropdown takes priority: a press on an option row picks it, a
    // press on an `<optgroup>` header does nothing (browsers keep the
    // dropdown open), a press on the field toggles it closed, any other press
    // closes it and then falls through to the normal dispatch.
    if let Some(open) = state.open_select.clone() {
        if let Some(row) = hit_select_option(&open, x, y) {
            // Option rows pick; `<optgroup>` headers pick their first option
            // (see [`row_option`]) — every visible row is actionable.
            if let Some(option_idx) = row_option(&open.rows, row) {
                close_select(state, arena, Some(option_idx), msgs, cmds);
            }
            return;
        }
        let clicked_self = clicked.is_some_and(|r| r.node_id == open.node_id);
        close_select(state, arena, None, msgs, cmds);
        if clicked_self {
            return;
        }
    }

    // A disabled form control swallows the press entirely: no focus, no
    // caret / drag-selection, no dropdown open — the mousedown analogue of
    // [`handle_click`]'s swallow rule (browsers ignore presses on disabled
    // controls). Placed after the open-dropdown block so an outside press
    // still closes a pending dropdown first.
    if clicked.is_some_and(|r| is_disabled_control(Some(arena), r.node_id)) {
        return;
    }

    // A click on a `<label>` delegates to its associated control; the caret /
    // drag-selection logic below only applies when the pointer is directly
    // over the field itself.
    let click_node = resolve_click_node(arena, clicked.map(|r| r.node_id));
    state.pressed_target = click_node;
    let direct_field_hit = clicked.is_some_and(|r| click_node == Some(r.node_id));
    let clicked_input = click_node.is_some_and(|id| is_text_field(arena, id));
    match button {
        MouseButton::Left => {
            if clicked_input {
                if let Some(node_id) = click_node {
                    if is_toggle_input(arena, node_id) {
                        // Checkbox / radio: the click handler performs the
                        // toggle — no text focus or drag selection here.
                    } else if is_button_input(arena, node_id) {
                        // Button input: the click handler activates it — no
                        // text focus or drag selection here.
                    } else if is_range_input(arena, node_id) {
                        focus_input(state, node_id, arena, msgs, cmds);
                        if let Some(region) = clicked {
                            state.drag_selecting = true;
                            if set_range_at_pointer(state, arena, node_id, region.rect, x, y) {
                                push_input_render(state, node_id, cmds);
                                msgs.push(ScriptMsg::RangeInput {
                                    node_id,
                                    value: state.input_value.clone(),
                                });
                            }
                        }
                    } else {
                        focus_input(state, node_id, arena, msgs, cmds);
                        // Pressing an `<input list>` opens its autocomplete
                        // dropdown (browser behavior: focus shows the
                        // suggestions).
                        refresh_datalist(state, Some(hit_map), Some(arena), cmds);
                        if direct_field_hit {
                            state.drag_selecting = true;
                            // Position the caret (and the selection anchor) at
                            // the press point within the field.
                            if let Some(region) = clicked {
                                let idx = if is_textarea_node(arena, node_id) {
                                    textarea_index_at(state, region.rect, x, y)
                                } else {
                                    let inner_x = region.rect.x;
                                    let inner_w = region.rect.width;
                                    char_index_at_x(
                                        &state.input_value,
                                        state.input_cursor,
                                        inner_w,
                                        (x as usize).saturating_sub(inner_x),
                                    )
                                };
                                state.input_sel_anchor = Some(idx);
                                state.input_cursor = idx;
                                push_input_render(state, node_id, cmds);
                            }
                        }
                    }
                }
            } else {
                // Mousedown outside a field blurs it (browser timing). The
                // flow text selection is deferred: browsers select only on a
                // real drag, so a jittery click leaves no residue. When the
                // flow copy feature is disabled the mode stays untouched, so
                // no selection can start.
                if state.focused_input.is_some() {
                    blur_input(state, msgs, cmds);
                }
                clear_text_selection(state, cmds);
                // Outside a laid-out box, the browser's implicit target is the
                // document body. Anchoring there still allows an upward drag
                // to select content, while normal box hits use their own
                // target's selection policy.
                let selection_node = click_node.or_else(|| find_body(arena));
                let selected = selection_node.is_some_and(|node_id| {
                    !blocks_page_selection(arena, node_id) && !hit_map.is_user_select_none(node_id)
                });
                if state.copy_flow && selected {
                    state.text_sel_mode = SelectionMode::Flow;
                    state.text_sel_armed = true;
                    state.drag_origin = Some((x, y));
                }
            }
        }
        // Right-drag starts a rectangular (block) selection; the focused field
        // and its caret stay untouched. Like the left flow selection it only
        // starts once the drag passes the threshold (see
        // [`handle_mouse_move`]); the press clears prior selections and
        // records the shape. When the block copy feature is disabled no
        // selection can start.
        MouseButton::Right => {
            clear_text_selection(state, cmds);
            if state.copy_block {
                state.text_sel_mode = SelectionMode::Block;
                state.text_sel_armed = true;
                state.drag_origin = Some((x, y));
            }
        }
        _ => {}
    }

    // A left press on a closed `<select>` opens its dropdown (blurring any
    // focused field, like a browser mousedown on a control).
    if button == MouseButton::Left
        && let Some(region) = clicked
        && arena
            .get(region.node_id)
            .is_some_and(|n| n.node_type == NodeType::Select)
    {
        if state.focused_input.is_some() {
            blur_input(state, msgs, cmds);
        }
        open_select(state, hit_map, arena, region.node_id, cmds);
    }
}

/// Handle a pointer move at canvas coordinates `(x, y)`.
///
/// While the left button is held this drives the active drag: a focused range
/// follows the pointer, other focused fields move their caret / character
/// selection, and over page content the text selection rectangle tracks the
/// pointer. A page text selection starts only once the drag has moved past
/// `TEXT_SEL_DRAG_THRESHOLD` cells from the press point — a press alone
/// never creates one. Without a press it queues a [`ScriptMsg::Move`] for the
/// script thread's hover machinery (pointer enter / leave + `MouseOver` /
/// `MouseOut`). Local render commands are appended to `cmds` in dispatch
/// order.
pub fn handle_mouse_move(
    state: &mut InputState,
    hit_map: &HitMap,
    arena: &NodeArena,
    x: u32,
    y: u32,
    msgs: &mut Vec<ScriptMsg>,
    cmds: &mut Vec<RenderCmd>,
) {
    // An open dropdown follows the pointer: hovering an option row moves the
    // highlight (group headers are skipped; the highlight stays put over them),
    // repainting only the overlay. This runs even while the button is held
    // (browser select: press-drag over the options) and short-circuits when
    // the highlight is unchanged.
    if let Some(row) = state
        .open_select
        .as_ref()
        .and_then(|open| hit_select_option(open, x, y))
        && let Some(idx) = state
            .open_select
            .as_ref()
            .and_then(|open| row_option(&open.rows, row))
        && state
            .open_select
            .as_ref()
            .is_some_and(|o| o.highlight != idx)
        && let Some(open) = &mut state.open_select
    {
        open.highlight = idx;
        cmds.push(RenderCmd::SelectMove {
            node_id: open.node_id,
            highlight: idx,
        });
    }

    if let Some((node_id, grab_offset)) = state.scroll_drag
        && let Some(scroll) = hit_map.scrollbars.get(&node_id)
    {
        state.show_scrollbar(node_id);
        let offset = scroll.offset_at(y, grab_offset);
        if state.scroll_offsets.get(&node_id) != Some(&offset) {
            state.scroll_offsets.insert(node_id, offset);
            cmds.push(RenderCmd::RenderScroll {
                node_id,
                scroll_y: offset,
            });
        }
        return;
    }

    if state.pressed.is_some() {
        // Dragging: the pointer is inside the field it started in, so the
        // selection stays within that field's rectangle.
        if state.drag_selecting {
            // Field selections use the same motion threshold as page selections:
            // a one-cell jiggle remains a click (the caret was already placed
            // by mousedown), not a character/range selection.
            if !state.field_drag_started {
                let Some((ax, ay)) = state.drag_origin else {
                    return;
                };
                if x.abs_diff(ax).max(y.abs_diff(ay)) < TEXT_SEL_DRAG_THRESHOLD {
                    return;
                }
                state.field_drag_started = true;
            }
            if let Some(focused) = state.focused_input
                && let Some(region) = hit_map.iter().find(|r| r.node_id == focused)
            {
                if state.focused_is_range {
                    if set_range_at_pointer(state, arena, focused, region.rect, x, y) {
                        push_input_render(state, focused, cmds);
                        msgs.push(ScriptMsg::RangeInput {
                            node_id: focused,
                            value: state.input_value.clone(),
                        });
                    }
                    return;
                }
                let idx = if state.focused_is_textarea {
                    textarea_index_at(state, region.rect, x, y)
                } else {
                    let inner_x = region.rect.x;
                    let inner_w = region.rect.width;
                    char_index_at_x(
                        &state.input_value,
                        state.input_cursor,
                        inner_w,
                        (x as usize).saturating_sub(inner_x),
                    )
                };
                state.input_cursor = idx;
                push_input_render(state, focused, cmds);
            }
        } else {
            // Page-content drag. The selection (flow for left, block for
            // right) starts only once the pointer has moved a real distance
            // from the press point — a jittery click (press, one-cell wiggle,
            // release) must not leave a highlighted character behind. The
            // anchor is the press point, so the selection starts where the
            // button went down (browser behaviour). A press whose copy feature
            // is disabled never armed the selection, so no drag can start one.
            if !state.text_sel_armed {
                return;
            }
            let anchor = match state.text_sel_anchor {
                Some(a) => a,
                None => {
                    let Some(&(ax, ay)) = state.drag_origin.as_ref() else {
                        return;
                    };
                    if x.abs_diff(ax).max(y.abs_diff(ay)) < TEXT_SEL_DRAG_THRESHOLD {
                        return;
                    }
                    state.text_sel_anchor = Some((ax, ay));
                    (ax, ay)
                }
            };
            state.text_sel_end = Some((x, y));
            cmds.push(RenderCmd::TextSelection {
                anchor: Some(anchor),
                end: Some((x, y)),
                mode: state.text_sel_mode,
            });
        }
        return;
    }
    // No button held: ordinary pointer movement → hover tracking on the
    // script thread.
    msgs.push(ScriptMsg::Move { x, y });
}

/// Convert a screen position to canvas coordinates by adding the scroll
/// offset. Zoom is not yet wired into layout, so no zoom scaling is applied.
pub(super) fn canvas_pos(state: &InputState, column: u16, row: u16) -> (u32, u32) {
    (column as u32, row as u32 + state.scroll_y)
}
