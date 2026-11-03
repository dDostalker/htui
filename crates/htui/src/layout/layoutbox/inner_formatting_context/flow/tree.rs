//! Block formatting context — tree builder.
//!
//! Walks the DOM through the [`DEFAULT_LAYOUT`](crate::layout::DEFAULT_LAYOUT)
//! registry and assembles a forest of [`LayoutBox`]es rooted at the document.
//! Block-level children of the document share a single vertical [`Cursor`],
//! so each block claims a deterministic row range.
//!
//! This module is the block counterpart of the flex dispatch in
//! [`crate::layout::layoutbox::inner_formatting_context::flex`].
//! Both register into the same layout registry; the flex dispatch claims
//! flex containers first, and non-flex nodes fall through to the per-element
//! block handlers declared in this module's siblings.

use std::collections::HashMap;

use crate::{
    html::css_parse::{FontContext, Viewport, WhiteSpace},
    layout::{
        BoxConstraint, Cursor, DEFAULT_LAYOUT, LayoutBox, LayoutContext, OuterDisplay, Rect,
        layoutbox::{
            GeneratedBox, GeneratedBoxKind, flatten_contents_children, generate_node,
            layout_generated_box, layout_generated_tree, resolve_positioned_rects,
            translate_subtree,
        },
    },
    tdom::node::{Node, NodeArena, NodeId, NodeType, is_collapsible_whitespace},
};

/// Build a layout forest for `root_id`'s block children.
///
/// `containing` is the root containing block (typically the full terminal
/// width). `cursor` tracks the next free row and is advanced in-place so
/// callers can chain multiple passes. `viewport_cols` / `viewport_rows`
/// supply the viewport size for `vw` / `vh` resolution.
pub fn layout_tree(
    arena: &NodeArena,
    root_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    viewport_cols: usize,
    viewport_rows: usize,
) -> Vec<LayoutBox> {
    let scroll_offsets = HashMap::new();
    layout_tree_with_scroll_offsets(
        arena,
        root_id,
        containing,
        cursor,
        viewport_cols,
        viewport_rows,
        1.0,
        &scroll_offsets,
        &NO_X,
    )
}

/// Empty x-offset table for the compatibility wrapper.
static NO_X: std::sync::LazyLock<HashMap<NodeId, u32>> = std::sync::LazyLock::new(HashMap::new);

/// Build a layout forest while preserving native scroll-container offsets.
pub fn layout_tree_with_scroll_offsets(
    arena: &NodeArena,
    root_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    viewport_cols: usize,
    viewport_rows: usize,
    zoom: f32,
    scroll_offsets: &HashMap<NodeId, u32>,
    scroll_x_offsets: &HashMap<NodeId, u32>,
) -> Vec<LayoutBox> {
    // The top level dispatches one box per rendered root child. Root-level
    // `display: contents` elements splice their children into the sibling
    // list first — the same flatten every block walk applies — and each
    // survivor is classified shallowly (no subtree recursion: the geometry
    // phase re-walks the DOM per box).
    let generated: Vec<GeneratedBox> = flatten_contents_children(arena, &arena[root_id].children)
        .iter()
        .filter_map(|&id| generate_node(arena, id))
        .collect();
    let mut boxes = layout_generated_tree(
        arena,
        &generated,
        containing,
        cursor,
        LayoutContext {
            font: FontContext::root_with_zoom(zoom),
            viewport: Viewport::new(viewport_cols, viewport_rows, zoom),
            scroll_offsets,
            scroll_x_offsets,
        },
    );
    // Positioned boxes resolve against their containing blocks once the whole
    // forest is assembled (`fixed` and unanchored `absolute` use the viewport
    // as the initial containing block).
    resolve_positioned_rects(&mut boxes, Rect::new(0, 0, viewport_cols, viewport_rows));
    boxes
}

