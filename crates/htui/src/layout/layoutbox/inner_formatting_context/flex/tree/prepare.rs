//! Per-item preparation: build a [`FlexItemPrep`] from CSS properties and
//! measure each item's natural cross-axis size.

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    html::css_parse::{
        AlignItems, AlignSelf, CssDisplay, FlexDirection, Length, LengthContext, WhiteSpace,
    },
    layout::{
        InlineRun, LayoutContext,
        layoutbox::collect_runs,
        layoutbox::generation::{GeneratedBoxKind, generate_node},
        layoutbox::geometry::select_options,
    },
    tdom::node::{
        NodeArena, NodeId, NodeType, effective_font_size_cells, is_collapsible_whitespace,
    },
    graphics::{FontStyle, char_width, grapheme_width},
};

use super::container::{AnonymousTextItem, FLEX_ITEM_MIN_CROSS, FlexItemPrep, FlexItemSource};

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
    let natural = natural_intrinsic_size(arena, item_id, context);
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
            let explicit = match direction.is_row() {
                true => css.get_width(containing_main, context.font, context.viewport),
                false => css.get_height(containing_main, context.font, context.viewport),
            };
            match direction.is_row() {
                true => explicit.or(Some(natural.width)),
                false => explicit.or(Some(natural.height)),
            }
        })
        .unwrap_or(FLEX_ITEM_MIN_CROSS);

    // The automatic minimum main size (CSS `min-height` / `min-width: auto`):
    // an item must never shrink below its min-content size. For `column`
    // direction the natural height is already content-sized (block content
    // stacks without compressing), so it doubles as the floor. For `row`
    // direction the floor is the widest unbreakable word (text wraps at
    // smaller widths). Without these floors, a definite-sized flex column
    // whose items' basis sums exceed its height would crush every item below
    // its content (e.g. stat cards collapsing to a 2-row strip).
    let min_main = match direction.is_row() {
        true => longest_breakable_width(arena, item_id),
        false => natural.height,
    };

    // `align-self: auto` defers to the container's `align-items` (CSS
    // default for `align-items` is `stretch`, so without an explicit
    // `align-items` this still ends up as `Stretch`).
    let css_min_main = match direction.is_row() {
        true => css.get_min_width(containing_main, context.font, context.viewport),
        false => css.get_min_height(containing_main, context.font, context.viewport),
    };
    let css_max_main = match direction.is_row() {
        true => css.get_max_width(containing_main, context.font, context.viewport),
        false => css.get_max_height(containing_main, context.font, context.viewport),
    };
    // Baseline (rows from the box top to the first text row), for
    // `align-items: baseline`: border-top + padding-top place the content
    // box, and the glyph row sits at the line-box's vertical centring offset.
    let own_font_size_cells =
        effective_font_size_cells(arena, item_id, context.font, context.viewport);
    let line_height_cells = crate::html::css_parse::resolve_line_height_cells(
        css.get_line_height(),
        own_font_size_cells,
        context.font,
        context.viewport,
    );
    let font = own_font_size_cells.max(1);
    let line_h = line_height_cells.max(1);
    let glyph_row = if line_h > font { (line_h - font) / 2 } else { (font - 1) / 2 };
    let edges = css.get_box_edges(usize::MAX, context.font, context.viewport);
    let margin = edges.margin;
    let (margin_main_before, margin_main_after, margin_cross_before, margin_cross_after) =
        match direction.is_row() {
            true => (margin.left, margin.right, margin.top, margin.bottom),
            false => (margin.top, margin.bottom, margin.left, margin.right),
        };
    let baseline = edges
        .border
        .top
        .thickness()
        .saturating_add(edges.padding.top)
        .saturating_add(glyph_row);
    FlexItemPrep {
        source: FlexItemSource::Node(item_id),
        basis: basis_resolved,
        min_main,
        css_min_main,
        css_max_main,
        grow: css.get_flex_grow(),
        shrink: css.get_flex_shrink(),
        align_self: match css.get_align_self() {
            AlignSelf::Auto => align_items,
            AlignSelf::Value(v) => v,
        },
        margin_main_before,
        margin_main_after,
        margin_cross_before,
        margin_cross_after,
        baseline,
        order: css.get_order(),
        natural_width: natural.width,
        natural_height: natural.height,
    }
}

