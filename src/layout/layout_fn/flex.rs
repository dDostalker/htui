//! Flexbox layout.
//!
//! Implements CSS flexbox for block elements. The entry point is
//! [`layout_flex_dispatch`], which is registered *first* in
//! [`DEFAULT_LAYOUT`](crate::layout::DEFAULT_LAYOUT) so it preempts the
//! element-specific block layout whenever the node has `display: flex`.
//! For non-flex nodes it returns `None`, letting the registry fall through
//! to the next claimer — this is what lets flex work uniformly on `<div>`,
//! `<section>`, `<header>`, etc. without modifying any of their layout
//! functions.
//!
//! ## Scope (v1)
//!
//! Supported CSS properties (full grammar; values are case-insensitive):
//!
//! - `display: flex`
//! - `flex-direction: row | column`
//! - `flex-wrap: nowrap | wrap`
//! - `justify-content: flex-start | flex-end | center | space-between |
//!   space-around | space-evenly`
//! - `align-items: stretch | flex-start | flex-end | center`
//! - `align-self: auto | <same as align-items>`
//! - `align-content: <same as justify-content>`
//! - `flex-grow`, `flex-shrink` (non-negative numbers)
//! - `flex-basis: auto | <length>`
//! - `order` (integer)
//! - `gap`, `row-gap`, `column-gap`
//!
//! Intentionally **not** supported in v1:
//!
//! - `flex-direction: row-reverse | column-reverse`
//! - `flex-wrap: wrap-reverse`
//! - `align-items: baseline` (terminals have no baseline concept)
//! - `display: inline-flex` (renderer is block-only)
//! - Anonymous flex items from raw text nodes inside a flex container
//!   (v1 skips them; the container's text / inline children are still
//!   collected as its own inline runs)
//! - `align-items: stretch` when the container has no explicit `height`
//!   (cross-axis size would be ambiguous — see the stretch section below)
//!
//! ## Algorithm
//!
//! Each flex container is laid out in two phases:
//!
//! 1. **Measure** every block child's `flex-basis` (resolved against the
//!    main-axis size), falling back to its CSS `width` / `height`, then to
//!    the text content width when nothing else is set.
//! 2. **Distribute** remaining main-axis space across a line (or wrap onto
//!    additional lines) according to `flex-grow` / `flex-shrink`, then
//!    apply `justify-content` to the line.
//!
//! The cross-axis size of each line is the max natural cross-axis size of
//! the items on it. `align-items: stretch` enlarges items to the line's
//! cross-axis size **only when the container has an explicit `height`** —
//! this matches the v1 contract: without a known cross-axis size, stretch
//! would otherwise silently change the container's height.
//!
//! ## Children policy
//!
//! Only the container's **block** children are flex items. The container's
//! own text / inline children (`Text`, `Span`, `A`, `Em`) are still
//! collected as the container's inline runs and rendered as a single row
//! of the container itself; they are **not** treated as anonymous flex
//! items. This is the v1 limitation — see the [scope](#scope-v1) section
//! above. The visual test `visual_test/src/flex/` shows the expected
//! shape.

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    html::css_parse::{AlignItems, AlignSelf, FlexDirection, FlexWrap, GapSizes, JustifyContent},
    layout::{
        BoxConstraint, BoxEdges, Cursor, FontContext, LayoutBox, Rect,
        tree::{collect_runs, layout_children},
    },
    tdom::node::{NodeArena, NodeId, NodeType, effective_font_size_cells},
    tui::graph_api::{block_style, grapheme_width},
};

/// Minimum cross-axis size of any flex item (1 row by default).
///
/// Single-row text items naturally take 1 row; this constant formalises
/// that floor so the line-height calculation can't go below it.
const FLEX_ITEM_MIN_CROSS: usize = 1;

/// Top-level flex entry point. Registered first in `DEFAULT_LAYOUT`.
///
/// Returns `None` for nodes that do not opt into `display: flex` so the
/// registry falls through to the element-specific block layout. Also
/// declines non-element nodes (text, head, style, …) which can never
/// host flex children.
pub fn layout_flex_dispatch(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if !node.css_attr.is_flex() {
        return None;
    }
    if !is_flex_capable_node(&node.node_type) {
        return None;
    }
    Some(layout_flex_container(
        arena, node_id, containing, cursor, font_ctx,
    ))
}

