//! Paint phase.
//!
//! Walks a [`LayoutBox`] forest produced by
//! [`super::layoutbox::inner_formatting_context::flow::layout_tree`] (and
//! flex / grid trees) and writes styled cells into the
//! [`TermLayout`](crate::tui::graph_api::TermLayout) canvas. Reads no DOM
//! state beyond each box's `node_id`: every paint decision is derived from
//! the box's own `rect`, `kind`, `paint_style`, and the CSS attributes
//! stored on the source node.
//!
//! ## Submodules
//!
//! - [`block`]: painting for `BoxKind::Normal` — backgrounds, borders, inline
//!   runs.
//! - [`border`]: the border-glyph rasteriser, shared with `block`.
//! - [`image`]: rasterising for `BoxKind::Replaced(ReplacedContent::Image)`.
//! - [`text`]: the inline-run painter, shared with `block`.

mod block;
mod border;
mod image;
mod input;
mod select;
mod text;

use std::collections::HashMap;

use crossterm::style::Color;

use crate::{
    html::css_parse::Position,
    layout::{BoxKind, LayoutBox, Rect, ReplacedContent},
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::{Block, ImageCache, InputRenderState, TermLayout},
};

use self::block::paint_block;
use self::image::paint_image;
use self::input::paint_input;
use self::select::paint_select;

/// Shared caret-centred value-window computation for `<input>` fields, used by
/// both the painter and the input loop's column→character mapping.
pub(crate) use self::input::value_window;

/// A borrow of the canvas that the painters actually need.
///
/// `paint_layout_tree` builds a single canvas over the whole
/// [`TermLayout::layout`] buffer and reuses it across the forest. The rest
/// of the canvas API (`columns`, row math) is read-only and cheap to copy.
pub(super) struct PaintCanvas<'a> {
    pub layout: &'a mut [Block],
    pub columns: u16,
    /// Image raster cache shared with the owning [`TermLayout`], so repeated
    /// `<img>` paints across frames reuse the decoded grid instead of
    /// re-reading the file.
    pub image_cache: &'a mut ImageCache,
    /// Optional absolute canvas rect restricting cell writes. `None` paints
    /// unrestricted (full frame); `Some` is the incremental-repaint region
    /// where every write outside it is dropped so the rest of the canvas
    /// keeps the previous frame's pixels.
    pub(super) region: Option<Rect>,
    /// `<input>` edit state read by the input painter (caret / value /
    /// focus). Immutable during a paint pass.
    pub input_states: &'a HashMap<NodeId, InputRenderState>,
    /// The `<select>` node whose dropdown is currently open (`None` = none).
    /// Read by the select painter for the open-frame colour.
    pub open_select: Option<NodeId>,
}

impl<'a> PaintCanvas<'a> {
    pub(super) fn cell_index(&self, x: usize, y: usize) -> usize {
        y.saturating_mul(self.columns as usize).saturating_add(x)
    }

    /// True when the canvas cell at 1-D index `idx` may be written by this
    /// pass. Unrestricted when no repaint region is set.
    #[inline]
    pub(super) fn in_region(&self, idx: usize) -> bool {
        match self.region {
            None => true,
            Some(r) => {
                let cols = self.columns as usize;
                let y = idx / cols;
                let x = idx - y * cols;
                x >= r.x
                    && x < r.x.saturating_add(r.width)
                    && y >= r.y
                    && y < r.y.saturating_add(r.height)
            }
        }
    }
}

/// Paint every box in `boxes` into `term_layout`.
///
/// Painting is deliberately serial. The top-level forest is a single box
/// for typical documents (root → `<html>`), so task-per-box parallelism
/// would spawn exactly one task and only pay `rayon` dispatch overhead;
/// the workload itself is memory-bound cell writes on a buffer of at most
/// the content height × `columns` cells. Top-level boxes occupy
/// disjoint `rect`s, so a sequential pass writes each cell at most once.
/// Children are painted inside each box — the parent must paint its
/// background before the child reads `effective_bg` for its own cells, so
/// the parent/child chain is inherently sequential anyway.
///
/// Earlier versions tried to parallelise: the original used raw-pointer
/// slicing of the whole buffer (`std::slice::from_raw_parts_mut` plus a
/// lifetime `transmute_copy`), which the project forbids as `unsafe`; a
/// later safe variant fanned out per-top-level-box bands via `split_at_mut`,
/// which measured no speedup for the single-box documents that occur in
/// practice.
pub fn paint_layout_tree(boxes: &[LayoutBox], arena: &NodeArena, term_layout: &mut TermLayout) {
    let mut max_y = 0usize;
    for b in boxes {
        let bottom = b.rect.y.saturating_add(b.rect.height.max(1));
        if bottom > max_y {
            max_y = bottom;
        }
    }
    if max_y as u32 > term_layout.rendered_rows {
        term_layout.rendered_rows = max_y as u32;
    }

    let columns = term_layout.columns;
    let mut canvas = PaintCanvas {
        layout: &mut term_layout.layout,
        columns,
        image_cache: &mut term_layout.image_cache,
        region: None,
        input_states: &term_layout.input_states,
        open_select: term_layout.open_select,
    };
    for b in boxes {
        paint_box(b, arena, &mut canvas, None, None);
    }
    fill_canvas_background(&mut canvas, boxes, arena, max_y);
}