/// Build a [`FlexItemPrep`] for an anonymous text item: contiguous raw text
/// children of a flex container. Anonymous items carry the initial flex
/// properties (`flex: 0 1 auto`, `order: 0`, `align-self: auto`).
pub(super) fn prepare_anonymous_text_item(
    item: AnonymousTextItem,
    direction: FlexDirection,
    align_items: AlignItems,
) -> FlexItemPrep {
    let text_w = runs_total_width(&item.runs);
    // Capture before `item` is moved into the prep's source below.
    let natural_height = item.line_height_cells.max(FLEX_ITEM_MIN_CROSS);
    let (basis, min_main) = match direction.is_row() {
        true => (
            text_w.max(FLEX_ITEM_MIN_CROSS),
            runs_longest_word(&item.runs),
        ),
        false => (FLEX_ITEM_MIN_CROSS, FLEX_ITEM_MIN_CROSS),
    };
    FlexItemPrep {
        source: FlexItemSource::AnonymousText(item),
        basis,
        min_main,
        css_min_main: None,
        css_max_main: None,
        grow: 0.0,
        shrink: 1.0,
        align_self: align_items,
        margin_main_before: 0,
        margin_main_after: 0,
        margin_cross_before: 0,
        margin_cross_after: 0,
        baseline: 0,
        order: 0,
        // Mirrors the intrinsic semantics of the DOM-item path: a text row's
        // natural height is its line-height, its natural width the total run
        // width (the direction-dependent selection happens at read time).
        natural_width: text_w.max(FLEX_ITEM_MIN_CROSS),
        natural_height,
    }
}

/// Sum the grapheme widths of every non-break run in `runs`.
fn runs_total_width(runs: &[InlineRun]) -> usize {
    runs.iter()
        .filter(|r| !r.force_break)
        .map(|r| r.text.graphemes(true).map(grapheme_width).sum::<usize>())
        .sum()
}

/// Widest whitespace-delimited token in `runs` (the min-content width of the
/// anonymous text item).
fn runs_longest_word(runs: &[InlineRun]) -> usize {
    runs.iter()
        .filter(|r| !r.force_break)
        .flat_map(|r| r.text.split_whitespace())
        .map(|token| token.graphemes(true).map(grapheme_width).sum::<usize>())
        .max()
        .unwrap_or(0)
        .max(1)
}

/// Sum the grapheme widths of every direct text leaf of `node_id`.
pub(super) fn measure_text_content_width(arena: &NodeArena, node_id: NodeId) -> Option<usize> {
    let white_space = arena[node_id]
        .css_attr
        .get_white_space(&arena[node_id].node_type);
    let mut w = measure_text_content_width_ws(arena, node_id, white_space)?;
    // `<q>` generates open + close quote marks (one cell each) that are not
    // part of its text children but still contribute to its intrinsic width.
    if arena[node_id].node_type == NodeType::Q {
        w = w.saturating_add(2);
    }
    Some(w)
}

fn measure_text_content_width_ws(
    arena: &NodeArena,
    node_id: NodeId,
    white_space: WhiteSpace,
) -> Option<usize> {
    let node = &arena[node_id];
    let mut total: Option<usize> = Some(0);
    for &child_id in &node.children {
        let child = &arena[child_id];
        match &child.node_type {
            NodeType::Text => {
                if child.text_is_painted(white_space)
                    && let Some(s) = child.data.as_deref()
                {
                    let w: usize = s.graphemes(true).map(grapheme_width).sum();
                    total = Some(total.unwrap_or(0).saturating_add(w));
                }
            }
            // Inline text-level wrappers hold the text that belongs to this
            // block's inline content; recurse so a block whose text lives
            // entirely inside inline elements still reports its intrinsic
            // width (flex-basis: auto relies on it).
            _ if child.node_type.ua_default_display() == CssDisplay::Inline => {
                let ws = child.white_space_or(white_space);
                let w = measure_text_content_width_ws(arena, child_id, ws).unwrap_or(0);
                let w = if child.node_type == NodeType::Q {
                    w.saturating_add(2)
                } else {
                    w
                };
                total = Some(total.unwrap_or(0).saturating_add(w));
            }
            _ => {}
        }
    }
    total
}