/// Whether a `NodeType` can host a flex container.
///
/// Text nodes, head/title/style/meta/link (which are parsed but never
/// reached as layout roots in normal documents), and the synthetic
/// `Global` root are all rejected. The `Selector::Tag` matcher
/// already prevents most of these from acquiring `display: flex` in the
/// first place — the guard is belt-and-braces.
fn is_flex_capable_node(nt: &NodeType) -> bool {
    !matches!(
        nt,
        NodeType::Text(_)
            | NodeType::Global
            | NodeType::Head
            | NodeType::Title
            | NodeType::Style
            | NodeType::Link
            | NodeType::Meta
    )
}

/// True when the node type is a block-level element that can act as a
/// flex *item* (i.e. can be a direct child of a flex container).
///
/// Inline elements (`Span`, `A`, `Em`) and `Text` leaves are excluded —
/// they participate in the container's inline runs instead. `<img>` is
/// also a block-level layout element and is included so images can sit
/// inside a flex container.
fn is_block_child_node(nt: &NodeType) -> bool {
    matches!(
        nt,
        NodeType::Div
            | NodeType::P
            | NodeType::H1
            | NodeType::H2
            | NodeType::H3
            | NodeType::H4
            | NodeType::H5
            | NodeType::Header
            | NodeType::Footer
            | NodeType::Aside
            | NodeType::Section
            | NodeType::Article
            | NodeType::Main
            | NodeType::Nav
            | NodeType::Figure
            | NodeType::Figcaption
            | NodeType::Details
            | NodeType::Summary
            | NodeType::Ul
            | NodeType::Ol
            | NodeType::Li
            | NodeType::Table
            | NodeType::Thead
            | NodeType::Tbody
            | NodeType::Tfoot
            | NodeType::Tr
            | NodeType::Th
            | NodeType::Td
            | NodeType::Img
            | NodeType::Form
            | NodeType::Button
            | NodeType::Input
            | NodeType::Select
            | NodeType::Textarea
            | NodeType::Label
    )
}