/// Repaint only the cells inside `region`, leaving the rest of the canvas
/// untouched.
///
/// Backs the incremental render path: the layout has already been re-run and
/// every box rect verified unchanged against the cached layout, so only the
/// cells the dirty nodes' boxes (and their ancestors' background fills) cover
/// can differ from the previous frame. Boxes whose rect does not intersect
/// `region` are skipped entirely; the rest paint normally, with every cell
/// write gated to the region by [`PaintCanvas::in_region`]. Because only the
/// region's cells are rewritten, the diff renderer emits exactly the cells
/// that changed.
pub fn paint_layout_tree_region(
    boxes: &[LayoutBox],
    arena: &NodeArena,
    term_layout: &mut TermLayout,
    region: Rect,
) {
    let mut max_y = 0usize;
    for b in boxes {
        let bottom = b.rect.y.saturating_add(b.rect.height.max(1));
        if bottom > max_y {
            max_y = bottom;
        }
    }
    if max_y as u32 > term_layout.rendered_rows {
        term_layout.rendered_rows = max_y as u32;
    }

    let columns = term_layout.columns;
    let mut canvas = PaintCanvas {
        layout: &mut term_layout.layout,
        columns,
        image_cache: &mut term_layout.image_cache,
        region: Some(region),
        input_states: &term_layout.input_states,
        open_select: term_layout.open_select,
    };
    for b in boxes {
        if intersect_rects(b.rect, region).is_some() {
            paint_box(b, arena, &mut canvas, None, None);
        }
    }
    fill_canvas_background(&mut canvas, boxes, arena, max_y);
}

/// Paint the canvas background below the content, browser-style.
///
/// In browsers the `<html>` element's `background-color` paints the whole
/// canvas; when `<html>` is transparent the `<body>` background propagates
/// instead. Boxes only fill their own rects, so a short page would otherwise
/// leave the terminal rows below the content unpainted. This fills every
/// still-default cell from the content bottom down to the canvas end with the
/// propagated colour. Cell writes stay gated by [`PaintCanvas::in_region`], so
/// the incremental-repaint path never touches cells outside its region.
fn fill_canvas_background(
    canvas: &mut PaintCanvas<'_>,
    boxes: &[LayoutBox],
    arena: &NodeArena,
    content_bottom: usize,
) {
    let Some(bg) = find_canvas_background(boxes, arena) else {
        return;
    };
    let cols = canvas.columns as usize;
    let rows = canvas.layout.len() / cols;
    let start_row = content_bottom.min(rows);
    let region = canvas.region;
    for y in start_row..rows {
        let row_start = y * cols;
        for (i, cell) in canvas.layout[row_start..row_start + cols].iter_mut().enumerate() {
            let in_region = match region {
                None => true,
                Some(r) => {
                    i >= r.x
                        && i < r.x.saturating_add(r.width)
                        && y >= r.y
                        && y < r.y.saturating_add(r.height)
                }
            };
            if !in_region {
                continue;
            }
            // Only still-default cells: never clobber content that happens to
            // sit below `content_bottom` (e.g. a positioned child).
            if cell.word == " " && cell.bcolor.is_none() && cell.style.is_empty() {
                cell.bcolor = Some(bg);
            }
        }
    }
}

/// The colour the canvas should paint: the `<html>` background, or the
/// `<body>` background when `<html>` has none (browser propagation). `None`
/// when neither element paints one.
fn find_canvas_background(boxes: &[LayoutBox], arena: &NodeArena) -> Option<Color> {
    for b in boxes {
        if let Some(bg) = box_canvas_background(b, arena) {
            return Some(bg);
        }
    }
    None
}

fn box_canvas_background(lb: &LayoutBox, arena: &NodeArena) -> Option<Color> {
    let node = &arena[lb.node_id];
    if node.node_type == NodeType::Html {
        if let Some(bg) = node.css_attr.get_bg_color() {
            return Some(bg);
        }
        for child in &lb.children {
            if arena[child.node_id].node_type == NodeType::Body
                && let Some(bg) = arena[child.node_id].css_attr.get_bg_color()
            {
                return Some(bg);
            }
        }
    }
    // Fragment documents may lack an `<html>` wrapper — recurse.
    for child in &lb.children {
        if let Some(bg) = box_canvas_background(child, arena) {
            return Some(bg);
        }
    }
    None
}

/// Clamp `a` to the intersection with `b`, or return `None` when the two
/// rectangles do not overlap. Both rects are in absolute canvas coordinates.
pub(super) fn intersect_rects(a: Rect, b: Rect) -> Option<Rect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = a.x.saturating_add(a.width).min(b.x.saturating_add(b.width));
    let y1 =
        a.y.saturating_add(a.height)
            .min(b.y.saturating_add(b.height));
    if x1 <= x0 || y1 <= y0 {
        None
    } else {
        Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
    }
}

