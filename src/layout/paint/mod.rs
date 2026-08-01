//! Paint phase.
//!
//! Walks a [`LayoutBox`] forest produced by
//! [`super::layoutbox::inner_formatting_context::flow::layout_tree`] (and
//! parallel-laid flex / grid trees) and writes styled cells into the
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
mod text;

use crossterm::style::Color;

use crate::{
    layout::{BoxKind, LayoutBox, Rect, ReplacedContent},
    tdom::node::NodeArena,
    tui::graph_api::{Block, TermLayout},
};

use self::block::paint_block;
use self::image::paint_image;

/// A borrow of the canvas that the painters actually need.
///
/// `paint_layout_tree` builds one of these per parallel task by re-borrowing
/// a disjoint slice of [`TermLayout::layout`], which lets us fan out across
/// threads without leaking the whole `TermLayout` (and its `Vec`) into every
/// task. The rest of the canvas API (`columns`, row math) is read-only and
/// cheap to copy.
pub(super) struct PaintCanvas<'a> {
    pub layout: &'a mut [Block],
    pub columns: u16,
}

impl<'a> PaintCanvas<'a> {
    pub(super) fn cell_index(&self, x: usize, y: usize) -> usize {
        y.saturating_mul(self.columns as usize).saturating_add(x)
    }
}

/// Encode the raw layout pointer as a `usize` so the parallel closure's
/// `Send` bound only sees primitive captures. The pointer is reconstructed
/// inside each task via `addr as *mut Block`. This sidesteps the `*mut T` is
/// `!Send + !Sync` problem entirely without needing `unsafe impl Send` on a
/// wrapper (the raw pointer never appears as a captured variable — `usize`
/// flows across the thread boundary instead).
#[derive(Clone, Copy)]
struct LayoutAddr(usize);

/// Paint every box in `boxes` into `term_layout`.
///
/// Boxes are independent: each paints into its own `rect` regardless of
/// sibling order, so the top-level forest is fanned out across threads via
/// `rayon::scope`. Children are still painted sequentially inside each
/// top-level task — the parent must paint its background before the child
/// reads `effective_bg` for its own cells, so the parent/child chain is a
/// serial dependency.
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
    // Capture the raw pointer as a `usize` so the closure's `Send` bound only
    // sees primitive captured values. The raw pointer is reconstructed
    // inside each task via `usize as *mut Block`.
    let layout_addr = LayoutAddr(term_layout.layout.as_mut_ptr() as usize);
    let layout_len = term_layout.layout.len();

    // SAFETY: `rayon::scope` blocks until every spawned task finishes, so the
    // arena borrow is live for the duration of the scope. The transmute_copy
    // extends the lifetime to `'static` only for the scope's bound; nothing
    // outlives this function.
    let arena_static: &'static NodeArena = unsafe { std::mem::transmute_copy(&arena) };

    // SAFETY: Each top-level box paints within its own `rect`, which is
    // disjoint from sibling boxes' rects by the CSS block-layout invariant
    // (siblings are stacked or laid out side-by-side, never overlapping).
    // The `from_raw_parts_mut` slices are therefore disjoint — every parallel
    // write goes to a unique memory region, so no data race occurs.
    rayon::scope(|s| {
        for b in boxes {
            let addr = layout_addr;
            let len = layout_len;
            let cols = columns;
            let arena = arena_static;
            s.spawn(move |_| {
                let layout_ptr = addr.0 as *mut Block;
                let layout_slice = unsafe { std::slice::from_raw_parts_mut(layout_ptr, len) };
                let mut canvas = PaintCanvas {
                    layout: layout_slice,
                    columns: cols,
                };
                paint_box(b, arena, &mut canvas, None, None);
            });
        }
    });
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
    for child in &lb.children {
        paint_box(child, arena, canvas, effective_bg, Some(children_clip));
    }
}