/// Build a [`LayoutBox`] for a flex container at `node_id`.
///
/// Performs the full two-phase flex algorithm and returns the assembled
/// container box. The outer cursor is advanced by the container's
/// border-box height + `margin.bottom`, mirroring the other block
/// layout functions in this module.
fn layout_flex_container(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> LayoutBox {
    let node = &arena[node_id];

    // 1. Font-size resolution: the container's own effective font-size
    //    cascades down to flex items and (transitively) their children.
    let own_font_size_cells = effective_font_size_cells(
        arena,
        node_id,
        font_ctx.font_size_cells,
        font_ctx.root_font_size_cells,
    );
    let child_font_ctx = font_ctx.child(own_font_size_cells);

    // 2. Box edges (padding / border / margin) — same resolution as
    //    `layout_block`, no flex-specific tweaks.
    let edges = node.css_attr.get_box_edges(
        containing.width,
        own_font_size_cells,
        font_ctx.root_font_size_cells,
    );

    // 3. Outer geometry: x = containing.x + margin.left,
    //    width = CSS width or containing.width.
    cursor.skip(edges.margin.top);
    let outer_y_top = cursor.y;

    let available = containing.width.saturating_sub(edges.margin.horizontal());
    let block_width = node
        .css_attr
        .get_width(
            available,
            own_font_size_cells,
            font_ctx.root_font_size_cells,
        )
        .unwrap_or(available)
        .min(available)
        .max(1);
    let outer_x = containing.x + edges.margin.left;

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
    let gap = node.css_attr.get_gap();
    let container_css_height = node.css_attr.get_height(
        content_width,
        own_font_size_cells,
        font_ctx.root_font_size_cells,
    );
    let has_explicit_height = container_css_height.is_some();

    // 5. Inline runs (container's own text / span / a / em children).
    //    These are rendered as the container's own row, ABOVE the flex
    //    items. v1 doesn't promote raw text into anonymous flex items.
    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let body_runs = collect_runs(arena, node, block_fg, block_bg, block_font_style);

    // 6. Flex items: the container's block children, in document order.
    //    Sort by `order` (stable sort — same `order` keeps DOM order).
    let block_child_ids: Vec<NodeId> = node
        .children
        .iter()
        .copied()
        .filter(|&id| is_block_child_node(&arena[id].node_type))
        .collect();

    // 7. Lay out the flex items using the two-phase algorithm. Each
    //    child is given a concrete rect by the algorithm; the recursive
    //    `layout_flex_item` lays out its nested children within that
    //    rect via the normal `layout_children` dispatch (so a child
    //    that itself has `display: flex` recurses into flex).
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
            content_y,
            child_font_ctx,
            has_explicit_height,
            container_css_height.unwrap_or(0),
        )
    };

    // 8. Container outer height: explicit `height` wins, otherwise the
    //    natural sum of line heights. When explicit, the container is
    //    still at least as tall as the natural layout (so child items
    //    never get clipped).
    let outer_height = if has_explicit_height {
        container_css_height.unwrap_or(0).max(lines_total_height)
    } else {
        lines_total_height
    };

    // 9. Advance the cursor past the border-box and margin-bottom so
    //    the next sibling starts at the right row.
    let total_box_height = outer_height
        .saturating_add(edges.border.top.thickness())
        .saturating_add(edges.padding.top)
        .saturating_add(edges.border.bottom.thickness())
        .saturating_add(edges.padding.bottom);
    cursor.y = outer_y_top
        .saturating_add(total_box_height)
        .saturating_add(edges.margin.bottom);

    // 10. Assemble the container's LayoutBox.
    let fill_bg = block_bg.is_some();
    let rect = Rect::new(outer_x, outer_y_top, block_width, total_box_height);

    LayoutBox::new_block(
        node_id,
        rect,
        edges,
        fill_bg,
        0,
        body_runs,
        child_boxes,
        own_font_size_cells,
    )
}

/// One block child of a flex container, prepared for the two-phase
/// algorithm.
struct FlexItemPrep {
    node_id: NodeId,
    /// Resolved `flex-basis` (0 when `auto` falls through to no value —
    /// the size is supplied by the grow / shrink distribution).
    basis: usize,
    grow: f32,
    shrink: f32,
    align_self: AlignItems,
    order: i32,
}