/// Widest unbreakable segment in `node_id`'s inline text — the item's
/// min-content main-axis size for `row` direction (CSS `min-width: auto`).
///
/// Latin words break at whitespace; CJK characters each offer their own
/// break opportunity, so a single CJK grapheme is the practical floor. If
/// the node has no inline text (e.g. a pure block container) the floor is
/// the [`FLEX_ITEM_MIN_CROSS`] single cell.
pub(super) fn longest_breakable_width(arena: &NodeArena, node_id: NodeId) -> usize {
    fn walk(arena: &NodeArena, node_id: NodeId, white_space: WhiteSpace, max_w: &mut usize) {
        let node = &arena[node_id];
        for &child_id in &node.children {
            let child = &arena[child_id];
            match &child.node_type {
                NodeType::Text => {
                    if child.text_is_painted(white_space)
                        && let Some(s) = child.data.as_deref()
                    {
                        for token in s.split_whitespace() {
                            let w: usize = token.graphemes(true).map(grapheme_width).sum();
                            if w > *max_w {
                                *max_w = w;
                            }
                        }
                    }
                }
                _ if child.node_type.ua_default_display() == CssDisplay::Inline => {
                    let ws = child.white_space_or(white_space);
                    walk(arena, child_id, ws, max_w);
                }
                _ => {}
            }
        }
    }
    let white_space = arena[node_id]
        .css_attr
        .get_white_space(&arena[node_id].node_type);
    let mut max_w: usize = FLEX_ITEM_MIN_CROSS;
    walk(arena, node_id, white_space, &mut max_w);
    max_w
}

