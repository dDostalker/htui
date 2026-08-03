//! Flex container orchestrator — runs the two-phase algorithm and assembles
//! the container's [`LayoutBox`].

use crate::{
    html::css_parse::BoxSizing,
    layout::{
        BoxConstraint, Cursor, LayoutBox, LayoutContext, PaintStyle, Rect,
        inner_formatting_context::flow::constrained_block_width,
        layoutbox::{
            collect_runs,
            generation::{GeneratedBoxKind, generate_node},
            wrap_runs,
        },
    },
    tdom::node::{NodeArena, NodeId, NodeType, effective_font_size_cells},
    tui::graph_api::block_style,
};

use crate::html::css_parse::resolve_line_height_cells;

use super::items::layout_flex_items;

/// Minimum cross-axis size of any flex item (1 row by default).
///
/// Single-row text items naturally take 1 row; this constant formalises
/// that floor so the line-height calculation can't go below it.
pub(super) const FLEX_ITEM_MIN_CROSS: usize = 1;

/// One block child of a flex container, prepared for the two-phase
/// algorithm.
pub(super) struct FlexItemPrep {
    pub(super) node_id: NodeId,
    /// Resolved `flex-basis` (0 when `auto` falls through to no value —
    /// the size is supplied by the grow / shrink distribution).
    pub(super) basis: usize,
    /// Automatic minimum main-axis size (CSS `min-width` / `min-height:
    /// auto`) — the item's min-content size. `flex-shrink` never takes an
    /// item below this floor.
    pub(super) min_main: usize,
    /// Explicit CSS `min-width` (row) / `min-height` (column) constraint on
    /// the main axis, in cells. `None` when unset.
    pub(super) css_min_main: Option<usize>,
    /// Explicit CSS `max-width` (row) / `max-height` (column) constraint on
    /// the main axis, in cells. `None` when unset / `none`.
    pub(super) css_max_main: Option<usize>,
    pub(super) grow: f32,
    pub(super) shrink: f32,
    pub(super) align_self: crate::html::css_parse::AlignItems,
    pub(super) order: i32,
}

