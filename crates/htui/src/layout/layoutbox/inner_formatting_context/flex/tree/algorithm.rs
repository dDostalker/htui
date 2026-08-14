//! Flex algorithm primitives: wrap into lines, distribute main-axis space,
//! apply `justify-content` / `align-content`, place items on the cross axis.

use crate::html::css_parse::{AlignContent, AlignItems, FlexDirection, GapSizes, JustifyContent};

use super::container::FlexItemPrep;

/// Effective per-item cross-axis alignment.
///
/// `align-self: auto` is resolved by the flex container in
/// [`super::prepare::prepare_flex_item`], so by the time the cross-axis is
/// placed the value already reflects `align-self` falling back to the
/// container's `align-items`. This function is the single seam where that
/// fallback could grow richer (e.g. honouring `auto` -> `stretch` per-item).
pub(super) fn effective_align(align_self: AlignItems, _align_items: AlignItems) -> AlignItems {
    align_self
}

/// Split items into lines for `flex-wrap: wrap`.
///
/// Greedy algorithm: start a new line whenever adding the next item
/// (plus the column gap) would exceed the available main-axis size.
/// Items that are themselves wider than the line get the full line to
/// themselves (they overflow rather than wrap mid-item).
pub(super) fn wrap_into_lines(
    preps: &[FlexItemPrep],
    direction: FlexDirection,
    available_main: usize,
    gap: GapSizes,
) -> Vec<Vec<usize>> {
    let column_gap = match direction.is_row() {
        true => gap.column,
        false => gap.row,
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
pub(super) fn distribute_main_axis(
    line: &[&FlexItemPrep],
    available: usize,
    column_gap: usize,
    indefinite: bool,
) -> (Vec<usize>, usize) {
    let n = line.len();
    if n == 0 {
        return (Vec::new(), 0);
    }
    let gaps = column_gap.saturating_mul(n.saturating_sub(1));
    let total_basis: usize = line.iter().map(|p| p.basis).sum();
    let mut sizes: Vec<usize> = line.iter().map(|p| p.basis).collect();

    // An indefinite main axis (a `column` flex container with no explicit
    // height) has no free space to distribute: `flex-grow` and
    // `flex-shrink` are no-ops and the container's main size is exactly
    // the sum of its items' bases. Without this guard `available` is
    // `usize::MAX`, which makes `free` overflow when cast to `isize` in
    // the grow branch below (debug-build panic: attempt to add with
    // overflow).
    if indefinite {
        return (sizes, 0);
    }

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
            // CSS `min-width` / `min-height: auto`: no item may shrink
            // below its min-content floor. Clamp the violated items and
            // re-distribute the remaining overflow among the items that
            // can still shrink. If every item is at its floor and the
            // line still overflows, the floors win and the line extends
            // beyond the container (visible overflow, like a browser).
            enforce_min_main_sizes(&mut sizes, line, available, column_gap);
        }
        (sizes, 0)
    } else {
        (sizes, 0)
    }
}

/// Clamp each size to its min-content floor and redistribute the leftover
/// overflow among items that can still shrink.
///
/// CSS flex layout resolves flexible lengths iteratively: items that hit
/// their minimum become frozen at that size, and the remaining shrink is
/// split among the still-shrinkable items. If clamping leaves the line
/// larger than the container, the floors are kept and the line overflows
/// (matching a browser's visible overflow for an over-constrained
/// container).
fn enforce_min_main_sizes(
    sizes: &mut [usize],
    line: &[&FlexItemPrep],
    available: usize,
    column_gap: usize,
) {
    let n = sizes.len();
    let mins: Vec<usize> = line.iter().map(|p| p.min_main.max(1)).collect();
    for i in 0..n {
        if sizes[i] < mins[i] {
            sizes[i] = mins[i];
        }
    }
    loop {
        let total: usize = sizes
            .iter()
            .sum::<usize>()
            .saturating_add(column_gap.saturating_mul(n.saturating_sub(1)));
        if total <= available {
            break;
        }
        let overflow = total - available;
        // Items still above their floor can absorb more shrink.
        let candidates: Vec<usize> = (0..n).filter(|&i| sizes[i] > mins[i]).collect();
        if candidates.is_empty() {
            break; // everything at its floor; the line overflows.
        }
        let total_excess: usize = candidates.iter().map(|&i| sizes[i] - mins[i]).sum();
        let mut reduced: usize = 0;
        for &i in &candidates {
            let share = overflow.saturating_mul(sizes[i] - mins[i]) / total_excess.max(1);
            let new = sizes[i].saturating_sub(share).max(mins[i]);
            reduced = reduced.saturating_add(sizes[i] - new);
            sizes[i] = new;
        }
        if reduced == 0 {
            break; // nothing changed; avoid an infinite loop.
        }
    }
}