/// Resolve an explicit `width` / `height` for intrinsic measurement.
///
/// A percentage against an unknown (indefinite) containing block resolves to
/// `auto` (`None`), matching CSS — resolving `100%` against the `usize::MAX`
/// sentinel would otherwise produce a huge size and overflow the canvas later.
/// Any other length resolves normally against a zero containing width (unused
/// by non-percentage lengths).
fn intrinsic_len(raw: Option<&str>, context: LayoutContext) -> Option<usize> {
    let len = Length::parse(raw?)?;
    match len {
        Length::Auto | Length::Pct(_) => None,
        len => Some(
            len.resolve(LengthContext {
                containing: 0,
                font: context.font,
                viewport: context.viewport,
            })
            .max(1),
        ),
    }
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

    // Replaced elements have an intrinsic box independent of their (empty)
    // children. `<input>` takes its HTML `width` attribute (default 8 cells)
    // and one row; `<select>` sizes to the longest option label plus the
    // frame / chevron — without these, flex measurement reports the 1x1
    // floor and a control inside a flex row collapses to a sliver. A CSS
    // `border:` adds its thickness on both axes, matching the geometry phase.
    if matches!(node.node_type, NodeType::Input) {
        // Checkbox / radio are two-cell toggles (glyph + spacing); other
        // kinds take the HTML `width` attribute (default 8 cells). Toggles
        // paint no chrome of their own, so they reserve no border cells.
        let is_toggle = node
            .get_type()
            .is_some_and(|t| matches!(t.to_ascii_lowercase().as_str(), "checkbox" | "radio"));
        let edges = css.get_box_edges(usize::MAX, child_context.font, child_context.viewport);
        let (border_h, border_v) = if is_toggle {
            (0, 0)
        } else {
            (
                edges.border.horizontal_thickness(),
                edges.border.vertical_thickness(),
            )
        };
        let w = if is_toggle {
            2
        } else {
            node.get_width().map(|w| w as usize).unwrap_or(8)
        };
        return NaturalIntrinsicSize {
            width: w.saturating_add(border_h).max(FLEX_ITEM_MIN_CROSS),
            height: 1usize.saturating_add(border_v).max(FLEX_ITEM_MIN_CROSS),
        };
    }
    if matches!(node.node_type, NodeType::Select) {
        let text_w: usize = select_options(arena, node_id)
            .iter()
            .map(|opt| opt.label.chars().map(char_width).sum::<usize>())
            .max()
            .unwrap_or(0);
        let edges = css.get_box_edges(usize::MAX, child_context.font, child_context.viewport);
        let w = node
            .get_width()
            .map(|w| w as usize)
            .unwrap_or(text_w.saturating_add(4));
        return NaturalIntrinsicSize {
            width: w
                .saturating_add(edges.border.horizontal_thickness())
                .max(FLEX_ITEM_MIN_CROSS),
            height: 1usize
                .saturating_add(edges.border.vertical_thickness())
                .max(FLEX_ITEM_MIN_CROSS),
        };
    }

    // We pass `usize::MAX` as the containing width so percentage-based
    // edges resolve to their full length. We only read padding /
    // border thicknesses, which are unaffected by the containing width
    // for non-percentage values, and at worst become the maximum for
    // percentage-based values — a safe upper bound for an intrinsic
    // measurement.
    let edges = css.get_box_edges(usize::MAX, child_context.font, child_context.viewport);

    let generated = generate_node(arena, node_id);
    let is_flex_box = generated
        .as_ref()
        .map(|g| {
            g.kind == GeneratedBoxKind::Normal
                && g.inner_formatting_context == crate::layout::FormattingContext::Flex
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

        let (inner_w, inner_h) = match inner_direction.is_row() {
            true => {
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
            false => {
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
        for &child_id in &node.children {
            let Some(child_gen) = generate_node(arena, child_id) else {
                continue;
            };
            let is_block =
                child_gen.kind == GeneratedBoxKind::Normal && !child_gen.outer_display.is_inline();
            if is_block {
                let cs = natural_intrinsic_size(arena, child_id, child_context);
                content_w = content_w.max(cs.width);
                content_h = content_h.saturating_add(cs.height);
            }
            // Non-block children (text / inline elements / anonymous inline
            // groups) fold into the block's own inline content, measured by
            // `collect_runs` / `measure_text_content_width` below.
        }
        let text_w = measure_text_content_width(arena, node_id).unwrap_or(0);
        content_w = content_w.max(text_w);
        // Inline content adds one intrinsic row, but a run made purely of
        // collapsible whitespace must not (it collapses away at a line edge).
        // NBSP is intentionally *not* collapsible, so it still counts.
        let has_inline = collect_runs(arena, node_id, node, None, None, FontStyle::empty())
            .iter()
            .any(|r| r.force_break || r.text.chars().any(|c| !is_collapsible_whitespace(c)))
            || text_w > 0;
        if has_inline {
            // Intrinsic (max-content) measurement does not wrap, so a
            // single line of inline content contributes one row scaled by
            // the block's effective font size.
            content_h = content_h.saturating_add(own_font_size_cells.max(1));
        }

        let content_box_w = intrinsic_len(css.0.get("width").map(|s| s.as_ref()), child_context)
            .unwrap_or(content_w.max(FLEX_ITEM_MIN_CROSS))
            .max(FLEX_ITEM_MIN_CROSS);
        // Some elements reserve trailing blank rows after their border-box
        // (the browser UA stylesheet's `spacing_after`): `<hr>` leaves one
        // row. The block layout adds this on top of `content_height`, so the
        // intrinsic measurement must too — otherwise a flex item sized from
        // the natural measure under-estimates its real height and the item's
        // last row bleeds into the parent's paint clip (silently dropped).
        let ua_spacing_after = match node.node_type {
            NodeType::Hr => 1,
            _ => 0,
        };
        let content_box_h = intrinsic_len(css.0.get("height").map(|s| s.as_ref()), child_context)
            .unwrap_or(content_h.max(FLEX_ITEM_MIN_CROSS))
            .max(FLEX_ITEM_MIN_CROSS)
            .saturating_add(ua_spacing_after);
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

/// Natural cross-axis size of a prepared flex item (its cached height for
/// `row`, its cached width for `column`).
///
/// The cross axis swaps between rows and columns:
///
/// - `row` direction: cross axis is vertical — the item's natural row count.
/// - `column` direction: cross axis is horizontal — the item's widest
///   intrinsic row.
///
/// Both values were measured once by `natural_intrinsic_size` when the prep
/// was built; re-reading them here keeps the wrap and placement passes from
/// re-running the recursive measurement with identical inputs.
pub(super) fn natural_cross_axis_size_for_prep(
    prep: &FlexItemPrep,
    direction: FlexDirection,
) -> usize {
    match direction.is_row() {
        true => prep.natural_height,
        false => prep.natural_width,
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
/// - `row` direction: cross axis is vertical. Stretching fills the
///   line height so a short sibling of a tall flex item grows to the
///   line instead of leaving blank rows.
/// - `column` direction: cross axis is horizontal. The container's
///   content width is always known (`content.width`), so stretch
///   always takes effect.
///
/// `margin_cross` is `(before, after)` along the cross axis; a stretched
/// item's border-box is the line cross minus its own cross-axis margins
/// (the margins sit outside the stretched box, like a browser).
pub(super) fn resolve_item_cross_size(
    align_self: AlignItems,
    line_cross: usize,
    natural_cross: usize,
    margin_cross: (usize, usize),
) -> usize {
    if matches!(align_self, AlignItems::Stretch) {
        line_cross
            .saturating_sub(margin_cross.0)
            .saturating_sub(margin_cross.1)
            .max(FLEX_ITEM_MIN_CROSS)
    } else {
        natural_cross.max(FLEX_ITEM_MIN_CROSS)
    }
}