/// Build a [`LayoutBox`] for a flex container at `node_id`.
///
/// Performs the full two-phase flex algorithm and returns the assembled
/// container box. The outer cursor is advanced by the container's
/// border-box height + `margin.bottom`, mirroring the other block
/// layout functions in this module.
pub(super) fn layout_flex_container(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> LayoutBox {
    let node = &arena[node_id];

    // 1. Font-size resolution: the container's own effective font-size
    //    cascades down to flex items and (transitively) their children.
    let own_font_size_cells =
        effective_font_size_cells(arena, node_id, context.font, context.viewport);
    let child_context = context.child(own_font_size_cells);

    // 2. Box edges (padding / border / margin) — same resolution as
    //    `layout_block`, no flex-specific tweaks.
    let edges =
        node.css_attr
            .get_box_edges(containing.width, child_context.font, child_context.viewport);
    let line_height_cells = resolve_line_height_cells(
        node.css_attr.get_line_height(),
        own_font_size_cells,
        child_context.font,
        child_context.viewport,
    );

    // A flex formatting context normally participates in block flow. When it
    // is itself a flex item, its parent has already chosen the border-box and
    // this context must lay out inside that rectangle instead.
    let (outer_x, outer_y_top, block_width, preallocated_height) = match pre_allocated_rect {
        Some(rect) => (rect.x, rect.y, rect.width.max(1), Some(rect.height.max(1))),
        None => {
            cursor.skip(edges.margin.top);
            let outer_y_top = cursor.y;
            let available = containing.width.saturating_sub(edges.margin.horizontal());
            let block_width = constrained_block_width(
                &node.css_attr,
                available,
                edges,
                child_context.font,
                child_context.viewport,
            );
            (
                containing.x + edges.margin.left,
                outer_y_top,
                block_width,
                None,
            )
        }
    };

    let content_x = outer_x + edges.border.left.thickness() + edges.padding.left;
    let content_width = block_width.saturating_sub(edges.inner_horizontal()).max(1);
    let content_y = outer_y_top + edges.border.top.thickness() + edges.padding.top;
    let content_constraint = BoxConstraint {
        x: content_x,
        width: content_width,
    };

    // 4. Flex properties (already-typed accessors; CSS initial values
    //    are returned when the property is missing).
    let direction = node.css_attr.get_flex_direction();
    let wrap = node.css_attr.get_flex_wrap();
    let justify = node.css_attr.get_justify_content();
    let align_items = node.css_attr.get_align_items();
    let align_content = node.css_attr.get_align_content();
    let gap = node
        .css_attr
        .get_gap(child_context.font, child_context.viewport);
    let container_css_height = preallocated_height
        .map(|height| height.saturating_sub(edges.inner_vertical()).max(1))
        .or_else(|| {
            node.css_attr
                .get_height(content_width, child_context.font, child_context.viewport)
        });
    let has_explicit_height = container_css_height.is_some();

    // 5. Inline runs (container's own text / span / a / em children).
    //    These are rendered as the container's own row, ABOVE the flex
    //    items. v1 doesn't promote raw text into anonymous flex items.
    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let body_runs = collect_runs(arena, node, block_fg, block_bg, block_font_style);

    // Wrap the container's own inline text so its rect grows to fit
    // the wrapped line count, and so the flex items are pushed down
    // by the height of the wrapped text.
    let flex_inline_lines = if body_runs.is_empty() {
        Vec::new()
    } else {
        wrap_runs(
            &body_runs,
            content_constraint.width.max(1),
            own_font_size_cells,
            node.css_attr.get_white_space(&node.node_type),
            node.css_attr.get_word_break(),
            node.css_attr.get_overflow_wrap(),
        )
    };
    let inline_line_count = flex_inline_lines.len();
    let inline_rows = inline_line_count.saturating_mul(line_height_cells.max(1));
    // Flex items begin below the container's own wrapped text.
    let content_y_with_inline = content_y.saturating_add(inline_rows);

    // 6. Flex items: the container's `BoxKind::Normal` children in document
    //    order. Inline and text nodes are already wrapped by the generator
    //    into anonymous boxes that contribute their runs to the container
    //    itself rather than participating as flex items.
    let block_child_ids: Vec<NodeId> = node
        .children
        .iter()
        .copied()
        .filter(|&id| {
            generate_node(arena, id)
                .map(|generated| {
                    // Only block-level children become flex items. Inline
                    // children (`<span>` / `<a>` / `<em>`) are folded into
                    // the container's own runs by `collect_runs`.
                    generated.kind == GeneratedBoxKind::Normal
                        && !generated.outer_display.is_inline()
                })
                .unwrap_or(false)
        })
        .collect();

    // A flex item is laid out by recursively dispatching its own element
    // handler with the rectangle allocated by the flex algorithm. This preserves
    // special item behaviour such as image painting, chrome, tables, and nested
    // flex containers.
    let (child_boxes, lines_total_height) = if block_child_ids.is_empty() {
        (Vec::new(), 0usize)
    } else {
        layout_flex_items(
            arena,
            &block_child_ids,
            direction,
            wrap,
            justify,
            align_items,
            align_content,
            gap,
            content_constraint,
            content_y_with_inline,
            child_context,
            has_explicit_height,
            container_css_height.unwrap_or(0),
        )
    };

    let mut outer_height = match preallocated_height {
        Some(height) => height,
        None if has_explicit_height => container_css_height.unwrap_or(0).max(lines_total_height),
        None => lines_total_height,
    };
    // `min-height` / `max-height` clamp the border-box height. `outer_height`
    // is a *content* height here (the border-box adds `inner_v`), so the
    // constraint is converted back to a content target. Preallocated
    // (flex-item) rects keep the size chosen by the parent context.
    if preallocated_height.is_none() {
        let inner_v = edges.padding.top
            + edges.padding.bottom
            + edges.border.top.thickness()
            + edges.border.bottom.thickness();
        let sizing = node.css_attr.get_box_sizing();
        let to_content = |h: usize| match sizing {
            BoxSizing::BorderBox => h.saturating_sub(inner_v),
            BoxSizing::ContentBox => h,
        };
        if let Some(m) = node
            .css_attr
            .get_min_height(content_width, child_context.font, child_context.viewport)
            .map(to_content)
        {
            outer_height = outer_height.max(m);
        }
        if let Some(m) = node
            .css_attr
            .get_max_height(content_width, child_context.font, child_context.viewport)
            .map(to_content)
        {
            outer_height = outer_height.min(m).max(1);
        }
    }

    let chrome = matches!(
        node.node_type,
        NodeType::Aside | NodeType::Header | NodeType::Footer
    );
    let spacing_after = if preallocated_height.is_some() {
        0
    } else if matches!(node.node_type, NodeType::Header | NodeType::Footer) {
        2
    } else {
        0
    };

    // Only block flow advances the shared cursor. A flex item already owns its
    // position and size in its parent's formatting context.
    if preallocated_height.is_none() {
        let total_box_height = outer_height
            .saturating_add(edges.border.top.thickness())
            .saturating_add(edges.padding.top)
            .saturating_add(edges.border.bottom.thickness())
            .saturating_add(edges.padding.bottom)
            .saturating_add(spacing_after);
        cursor.y = outer_y_top
            .saturating_add(total_box_height)
            .saturating_add(edges.margin.bottom);
    }

    // 10. Assemble the container's LayoutBox.
    let total_box_height = if preallocated_height.is_some() {
        outer_height
    } else {
        outer_height
            .saturating_add(edges.border.top.thickness())
            .saturating_add(edges.padding.top)
            .saturating_add(edges.border.bottom.thickness())
            .saturating_add(edges.padding.bottom)
            .saturating_add(spacing_after)
    };
    let fill_bg = block_bg.is_some() || chrome;
    let rect = Rect::new(outer_x, outer_y_top, block_width, total_box_height);

    let paint_style = PaintStyle {
        box_edges: edges,
        runs: if flex_inline_lines.is_empty() || flex_inline_lines.iter().all(|l| l.is_empty()) {
            None
        } else {
            Some(flex_inline_lines)
        },
        font_size_cells: own_font_size_cells,
        line_height_cells,
        border_radius: node
            .css_attr
            .get_border_radius(content_width, child_context.font, child_context.viewport),
        fill_bg,
        spacing_after,
        chrome,
    };

    LayoutBox::new_flex_container(
        node_id,
        rect,
        paint_style,
        arena[node_id]
            .css_attr
            .outer_display(&arena[node_id].node_type),
        child_boxes,
    )
}
