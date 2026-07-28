//! Paint phase.
//!
//! Walks a [`LayoutBox`] forest produced by [`tree::layout_tree`] and writes
//! styled cells into the [`TermLayout`](crate::tui::graph_api::TermLayout)
//! canvas. Reads no DOM state: every paint decision is derived from the box's
//! own `rect`, `kind`, and `box_edges` plus the CSS attributes stored on the
//! source node.

mod block;
mod border;
mod image;
mod text;

use crossterm::style::Color;

use crate::{
    layout::{BoxKind, LayoutBox},
    tdom::node::NodeArena,
    tui::graph_api::TermLayout,
};

use self::{block::paint_block, image::paint_image};

/// Paint every box in `boxes` into `term_layout`.
///
/// Boxes are independent: each paints into its own `rect` regardless of
/// sibling order, so painters may run concurrently in the future if needed.
pub fn paint_layout_tree(boxes: &[LayoutBox], arena: &NodeArena, term_layout: &mut TermLayout) {
    let mut max_y = 0usize;
    for b in boxes {
        paint_box(b, arena, term_layout, &mut max_y, None);
    }
    if max_y as u32 > term_layout.rendered_rows {
        term_layout.rendered_rows = max_y as u32;
    }
}

fn paint_box(
    lb: &LayoutBox,
    arena: &NodeArena,
    term_layout: &mut TermLayout,
    max_y: &mut usize,
    parent_bg: Option<Color>,
) {
    let bottom = lb.rect.y.saturating_add(lb.rect.height.max(1));
    if bottom > *max_y {
        *max_y = bottom;
    }
    let my_bg = match lb.kind {
        BoxKind::Block { .. } => arena[lb.node_id].css_attr.get_bg_color(),
        BoxKind::Image { .. } => None,
    };
    match lb.kind {
        BoxKind::Block {
            fill_bg,
            spacing_after,
            chrome,
        } => {
            paint_block(
                lb,
                arena,
                term_layout,
                fill_bg,
                spacing_after,
                chrome,
                my_bg,
                parent_bg,
            );
        }
        BoxKind::Image { cols, rows } => {
            paint_image(lb, term_layout, cols, rows);
        }
    }
    // The effective bg visible at this box's rect becomes the parent bg for
    // any descendant. If the current box has its own bg, that wins; otherwise
    // we inherit. This is what lets `<aside bg=cyan><p>30col</p></aside>`
    // render the text with cyan bg even though `<p>` itself has no bg.
    let effective_bg = my_bg.or(parent_bg);
    for child in &lb.children {
        paint_box(child, arena, term_layout, max_y, effective_bg);
    }
}

pub(super) fn cell_index(term_layout: &TermLayout, x: usize, y: usize) -> usize {
    y.saturating_mul(term_layout.columns as usize)
        .saturating_add(x)
}