/// One inline-level entry of a shared line: the `inline-block` element plus
/// the collapsed width of the whitespace that separated it from the previous
/// inline-level sibling (0 when they were adjacent in the source).
#[derive(Debug, Clone, Copy)]
struct InlineRowItem {
    id: NodeId,
    /// Collapsed-width cells of the inter-element whitespace (browsers
    /// collapse a whitespace run between two inline-level boxes to a single
    /// space that participates in the line layout). Line-start spaces are
    /// removed at placement time, like a browser.
    leading_space: usize,
}

/// The collapsed width (in cells) a whitespace-only text node contributes
/// between two inline-level boxes.
///
/// Collapsing `white-space` modes (`normal` / `nowrap` / `pre-line`) fold any
/// whitespace run to a single space; preserving modes (`pre` / `pre-wrap`)
/// keep every whitespace grapheme as its own cell. Text holding real words
/// contributes 0 here — its inline placement inside an `inline-block` row is
/// not yet modelled (it is painted by the parent block's own runs).
fn collapsed_interitem_space(node: &Node, parent_white_space: WhiteSpace) -> usize {
    if node.node_type != NodeType::Text {
        return 0;
    }
    let Some(s) = node.data.as_deref() else {
        return 0;
    };
    if s.is_empty() || !s.chars().all(is_collapsible_whitespace) {
        return 0;
    }
    if parent_white_space.preserves_whitespace() {
        s.chars().count()
    } else {
        1
    }
}

/// Lay out a sequence of sibling node IDs under `containing`.
pub fn layout_children(
    arena: &NodeArena,
    dom_children: &[NodeId],
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
) -> Vec<LayoutBox> {
    // The `white-space` mode governing inter-element whitespace: the parent
    // element's own declaration when it has one (default `normal`). The
    // cascade does not inherit `white-space`, matching `white_space_or`.
    // Read from the *un-expanded* sibling list — a spliced `display: contents`
    // grandchild's parent is the contents element, not the block being filled.
    let parent_white_space = dom_children
        .first()
        .and_then(|&first| arena.get(first))
        .and_then(|first| first.parent)
        .map(|pid| arena[pid].css_attr.get_white_space(&arena[pid].node_type))
        .unwrap_or(WhiteSpace::Normal);
    // `display: contents` elements are spliced out of the sibling list
    // (recursively) before the walk: the walker visits one DOM level and a
    // contents element produces no box, so without the splice its entire
    // subtree would be silently dropped.
    let children = flatten_contents_children(arena, dom_children);
    let children = &children;
    let mut boxes = Vec::new();
    // Consecutive inline-level boxes (`display: inline-block`) share one
    // line: they flow left-to-right and wrap at the containing width, like
    // a browser. They are batched here and placed by [`layout_inline_row`]
    // once a block-level sibling (or the end of the list) breaks the run.
    let mut inline_row: Vec<InlineRowItem> = Vec::new();
    // Collapsed whitespace accumulated since the last inline-level item,
    // waiting to be attached as the next item's `leading_space`.
    let mut pending_space: usize = 0;
    for &child_id in children {
        let Some(generated) = generate_node(arena, child_id) else {
            continue;
        };
        // An inline container (`<span>` / `<label>` / `<a>` …) has no box of
        // its own: its text is folded into the parent's inline runs, but its
        // replaced / block descendants (a checkbox inside a `<label>`, an
        // `<img>` inside a `<span>`) still need their own boxes, laid out
        // below the parent's text line like any other inline-level child.
        // `inline-block` (`Inline { block: true }`) is different: it is an
        // inline-level box that establishes its own formatting context, so it
        // must produce its own box instead of flattening its children — and
        // it participates in the inline row flow.
        match generated.outer_display {
            OuterDisplay::Inline { block: false } if generated.kind == GeneratedBoxKind::Normal => {
                // An inline container does not own a line box. Keep the
                // caller's inline-row state open while traversing it so an
                // `inline-block` ending one container can share a row with an
                // `inline-block` starting the next one (`<main>...</main>
                // <aside>...</aside>`).
                layout_inline_container_descendants(
                    arena,
                    child_id,
                    containing,
                    cursor,
                    context,
                    &mut inline_row,
                    &mut pending_space,
                    &mut boxes,
                );
            }
            OuterDisplay::Inline { block: true } if generated.kind == GeneratedBoxKind::Normal => {
                inline_row.push(InlineRowItem {
                    id: child_id,
                    leading_space: pending_space,
                });
                pending_space = 0;
            }
            _ => {
                // Text / anonymous (`display: contents`) nodes produce no
                // box and must not break a run of inline-level siblings
                // (indentation whitespace between two `inline-block`s is a
                // Text node). A whitespace-only run instead collapses to a
                // space that separates the neighbouring inline-level boxes —
                // the browser's inter-element whitespace. Only a real
                // block-level box flushes the row.
                if generated.kind == GeneratedBoxKind::Text
                    || generated.kind == GeneratedBoxKind::Anonymous
                {
                    pending_space = pending_space.max(collapsed_interitem_space(
                        &arena[child_id],
                        parent_white_space,
                    ));
                    continue;
                }
                flush_inline_row(
                    arena,
                    &mut inline_row,
                    containing,
                    cursor,
                    context,
                    &mut boxes,
                );
                pending_space = 0;
                if let Some(b) =
                    layout_generated_box(arena, &generated, containing, cursor, context, None)
                {
                    boxes.push(b);
                }
            }
        }
    }
    flush_inline_row(
        arena,
        &mut inline_row,
        containing,
        cursor,
        context,
        &mut boxes,
    );
    boxes
}

