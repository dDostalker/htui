use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, Cursor, InlineRun, LayoutBox,
        tree::{collect_runs, layout_children},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::block_style,
};

/// Horizontal indent applied per nesting level of `<ul>`/`<ol>`.
const LIST_INDENT_PER_LEVEL: usize = 2;

/// Build a [`LayoutBox`] for a `<ul>` element.
///
/// `<ul>` is a structural list container. Each `<li>` child is rendered on its
/// own row by [`layout_li`]; the list itself only contributes spacing and an
/// indent for nested levels. The rect spans the indented region so any CSS
/// `background-color` paints the whole column reserved for the list.
pub fn layout_ul(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>> {
    layout_list(arena, node_id, containing, cursor, NodeType::Ul)
}

/// Build a [`LayoutBox`] for an `<ol>` element.
///
/// Same container shape as `<ul>`; [`layout_li`] distinguishes the bullet
/// style (`1. `, `2. `, …) from the unordered `• ` marker.
pub fn layout_ol(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>> {
    layout_list(arena, node_id, containing, cursor, NodeType::Ol)
}

/// Build a [`LayoutBox`] for an `<li>` element.
///
/// Allocates one row and prepends a bullet/marker run (`• ` for `<ul>` items,
/// `<n>. ` for `<ol>` items, `- ` as a fallback when the `<li>` has no list
/// parent) to the body's inline runs. Nested children are then laid out in a
/// containing block shifted past the marker so any nested `<ul>`/`<ol>` sits
/// under the body text.
pub fn layout_li(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Li {
        return Ok(None);
    }

    let (enclosing_list, ordinal) = list_context_for_li(arena, node_id);
    let marker = match enclosing_list {
        Some(NodeType::Ul) => "• ".to_string(),
        Some(NodeType::Ol) => format!("{}. ", ordinal.unwrap_or(1)),
        // Orphan `<li>` outside any list — fall back to a dash so we still
        // render a visible marker rather than a blank row.
        _ => "- ".to_string(),
    };

    let block_width = containing.width;
    let x = containing.x;
    let y = cursor.allocate(1);

    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let body_runs = collect_runs(arena, node, block_fg, block_bg, block_font_style);

    // Marker is its own inline run carried at the li's resolved style, so a
    // <span> inside the body can still apply its own flags on top.
    let marker_run = InlineRun::new(marker, block_font_style, block_fg, block_bg);
    let marker_width = marker_run.width();
    let mut runs = Vec::with_capacity(body_runs.len() + 1);
    runs.push(marker_run);
    runs.extend(body_runs);

    // Children (typically a nested <ul>/<ol>) sit on rows below the marker
    // row, indented past the marker width so they line up with the body text.
    let child_containing = BoxConstraint {
        x: x.saturating_add(marker_width),
        width: block_width.saturating_sub(marker_width).max(1),
    };
    let mut child_cursor = Cursor { y: cursor.peek() };
    let li_children = node.children.clone();
    let children = layout_children(arena, &li_children, child_containing, &mut child_cursor)?;
    cursor.y = child_cursor.y;

    let fill_bg = block_bg.is_some();

    Ok(Some(LayoutBox::new_block(
        node_id,
        crate::layout::Rect::new(x, y, block_width, 1),
        fill_bg,
        0,
        runs,
        children,
    )))
}

fn layout_list(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    expected: NodeType,
) -> Result<Option<LayoutBox>> {
    let node = &arena[node_id];
    if node.node_type != expected {
        return Ok(None);
    }

    let depth = list_depth(arena, node_id);
    let indent = LIST_INDENT_PER_LEVEL * depth;
    let x = containing.x.saturating_add(indent);
    let width = containing.width.saturating_sub(indent).max(1);

    // Like <aside>: peek the next row without reserving an extra row for the
    // list itself, so the rect wraps the children directly.
    let y = cursor.peek();

    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let runs = collect_runs(arena, node, block_fg, block_bg, block_font_style);

    let fill_bg = block_bg.is_some();

    let child_containing = BoxConstraint { x, width };
    let mut child_cursor = Cursor { y };
    let list_children = node.children.clone();
    let children = layout_children(arena, &list_children, child_containing, &mut child_cursor)?;

    let height = child_cursor.y.saturating_sub(y).max(1);
    cursor.y = y.saturating_add(height);

    Ok(Some(LayoutBox::new_block(
        node_id,
        crate::layout::Rect::new(x, y, width, height),
        fill_bg,
        0,
        runs,
        children,
    )))
}

/// Count `<ul>`/`<ol>` ancestors of `node_id`.
///
/// A top-level list returns `0` so its items sit at `containing.x`; each level
/// of nesting adds [`LIST_INDENT_PER_LEVEL`] columns to the items inside.
fn list_depth(arena: &NodeArena, node_id: NodeId) -> usize {
    let mut depth = 0usize;
    let mut current = arena[node_id].parent;
    while let Some(pid) = current {
        if matches!(arena[pid].node_type, NodeType::Ul | NodeType::Ol) {
            depth = depth.saturating_add(1);
        }
        current = arena[pid].parent;
    }
    depth
}

/// Detect the enclosing list type of an `<li>` and, when inside `<ol>`, the
/// 1-based ordinal position among its `<li>` siblings.
///
/// Sibling text nodes and unknown inline tags are skipped so the count tracks
/// only actual list items.
fn list_context_for_li(arena: &NodeArena, li_id: NodeId) -> (Option<NodeType>, Option<usize>) {
    let parent_id = match arena[li_id].parent {
        Some(p) => p,
        None => return (None, None),
    };
    let parent = &arena[parent_id];
    if !matches!(parent.node_type, NodeType::Ul | NodeType::Ol) {
        return (None, None);
    }
    let enclosing = parent.node_type.clone();
    let ordinal = if matches!(enclosing, NodeType::Ol) {
        let mut count = 1usize;
        for &child_id in &parent.children {
            if child_id == li_id {
                break;
            }
            if matches!(arena[child_id].node_type, NodeType::Li) {
                count = count.saturating_add(1);
            }
        }
        Some(count)
    } else {
        None
    };
    (Some(enclosing), ordinal)
}