/// Paint one box (and its subtree) into the canvas.
///
/// `clip` is the box's inherited clip region (absolute canvas coordinates):
/// the intersection of every ancestor's border-box. Content that would be
/// painted outside the clip is dropped instead of spilling onto rows below
/// or beyond the box — on very small terminals the flexible algorithm can
/// hand a box a rect smaller than its content, and without this clip the
/// wrapped lines / block children would paint over the next sibling.
fn paint_box(
    lb: &LayoutBox,
    arena: &NodeArena,
    canvas: &mut PaintCanvas<'_>,
    parent_bg: Option<Color>,
    clip: Option<Rect>,
) {
    // This box's own clip = inherited clip ∩ own border-box. A box never
    // paints outside its ancestors' borders.
    let own_clip = match clip {
        Some(c) => match intersect_rects(c, lb.rect) {
            Some(r) => r,
            None => return,
        },
        None => lb.rect,
    };
    if own_clip.width == 0 || own_clip.height == 0 {
        // Zero-area box: no chrome of its own, but positioned children are
        // anchored to a containing block (or offset out of this area) and
        // must still paint. Static / relative-in-place children stay clipped
        // away by the zero clip.
        for child in sorted_children(lb) {
            if !matches!(child.positioning.position, Position::Static) {
                paint_box(child, arena, canvas, parent_bg, None);
            }
        }
        return;
    }
    let my_bg = match &lb.kind {
        BoxKind::Normal => arena[lb.node_id].css_attr.get_bg_color(),
        BoxKind::Replaced(_) => None,
        BoxKind::Anonymous => None,
    };
    match &lb.kind {
        BoxKind::Normal => {
            paint_block(lb, arena, canvas, my_bg, parent_bg, own_clip);
        }
        BoxKind::Replaced(ReplacedContent::Image { cols, rows, .. }) => {
            paint_image(lb, canvas, *cols, *rows, own_clip);
        }
        BoxKind::Replaced(ReplacedContent::Input) => {
            paint_input(lb, arena, canvas, own_clip);
        }
        BoxKind::Replaced(ReplacedContent::Select) => {
            paint_select(lb, arena, canvas, own_clip);
        }
        BoxKind::Anonymous => {
            // Anonymous boxes have no chrome of their own; their visible
            // background is whatever their children paint, so just descend.
        }
    }
    // The effective bg visible at this box's rect becomes the parent bg for
    // any descendant. If the current box has its own bg, that wins; otherwise
    // we inherit. This is what lets `<aside bg=cyan><p>30col</p></aside>`
    // render the text with cyan bg even though `<p>` itself has no bg.
    let effective_bg = my_bg.or(parent_bg);
    // Children are positioned inside the box's content-box (the rect shrunk
    // by border + padding); anything that spills out of it — e.g. a squeezed
    // flex item's wrapped lines — is dropped rather than painted over the
    // padding / border rows of this box or the sibling below.
    let edges = &lb.paint_style.box_edges;
    let content_area = Rect::new(
        lb.rect
            .x
            .saturating_add(edges.border.left.thickness())
            .saturating_add(edges.padding.left),
        lb.rect
            .y
            .saturating_add(edges.border.top.thickness())
            .saturating_add(edges.padding.top),
        lb.rect
            .width
            .saturating_sub(edges.inner_horizontal())
            .max(1),
        lb.rect.height.saturating_sub(edges.inner_vertical()).max(1),
    );
    let children_clip = match intersect_rects(own_clip, content_area) {
        Some(c) => c,
        None => return,
    };
    paint_children(lb, arena, canvas, effective_bg, Some(children_clip));
}

/// Paint `lb`'s children in stacking order.
///
/// `children_clip` is inherited by static / relative children (the parent's
/// content area). Detached positioned children are not clipped by the parent
/// — they are anchored to a containing block that may extend beyond it — and
/// their own rects clip their descendants via [`paint_box`].
fn paint_children(
    lb: &LayoutBox,
    arena: &NodeArena,
    canvas: &mut PaintCanvas<'_>,
    effective_bg: Option<Color>,
    children_clip: Option<Rect>,
) {
    for child in sorted_children(lb) {
        // Positioned children (absolute / fixed / relative) may paint outside
        // the parent's content area — they are anchored to a containing block
        // or offset out of flow — so they are not clipped by the ancestor
        // chain. Their own rects still clip their descendants via
        // [`paint_box`]. Static children inherit the parent's clip.
        let clip = match child.positioning.position {
            Position::Static => children_clip,
            _ => None,
        };
        paint_box(child, arena, canvas, effective_bg, clip);
    }
}

/// Children in paint stacking order.
///
/// Higher `z-index` paints later (on top); among equal values a positioned
/// box paints above static content (browser rule); the stable sort keeps DOM
/// order within the same level.
fn sorted_children(lb: &LayoutBox) -> Vec<&LayoutBox> {
    let mut children: Vec<&LayoutBox> = lb.children.iter().collect();
    children.sort_by_key(|c| {
        let positioned = !matches!(c.positioning.position, Position::Static);
        (c.positioning.z_index.unwrap_or(0), positioned as u8)
    });
    children
}
