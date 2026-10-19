//! Flex container orchestrator — runs the two-phase algorithm and assembles
//! the container's [`LayoutBox`].

use crate::{
    graphics::block_style,
    html::css_parse::{BoxSizing, OverflowWrap, WhiteSpace, WordBreak},
    layout::{
        BoxConstraint, Cursor, InlineRun, LayoutBox, LayoutContext, PaintStyle, Rect,
        inner_formatting_context::flow::{constrained_block_width, dispatch},
        layoutbox::{
            generation::{GeneratedBoxKind, generate_node},
            text_node_run, visible_children,
        },
    },
    tdom::node::{NodeArena, NodeId, NodeType, effective_font_size_cells},
};

use crate::html::css_parse::Position;
use crate::html::css_parse::resolve_line_height_cells;

use super::items::layout_flex_items;
use super::prepare::{prepare_anonymous_text_item, prepare_flex_item};

/// Minimum cross-axis size of any flex item (1 row by default).
///
/// Single-row text items naturally take 1 row; this constant formalises
/// that floor so the line-height calculation can't go below it.
pub(super) const FLEX_ITEM_MIN_CROSS: usize = 1;

/// Where a flex item's content comes from: a DOM element or an anonymous
/// text item synthesised from the container's raw text children.
pub(super) enum FlexItemSource {
    Node(NodeId),
    AnonymousText(AnonymousTextItem),
}

/// Styling + text runs of an anonymous flex item (contiguous raw text
/// children of a flex container).
pub(super) struct AnonymousTextItem {
    pub(super) runs: Vec<InlineRun>,
    pub(super) font_size_cells: usize,
    pub(super) line_height_cells: usize,
    pub(super) white_space: WhiteSpace,
    pub(super) word_break: WordBreak,
    pub(super) overflow_wrap: OverflowWrap,
}

