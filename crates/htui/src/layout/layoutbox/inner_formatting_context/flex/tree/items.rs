//! Per-line placement: prepare items, distribute main-axis space, run
//! `justify-content` and `align-items`, recurse into the element registry.

use crate::{
    html::box_edges::BoxEdges,
    html::css_parse::{AlignContent, AlignItems, FlexDirection, FlexWrap, GapSizes, JustifyContent},
    layout::{
        BoxConstraint, Cursor, LayoutBox, LayoutContext, OuterDisplay, PaintStyle, Rect,
        layoutbox::wrap_runs,
    },
    tdom::node::{NodeArena, NodeId},
};

use super::algorithm::{
    align_content_spacing, align_on_cross_axis, distribute_main_axis, effective_align,
    justify_spacing, wrap_into_lines,
};
use super::container::{AnonymousTextItem, FLEX_ITEM_MIN_CROSS, FlexItemPrep, FlexItemSource};
use super::prepare::{natural_cross_axis_size_for_prep, resolve_item_cross_size};

/// Lay out every flex item across one or more lines and return the
/// positioned children plus the vertical extent they consume (used to size
/// the container).
#[allow(clippy::too_many_arguments)]
pub(super) fn layout_flex_items(
    arena: &NodeArena,
    container_node_id: NodeId,
    mut preps: Vec<FlexItemPrep>,
    direction: FlexDirection,
    wrap: FlexWrap,
    justify: JustifyContent,
    align_items: AlignItems,
    align_content: AlignContent,
    gap: GapSizes,
    content: BoxConstraint,
    content_y: usize,
    context: LayoutContext,
    has_explicit_height: bool,
    container_height: usize,
) -> (Vec<LayoutBox>, usize) {
    // 1. Sort every prepared item by `order`.
    preps.sort_by_key(|p| p.order);

    // 2. Split into lines based on `wrap`. The wrap threshold is the main
    //    axis size: the container width for rows, the explicit height for
    //    columns (an indefinite column has no wrap threshold).
    let available_main = match direction.is_row() {
        true => content.width,
        false if has_explicit_height => container_height,
        false => usize::MAX,
    };
    let lines: Vec<Vec<usize>> = match wrap {
        FlexWrap::NoWrap => vec![(0..preps.len()).collect()],
        FlexWrap::Wrap | FlexWrap::WrapReverse => {
            wrap_into_lines(&preps, direction, available_main, gap)
        }
    };

    // 3. Lay out each line and stack them along the cross axis.
    let mut placed: Vec<LayoutBox> = Vec::with_capacity(preps.len());
    // `current_cross` is the cursor on the axis perpendicular to the
    // flex main axis: vertical for rows, horizontal for columns.
    let cross_origin = match direction.is_row() {
        true => content_y,
        false => content.x,
    };
    let mut current_cross: usize = cross_origin;
    let mut total_main: usize = 0;
    let line_count = lines.len();
    let (cross_gap, main_gap) = match direction.is_row() {
        true => (gap.row, gap.column),
        false => (gap.column, gap.row),
    };
    // Actual cross-axis extent of each line once its items are laid out
    // (relative to the line origin). An item can grow taller than the
    // natural max-content measure assumed (text wrapping at the item's
    // final width), so the line / container must expand to cover it.
    let mut line_extents: Vec<usize> = Vec::with_capacity(line_count);

    // Pre-measure every line's natural cross size so `align-content:
    // stretch` can grow the lines (filling the container edge-to-edge — the
    // browser semantics) instead of spacing them. Non-stretch values keep
    // the classic leading / between offset computed from the same free
    // space. A row container without an explicit height has no definite
    // cross axis, so `stretch` is a no-op there (CSS).
    let mut natural_crosses: Vec<usize> = Vec::with_capacity(line_count);
    for line_indices in &lines {
        let line_preps: Vec<&FlexItemPrep> = line_indices.iter().map(|&i| &preps[i]).collect();
        natural_crosses.push(
            line_preps
                .iter()
                .map(|p| {
                    natural_cross_axis_size_for_prep(arena, p, direction, context)
                        .saturating_add(p.margin_cross_before)
                        .saturating_add(p.margin_cross_after)
                })
                .max()
                .unwrap_or(FLEX_ITEM_MIN_CROSS)
                .max(FLEX_ITEM_MIN_CROSS),
        );
    }
    let stretch_cross_axis = match direction.is_row() {
        true => has_explicit_height.then_some(container_height),
        false => Some(content.width),
    };
    let total_natural_cross: usize = natural_crosses
        .iter()
        .enumerate()
        .map(|(i, &n)| n + if i + 1 < line_count { cross_gap } else { 0 })
        .sum();
    let align_free_space = stretch_cross_axis
        .map(|size| size.saturating_sub(total_natural_cross))
        .unwrap_or(0);
    // `stretch` grows every line equally; the remainder is left unused
    // (the last line ends short of the container edge, matching the
    // browser's float math once truncated to integer cells).
    let stretch_extra = if matches!(align_content, AlignContent::Stretch) && line_count > 1 {
        align_free_space / line_count
    } else {
        0
    };

    for (line_idx, line_indices) in lines.iter().enumerate() {
        // 3a. Distribute the line's main-axis space.
        let line_preps: Vec<&FlexItemPrep> = line_indices.iter().map(|&i| &preps[i]).collect();
        // The main-axis size is definite for `row` (always `content.width`)
        // and for `column` when the container has an explicit height (from
        // CSS `height` or a preallocated rect). A `column` without an
        // explicit height is *indefinite* — its main size is determined by
        // the items themselves, so there is no free space for `flex-grow`
        // to consume (CSS Flexbox §9.7). `indefinite` is forwarded to
        // `distribute_main_axis` so it can skip grow/shrink rather than
        // distribute an `usize::MAX` free space that overflows `isize`.
        let indefinite = direction.is_column() && !has_explicit_height;
        let line_main_size = match direction.is_row() {
            true => content.width,
            false if has_explicit_height => container_height,
            false => usize::MAX,
        };
        let (main_sizes, free_for_justify) =
            distribute_main_axis(&line_preps, line_main_size, main_gap, indefinite);

        // 3b. Natural cross-axis size of this line (pre-measured above).
        let natural_line_cross = natural_crosses[line_idx];

        // 3c. Effective cross-axis size. The cross axis swaps between the
        //     two directions, so this is direction-aware:
        //
        //     - `row` direction: cross axis is vertical. Explicit
        //       container height wins (and only for a single line —
        //       otherwise every line would overflow the container).
        //       Without an explicit height we use the natural line cross
        //       to avoid a circular dependency where stretching items
        //       would silently enlarge the container.
        //     - `column` direction: cross axis is horizontal. The
        //       container's content width is always known (it's
        //       `content.width`), so the line stretches to that
        //       regardless of an explicit height. This must NOT be
        //       `max`ed with the natural (max-content) line cross: the
        //       intrinsic width is width-independent, so a column whose
        //       content is wider than the viewport (e.g. a sidebar +
        //       main layout on a narrow terminal) would size every line
        //       to the intrinsic width and overflow the container. A
        //       multi-line column container has no concept of
        //       cross-axis stretching per line — lines stack along the
        //       main axis, so we keep the natural line cross for them.
        //       The previous `container_height` shortcut was incorrect
        //       for column direction: it used a vertical measurement as
        //       a horizontal size, which made children of a flex column
        //       stretch to the column's height — and overflow the
        //       column's own width.
        let line_cross = match direction.is_row() {
            true => {
                if has_explicit_height && line_count == 1 {
                    // A single-line row container with an explicit height
                    // sizes its line to that height (so `align-items:
                    // stretch` fills the container). But never smaller than
                    // the line's natural cross size: a flex item taller than
                    // the container (e.g. nested content that measures
                    // larger than the viewport-height constraint) must not
                    // overflow into the paint clip, or its last rows are
                    // silently dropped.
                    container_height.max(natural_line_cross)
                } else {
                    natural_line_cross.saturating_add(stretch_extra)
                }
            }
            false => {
                if line_count == 1 {
                    content.width
                } else {
                    natural_line_cross.saturating_add(stretch_extra)
                }
            }
        };

        // 3d. Compute justify-content leading + between spacing.
        let (justify_leading, justify_between) =
            justify_spacing(justify, free_for_justify, line_preps.len());

        // 3e. Per-line placement, factored so the line's cross size can be
        // measured first and the line re-placed at the final size. An item's
        // content can wrap taller than the natural max-content measure
        // (e.g. `812 MB/s` in a narrow card), so the first pass measures the
        // real line extent; the second pass stretches every `align-items:
        // stretch` item to it — matching a browser where the line's cross
        // size is the largest item's, and the items fill it evenly.
        let line_origin = current_cross;
        let place_line = |line_cross: usize| -> (Vec<LayoutBox>, usize, usize) {
            let mut boxes: Vec<LayoutBox> = Vec::with_capacity(line_preps.len());
            let mut line_bottom: usize = line_origin.saturating_add(line_cross);
            let mut cursor_main: usize = match direction.is_row() {
                true => content.x,
                false => content_y,
            };
            cursor_main = cursor_main.saturating_add(justify_leading);
            // Main-axis space the line actually occupies (row: sum of item
            // widths; column: sum of item heights, which can exceed the
            // distributed sizes when wrapped text grows an item).
            let mut placed_main: usize = 0;
            // The line's shared text baseline (row direction): the largest
            // first-text-row offset among `baseline`-aligned items, so those
            // items' first text rows coincide (the terminal cell-grid
            // analogue of the CSS baseline).
            let line_baseline = line_preps
                .iter()
                .filter(|p| {
                    direction.is_row()
                        && matches!(
                            effective_align(p.align_self, align_items),
                            AlignItems::Baseline
                        )
                })
                .map(|p| p.baseline)
                .max()
                .unwrap_or(0);
            for (prep_idx, prep) in line_preps.iter().enumerate() {
                // CSS `min-width` / `max-width` (row) or `min-height` /
                // `max-height` (column) clamp the final main-axis size. The
                // automatic min-content floor (`min_main`) is already enforced
                // by the shrink distribution; explicit constraints layer on top.
                // When min > max, the minimum wins (CSS).
                let lower = prep.min_main.max(prep.css_min_main.unwrap_or(0));
                let main_size = match prep.css_max_main {
                    Some(max) => main_sizes[prep_idx].clamp(lower, max.max(lower)),
                    None => main_sizes[prep_idx].max(lower),
                };
                // Natural cross-axis size of this item (used as the fallback
                // for non-stretch alignments and for stretch when the
                // container's cross-axis size is unknown).
                let item_natural_cross =
                    natural_cross_axis_size_for_prep(arena, prep, direction, context);
                let item_cross_size = resolve_item_cross_size(
                    prep.align_self,
                    line_cross,
                    item_natural_cross,
                    (prep.margin_cross_before, prep.margin_cross_after),
                );
                let align = effective_align(prep.align_self, align_items);
                let margin_before = prep.margin_main_before;
                let margin_after = prep.margin_main_after;
                // Cross-axis alignment operates on the item's *margin-box*
                // (browsers align the margin box within the line). The
                // leading margin then offsets the border-box from the
                // margin-box top.
                let margin_box = item_cross_size
                    .saturating_add(prep.margin_cross_before)
                    .saturating_add(prep.margin_cross_after);
                let (item_x, item_y, item_w, item_h) = match direction.is_row() {
                    true => {
                        let y = if matches!(align, AlignItems::Baseline) {
                            // Baseline: the item's first text row sits on the
                            // line's shared baseline.
                            line_origin
                                .saturating_add(line_baseline)
                                .saturating_sub(prep.baseline)
                        } else {
                            align_on_cross_axis(line_origin, line_cross, margin_box, align)
                                .saturating_add(prep.margin_cross_before)
                        };
                        (
                            cursor_main.saturating_add(margin_before),
                            y,
                            main_size,
                            item_cross_size,
                        )
                    }
                    false => {
                        let x = align_on_cross_axis(line_origin, line_cross, margin_box, align)
                            .saturating_add(prep.margin_cross_before);
                        (
                            x,
                            cursor_main.saturating_add(margin_before),
                            item_cross_size,
                            main_size,
                        )
                    }
                };
                let item_rect = Rect::new(item_x, item_y, item_w.max(1), item_h.max(1));
                let child_box = match &prep.source {
                    FlexItemSource::Node(item_id) => {
                        crate::layout::inner_formatting_context::flow::dispatch(
                            arena,
                            *item_id,
                            BoxConstraint {
                                x: item_x,
                                width: item_w.max(1),
                            },
                            &mut Cursor {
                                y: item_y,
                                prev_bottom_margin: 0,
                            },
                            context,
                            Some(item_rect),
                        )
                    }
                    FlexItemSource::AnonymousText(text) => {
                        Some(layout_anonymous_text(text, item_rect, container_node_id))
                    }
                };
                // Main-axis extent the item actually occupies, measured from
                // the placement origin. For `row` it equals the distributed
                // width; for `column` the laid-out box can grow taller than
                // the distributed main size (wrapped text), and the next item
                // must start below the real extent instead of overlapping it.
                let mut actual_main_extent: usize = 0;
                if let Some(child_box) = child_box {
                    // A flex item keeps its own inner formatting context (a
                    // plain `<div>` item remains `Flow`). The flex container
                    // itself is what establishes `FormattingContext::Flex`
                    // for its children, not each child.
                    //
                    // The item's content can exceed the pre-allocated cross
                    // size (text wrapping at the final width makes a card
                    // taller than the max-content measure), so grow the line
                    // to the actual box extent — otherwise the container's
                    // paint clip cuts the item's bottom border and its last
                    // row bleeds into the border.
                    line_bottom = line_bottom.max(
                        child_box
                            .rect
                            .y
                            .saturating_add(child_box.rect.height.max(1))
                            // The item's trailing cross-axis margin sits below
                            // its border-box, so the line must cover it too
                            // (the line is the largest item's margin-box).
                            .saturating_add(prep.margin_cross_after),
                    );
                    if direction.is_column() {
                        actual_main_extent = child_box
                            .rect
                            .y
                            .saturating_add(child_box.rect.height)
                            .saturating_sub(cursor_main);
                    }
                    boxes.push(child_box);
                } else if let FlexItemSource::Node(item_id) = &prep.source {
                    log::warn!(
                        "No layout handler for flex item {:?}",
                        arena[*item_id].node_type
                    );
                }
                let step = match direction.is_row() {
                    true => main_size,
                    false => main_size.max(actual_main_extent),
                };
                // The item's margins occupy the line on both sides of its
                // border-box and advance the next item.
                let step = step.saturating_add(margin_before).saturating_add(margin_after);
                cursor_main = cursor_main.saturating_add(step);
                placed_main = placed_main.saturating_add(step);
                if prep_idx + 1 < line_preps.len() {
                    cursor_main = cursor_main.saturating_add(main_gap);
                    cursor_main = cursor_main.saturating_add(justify_between);
                    placed_main = placed_main.saturating_add(main_gap);
                    placed_main = placed_main.saturating_add(justify_between);
                }
            }
            // The line's cross-axis extent used to stack the next line:
            // rows grow vertically (wrapped content), columns stack along
            // the fixed content width (the line cross), never the item
            // heights (which are the column *main* axis).
            let extent = if direction.is_row() {
                line_bottom.saturating_sub(line_origin)
            } else {
                line_cross
            };
            (boxes, extent, placed_main)
        };

        // Measurement pass at the tentative (natural) cross size, then a
        // placement pass at the final size when items wrapped taller. The
        // re-layout only happens when the final line cross differs.
        let (first_boxes, first_extent, first_placed_main) = place_line(line_cross);
        // For `row` direction the cross axis is vertical, so wrapped content
        // can grow the line and the re-place stretches `align-items: stretch`
        // items to the real height. For `column` direction the cross axis is
        // the container's fixed content width: `first_extent` here is the
        // line's *main-axis* (height) extent, so using it as the cross size
        // would re-place every item WIDER than the container whenever the
        // content is taller than the viewport is wide (a dashboard on a
        // narrow terminal) — the overflow. A column's line cross is always
        // the container width, so no re-place.
        let final_line_cross = match direction.is_row() {
            true => first_extent.max(line_cross),
            false => line_cross,
        };
        let (line_boxes, actual_extent, placed_main) = if final_line_cross > line_cross {
            place_line(final_line_cross)
        } else {
            (first_boxes, first_extent, first_placed_main)
        };
        placed.extend(line_boxes);

        // The container height is the line's main-axis extent in column
        // direction. `current_cross` tracks the perpendicular axis and is
        // therefore not suitable for sizing a column's block-height. The
        // actual `placed_main` is used so wrapped (grown) items extend the
        // container instead of being clipped by it.
        total_main = total_main.max(justify_leading.saturating_add(placed_main));
        line_extents.push(actual_extent);

        // 3f. Advance the cross-axis cursor for the next line.
        current_cross = line_origin.saturating_add(actual_extent);
        if line_idx + 1 < line_count {
            current_cross = current_cross.saturating_add(cross_gap);
        }
    }

    let total_cross = current_cross.saturating_sub(cross_origin);

    // 4. Apply `align-content` to shift the stacked lines. We do this
    //    by re-translating each already-placed child's cross-axis
    //    coordinate by the same delta. The available cross-axis size is
    //    direction-aware: row containers may use an explicit height,
    //    while column containers use their known content width.
    if line_count > 1 {
        let cross_axis_size = match direction.is_row() {
            true => {
                if has_explicit_height {
                    container_height
                } else {
                    total_cross
                }
            }
            false => content.width,
        };
        let total_lines_cross: usize = line_extents
            .iter()
            .copied()
            .enumerate()
            .map(|(i, ext)| ext + if i + 1 < line_count { cross_gap } else { 0 })
            .sum();
        let free_space = cross_axis_size.saturating_sub(total_lines_cross);
        let (leading, between) = align_content_spacing(align_content, free_space, line_count);
        let mut shift: usize = leading;
        let mut running_offset: usize = 0;
        for (i, indices) in lines.iter().enumerate() {
            if i > 0 {
                shift = shift.saturating_add(between);
            }
            for (idx_pos, &_prep_idx) in indices.iter().enumerate() {
                let child = &mut placed[running_offset + idx_pos];
                let delta = match direction.is_row() {
                    true => (0, shift as isize),
                    false => (shift as isize, 0),
                };
                child.rect.x = (child.rect.x as isize + delta.0).max(0) as usize;
                child.rect.y = (child.rect.y as isize + delta.1).max(0) as usize;
            }
            running_offset = running_offset.saturating_add(indices.len());
        }
    }

    // 5. Reverse axes. `row-reverse` / `column-reverse` mirror each item
    //    along the main axis (free space moves to the other side, which is
    //    exactly what the reversed `justify-content` semantics need);
    //    `wrap-reverse` mirrors the stacked lines along the cross axis.
    //    Mirroring *after* `align-content` maps its top-anchored
    //    distribution to the bottom-anchored one wrap-reverse implies.
    let main_origin = match direction.is_row() {
        true => content.x,
        false => content_y,
    };
    let main_end = match direction.is_row() {
        true => content.x.saturating_add(content.width),
        false => {
            if has_explicit_height {
                content_y.saturating_add(container_height)
            } else {
                content_y.saturating_add(total_main)
            }
        }
    };
    if direction.is_reverse() {
        for child in &mut placed {
            let (main_pos, main_size) = if direction.is_row() {
                (child.rect.x, child.rect.width)
            } else {
                (child.rect.y, child.rect.height)
            };
            let new_main = main_end
                .saturating_sub(main_pos.saturating_sub(main_origin))
                .saturating_sub(main_size);
            if direction.is_row() {
                child.rect.x = new_main;
            } else {
                child.rect.y = new_main;
            }
        }
    }
    if matches!(wrap, FlexWrap::WrapReverse) {
        // The mirror axis is the container's cross-axis extent (a definite
        // height for rows, the content width for columns), so wrapped lines
        // anchor to the cross-end — matching a browser where `wrap-reverse`
        // stacks lines from the cross-start (the bottom / right edge). Using
        // the *content* extent instead would float the lines in the middle
        // of the container whenever it has free cross space.
        let cross_end = match direction.is_row() {
            true if has_explicit_height => cross_origin.saturating_add(container_height),
            false => cross_origin.saturating_add(content.width),
            _ => cross_origin.saturating_add(total_cross),
        };
        for child in &mut placed {
            let (cross_pos, cross_size) = if direction.is_row() {
                (child.rect.y, child.rect.height)
            } else {
                (child.rect.x, child.rect.width)
            };
            let new_cross = cross_end
                .saturating_sub(cross_pos.saturating_sub(cross_origin))
                .saturating_sub(cross_size);
            if direction.is_row() {
                child.rect.y = new_cross;
            } else {
                child.rect.x = new_cross;
            }
        }
    }

    let total_height = match direction.is_row() {
        true => total_cross,
        false => total_main,
    };

    (placed, total_height)
}

