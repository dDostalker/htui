use crate::events::HitMap;
use crate::layout::layoutbox::geometry::{
    SelectRow, overlay_options, select_overlay_rect, select_rows,
};
use crate::script::ScriptMsg;
use crate::tdom::node::{NodeArena, NodeId, NodeType};

use super::InputState;
use super::cmd::RenderCmd;

/// What an open dropdown overlay is attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    /// A `<select>` — picking an option commits its value and fires `change`.
    Select,
    /// An `<input list="…">` autocomplete — picking fills the field.
    Datalist,
}

/// Canvas-space state of an open dropdown overlay, held by the input loop so
/// clicks and keyboard navigation can be resolved without asking the render
/// thread.
#[derive(Debug, Clone)]
pub struct SelectOpenState {
    /// The owning `<select>` / `<input>` node id.
    pub node_id: NodeId,
    /// What the overlay is attached to (select vs input autocomplete).
    pub kind: OverlayKind,
    /// Overlay rectangle (canvas coordinates, directly below the field).
    pub rect: crate::layout::Rect,
    /// The flat option list (`value`, `label`, optional group).
    pub options: Vec<crate::layout::layoutbox::geometry::SelectOption>,
    /// The dropdown rows (group headers interleaved with options).
    pub rows: Vec<crate::layout::layoutbox::geometry::SelectRow>,
    /// Index of the currently selected option.
    pub selected: usize,
    /// Index highlighted in the overlay (keyboard / hover cursor).
    pub highlight: usize,
}

/// The overlay option row containing the canvas point `(x, y)`, if any.
pub(super) fn hit_select_option(open: &SelectOpenState, x: u32, y: u32) -> Option<usize> {
    if y >= open.rect.y as u32
        && y < open.rect.y as u32 + open.rect.height as u32
        && x >= open.rect.x as u32
        && x < open.rect.x as u32 + open.rect.width as u32
    {
        Some((y - open.rect.y as u32) as usize)
    } else {
        None
    }
}

/// Resolve a dropdown row click to an option index.
///
/// Option rows map to themselves; `<optgroup>` header rows map to the first
/// option of that group — a terminal convenience so every visible row is
/// actionable (the browser would ignore a header click). `None` when the row
/// is out of range.
pub(super) fn row_option(rows: &[SelectRow], row: usize) -> Option<usize> {
    match rows.get(row)? {
        SelectRow::Item(i) => Some(*i),
        SelectRow::Group(_) => rows[row..].iter().find_map(|r| match r {
            SelectRow::Item(i) => Some(*i),
            SelectRow::Group(_) => None,
        }),
    }
}

/// Open the dropdown overlay for `node_id`: resolve the options (a
/// `<select>`'s children, or an `<input list>`'s `<datalist>` filtered by
/// `filter`), the rows and overlay geometry from the DOM, remember them on the
/// input loop, and ask the render thread to paint the overlay.
fn open_overlay(
    state: &mut InputState,
    hit_map: &HitMap,
    arena: &NodeArena,
    node_id: NodeId,
    kind: OverlayKind,
    filter: Option<&str>,
    cmds: &mut Vec<RenderCmd>,
) {
    let Some(region) = hit_map.iter().find(|r| r.node_id == node_id) else {
        return;
    };
    let options = overlay_options(arena, node_id, filter);
    let rows = select_rows(&options);
    let selected = match kind {
        OverlayKind::Select => {
            let value = arena
                .get(node_id)
                .and_then(|n| n.attributes.get("value"))
                .cloned()
                .unwrap_or_default();
            options.iter().position(|o| o.value == value).unwrap_or(0)
        }
        OverlayKind::Datalist => 0,
    };
    let rect = select_overlay_rect(&options, &rows, region.rect);
    state.open_select = Some(SelectOpenState {
        node_id,
        kind,
        rect,
        options,
        rows,
        selected,
        highlight: selected,
    });
    cmds.push(RenderCmd::SelectOpen {
        node_id,
        filter: filter.map(str::to_string),
    });
}

/// Open the `<select>` dropdown for `node_id` (see [`open_overlay`]).
pub(super) fn open_select(
    state: &mut InputState,
    hit_map: &HitMap,
    arena: &NodeArena,
    node_id: NodeId,
    cmds: &mut Vec<RenderCmd>,
) {
    open_overlay(
        state,
        hit_map,
        arena,
        node_id,
        OverlayKind::Select,
        None,
        cmds,
    );
}

/// Open / refresh the autocomplete dropdown of a focused `<input list>`.
///
/// The overlay tracks the field's current text: an empty value shows every
/// `<datalist>` suggestion (the browser's focus behavior), otherwise the
/// options are filtered by the current text (case-insensitive substring).
/// `hit_map` / `arena` being `None` (tests / hosts that disable interaction)
/// skips the overlay entirely.
pub(super) fn refresh_datalist(
    state: &mut InputState,
    hit_map: Option<&HitMap>,
    arena: Option<&NodeArena>,
    cmds: &mut Vec<RenderCmd>,
) {
    let Some(focused) = state.focused_input else {
        return;
    };
    let (Some(hit_map), Some(arena)) = (hit_map, arena) else {
        return;
    };
    let has_list = arena
        .get(focused)
        .is_some_and(|n| n.node_type == NodeType::Input && n.attributes.contains_key("list"));
    if !has_list {
        if let Some(open) = state.open_select.take() {
            cmds.push(RenderCmd::SelectClose {
                node_id: open.node_id,
            });
        }
        return;
    }
    let filter = state.input_value.clone();
    open_overlay(
        state,
        hit_map,
        arena,
        focused,
        OverlayKind::Datalist,
        Some(&filter),
        cmds,
    );
}

/// Close the open `<select>` dropdown. When `pick` is `Some`, the option is
/// selected: its value is committed to the DOM (`set_value_dirty`) and a
/// `change` event fires when the value actually changed; the commit + event
/// run on the script thread via [`ScriptMsg::SelectPick`]. `pick: None` just
/// closes without changing the value.
pub(super) fn close_select(
    state: &mut InputState,
    arena: &NodeArena,
    pick: Option<usize>,
    msgs: &mut Vec<ScriptMsg>,
    cmds: &mut Vec<RenderCmd>,
) {
    let Some(open) = state.open_select.take() else {
        return;
    };
    cmds.push(RenderCmd::SelectClose {
        node_id: open.node_id,
    });
    if let Some(idx) = pick
        && idx < open.options.len()
    {
        let value = open.options[idx].value.clone();
        let old = arena
            .get(open.node_id)
            .and_then(|n| n.attributes.get("value"))
            .cloned()
            .unwrap_or_default();
        if value != old {
            msgs.push(ScriptMsg::SelectPick {
                node_id: open.node_id,
                value,
            });
        }
    }
}