/// One child of a flex container, prepared for the two-phase algorithm.
pub(super) struct FlexItemPrep {
    pub(super) source: FlexItemSource,
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
    /// Explicit CSS cross-axis size (row: `height`; column: `width`), in
    /// cells. `None` when the property is `auto` / unset. A definite cross
    /// size opts the item out of `align-items: stretch` (CSS Flexbox §9.4:
    /// only `auto`-sized items stretch).
    pub(super) css_cross: Option<usize>,
    pub(super) grow: f32,
    pub(super) shrink: f32,
    pub(super) align_self: crate::html::css_parse::AlignItems,
    /// Main-axis leading margin (row: `margin-left`; column: `margin-top`),
    /// applied as an offset before the item's border-box. Margins occupy the
    /// line but never grow / shrink with the item.
    pub(super) margin_main_before: usize,
    /// Main-axis trailing margin (row: `margin-right`; column:
    /// `margin-bottom`).
    pub(super) margin_main_after: usize,
    /// Cross-axis leading margin (row: `margin-top`; column: `margin-left`),
    /// applied when aligning the item on the cross axis.
    pub(super) margin_cross_before: usize,
    /// Cross-axis trailing margin (row: `margin-bottom`; column:
    /// `margin-right`). Together with [`Self::margin_cross_before`] it forms
    /// the item's margin-box extent the line must cover.
    pub(super) margin_cross_after: usize,
    /// Main-axis `margin: auto` flags `(before, after)` (row:
    /// `margin-left: auto` / `margin-right: auto`; column:
    /// `margin-top` / `margin-bottom`). Auto margins absorb the line's
    /// free space before `justify-content` applies, and disable it (CSS
    /// Flexbox §8.1).
    pub(super) margin_auto_main: (bool, bool),
    /// Offset (rows) from the box top to its first text row — the terminal
    /// cell-grid analogue of the CSS text baseline. Used by
    /// `align-items: baseline` to align the items' first text rows.
    pub(super) baseline: usize,
    pub(super) order: i32,
    /// Cached intrinsic border-box width measured once by
    /// `natural_intrinsic_size` during prep. The wrap / placement passes
    /// re-read it instead of re-running the recursive measurement (the
    /// measurement is a pure function of the same arena + context, so the
    /// cached value is exact).
    pub(super) natural_width: usize,
    /// Cached intrinsic border-box height — see [`Self::natural_width`].
    pub(super) natural_height: usize,
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
            // Collapse the top margin with the previous in-flow sibling's
            // bottom margin (CSS vertical margin collapsing: the larger wins).
            cursor.collapse_and_skip(edges.margin.top);
            let outer_y_top = cursor.y;
            let available =
                (containing.width as i64 - edges.margin.horizontal() as i64).max(0) as usize;
            let block_width = constrained_block_width(
                &node.css_attr,
                available,
                edges,
                child_context.font,
                child_context.viewport,
            );
            let mut edges = edges;
            // A flex container is a block-level box in its own flow, so
            // `margin: 0 auto` centers it the same way `layout_block` does.
            let margin_auto = node.css_attr.get_margin_auto();
            if margin_auto.left || margin_auto.right {
                let free = available.saturating_sub(block_width);
                match (margin_auto.left, margin_auto.right) {
                    (true, true) => edges.margin.left = (free / 2) as isize,
                    (true, false) => edges.margin.left = free as isize,
                    _ => {}
                }
            }
            (
                (containing.x as i64 + edges.margin.left as i64).max(0) as usize,
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
            // Percentage heights resolve against the containing block's
            // height; in block flow it is indefinite, so a percentage
            // computes to `auto` (CSS 10.5) — never against the width.
            node.css_attr
                .get_height(None, child_context.font, child_context.viewport)
        });
    let has_explicit_height = container_css_height.is_some();

    // 5. Build flex items in document order. Every in-flow element child
    //    becomes a flex item (block, blockified inline, or replaced);
    //    contiguous raw text children are wrapped into anonymous flex items
    //    (CSS Flexbox §4).
    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let transform = node.css_attr.get_text_transform();
    let white_space = node.css_attr.get_white_space(&node.node_type);
    let word_break = node.css_attr.get_word_break();
    let overflow_wrap = node.css_attr.get_overflow_wrap();

    let mut positioned_boxes: Vec<LayoutBox> = Vec::new();
    let mut item_preps: Vec<FlexItemPrep> = Vec::new();
    let mut text_runs: Vec<InlineRun> = Vec::new();

    for &child_id in &visible_children(arena, node_id) {
        let child = &arena[child_id];
        if child.node_type == NodeType::Text {
            if let Some(run) = text_node_run(
                arena,
                child_id,
                block_fg,
                block_bg,
                block_font_style,
                transform,
                white_space,
            ) {
                text_runs.push(run);
            }
            continue;
        }

        // Flush the preceding raw text into an anonymous flex item.
        if !text_runs.is_empty() {
            item_preps.push(prepare_anonymous_text_item(
                AnonymousTextItem {
                    runs: std::mem::take(&mut text_runs),
                    font_size_cells: own_font_size_cells,
                    line_height_cells,
                    white_space,
                    word_break,
                    overflow_wrap,
                },
                direction,
                align_items,
            ));
        }

        let Some(generated) = generate_node(arena, child_id) else {
            continue;
        };
        if !matches!(
            generated.kind,
            GeneratedBoxKind::Normal | GeneratedBoxKind::Replaced
        ) {
            continue;
        }

        // Detached positioned children lay out against a local cursor (no row
        // claims) and are re-attached below the flex layout.
        let positioning = arena[child_id].css_attr.get_positioning(
            content_width,
            child_context.font,
            child_context.viewport,
        );
        match positioning.position {
            Position::Absolute | Position::Fixed => {
                let mut local_cursor = Cursor {
                    y: 0,
                    prev_bottom_margin: 0,
                };
                if let Some(b) = dispatch(
                    arena,
                    child_id,
                    content_constraint,
                    &mut local_cursor,
                    child_context,
                    None,
                ) {
                    positioned_boxes.push(b);
                }
            }
            _ => item_preps.push(prepare_flex_item(
                arena,
                child_id,
                direction,
                content_width,
                // Percentage / explicit heights on a column item's main axis
                // resolve against the container's definite content height.
                if direction.is_column() {
                    container_css_height
                } else {
                    None
                },
                child_context,
                align_items,
            )),
        }
    }

    // Flush trailing raw text.
    if !text_runs.is_empty() {
        item_preps.push(prepare_anonymous_text_item(
            AnonymousTextItem {
                runs: std::mem::take(&mut text_runs),
                font_size_cells: own_font_size_cells,
                line_height_cells,
                white_space,
                word_break,
                overflow_wrap,
            },
            direction,
            align_items,
        ));
    }

    // A flex item is laid out by recursively dispatching its own element
    // handler with the rectangle allocated by the flex algorithm; anonymous
    // text items wrap their runs directly. Detached positioned children are
    // excluded above and re-attached below.
    let (mut child_boxes, lines_total_height) = if item_preps.is_empty() {
        (Vec::new(), 0usize)
    } else {
        layout_flex_items(
            arena,
            node_id,
            item_preps,
            direction,
            wrap,
            justify,
            align_items,
            align_content,
            gap,
            content_constraint,
            content_y,
            child_context,
            has_explicit_height,
            container_css_height.unwrap_or(0),
        )
    };
    // Re-attach detached positioned children (they occupy no flex space; the
    // post-layout pass resolves their final rectangles).
    child_boxes.extend(positioned_boxes);

    let mut outer_height = match preallocated_height {
        Some(height) => {
            // A pre-allocated rect (this flex container is itself a flex
            // item) fixes the border-box height, but the lines laid out
            // inside can grow taller than the parent's max-content measure
            // assumed (text wrapping at the item's final width). Grow the
            // rect to cover the actual content instead of clipping the last
            // rows — mirrors `layout_block`'s pre-allocated growth.
            let inner_v = edges.padding.top
                + edges.padding.bottom
                + edges.border.top.thickness()
                + edges.border.bottom.thickness();
            height.max(lines_total_height.saturating_add(inner_v))
        }
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
            .get_min_height(None, child_context.font, child_context.viewport)
            .map(to_content)
        {
            outer_height = outer_height.max(m);
        }
        if let Some(m) = node
            .css_attr
            .get_max_height(None, child_context.font, child_context.viewport)
            .map(to_content)
        {
            outer_height = outer_height.min(m).max(1);
        }
    }

    // Only block flow advances the shared cursor. A flex item already owns its
    // position and size in its parent's formatting context.
    if preallocated_height.is_none() {
        let total_box_height = outer_height
            .saturating_add(edges.border.top.thickness())
            .saturating_add(edges.padding.top)
            .saturating_add(edges.border.bottom.thickness())
            .saturating_add(edges.padding.bottom);
        cursor.y = outer_y_top
            .saturating_add(total_box_height)
            .saturating_add(edges.margin.bottom);
        // Leave the bottom margin for the next in-flow sibling to collapse
        // its top margin against.
        cursor.leave_bottom_margin(edges.margin.bottom);
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
    };
    let fill_bg = block_bg.is_some();
    let rect = Rect::new(outer_x, outer_y_top, block_width, total_box_height);

    let paint_style = PaintStyle {
        box_edges: edges,
        runs: None,
        column_layout: None,
        font_size_cells: own_font_size_cells,
        line_height_cells,
        border_radius: node.css_attr.get_border_radius(
            content_width,
            child_context.font,
            child_context.viewport,
        ),
        fill_bg,
        spacing_after: 0,
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
