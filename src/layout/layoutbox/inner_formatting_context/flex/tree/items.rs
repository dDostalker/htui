//! Per-line placement: prepare items, distribute main-axis space, run
//! `justify-content` and `align-items`, recurse into the element registry.

use crate::{
    html::css_parse::{AlignItems, FlexDirection, FlexWrap, GapSizes, JustifyContent},
    layout::{BoxConstraint, Cursor, FontContext, LayoutBox, Rect},
    tdom::node::NodeArena,
};

use super::algorithm::{
    align_content_spacing, align_on_cross_axis, distribute_main_axis, effective_align,
    justify_spacing, wrap_into_lines,
};
use super::container::{FlexItemPrep, FLEX_ITEM_MIN_CROSS};
use super::prepare::{
    natural_cross_axis_size, prepare_flex_item, resolve_item_cross_size,
};

/// Lay out every flex item across one or more lines and return the
/// positioned children plus the total main-axis-perpendicular height
/// they consume (used to size the container).
#[allow(clippy::too_many_arguments)]
pub(super) fn layout_flex_items(
    arena: &NodeArena,
    item_ids: &[crate::tdom::node::NodeId],
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
            let child_box = crate::layout::inner_formatting_context::flow::dispatch(
                arena,
                prep.node_id,
                BoxConstraint {
                    x: item_x,
                    width: item_w.max(1),
                },
                &mut Cursor { y: item_y },
                font_ctx,
                Some(Rect::new(item_x, item_y, item_w.max(1), item_h.max(1))),
            );
            if let Some(child_box) = child_box {
                // A flex item keeps its own inner formatting context (a plain
                // `<div>` item remains `Flow`). The flex container itself
                // is what establishes `FormattingContext::Flex` for its
                // children, not each child.
                placed.push(child_box);
            } else {
                log::warn!(
                    "No layout handler for flex item {:?}",
                    arena[prep.node_id].node_type
                );
            }
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
