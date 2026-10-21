//! Shared option / row model for `<select>` and datalist dropdown overlays.

use compact_str::CompactString;

use crate::{
    graphics::char_width,
    layout::Rect,
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Read a `<select>`'s options as `(value, label)` pairs from its
/// `<option>` children. The `value` falls back to the option's text when the
/// `value` attribute is absent. Empty `value` attributes are treated as the
/// empty string (a browser option with `value=""`).
/// One dropdown option: the submitted value, the painted label, and the
/// `<optgroup label>` it belongs to (if any).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    pub value: CompactString,
    pub label: CompactString,
    pub group: Option<CompactString>,
}

/// One dropdown row: a non-selectable `<optgroup>` header or an option item
/// (the `usize` is the index into the flat option list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectRow {
    /// `<optgroup label>` header — dimmed, never highlightable.
    Group(String),
    /// One selectable option; index into the option list.
    Item(usize),
}

/// Collect the `<select>`'s options in document order, flattening
/// `<optgroup>` children so each option carries its group's `label`.
pub(crate) fn select_options(arena: &NodeArena, node_id: NodeId) -> Vec<SelectOption> {
    let mut out = Vec::new();
    let Some(node) = arena.get(node_id) else {
        return out;
    };
    for &child_id in &node.children {
        let child = &arena[child_id];
        match child.node_type {
            NodeType::HtmlOption => out.push(option_value(arena, child_id, None)),
            NodeType::Optgroup => {
                let group = child
                    .attributes
                    .get("label")
                    .cloned()
                    .map(CompactString::from);
                for &option_id in &child.children {
                    if arena[option_id].node_type == NodeType::HtmlOption {
                        out.push(option_value(arena, option_id, group.clone()));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The [`SelectOption`] for one `<option>` node, with the `value` attribute
/// falling back to the label like a browser.
fn option_value(
    arena: &NodeArena,
    option_id: NodeId,
    group: Option<CompactString>,
) -> SelectOption {
    let child = &arena[option_id];
    let label: CompactString = child
        .children
        .iter()
        .filter_map(|&c| match &arena[c].node_type {
            NodeType::Text => arena[c].data.as_deref(),
            _ => None,
        })
        .collect();
    let value = child
        .attributes
        .get("value")
        .cloned()
        .map(CompactString::from)
        .unwrap_or_else(|| label.clone());
    SelectOption {
        value,
        label,
        group,
    }
}

/// The dropdown options for `node_id`: a `<select>` reads its children; an
/// `<input list="…">` reads the matching `<datalist>` options filtered by
/// `filter` (the field's current text, case-insensitive substring match on
/// value / label). Everything else has no dropdown.
pub(crate) fn overlay_options(
    arena: &NodeArena,
    node_id: NodeId,
    filter: Option<&str>,
) -> Vec<SelectOption> {
    let Some(node) = arena.get(node_id) else {
        return Vec::new();
    };
    if node.node_type == NodeType::Select {
        return select_options(arena, node_id);
    }
    if node.node_type == NodeType::Input
        && let Some(list) = node.attributes.get("list")
    {
        return datalist_options(arena, list, filter.unwrap_or(""));
    }
    Vec::new()
}

/// The `<datalist>` options for an `<input list="…">` field, filtered by the
/// field's current text (case-insensitive substring on value / label). An
/// empty filter matches everything (the browser shows all suggestions).
fn datalist_options(arena: &NodeArena, list_id: &str, filter: &str) -> Vec<SelectOption> {
    let f = filter.to_lowercase();
    let mut out = Vec::new();
    for (_, node) in arena.iter() {
        if node.node_type != NodeType::Datalist || !node.get_id().is_some_and(|id| id == list_id) {
            continue;
        }
        for &child_id in &node.children {
            if arena[child_id].node_type != NodeType::HtmlOption {
                continue;
            }
            let opt = option_value(arena, child_id, None);
            if f.is_empty()
                || opt.value.to_lowercase().contains(&f)
                || opt.label.to_lowercase().contains(&f)
            {
                out.push(opt);
            }
        }
    }
    out
}

/// Expand options into dropdown rows, inserting a group header before the
/// first option of each `<optgroup>`.
pub(crate) fn select_rows(options: &[SelectOption]) -> Vec<SelectRow> {
    let mut rows = Vec::new();
    let mut last_group: Option<&str> = None;
    for (i, opt) in options.iter().enumerate() {
        if opt.group.as_deref() != last_group {
            if let Some(g) = opt.group.as_deref() {
                rows.push(SelectRow::Group(g.to_string()));
            }
            last_group = opt.group.as_deref();
        }
        rows.push(SelectRow::Item(i));
    }
    rows
}

/// The option indices in row order — the selectable subset of the rows, used
/// for keyboard navigation (group headers are skipped).
pub(crate) fn select_item_indices(rows: &[SelectRow]) -> Vec<usize> {
    rows.iter()
        .filter_map(|r| match r {
            SelectRow::Item(i) => Some(*i),
            SelectRow::Group(_) => None,
        })
        .collect()
}

/// Dropdown overlay rectangle for an open `<select>` / datalist dropdown:
/// directly below the field, wide enough for the widest row (option label or
/// group header, plus the selection marker), at least as wide as the field.
pub(crate) fn select_overlay_rect(
    options: &[SelectOption],
    rows: &[SelectRow],
    select_rect: Rect,
) -> Rect {
    let text_w: usize = rows
        .iter()
        .map(|r| match r {
            SelectRow::Group(g) => g.chars().map(char_width).sum::<usize>(),
            SelectRow::Item(i) => options
                .get(*i)
                .map(|o| o.label.chars().map(char_width).sum::<usize>())
                .unwrap_or(0),
        })
        .max()
        .unwrap_or(0);
    let width = select_rect.width.max(text_w.saturating_add(2)).max(1);
    Rect::new(
        select_rect.x,
        select_rect.y.saturating_add(1),
        width,
        rows.len().max(1),
    )
}
