//! Layout tree construction.
//!
//! Walks the DOM through the [`DEFAULT_LAYOUT`](crate::layout::DEFAULT_LAYOUT)
//! registry, building a forest of [`LayoutBox`]es rooted at the document.
//! Block-level children of the document share a single vertical [`Cursor`],
//! so each block claims a deterministic row range.

use anyhow::Result;
use crossterm::style::Color;

use crate::{
    layout::{BoxConstraint, Cursor, DEFAULT_LAYOUT, InlineRun, LayoutBox},
    tdom::node::{Node, NodeArena, NodeId, NodeType},
    tui::graph_api::{FontStyle, block_style},
};

/// Build a layout forest for `root_id`'s block children.
///
/// `containing` is the root containing block (typically the full terminal
/// width). `cursor` tracks the next free row and is advanced in-place so
/// callers can chain multiple passes.
pub fn layout_tree(
    arena: &NodeArena,
    root_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Vec<LayoutBox>> {
    let children: Vec<NodeId> = arena[root_id].children.clone();
    layout_children(arena, &children, containing, cursor)
}

/// Lay out a sequence of sibling node IDs under `containing`.
pub fn layout_children(
    arena: &NodeArena,
    children: &[NodeId],
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Vec<LayoutBox>> {
    let mut boxes = Vec::new();
    for &child_id in children {
        if let Some(b) = dispatch(arena, child_id, containing, cursor)? {
            boxes.push(b);
        }
    }
    Ok(boxes)
}

/// Iterate the layout registry; first claim wins.
fn dispatch(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>> {
    for layout_fn in DEFAULT_LAYOUT {
        if let Some(b) = layout_fn(arena, node_id, containing, cursor)? {
            return Ok(Some(b));
        }
    }
    log::debug!("No layout handler for {:?}", arena[node_id].node_type);
    Ok(None)
}

/// Collect inline runs from a block's inline tree, preserving `<span>` style
/// boundaries.
///
/// Walks `node`'s direct children in document order. Each `Text` leaf becomes
/// one run that inherits the currently-active style. `<span>` boundaries push
/// a new style onto the stack: the span's own `font-weight` / `font-style` /
/// `text-decoration` flags are OR-ed into the inherited `FontStyle`, and the
/// span's own `color` / `background-color` override the inherited `fg` / `bg`
/// when present. Block-level children are skipped — they are dispatched as
/// their own `LayoutBox` by the layout registry, so folding them into the
/// parent's runs would either double-render them or wrap them in an
/// unintended inline context.
pub fn collect_runs(
    arena: &NodeArena,
    node: &Node,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
) -> Vec<InlineRun> {
    let mut runs = Vec::new();
    walk_runs(arena, node, fg, bg, style, &mut runs);
    runs
}

fn walk_runs(
    arena: &NodeArena,
    node: &Node,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
    runs: &mut Vec<InlineRun>,
) {
    for child_id in node.child_ids() {
        let child = &arena[child_id];
        match &child.node_type {
            NodeType::Text(s) => {
                if !s.is_empty() {
                    runs.push(InlineRun::new(s.clone(), style, fg, bg));
                }
            }
            NodeType::Span => {
                // Span inherits parent's fg/bg unless explicit overrides exist.
                let span_fg = child.css_attr.get_color().or(fg);
                let span_bg = child.css_attr.get_bg_color().or(bg);
                let span_style = style | block_style(&child.css_attr);
                walk_runs(arena, child, span_fg, span_bg, span_style, runs);
            }
            // Block-level nested elements live in their own LayoutBox;
            // unknown inline tags are flattened by the HTML parser so they
            // never reach here as an unknown NodeType.
            _ => {}
        }
    }
}

/// Resolve a block-level element's CSS width into a column count, clamped to
/// the containing block.
pub fn resolve_block_width(css_width: Option<usize>, containing_width: usize) -> usize {
    css_width.unwrap_or(containing_width).min(containing_width)
}

/// Compute the left offset for a block-level element inside its containing
/// block.
///
/// Mirrors CSS block-layout default: a block with an explicit `width` sits
/// flush against the containing block's left edge (i.e. `margin-left: 0`,
/// `margin-right: auto` only when both are auto — which would need a
/// `margin: auto` rule we don't currently parse). `text-align` is a
/// separate concern handled in the paint phase.
///
/// `block_width` is ignored; it is kept in the signature so every
/// block-level layout function can call this uniformly.
pub fn block_left(containing: BoxConstraint, _block_width: usize) -> usize {
    containing.x
}