/// Lay out every flex item across one or more lines and return the
/// positioned children plus the total main-axis-perpendicular height
/// they consume (used to size the container).
#[allow(clippy::too_many_arguments)]
fn layout_flex_items(
    arena: &NodeArena,
    item_ids: &[NodeId],
    direction: FlexDirection,
    wrap: FlexWrap,
    justify: JustifyContent,
    align_items: AlignItems,
    align_content: JustifyContent,
    gap: GapSizes,
    content: BoxConstraint,
    content_y: usize,
    font_ctx: FontContext,
    has_explicit_height: bool,
    container_height: usize,
) -> (Vec<LayoutBox>, usize) {
    // 1. Prepare + sort every item.
    let mut preps: Vec<FlexItemPrep> = item_ids
        .iter()
        .map(|&id| prepare_flex_item(arena, id, direction, content.width, font_ctx, align_items))
        .collect();
    preps.sort_by_key(|p| p.order);

    // 2. Split into lines based on `wrap`.
    let lines: Vec<Vec<usize>> = match wrap {
        FlexWrap::NoWrap => vec![(0..preps.len()).collect()],
        FlexWrap::Wrap => wrap_into_lines(&preps, direction, content.width, gap),
    };

    // 3. Lay out each line and stack them along the cross axis.
    let mut placed: Vec<LayoutBox> = Vec::with_capacity(preps.len());
    let mut current_cross: usize = content_y;
    let line_count = lines.len();
    let (cross_gap, main_gap) = match direction {
        FlexDirection::Row => (gap.row, gap.column),
        FlexDirection::Column => (gap.column, gap.row),
    };

    for (line_idx, line_indices) in lines.iter().enumerate() {
        // 3a. Distribute the line's main-axis space.
        let line_preps: Vec<&FlexItemPrep> = line_indices.iter().map(|&i| &preps[i]).collect();
        let line_main_size = match direction {
            FlexDirection::Row => content.width,
            FlexDirection::Column => usize::MAX, // column direction: vertical unbounded
        };
        let (main_sizes, free_for_justify) =
            distribute_main_axis(&line_preps, line_main_size, main_gap);

        // 3b. Compute the line's natural cross-axis size.
        let natural_line_cross = line_preps
            .iter()
            .map(|p| natural_cross_axis_size(arena, p.node_id, direction, font_ctx))
            .max()
            .unwrap_or(FLEX_ITEM_MIN_CROSS)
            .max(FLEX_ITEM_MIN_CROSS);

        // 3c. Effective cross-axis size: explicit container height wins
        //     when present, otherwise the natural line cross.
        let line_cross = if has_explicit_height {
            container_height
        } else {
            natural_line_cross
        };

        // 3d. Compute justify-content leading + between spacing.
        let (justify_leading, justify_between) =
            justify_spacing(justify, free_for_justify, line_preps.len());

        // 3e. Place every item on the line.
        let mut cursor_main: usize = match direction {
            FlexDirection::Row => content.x,
            FlexDirection::Column => current_cross,
        };
        cursor_main = cursor_main.saturating_add(justify_leading);
        for (prep_idx, prep) in line_preps.iter().enumerate() {
            let main_size = main_sizes[prep_idx];
            // Natural cross-axis size of this item (used as the fallback
            // for non-stretch alignments and for stretch when the
            // container's cross-axis size is unknown).
            let item_natural_cross =
                natural_cross_axis_size(arena, prep.node_id, direction, font_ctx);
            let item_cross_size = resolve_item_cross_size(
                prep.align_self,
                line_cross,
                item_natural_cross,
                has_explicit_height,
                direction,
            );
            let (item_x, item_y, item_w, item_h) = match direction {
                FlexDirection::Row => {
                    let y = align_on_cross_axis(
                        current_cross,
                        line_cross,
                        item_cross_size,
                        effective_align(prep.align_self, align_items),
                    );
                    (cursor_main, y, main_size, item_cross_size)
                }
                FlexDirection::Column => {
                    let x = align_on_cross_axis(
                        content.x,
                        line_cross,
                        item_cross_size,
                        effective_align(prep.align_self, align_items),
                    );
                    (x, cursor_main, item_cross_size, main_size)
                }
            };
            let child_box = layout_flex_item(
                arena,
                prep.node_id,
                Rect::new(item_x, item_y, item_w, item_h),
                font_ctx,
            );
            placed.push(child_box);
            cursor_main = cursor_main.saturating_add(main_size);
            if prep_idx + 1 < line_preps.len() {
                cursor_main = cursor_main.saturating_add(main_gap);
                cursor_main = cursor_main.saturating_add(justify_between);
            }
        }

        // 3f. Advance the cross-axis cursor for the next line.
        current_cross = current_cross.saturating_add(line_cross);
        if line_idx + 1 < line_count {
            current_cross = current_cross.saturating_add(cross_gap);
        }
    }

    let total_cross = current_cross.saturating_sub(content_y);

    // 4. Apply `align-content` to shift the stacked lines. We do this
    //    by re-translating each already-placed child's cross-axis
    //    coordinate by the same delta; the size of the container
    //    itself is unaffected (the cross-axis size of a flex container
    //    is the natural / explicit height, not the content height of
    //    the lines).
    if line_count > 1 {
        let cross_axis_size = if has_explicit_height {
            container_height
        } else {
            total_cross
        };
        let total_lines_height: usize = (0..line_count)
            .map(|i| {
                let indices = &lines[i];
                let line_preps: Vec<&FlexItemPrep> = indices.iter().map(|j| &preps[*j]).collect();
                let natural = line_preps
                    .iter()
                    .map(|p| natural_cross_axis_size(arena, p.node_id, direction, font_ctx))
                    .max()
                    .unwrap_or(FLEX_ITEM_MIN_CROSS)
                    .max(FLEX_ITEM_MIN_CROSS);
                let line_cross = if has_explicit_height {
                    container_height
                } else {
                    natural
                };
                line_cross + if i + 1 < line_count { cross_gap } else { 0 }
            })
            .sum();
        let free_space = cross_axis_size.saturating_sub(total_lines_height);
        let (leading, between) = align_content_spacing(align_content, free_space, line_count);
        let mut shift: usize = leading;
        let mut running_offset: usize = 0;
        for (i, indices) in lines.iter().enumerate() {
            if i > 0 {
                shift = shift.saturating_add(between);
            }
            for (idx_pos, &_prep_idx) in indices.iter().enumerate() {
                let child = &mut placed[running_offset + idx_pos];
                let delta = match direction {
                    FlexDirection::Row => (0, shift as isize),
                    FlexDirection::Column => (shift as isize, 0),
                };
                child.rect.x = (child.rect.x as isize + delta.0).max(0) as usize;
                child.rect.y = (child.rect.y as isize + delta.1).max(0) as usize;
            }
            running_offset = running_offset.saturating_add(indices.len());
        }
    }

    (placed, total_cross)
}