/// Lay out an anonymous text flex item: wrap its runs at the allocated width
/// and build a block box. Anonymous items have no DOM node of their own, so
/// the box reports the flex container's node id for hit-testing.
fn layout_anonymous_text(
    text: &AnonymousTextItem,
    rect: Rect,
    container_node_id: NodeId,
) -> LayoutBox {
    let wrapped = wrap_runs(
        &text.runs,
        rect.width.max(1),
        text.font_size_cells,
        text.white_space,
        text.word_break,
        text.overflow_wrap,
    );
    let has_lines = !wrapped.is_empty() && wrapped.iter().any(|l| !l.is_empty());
    let inline_rows = if has_lines {
        wrapped.len().saturating_mul(text.line_height_cells.max(1))
    } else {
        0
    };
    let rect_height = rect.height.max(inline_rows.max(1));
    let paint_style = PaintStyle {
        box_edges: BoxEdges::default(),
        runs: if has_lines { Some(wrapped) } else { None },
        font_size_cells: text.font_size_cells,
        line_height_cells: text.line_height_cells,
        ..PaintStyle::default()
    };
    LayoutBox::new_normal(
        container_node_id,
        Rect::new(rect.x, rect.y, rect.width, rect_height),
        paint_style,
        OuterDisplay::BlockLevel { container: true },
        Vec::new(),
    )
}
