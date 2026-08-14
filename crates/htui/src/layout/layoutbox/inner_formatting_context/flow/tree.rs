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
        BoxConstraint, Cursor, DEFAULT_LAYOUT, LayoutBox, LayoutContext, Rect,
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
    for &child_id in children {
        let Some(generated) = generate_node(arena, child_id) else {
            continue;
        };
        // An inline container (e.g. `<label>`, `<span>`) has no box of its
        // own: its text is folded into the parent's inline runs, but its
        // replaced / block descendants (a checkbox inside a `<label>`, an
        // `<img>` inside a `<span>`) still need their own boxes, laid out
        // below the parent's text line like any other inline-level child.
        if generated.kind == GeneratedBoxKind::Normal && generated.outer_display.is_inline() {
            boxes.extend(layout_inline_container_descendants(
                arena, child_id, containing, cursor, context,
            ));
            continue;
        }
        if let Some(b) = layout_generated_box(arena, &generated, containing, cursor, context, None)
        {
            boxes.push(b);
        }
    }
    boxes
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