/// Build a [`FlexItemPrep`] for one block child: resolve `flex-basis`,
/// read `flex-grow` / `flex-shrink` / `align-self` / `order`, and capture
/// the resolved main-axis size.
fn prepare_flex_item(
    arena: &NodeArena,
    item_id: NodeId,
    direction: FlexDirection,
    containing_main: usize,
    font_ctx: FontContext,
    align_items: AlignItems,
) -> FlexItemPrep {
    let node = &arena[item_id];
    let css = &node.css_attr;
    let basis_resolved = css
        .get_flex_basis_resolved(
            containing_main,
            font_ctx.font_size_cells,
            font_ctx.root_font_size_cells,
        )
        .or_else(|| {
            // `auto` fallback chain depends on the direction:
            // - `row` direction: main axis is horizontal, so an explicit
            //   `width` wins, and the natural fallback is the text
            //   content width (1 row of text typically has 1 cell per
            //   grapheme).
            // - `column` direction: main axis is vertical, so an
            //   explicit `height` wins. With no explicit height, the
            //   item is one row of text by default — `flex-grow` will
            //   pick up any remaining vertical space if needed.
            let explicit = match direction {
                FlexDirection::Row => css.get_width(
                    containing_main,
                    font_ctx.font_size_cells,
                    font_ctx.root_font_size_cells,
                ),
                FlexDirection::Column => css.get_height(
                    containing_main,
                    font_ctx.font_size_cells,
                    font_ctx.root_font_size_cells,
                ),
            };
            match direction {
                FlexDirection::Row => {
                    explicit.or_else(|| measure_text_content_width(arena, item_id))
                }
                FlexDirection::Column => explicit.or(Some(FLEX_ITEM_MIN_CROSS)),
            }
        })
        .unwrap_or(FLEX_ITEM_MIN_CROSS);

    // `align-self: auto` defers to the container's `align-items` (CSS
    // default for `align-items` is `stretch`, so without an explicit
    // `align-items` this still ends up as `Stretch`).
    FlexItemPrep {
        node_id: item_id,
        basis: basis_resolved,
        grow: css.get_flex_grow(),
        shrink: css.get_flex_shrink(),
        align_self: match css.get_align_self() {
            AlignSelf::Auto => align_items,
            AlignSelf::Value(v) => v,
        },
        order: css.get_order(),
    }
}