/// Compute the leading + between spacing for `justify-content` /
/// `align-content`.
///
/// Mirrors the CSS distribution rules: `space-between` puts all free
/// space *between* items (no leading), `space-around` puts half a
/// share at the leading and trailing edges, `space-evenly` puts a
/// full share at every edge and gap.
pub(super) fn justify_spacing(
    justify: JustifyContent,
    free_space: usize,
    n_items: usize,
) -> (usize, usize) {
    if n_items <= 1 {
        return match justify {
            JustifyContent::FlexStart | JustifyContent::SpaceBetween => (0, 0),
            JustifyContent::FlexEnd => (free_space, 0),
            JustifyContent::Center | JustifyContent::SpaceAround => (free_space / 2, 0),
            JustifyContent::SpaceEvenly => (free_space, 0),
        };
    }
    match justify {
        JustifyContent::FlexStart => (0, 0),
        JustifyContent::FlexEnd => (free_space, 0),
        JustifyContent::Center => (free_space / 2, 0),
        JustifyContent::SpaceBetween => (0, free_space / (n_items - 1)),
        JustifyContent::SpaceAround => {
            // CSS space-around: each edge gap = half of the between gap.
            // Total = 2 * edge + (n-1) * between = n * between, so
            //   between = free_space / n
            //   leading = (free_space - (n - 1) * between) / 2
            // The previous formula `share = f / (2n), (share, share*2)`
            // collapses to zero whenever `free_space < 2n` (3 items in 4
            // cells of free space → all gaps = 0, items stacked flush).
            let between = free_space / n_items;
            let leading = (free_space - between * (n_items - 1)) / 2;
            (leading, between)
        }
        JustifyContent::SpaceEvenly => {
            let share = free_space / (n_items + 1);
            (share, share)
        }
    }
}

/// Same as `justify_spacing` but for `align-content` (which uses the
/// same value set plus `stretch`). Lines count == n_items for this
/// calculation.
pub(super) fn align_content_spacing(
    align: AlignContent,
    free_space: usize,
    n_lines: usize,
) -> (usize, usize) {
    match align {
        // `stretch` fills the free cross space by growing each line's cross
        // size by `free / n` (no leading / trailing / inter-line gap) — the
        // browser semantics. The line growth itself is applied in
        // `layout_flex_items` via `stretch_extra`; the pair returned here is
        // only the line-offset distribution, which for stretch is zero (lines
        // are already stacked edge-to-edge by their grown sizes).
        AlignContent::Stretch => (0, 0),
        AlignContent::FlexStart
        | AlignContent::FlexEnd
        | AlignContent::Center
        | AlignContent::SpaceBetween
        | AlignContent::SpaceAround
        | AlignContent::SpaceEvenly => justify_spacing(
            match align {
                AlignContent::FlexStart => JustifyContent::FlexStart,
                AlignContent::FlexEnd => JustifyContent::FlexEnd,
                AlignContent::Center => JustifyContent::Center,
                AlignContent::SpaceBetween => JustifyContent::SpaceBetween,
                AlignContent::SpaceAround => JustifyContent::SpaceAround,
                _ => JustifyContent::SpaceEvenly,
            },
            free_space,
            n_lines,
        ),
    }
}

/// Position an item along the cross axis within its line.
///
/// `Baseline` is handled by the caller (it needs the per-item baseline
/// offset); reaching it here falls back to [`AlignItems::FlexStart`].
pub(super) fn align_on_cross_axis(
    line_origin: usize,
    line_size: usize,
    item_size: usize,
    align: AlignItems,
) -> usize {
    match align {
        AlignItems::FlexStart | AlignItems::Baseline => line_origin,
        AlignItems::FlexEnd => line_origin.saturating_add(line_size.saturating_sub(item_size)),
        AlignItems::Center => line_origin.saturating_add(line_size.saturating_sub(item_size) / 2),
        AlignItems::Stretch => line_origin,
    }
}
