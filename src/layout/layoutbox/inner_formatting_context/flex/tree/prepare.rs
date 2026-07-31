//! Per-item preparation: build a [`FlexItemPrep`] from CSS properties and
//! measure each item's natural cross-axis size.

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    html::css_parse::{AlignItems, AlignSelf, FlexDirection},
    layout::{
        LayoutContext,
        layoutbox::generation::{GeneratedBoxKind, generate_node},
    },
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
    context: LayoutContext,
    align_items: AlignItems,
) -> FlexItemPrep {
    let node = &arena[item_id];
    let css = &node.css_attr;
    let basis_resolved = css
        .get_flex_basis_resolved(containing_main, context.font, context.viewport)
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
                FlexDirection::Row => css.get_width(containing_main, context.font, context.viewport),
                FlexDirection::Column => css.get_height(containing_main, context.font, context.viewport),
            };
            match direction {
                FlexDirection::Row => explicit.or_else(|| {
                    let intrinsic = natural_intrinsic_size(arena, item_id, context);
                    Some(intrinsic.width)
                }),
                FlexDirection::Column => explicit.or_else(|| {
                    let intrinsic = natural_intrinsic_size(arena, item_id, context);
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

/// Sum the grapheme widths of every direct text leaf of `node_id`.
///
/// The generator classifies direct text children as
/// [`GeneratedBoxKind::Text`]; this helper therefore walks the generated
/// tree rather than the raw DOM so flex size-probing never inspects the
/// element tag.
pub(super) fn measure_text_content_width(arena: &NodeArena, node_id: NodeId) -> Option<usize> {
    let node = &arena[node_id];
    let mut total: Option<usize> = Some(0);
    for &child_id in &node.children {
        let Some(child) = generate_node(arena, child_id) else {
            continue;
        };
        if child.kind == GeneratedBoxKind::Text {
            if let NodeType::Text(s) = &arena[child_id].node_type {
                let w: usize = s.graphemes(true).map(grapheme_width).sum();
                total = Some(total.unwrap_or(0).saturating_add(w));
            }
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
    context: LayoutContext,
) -> NaturalIntrinsicSize {
    let node = &arena[node_id];
    let css = &node.css_attr;
    let own_font_size_cells =
        effective_font_size_cells(arena, node_id, context.font, context.viewport);
    let child_context = context.child(own_font_size_cells);
    // We pass `usize::MAX` as the containing width so percentage-based
    // edges resolve to their full length. We only read padding /
    // border thicknesses, which are unaffected by the containing width
    // for non-percentage values, and at worst become the maximum for
    // percentage-based values — a safe upper bound for an intrinsic
    // measurement.
    let edges = css.get_box_edges(
        usize::MAX,
        child_context.font,
        child_context.viewport,
    );

    let generated = generate_node(arena, node_id);
    let is_flex_box = generated
        .as_ref()
        .map(|g| {
            g.kind == GeneratedBoxKind::Normal
                && g.inner_formatting_context
                    == crate::layout::FormattingContext::Flex
        })
        .unwrap_or(false);

    let res = if css.display().is_some_and(|d| d.is_flex()) && is_flex_box {
        let inner_direction = css.get_flex_direction();
        let gap = css.get_gap(child_context.font, child_context.viewport);

        let mut child_sizes: Vec<NaturalIntrinsicSize> = Vec::new();
        for &child_id in &node.children {
            let Some(child_generated) = generate_node(arena, child_id) else {
                continue;
            };
            if child_generated.kind == GeneratedBoxKind::Normal {
                child_sizes.push(natural_intrinsic_size(arena, child_id, child_context));
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
        // Non-flex block: measure content by recursing into block children
        // (sum their heights, take the max width) and adding one
        // max-content row for any inline / text content. Explicit CSS
        // `width` / `height` always win — this only supplies the `auto`
        // fallback so a block with nested content no longer collapses to
        // a single font-size row when used as a flex item (CSS `flex-basis:
        // auto` resolves to the content-based size). Mirrors the
        // flex-container branch above, which already recurses.
        //
        // We deliberately ignore the containing block here — that value is
        // meant for resolving percentages during layout, not for the
        // recursive pre-layout measurement pass.
        let mut content_w: usize = 0;
        let mut content_h: usize = 0;
        let mut has_inline = false;
        for &child_id in &node.children {
            let Some(child_gen) = generate_node(arena, child_id) else {
                continue;
            };
            let is_block = child_gen.kind == GeneratedBoxKind::Normal
                && !child_gen.outer_display.is_inline();
            if is_block {
                let cs = natural_intrinsic_size(arena, child_id, child_context);
                content_w = content_w.max(cs.width);
                content_h = content_h.saturating_add(cs.height);
            } else {
                // Text nodes, inline elements and anonymous wrappers all
                // contribute to the block's own inline content.
                has_inline = true;
            }
        }
        let text_w = measure_text_content_width(arena, node_id).unwrap_or(0);
        if text_w > 0 {
            has_inline = true;
            content_w = content_w.max(text_w);
        }
        if has_inline {
            // Intrinsic (max-content) measurement does not wrap, so a
            // single line of inline content contributes one row scaled by
            // the block's effective font size.
            content_h = content_h.saturating_add(own_font_size_cells.max(1));
        }

        let content_box_w = css
            .get_width(usize::MAX, child_context.font, child_context.viewport)
            .unwrap_or(content_w.max(FLEX_ITEM_MIN_CROSS))
            .max(FLEX_ITEM_MIN_CROSS);
        let content_box_h = css
            .get_height(usize::MAX, child_context.font, child_context.viewport)
            .unwrap_or(content_h.max(FLEX_ITEM_MIN_CROSS))
            .max(FLEX_ITEM_MIN_CROSS);
        // Report the border-box (outer) size, like the flex-container branch
        // above: padding + border sit on top of the content-box measurement
        // so the flex algorithm sizes the item's full border-box.
        let width = content_box_w
            .saturating_add(edges.padding.horizontal())
            .saturating_add(edges.border.horizontal_thickness());
        let height = content_box_h
            .saturating_add(edges.padding.vertical())
            .saturating_add(edges.border.vertical_thickness());
        NaturalIntrinsicSize {
            width: width.max(FLEX_ITEM_MIN_CROSS),
            height: height.max(FLEX_ITEM_MIN_CROSS),
        }
    };

    log::debug!(
        target: "htui::intrinsic",
        "natural_intrinsic_size node={:?} type={:?} class={:?} flex={} -> {}x{}",
        node_id,
        node.node_type,
        node.attributes.get("class").map(|s| s.as_str()),
        is_flex_box,
        res.width,
        res.height
    );
    res
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
    context: LayoutContext,
) -> usize {
    let intrinsic = natural_intrinsic_size(arena, node_id, context);
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