/// Place any pending inline-level siblings and reset their shared row state.
fn flush_inline_row(
    arena: &NodeArena,
    inline_row: &mut Vec<InlineRowItem>,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    boxes: &mut Vec<LayoutBox>,
) {
    if !inline_row.is_empty() {
        boxes.extend(layout_inline_row(
            arena, inline_row, containing, cursor, context,
        ));
        inline_row.clear();
    }
}

/// Place a run of consecutive `inline-block` boxes side by side on one or
/// more lines (browser inline-level flow: left-to-right, wrapping at the
/// containing width). Each item keeps its own horizontal margins; the
/// collapsed inter-element whitespace separates neighbours (one cell under
/// `white-space: normal`) and counts toward the wrap threshold, while a
/// space at the start of a wrapped line is removed — the browser's line
/// leading-space rule.
///
fn layout_inline_row(
    arena: &NodeArena,
    items: &[InlineRowItem],
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
) -> Vec<LayoutBox> {
    let mut measured: Vec<Option<LayoutBox>> = items.iter().map(|_| None).collect();
    for (idx, item) in items.iter().enumerate() {
        let Some(generated) = generate_node(arena, item.id) else {
            continue;
        };
        let mut scratch = Cursor {
            y: 0,
            prev_bottom_margin: 0,
        };
        if let Some(mut b) =
            layout_generated_box(arena, &generated, containing, &mut scratch, context, None)
        {
            // Measurement starts at a scratch vertical origin. Only the final
            // translation differs from placement, so retain the whole subtree.
            let dy = -(b.rect.y as i64);
            translate_subtree(&mut b, 0, dy);
            measured[idx] = Some(b);
        }
    }
    let max_h = measured
        .iter()
        .filter_map(|m| m.as_ref().map(|m| m.rect.height))
        .max()
        .unwrap_or(1);
    let max_x = containing.x.saturating_add(containing.width);

    // Translate a measured subtree to its absolute border-box position. Width
    // is unchanged because both passes use the same containing width.
    let place = |mut b: LayoutBox, start: usize, y: usize| {
        let dx = start as i64 - b.rect.x as i64;
        let dy = y as i64 - b.rect.y as i64;
        translate_subtree(&mut b, dx, dy);
        b
    };

    // Place left-to-right, wrapping when the next item (with its leading
    // space and margin) would overflow the line.
    let mut x = containing.x;
    let mut y = cursor.y;
    let mut out: Vec<LayoutBox> = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        let Some(Some(m)) = measured.get_mut(idx).map(|m| m.take()) else {
            continue;
        };
        // A space at the start of a line is removed (browser collapsing
        // rule), so it only applies when the line already has content.
        let space = if x > containing.x {
            item.leading_space
        } else {
            0
        };
        let margin_before = m.paint_style.box_edges.margin.left.max(0) as usize;
        let margin_after = m.paint_style.box_edges.margin.right.max(0) as usize;
        let width = m.rect.width;
        let start = x.saturating_add(space).saturating_add(margin_before);
        // The trailing margin may overflow the line without forcing a wrap
        // (the box itself fits), so only the box + leading space + leading
        // margin count.
        if start > containing.x && start.saturating_add(m.rect.width) > max_x {
            x = containing.x;
            y = y.saturating_add(max_h);
            // Re-anchored at the line start: the separating space collapses
            // away there.
            let start = x.saturating_add(margin_before);
            out.push(place(m, start, y));
            x = start.saturating_add(width).saturating_add(margin_after);
            continue;
        }
        let start = x.saturating_add(space).saturating_add(margin_before);
        out.push(place(m, start, y));
        x = start.saturating_add(width).saturating_add(margin_after);
    }
    cursor.y = y.saturating_add(max_h);
    out
}

