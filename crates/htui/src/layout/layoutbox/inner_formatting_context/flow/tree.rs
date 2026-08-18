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

use crate::{
    html::css_parse::{FontContext, Viewport},
    layout::{
        BoxConstraint, Cursor, DEFAULT_LAYOUT, LayoutBox, LayoutContext, OuterDisplay, Rect,
        layoutbox::{
            GeneratedBoxKind, generate_box_tree, generate_node, layout_generated_box,
            layout_generated_tree, resolve_positioned_rects,
        },
    },
    tdom::node::{NodeArena, NodeId},
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
    let generated = generate_box_tree(arena, root_id);
    let mut boxes = layout_generated_tree(
        arena,
        &generated,
        containing,
        cursor,
        LayoutContext {
            font: FontContext::root(),
            viewport: Viewport {
                cols: viewport_cols,
                rows: viewport_rows,
            },
        },
    );
    // Positioned boxes resolve against their containing blocks once the whole
    // forest is assembled (`fixed` and unanchored `absolute` use the viewport
    // as the initial containing block).
    resolve_positioned_rects(&mut boxes, Rect::new(0, 0, viewport_cols, viewport_rows));
    boxes
}

/// Lay out a sequence of sibling node IDs under `containing`.
pub fn layout_children(
    arena: &NodeArena,
    children: &[NodeId],
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
) -> Vec<LayoutBox> {
    let mut boxes = Vec::new();
    // Consecutive inline-level boxes (`display: inline-block`) share one
    // line: they flow left-to-right and wrap at the containing width, like
    // a browser. They are batched here and placed by [`layout_inline_row`]
    // once a block-level sibling (or the end of the list) breaks the run.
    let mut inline_row: Vec<NodeId> = Vec::new();
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
                if !inline_row.is_empty() {
                    boxes.extend(layout_inline_row(
                        arena,
                        &inline_row,
                        containing,
                        cursor,
                        context,
                    ));
                    inline_row.clear();
                }
                boxes.extend(layout_inline_container_descendants(
                    arena, child_id, containing, cursor, context,
                ));
            }
            OuterDisplay::Inline { block: true } if generated.kind == GeneratedBoxKind::Normal => {
                inline_row.push(child_id);
            }
            _ => {
                // Text / anonymous (`display: contents`) nodes produce no
                // box and must not break a run of inline-level siblings
                // (indentation whitespace between two `inline-block`s is a
                // Text node). Only a real block-level box flushes the row.
                if generated.kind == GeneratedBoxKind::Text
                    || generated.kind == GeneratedBoxKind::Anonymous
                {
                    continue;
                }
                if !inline_row.is_empty() {
                    boxes.extend(layout_inline_row(
                        arena,
                        &inline_row,
                        containing,
                        cursor,
                        context,
                    ));
                    inline_row.clear();
                }
                if let Some(b) =
                    layout_generated_box(arena, &generated, containing, cursor, context, None)
                {
                    boxes.push(b);
                }
            }
        }
    }
    if !inline_row.is_empty() {
        boxes.extend(layout_inline_row(
            arena,
            &inline_row,
            containing,
            cursor,
            context,
        ));
    }
    boxes
}

/// Place a run of consecutive `inline-block` boxes side by side on one or
/// more lines (browser inline-level flow: left-to-right, wrapping at the
/// containing width). Each item keeps its own horizontal margins.
///
/// Two passes: the first measures each item's border-box size by laying it
/// out against a scratch cursor (an `inline-block` with a declared `width`
/// reports it; a block child of the document would otherwise claim the full
/// containing width), the second places every item at its final rect via
/// `pre_allocated_rect` so the box tree carries absolute coordinates.
fn layout_inline_row(
    arena: &NodeArena,
    items: &[NodeId],
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
) -> Vec<LayoutBox> {
    // Pass 1: measure widths / heights / horizontal margins.
    struct Measured {
        width: usize,
        height: usize,
        margin_before: usize,
        margin_after: usize,
    }
    let mut measured: Vec<Measured> = Vec::new();
    for &id in items {
        let Some(generated) = generate_node(arena, id) else {
            continue;
        };
        let mut scratch = Cursor {
            y: 0,
            prev_bottom_margin: 0,
        };
        if let Some(b) =
            layout_generated_box(arena, &generated, containing, &mut scratch, context, None)
        {
            let edges = &b.paint_style.box_edges;
            measured.push(Measured {
                width: b.rect.width,
                height: b.rect.height,
                margin_before: edges.margin.left,
                margin_after: edges.margin.right,
            });
        }
    }
    let max_h = measured.iter().map(|m| m.height).max().unwrap_or(1);
    let max_x = containing.x.saturating_add(containing.width);

    // Pass 2: place left-to-right, wrapping when the next item (with its
    // leading margin) would overflow the line.
    let mut x = containing.x;
    let mut y = cursor.y;
    let mut out: Vec<LayoutBox> = Vec::new();
    for (idx, &id) in items.iter().enumerate() {
        let Some(m) = measured.get(idx) else {
            continue;
        };
        let start = x.saturating_add(m.margin_before);
        // The trailing margin may overflow the line without forcing a wrap
        // (the box itself fits), so only the box + leading margin count.
        if start > containing.x && start.saturating_add(m.width) > max_x {
            x = containing.x;
            y = y.saturating_add(max_h);
        }
        let rect = Rect::new(
            x.saturating_add(m.margin_before),
            y,
            m.width.max(1),
            m.height.max(1),
        );
        let Some(generated) = generate_node(arena, id) else {
            continue;
        };
        if let Some(b) = layout_generated_box(
            arena,
            &generated,
            containing,
            &mut Cursor {
                y,
                prev_bottom_margin: 0,
            },
            context,
            Some(rect),
        ) {
            out.push(b);
        }
        x = x
            .saturating_add(m.margin_before)
            .saturating_add(m.width)
            .saturating_add(m.margin_after);
    }
    cursor.y = y.saturating_add(max_h);
    out
}

/// Lay out the box-producing descendants of an inline container (depth
/// first). The container itself never produces a box — [`collect_runs`]
/// folds its text into the parent's inline runs — so only descendants with
/// real geometry (replaced elements, block-level boxes) are dispatched,
/// exactly like the container were a transparent wrapper.
fn layout_inline_container_descendants(
    arena: &NodeArena,
    container_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
) -> Vec<LayoutBox> {
    let mut boxes = Vec::new();
    let mut stack: Vec<NodeId> = arena[container_id].children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        let Some(generated) = generate_node(arena, id) else {
            continue;
        };
        if generated.kind == GeneratedBoxKind::Text {
            // Text is handled by the parent's run collection; no box.
            continue;
        }
        if let Some(b) = layout_generated_box(arena, &generated, containing, cursor, context, None)
        {
            boxes.push(b);
        } else {
            // Another inline container (nested `<label>` / `<span>` …):
            // recurse into its children as well.
            stack.extend(arena[id].children.iter().rev().copied());
        }
    }
    boxes
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

/// Run the legacy element handlers after the geometry dispatcher has selected
/// normal flow.
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
