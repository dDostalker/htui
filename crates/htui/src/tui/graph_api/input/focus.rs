use crate::script::ScriptMsg;
use crate::tdom::node::{NodeArena, NodeId, NodeType};

use super::InputState;
use super::cmd::RenderCmd;
use super::edit::selection_range;

/// Focus `node_id` (an `<input>`), seeding the edit buffer from the DOM
/// `value` attribute and repainting the previously focused field (if any).
/// Queues `focus` on the newly focused field and `blur` / `change` on the
/// previously focused one as script messages.
pub(super) fn focus_input(
    state: &mut InputState,
    node_id: NodeId,
    arena: &NodeArena,
    msgs: &mut Vec<ScriptMsg>,
    cmds: &mut Vec<RenderCmd>,
) {
    if state.focused_input == Some(node_id) {
        return;
    }
    state.focused_is_textarea = arena
        .get(node_id)
        .is_some_and(|n| n.node_type == NodeType::Textarea);
    if let Some(prev) = state.focused_input.replace(node_id) {
        // Commit the previous field's edits back to the DOM before switching
        // focus, firing `blur` (+ `change` when the value differs).
        msgs.push(ScriptMsg::BlurInput {
            node_id: prev,
            value: state.input_value.clone(),
        });
        state.input_sel_anchor = None;
        cmds.push(RenderCmd::RenderInput {
            node_id: prev,
            value: std::mem::take(&mut state.input_value),
            cursor: std::mem::take(&mut state.input_cursor),
            focused: false,
            selection: None,
        });
    }
    // `<input>` fields read the DOM `value` attribute; a `<textarea>`'s
    // initial value is its text content.
    let value = match arena.get(node_id) {
        Some(n) if n.node_type == NodeType::Textarea => n.text_content(arena),
        Some(n) => n.attributes.get("value").cloned().unwrap_or_default(),
        None => String::new(),
    };
    let cursor = value.chars().count();
    state.input_value = value;
    state.input_cursor = cursor;
    state.input_sel_anchor = None;
    cmds.push(RenderCmd::RenderInput {
        node_id,
        value: state.input_value.clone(),
        cursor,
        focused: true,
        selection: None,
    });
    // Re-cascade CSS `:focus` rules for the newly focused field.
    cmds.push(RenderCmd::Focus {
        node_id: Some(node_id),
    });
    // Fire `focus` on the newly focused field.
    msgs.push(ScriptMsg::FocusInput { node_id });
}

/// Blur the focused `<input>`: repaint it unfocused, commit the edited value
/// back into the DOM (so a later focus or re-render sees it), and clear the
/// edit session. The commit and the `blur` / `change` events themselves run on
/// the script thread via [`ScriptMsg::BlurInput`].
pub(super) fn blur_input(
    state: &mut InputState,
    msgs: &mut Vec<ScriptMsg>,
    cmds: &mut Vec<RenderCmd>,
) {
    let Some(focused) = state.focused_input.take() else {
        return;
    };
    state.focused_is_textarea = false;
    state.input_sel_anchor = None;
    msgs.push(ScriptMsg::BlurInput {
        node_id: focused,
        value: state.input_value.clone(),
    });
    cmds.push(RenderCmd::RenderInput {
        node_id: focused,
        value: std::mem::take(&mut state.input_value),
        cursor: std::mem::take(&mut state.input_cursor),
        focused: false,
        selection: None,
    });
    // Clear the CSS `:focus` highlight.
    cmds.push(RenderCmd::Focus { node_id: None });
}

/// Queue a repaint of the focused field with its current edit state.
pub(super) fn push_input_render(state: &InputState, focused: NodeId, cmds: &mut Vec<RenderCmd>) {
    cmds.push(RenderCmd::RenderInput {
        node_id: focused,
        value: state.input_value.clone(),
        cursor: state.input_cursor,
        focused: true,
        selection: state
            .input_sel_anchor
            .map(|anchor| selection_range(anchor, state.input_cursor)),
    });
}

/// Filter JS-initiated focus changes out of `cmds` and apply them to the
/// focus state.
///
/// [`RenderCmd::FocusRequest`] / [`RenderCmd::BlurRequest`] travel from the
/// script thread through the ordinary render-command channel (`el.focus()` /
/// `el.blur()`), but they target input-loop state: the edit session lives in
/// [`InputState`], and the `focus` / `blur` (+ `change`) events are routed
/// back through the script-message path so both sides stay in sync. Each
/// request is expanded into the focus repaints (`RenderInput`) plus the
/// `BlurInput` / `FocusInput` script messages for `msgs`; everything else
/// passes through untouched. The returned commands are the ones that should
/// reach the render thread.
pub fn apply_focus_requests(
    state: &mut InputState,
    cmds: Vec<RenderCmd>,
    arena: Option<&NodeArena>,
    msgs: &mut Vec<ScriptMsg>,
) -> Vec<RenderCmd> {
    let mut out: Vec<RenderCmd> = Vec::with_capacity(cmds.len());
    for cmd in cmds {
        match cmd {
            RenderCmd::FocusRequest { node_id } => {
                // `focus()` on an already-focused field is a no-op (browsers
                // do not re-fire `focus`); `focus_input` early-returns.
                if let Some(arena) = arena {
                    focus_input(state, node_id, arena, msgs, &mut out);
                }
            }
            RenderCmd::BlurRequest { node_id } => {
                // `blur()` only takes effect on the focused element.
                if state.focused_input == Some(node_id) {
                    blur_input(state, msgs, &mut out);
                }
            }
            other => out.push(other),
        }
    }
    out
}
