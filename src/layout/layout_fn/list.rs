use crate::{
    layout::{
        BoxConstraint, Cursor, FontContext, InlineRun, LayoutBox,
        tree::{LayoutBlockOptions, layout_block},
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
/// `background-color` / `border` / `padding` paints the whole column reserved
/// for the list.
pub fn layout_ul(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    layout_list(arena, node_id, containing, cursor, NodeType::Ul, font_ctx)
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
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    layout_list(arena, node_id, containing, cursor, NodeType::Ol, font_ctx)
}

/// Build a [`LayoutBox`] for an `<li>` element.
///
/// Allocates one row and prepends a bullet/marker run (`• ` for `<ul>` items,
/// `<n>. ` for `<ol>` items, `- ` as a fallback when the `<li>` has no list
/// parent) to the body's inline runs. Nested children are then laid out in a
/// containing block shifted past the marker so any nested `<ul>`/`<ol>` sits
/// under the body text.
///
/// Padding / border / margin on an `<li>` shrink the content area and offset
/// the marker / body runs into the content row, mirroring the behaviour of
/// `<p>` and `<div>`.
pub fn layout_li(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Li {
        return None;
    }

    let marker = list_marker_for_li(arena, node_id);
    let block_font_style = block_style(&node.css_attr);
    let marker_run = InlineRun::new(
        marker.clone(),
        block_font_style,
        node.css_attr.get_color(),
        node.css_attr.get_bg_color(),
    );
    let marker_width = marker_run.width();

    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions {
            prefix: Some(marker_run),
            // Shift the children's containing block right by the marker
            // width and shrink the inner width by the same amount, so a
            // nested <ul>/<ol> sits under the body text rather than under
            // the marker glyph.
            child_containing_shift: Some((marker_width, -(marker_width as isize))),
            ..Default::default()
        },
        font_ctx,
    ))
}

fn layout_list(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    expected: NodeType,
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != expected {
        return None;
    }

    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions {
            indent: LIST_INDENT_PER_LEVEL * list_depth(arena, node_id),
            ..Default::default()
        },
        font_ctx,
    ))
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

/// Compute the marker string for an `<li>` based on its enclosing list.
///
/// Returns `• ` inside `<ul>`, `<n>. ` inside `<ol>`, or `- ` for an orphan
/// `<li>` (no list parent) so we still render a visible marker.
fn list_marker_for_li(arena: &NodeArena, li_id: NodeId) -> String {
    let Some(parent_id) = arena[li_id].parent else {
        return "- ".to_string();
    };
    let parent = &arena[parent_id];
    match parent.node_type {
        NodeType::Ul => "• ".to_string(),
        NodeType::Ol => {
            let ordinal = li_ordinal(arena, li_id, parent_id);
            format!("{ordinal}. ")
        }
        _ => "- ".to_string(),
    }
}

/// Compute the 1-based ordinal of `<li>` among its `<ol>` parent's `<li>`
/// children, skipping non-`<li>` siblings.
fn li_ordinal(arena: &NodeArena, li_id: NodeId, parent_id: NodeId) -> usize {
    let parent = &arena[parent_id];
    let mut count = 1usize;
    for &child_id in &parent.children {
        if child_id == li_id {
            return count;
        }
        if matches!(arena[child_id].node_type, NodeType::Li) {
            count = count.saturating_add(1);
        }
    }
    count
}