/// Split items into lines for `flex-wrap: wrap`.
///
/// Greedy algorithm: start a new line whenever adding the next item
/// (plus the column gap) would exceed the available main-axis size.
/// Items that are themselves wider than the line get the full line to
/// themselves (they overflow rather than wrap mid-item).
fn wrap_into_lines(
    preps: &[FlexItemPrep],
    direction: FlexDirection,
    available_main: usize,
    gap: GapSizes,
) -> Vec<Vec<usize>> {
    let column_gap = match direction {
        FlexDirection::Row => gap.column,
        FlexDirection::Column => gap.row,
    };
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut used: usize = 0;
    for (idx, prep) in preps.iter().enumerate() {
        let item_size = prep.basis.max(1);
        let needed = if current.is_empty() {
            item_size
        } else {
            used.saturating_add(column_gap).saturating_add(item_size)
        };
        if !current.is_empty() && needed > available_main {
            lines.push(std::mem::take(&mut current));
            used = 0;
        }
        current.push(idx);
        used = if current.len() == 1 {
            item_size
        } else {
            used.saturating_add(column_gap).saturating_add(item_size)
        };
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Distribute the line's main-axis size among its items.
///
/// Returns the final main-axis size of each item (in input order)
/// plus the remaining free space to be distributed by `justify-content`
/// during placement.
///
/// Algorithm:
/// 1. If `sum(basis) + gaps < available` AND at least one item has
///    `flex-grow > 0`, distribute the free space via `flex-grow`. The
///    grow absorbs the entire free space, leaving 0 for justify.
/// 2. If the line under-fills but no item has grow, the free space
///    passes through as the second return value for `justify-content`.
/// 3. If `sum(basis) + gaps > available`, distribute the overflow via
///    `flex-shrink` (no free space for justify).
/// 4. If the line exactly matches the available size, neither grow /
///    shrink nor justify has anything to do.
fn distribute_main_axis(
    line: &[&FlexItemPrep],
    available: usize,
    column_gap: usize,
) -> (Vec<usize>, usize) {
    let n = line.len();
    if n == 0 {
        return (Vec::new(), 0);
    }
    let gaps = column_gap.saturating_mul(n.saturating_sub(1));
    let total_basis: usize = line.iter().map(|p| p.basis).sum();
    let mut sizes: Vec<usize> = line.iter().map(|p| p.basis).collect();

    let used_after_basis = total_basis.saturating_add(gaps);
    if used_after_basis < available {
        // Free space exists; first try flex-grow, otherwise pass the
        // free space through to justify-content.
        let free = available - gaps - total_basis;
        let total_grow: f32 = line.iter().map(|p| p.grow).sum();
        if total_grow > 0.0 {
            let mut distributed: isize = 0;
            for (i, p) in line.iter().enumerate() {
                let share = ((free as f32) * (p.grow / total_grow)).floor() as isize;
                sizes[i] = (sizes[i] as isize + share).max(1) as usize;
                distributed += share;
            }
            // Distribute any floor-truncation remainder one cell at a
            // time across the first items, so the line exactly fills
            // the available main-axis size.
            let mut leftover = (free as isize) - distributed;
            let mut k = 0;
            while leftover > 0 && k < n {
                sizes[k] += 1;
                leftover -= 1;
                k += 1;
            }
            (sizes, 0)
        } else {
            (sizes, free)
        }
    } else if used_after_basis > available {
        // Overflow: distribute the overflow via flex-shrink.
        let overflow = used_after_basis.saturating_sub(available);
        let total_scaled_shrink: f32 = line.iter().map(|p| p.shrink * p.basis as f32).sum();
        if total_scaled_shrink > 0.0 {
            let mut distributed: isize = 0;
            for (i, p) in line.iter().enumerate() {
                let share = ((overflow as f32) * (p.shrink * p.basis as f32) / total_scaled_shrink)
                    .floor() as isize;
                let new_size = (sizes[i] as isize - share).max(1) as usize;
                distributed += (sizes[i] as isize) - (new_size as isize);
                sizes[i] = new_size;
            }
            // If the floor-truncation sum is short of the overflow,
            // trim additional cells from the largest items to make
            // up the difference.
            let mut leftover = overflow as isize - distributed;
            let mut k = 0;
            while leftover > 0 && k < n {
                if sizes[k] > 1 {
                    sizes[k] -= 1;
                    leftover -= 1;
                }
                k += 1;
            }
        }
        (sizes, 0)
    } else {
        (sizes, 0)
    }
}

/// Compute the leading + between spacing for `justify-content` /
/// `align-content`.
///
/// Mirrors the CSS distribution rules: `space-between` puts all free
/// space *between* items (no leading), `space-around` puts half a
/// share at the leading and trailing edges, `space-evenly` puts a
/// full share at every edge and gap.
fn justify_spacing(justify: JustifyContent, free_space: usize, n_items: usize) -> (usize, usize) {
    if n_items <= 1 {
        return match justify {
            JustifyContent::FlexStart | JustifyContent::SpaceBetween | JustifyContent::FlexEnd => {
                (free_space, 0)
            }
            JustifyContent::Center => (free_space / 2, 0),
            JustifyContent::SpaceAround => (free_space / 2, 0),
            JustifyContent::SpaceEvenly => (free_space, 0),
        };
    }
    match justify {
        JustifyContent::FlexStart => (0, 0),
        JustifyContent::FlexEnd => (free_space, 0),
        JustifyContent::Center => (free_space / 2, 0),
        JustifyContent::SpaceBetween => (0, free_space / (n_items - 1)),
        JustifyContent::SpaceAround => {
            let share = free_space / (2 * n_items);
            (share, share * 2)
        }
        JustifyContent::SpaceEvenly => {
            let share = free_space / (n_items + 1);
            (share, share)
        }
    }
}

/// Same as `justify_spacing` but for `align-content` (which uses the
/// same value set). Lines count == n_items for this calculation.
fn align_content_spacing(
    justify: JustifyContent,
    free_space: usize,
    n_lines: usize,
) -> (usize, usize) {
    justify_spacing(justify, free_space, n_lines)
}

/// Sum the grapheme widths of every direct `Text` descendant of `node_id`.
///
/// Used as the `flex-basis: auto` fallback when neither `width` /
/// `height` nor a `flex-basis` declaration is present. For a node with
/// nested block children this only counts its own direct text; a
/// recursive measure would be more accurate but is a v2 concern.
fn measure_text_content_width(arena: &NodeArena, node_id: NodeId) -> Option<usize> {
    let node = &arena[node_id];
    let mut total: Option<usize> = Some(0);
    for &child_id in &node.children {
        if let NodeType::Text(s) = &arena[child_id].node_type {
            let w: usize = s.graphemes(true).map(grapheme_width).sum();
            total = Some(total.unwrap_or(0).saturating_add(w));
        }
    }
    total
}

/// Natural cross-axis size of an item (its own height for `row`, its
/// own width for `column`).
///
/// The natural cross-axis size depends on the direction because the
/// cross axis swaps between rows and columns:
///
/// - `row` direction: cross axis is vertical. The natural size is the
///   item's own row count (`max(1, font_size_cells)`) — a single-row
///   text block is 1 row; a heading with `font-size: 2` is 2 rows.
/// - `column` direction: cross axis is horizontal. The natural size is
///   the item's text content width (sum of grapheme widths). For a
///   nested block item this only counts the direct text, so it may
///   under-report — see the v1 scope note in the module header.
///
/// Used by the line-height calculation in [`layout_flex_items`].
fn natural_cross_axis_size(
    arena: &NodeArena,
    node_id: NodeId,
    direction: FlexDirection,
    font_ctx: FontContext,
) -> usize {
    match direction {
        FlexDirection::Row => {
            let own_font_size_cells = effective_font_size_cells(
                arena,
                node_id,
                font_ctx.font_size_cells,
                font_ctx.root_font_size_cells,
            );
            own_font_size_cells.max(FLEX_ITEM_MIN_CROSS)
        }
        FlexDirection::Column => measure_text_content_width(arena, node_id)
            .unwrap_or(FLEX_ITEM_MIN_CROSS)
            .max(FLEX_ITEM_MIN_CROSS),
    }
}

/// Resolve an item's cross-axis size in the current line.
///
/// Honours `align-self` (item-level, already-resolved from `auto` by
/// `prepare_flex_item`). The natural cross-axis size of the item is
/// the default; `align-self: stretch` upgrades it to the line's
/// cross-axis size when the container has a known cross-axis extent:
///
/// - `row` direction: cross axis is vertical. The container's height
///   is only known when `height` is explicitly set, so stretch only
///   takes effect in that case (v1 contract — this avoids a circular
///   dependency where stretching items would enlarge the container).
/// - `column` direction: cross axis is horizontal. The container's
///   width is always known (it's `content.width`), so stretch always
///   takes effect.
fn resolve_item_cross_size(
    align_self: AlignItems,
    line_cross: usize,
    natural_cross: usize,
    has_explicit_height: bool,
    direction: FlexDirection,
) -> usize {
    if matches!(align_self, AlignItems::Stretch) {
        let can_stretch = match direction {
            FlexDirection::Row => has_explicit_height,
            FlexDirection::Column => true,
        };
        if can_stretch {
            line_cross
        } else {
            natural_cross.max(FLEX_ITEM_MIN_CROSS)
        }
    } else {
        natural_cross.max(FLEX_ITEM_MIN_CROSS)
    }
}

/// Effective per-item cross-axis alignment.
fn effective_align(align_self: AlignItems, _align_items: AlignItems) -> AlignItems {
    // The item-level `align-self` was already folded into `align_self`
    // by the caller (`prepare_flex_item`), so this is a pass-through.
    align_self
}

/// Position an item along the cross axis within its line.
fn align_on_cross_axis(
    line_origin: usize,
    line_size: usize,
    item_size: usize,
    align: AlignItems,
) -> usize {
    match align {
        AlignItems::FlexStart => line_origin,
        AlignItems::FlexEnd => line_origin.saturating_add(line_size.saturating_sub(item_size)),
        AlignItems::Center => line_origin.saturating_add(line_size.saturating_sub(item_size) / 2),
        AlignItems::Stretch => line_origin,
    }
}

/// Build a single flex item's [`LayoutBox`] at the given `rect`.
///
/// Mirrors the body of `layout_block` but skips the cursor reservation
/// — the item's `y` (for row direction) is fixed by the flex algorithm,
/// not by a shared vertical cursor. The item's own padding / border /
/// margin shrink the content area for its nested children, which are
/// laid out by the normal `layout_children` dispatch.
fn layout_flex_item(
    arena: &NodeArena,
    item_id: NodeId,
    rect: Rect,
    font_ctx: FontContext,
) -> LayoutBox {
    let node = &arena[item_id];

    let own_font_size_cells = effective_font_size_cells(
        arena,
        item_id,
        font_ctx.font_size_cells,
        font_ctx.root_font_size_cells,
    );
    let child_font_ctx = font_ctx.child(own_font_size_cells);

    let edges: BoxEdges = node.css_attr.get_box_edges(
        rect.width,
        own_font_size_cells,
        font_ctx.root_font_size_cells,
    );

    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let body_runs = collect_runs(arena, node, block_fg, block_bg, block_font_style);
    let fill_bg = block_bg.is_some();

    let inner_h = edges.inner_horizontal();
    let content_x = rect.x + edges.border.left.thickness() + edges.padding.left;
    let content_y = rect.y + edges.border.top.thickness() + edges.padding.top;
    let content_width = rect.width.saturating_sub(inner_h).max(1);

    let child_constraint = BoxConstraint {
        x: content_x,
        width: content_width,
    };
    let mut child_cursor = Cursor { y: content_y };
    let nested_children = node.children.clone();
    // Filter nested children to block-level ones for the recursive
    // dispatch. (The dispatcher will already drop non-block types, but
    // the same guard keeps the v1 contract visible: only block
    // children participate as flex items, and the same policy applies
    // to nested levels.)
    let block_nested: Vec<NodeId> = nested_children
        .into_iter()
        .filter(|id| {
            is_block_child_node(&arena[*id].node_type) || {
                // A flex container child can also be a non-block element
                // that has `display: flex` set (the dispatcher will
                // promote it to a flex container). Detect that explicitly.
                arena[*id].css_attr.is_flex()
            }
        })
        .collect();
    let children = layout_children(
        arena,
        &block_nested,
        child_constraint,
        &mut child_cursor,
        child_font_ctx,
    );

    LayoutBox::new_block(
        item_id,
        rect,
        edges,
        fill_bg,
        0,
        body_runs,
        children,
        own_font_size_cells,
    )
}
