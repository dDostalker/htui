//! Per-item preparation: build a [`FlexItemPrep`] from CSS properties and
//! measure each item's natural cross-axis size.

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    html::css_parse::{AlignItems, AlignSelf, FlexDirection},
    layout::FontContext,
    tdom::node::{NodeArena, NodeId, NodeType, effective_font_size_cells},
    tui::graph_api::grapheme_width,
};

use super::container::{FlexItemPrep, FLEX_ITEM_MIN_CROSS};
use super::dispatch::is_flex_capable_node;


/// Build a [`FlexItemPrep`] for one block child: resolve `flex-basis`,
/// read `flex-grow` / `flex-shrink` / `align-self` / `order`, and capture
/// the resolved main-axis size.
pub(super) fn prepare_flex_item(
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
            //
            // When the item is itself a flex container, the recursive
            // intrinsic measurement replaces `measure_text_content_width`
            // so a flex column nested inside a flex row (or any deeper
            // nesting) reports its real content size — not 0 — which
            // keeps the parent from collapsing the nested container to
            // 1 cell.
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
                FlexDirection::Row => explicit.or_else(|| {
                    let intrinsic = natural_intrinsic_size(arena, item_id, font_ctx);
                    Some(intrinsic.width)
                }),
                FlexDirection::Column => explicit.or_else(|| {
                    let intrinsic = natural_intrinsic_size(arena, item_id, font_ctx);
                    Some(intrinsic.height)
                }),
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

/// Sum the grapheme widths of every direct `Text` descendant of `node_id`.
///
/// Used as a low-level measurement helper by the recursive
/// [`natural_intrinsic_size`] pass: a non-flex leaf is sized by the
/// grapheme widths of its own direct text. For a node with nested block
/// children this only counts the direct text — nested blocks are
/// measured by [`natural_intrinsic_size`] itself.
pub(super) fn measure_text_content_width(arena: &NodeArena, node_id: NodeId) -> Option<usize> {
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

/// Intrinsic outer (border-box) size of one block node, computed
/// recursively so nested flex containers report the real dimensions of
/// their children rather than collapsing to a font-size placeholder.
///
/// Returns `width` / `height` in terminal cells. Padding and border are
/// added on top of the inner content size, mirroring how
/// [`crate::html::css_parse::properties::CssAttr::get_box_edges`] is
/// applied in the layout functions.
///
/// Behaviour:
///
/// - For a non-flex block leaf, falls back to the existing logic: an
///   explicit `width` / `height` wins, otherwise the text content width
///   (for width) or font-size (for height).
/// - For a flex container, recursively measures each block child. The
///   flex container's own `flex-direction` decides whether the main-axis
///   size is a sum (single line / column) or a max (cross axis). Gaps
///   between children are added in the appropriate direction. Then
///   padding and border are added to give the border-box size.
fn natural_intrinsic_size(
    arena: &NodeArena,
    node_id: NodeId,
    font_ctx: FontContext,
) -> NaturalIntrinsicSize {
    let node = &arena[node_id];
    let css = &node.css_attr;
    let own_font_size_cells = effective_font_size_cells(
        arena,
        node_id,
        font_ctx.font_size_cells,
        font_ctx.root_font_size_cells,
    );
    let child_font_ctx = font_ctx.child(own_font_size_cells);
    // We pass `usize::MAX` as the containing width so percentage-based
    // edges resolve to their full length. We only read padding /
    // border thicknesses, which are unaffected by the containing width
    // for non-percentage values, and at worst become the maximum for
    // percentage-based values — a safe upper bound for an intrinsic
    // measurement.
    let edges = css.get_box_edges(
        usize::MAX,
        own_font_size_cells,
        font_ctx.root_font_size_cells,
    );

    if css.is_flex() && is_flex_capable_node(&node.node_type) {
        let inner_direction = css.get_flex_direction();
        let gap = css.get_gap();

        let mut child_sizes: Vec<NaturalIntrinsicSize> = Vec::new();
        for &child_id in &node.children {
            if is_flex_capable_node(&arena[child_id].node_type) {
                child_sizes.push(natural_intrinsic_size(arena, child_id, child_font_ctx));
            }
        }

        let (inner_w, inner_h) = match inner_direction {
            FlexDirection::Row => {
                // Row: main axis = horizontal, items flow left→right.
                // Main size is sum of widths + column gaps; cross size
                // is the max of heights.
                let total_w: usize = child_sizes
                    .iter()
                    .map(|s| s.width)
                    .fold(0usize, usize::saturating_add);
                let gaps_w = if child_sizes.len() > 1 {
                    gap.column.saturating_mul(child_sizes.len() - 1)
                } else {
                    0
                };
                let max_h = child_sizes
                    .iter()
                    .map(|s| s.height)
                    .max()
                    .unwrap_or(FLEX_ITEM_MIN_CROSS);
                (
                    total_w.saturating_add(gaps_w).max(FLEX_ITEM_MIN_CROSS),
                    max_h.max(FLEX_ITEM_MIN_CROSS),
                )
            }
            FlexDirection::Column => {
                // Column: main axis = vertical, items stack top→bottom.
                // Main size is sum of heights + row gaps; cross size
                // is the max of widths.
                let max_w = child_sizes
                    .iter()
                    .map(|s| s.width)
                    .max()
                    .unwrap_or(FLEX_ITEM_MIN_CROSS);
                let total_h: usize = child_sizes
                    .iter()
                    .map(|s| s.height)
                    .fold(0usize, usize::saturating_add);
                let gaps_h = if child_sizes.len() > 1 {
                    gap.row.saturating_mul(child_sizes.len() - 1)
                } else {
                    0
                };
                (
                    max_w.max(FLEX_ITEM_MIN_CROSS),
                    total_h.saturating_add(gaps_h).max(FLEX_ITEM_MIN_CROSS),
                )
            }
        };

        let outer_w = inner_w
            .saturating_add(edges.padding.horizontal())
            .saturating_add(edges.border.horizontal_thickness());
        let outer_h = inner_h
            .saturating_add(edges.padding.vertical())
            .saturating_add(edges.border.vertical_thickness());
        NaturalIntrinsicSize {
            width: outer_w.max(FLEX_ITEM_MIN_CROSS),
            height: outer_h.max(FLEX_ITEM_MIN_CROSS),
        }
    } else {
        // Non-flex block: use CSS width / height or the existing text /
        // font-size fallbacks. We deliberately ignore the containing
        // block for an intrinsic measurement — that value is meant for
        // resolving percentages during layout, not for the recursive
        // pre-layout measurement pass.
        let width = css
            .get_width(usize::MAX, own_font_size_cells, font_ctx.root_font_size_cells)
            .or_else(|| measure_text_content_width(arena, node_id))
            .unwrap_or(FLEX_ITEM_MIN_CROSS)
            .max(FLEX_ITEM_MIN_CROSS);
        let height = css
            .get_height(usize::MAX, own_font_size_cells, font_ctx.root_font_size_cells)
            .unwrap_or(own_font_size_cells)
            .max(FLEX_ITEM_MIN_CROSS);
        NaturalIntrinsicSize {
            width,
            height,
        }
    }
}

/// Intrinsic border-box dimensions of a node, in terminal cells.
struct NaturalIntrinsicSize {
    width: usize,
    height: usize,
}

/// Natural cross-axis size of an item (its own height for `row`, its
/// own width for `column`).
///
/// The natural cross-axis size depends on the direction because the
/// cross axis swaps between rows and columns:
///
/// - `row` direction: cross axis is vertical. The natural size is the
///   item's own row count (`max(1, font_size_cells)`) — a single-row
///   text block is 1 row; a heading with `font-size: 2` is 2 rows. A
///   nested flex container reports its real inner height, so a flex
///   column with N stacked rows adds N to the line height rather than
///   collapsing to 1.
/// - `column` direction: cross axis is horizontal. The natural size is
///   the widest row of text — a single-line text block is 1 cell; a
///   nested flex row reports its widest child's intrinsic width.
///
/// Used by the line-height calculation in [`super::items::layout_flex_items`].
pub(super) fn natural_cross_axis_size(
    arena: &NodeArena,
    node_id: NodeId,
    direction: FlexDirection,
    font_ctx: FontContext,
) -> usize {
    let intrinsic = natural_intrinsic_size(arena, node_id, font_ctx);
    match direction {
        FlexDirection::Row => intrinsic.height,
        FlexDirection::Column => intrinsic.width,
    }
    .max(FLEX_ITEM_MIN_CROSS)
}

/// Resolve an item's cross-axis size in the current line.
///
/// Honours `align-self` (item-level, already-resolved from `auto` by
/// `prepare_flex_item`). The natural cross-axis size of the item is
/// the default; `align-self: stretch` upgrades it to the line's
/// cross-axis size whenever the line's cross-axis size is already
/// known.
///
/// The `has_explicit_height` parameter is kept for historical reasons
/// but is no longer used to gate stretch in `row` direction: the line
/// cross-axis size is now always known by the time we reach this
/// function — it comes from either the explicit container height
/// (CSS) or the recursive [`natural_intrinsic_size`] measurement that
/// walks through nested flex containers. Stretching an item to that
/// fixed `line_cross` therefore never grows the container, so the old
/// "no-stretch without explicit height" guard (which existed to avoid
/// a circular dependency in the v1 era before nested flex containers
/// were considered) is no longer needed.
///
/// - `row` direction: cross axis is vertical. Stretching fills the
///   line height so a short sibling of a tall flex item grows to the
///   line instead of leaving blank rows.
/// - `column` direction: cross axis is horizontal. The container's
///   content width is always known (`content.width`), so stretch
///   always takes effect.
pub(super) fn resolve_item_cross_size(
    align_self: AlignItems,
    line_cross: usize,
    natural_cross: usize,
    _has_explicit_height: bool,
    direction: FlexDirection,
) -> usize {
    if matches!(align_self, AlignItems::Stretch) {
        let can_stretch = match direction {
            FlexDirection::Row => true,
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


