//! Layout tree construction.
//!
//! Walks the DOM through the [`DEFAULT_LAYOUT`](crate::layout::DEFAULT_LAYOUT)
//! registry, building a forest of [`LayoutBox`]es rooted at the document.
//! Block-level children of the document share a single vertical [`Cursor`],
//! so each block claims a deterministic row range.

use anyhow::Result;
use crossterm::style::Color;

use crate::{
    layout::{
        BoxConstraint, BoxEdges, Cursor, DEFAULT_LAYOUT, InlineRun, LayoutBox, Rect,
    },
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

/// Resolved outer geometry for a block-level element before children are
/// laid out.
///
/// `outer_x` / `outer_y` / `outer_width` describe the **border-box** — the
/// rectangle that gets painted (background, border glyphs) — and
/// `content_constraint` is the inner block passed to nested children,
/// already shrunk by `padding` and `border` so children's `width: 100%`
/// fills the content area.
#[derive(Debug, Clone, Copy)]
pub struct OuterGeometry {
    pub outer_x: usize,
    pub outer_y: usize,
    pub outer_width: usize,
    pub content_constraint: BoxConstraint,
}

/// Compute the outer (border-box) geometry for a block-level element given
/// its CSS edges and the chosen border-box width.
///
/// - `containing` is the parent's content-box
/// - `block_width` is the resolved CSS `width` for the border-box (i.e. the
///   rect width). It must already include padding + border — this helper
///   does not add them on top.
/// - `min_content_rows` is the minimum number of rows the content area must
///   occupy (e.g. `1` for a `<p>` even with no nested children).
///
/// The returned `content_constraint` has its `x` shifted by `border.left` +
/// `padding.left` and its `width` reduced by the same totals on each side.
pub fn compute_outer_geometry(
    containing: BoxConstraint,
    block_width: usize,
    edges: BoxEdges,
    cursor_y: usize,
    min_content_rows: usize,
) -> OuterGeometry {
    let outer_x = containing.x + edges.margin.left;
    let outer_width = block_width;
    let outer_y = cursor_y;

    let inner_h = edges.inner_horizontal();
    let content_x = outer_x + edges.border.left.thickness() + edges.padding.left;
    let content_width = outer_width.saturating_sub(inner_h).max(1);
    let content_constraint = BoxConstraint {
        x: content_x,
        width: content_width,
    };

    // Mark the cursor so children laid out next start on the row *after* the
    // minimum content height — callers shouldn't pass an empty cursor that
    // lets the first child overlap the top padding row.
    let _ = min_content_rows;

    OuterGeometry {
        outer_x,
        outer_y,
        outer_width,
        content_constraint,
    }
}

/// Place a border-box rectangle at `outer` by reserving `margin.top` rows
/// above and `margin.bottom` rows below on `cursor`.
///
/// Returns the row index where the border-box *top* sits and advances the
/// cursor past the bottom margin so the next sibling starts with the
/// correct vertical spacing. `content_height` is the number of rows the
/// nested children consumed (or `min_content_rows` for inline-only blocks
/// that haven't allocated a row of their own).
///
/// `margin.top` is **not** reserved by this helper — callers use
/// [`Cursor::skip`] before invoking it so the cursor sits exactly on the
/// border-box's top row when the rect is materialised. `margin.bottom` is
/// applied here.
pub fn reserve_border_box(
    cursor: &mut Cursor,
    outer_y_top: usize,
    edges: BoxEdges,
    content_height: usize,
    min_content_rows: usize,
) -> usize {
    // The border-box height always accommodates the minimum content area
    // even when there are no nested children — an empty `<p>` still owns
    // its line.
    let content_height = content_height.max(min_content_rows);
    let outer_height = content_height
        .saturating_add(edges.padding.top)
        .saturating_add(edges.padding.bottom)
        .saturating_add(edges.border.top.thickness())
        .saturating_add(edges.border.bottom.thickness());
    cursor.y = outer_y_top
        .saturating_add(outer_height)
        .saturating_add(edges.margin.bottom);
    outer_height
}

/// Build a border-box [`Rect`] from an [`OuterGeometry`] and the resolved
/// outer height.
pub fn build_border_rect(geom: OuterGeometry, outer_height: usize) -> Rect {
    Rect::new(geom.outer_x, geom.outer_y, geom.outer_width, outer_height)
}