/// Lay out the box-producing descendants of an inline container (depth
/// first). The container itself never produces a box — [`collect_runs`]
/// folds its text into the parent's inline runs — so only descendants with
/// real geometry participate in the caller's flow.
///
/// The inline-row state is shared with the caller on purpose. CSS inline
/// containers are fragments of one parent line, not independent block
/// boundaries; this lets consecutive inline-level descendants separated by
/// `<span>/<main>/<aside>` wrap and align together like anonymous inline
/// boxes do in a browser.
fn layout_inline_container_descendants(
    arena: &NodeArena,
    container_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    inline_row: &mut Vec<InlineRowItem>,
    pending_space: &mut usize,
    boxes: &mut Vec<LayoutBox>,
) {
    for &id in arena[container_id].children.iter() {
        let Some(generated) = generate_node(arena, id) else {
            continue;
        };
        match generated.outer_display {
            OuterDisplay::Inline { block: false } if generated.kind == GeneratedBoxKind::Normal => {
                layout_inline_container_descendants(
                    arena,
                    id,
                    containing,
                    cursor,
                    context,
                    inline_row,
                    pending_space,
                    boxes,
                );
            }
            OuterDisplay::Inline { block: true } if generated.kind == GeneratedBoxKind::Normal => {
                inline_row.push(InlineRowItem {
                    id,
                    leading_space: *pending_space,
                });
                *pending_space = 0;
            }
            _ => {
                if generated.kind == GeneratedBoxKind::Text
                    || generated.kind == GeneratedBoxKind::Anonymous
                {
                    *pending_space = (*pending_space).max(collapsed_interitem_space(
                        &arena[id],
                        arena[container_id]
                            .css_attr
                            .get_white_space(&arena[container_id].node_type),
                    ));
                    continue;
                }
                flush_inline_row(arena, inline_row, containing, cursor, context, boxes);
                *pending_space = 0;
                if let Some(b) =
                    layout_generated_box(arena, &generated, containing, cursor, context, None)
                {
                    boxes.push(b);
                }
            }
        }
    }
}

/// Iterate the layout registry; first claim wins.
///
/// `pre_allocated_rect` is forwarded to every layout function so the flex
/// layout can hand each flex item a fixed rectangle. Public so the flex
/// layout can recurse into the dispatcher after deciding each item's rect.
pub fn dispatch(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    let generated = generate_node(arena, node_id)?;
    layout_generated_box(
        arena,
        &generated,
        containing,
        cursor,
        context,
        pre_allocated_rect,
    )
}

/// Run the specialized element handlers after the geometry dispatcher has
/// selected normal flow.
pub(crate) fn dispatch_flow_geometry(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    for layout_fn in DEFAULT_LAYOUT {
        if let Some(b) = layout_fn(
            arena,
            node_id,
            containing,
            cursor,
            context,
            pre_allocated_rect,
        ) {
            return Some(b);
        }
    }
    log::debug!("No layout handler for {:?}", arena[node_id].node_type);
    None
}
