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

/// Sum the grapheme widths of every direct `Text` descendant of `node_id`.
///
/// Used as the `flex-basis: auto` fallback when neither `width` /
/// `height` nor a `flex-basis` declaration is present. For a node with
/// nested block children this only counts its own direct text; a
/// recursive measure would be more accurate but is a v2 concern.
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
/// Used by the line-height calculation in [`super::items::layout_flex_items`].
pub(super) fn natural_cross_axis_size(
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
pub(super) fn resolve_item_cross_size(
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


