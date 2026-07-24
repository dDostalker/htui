//! Paint phase.
//!
//! Walks a [`LayoutBox`] forest produced by [`tree::layout_tree`] and writes
//! styled cells into the [`TermLayout`](crate::tui::graph_api::TermLayout)
//! canvas. Reads no DOM state: every paint decision is derived from the box's
//! own `rect` and `kind` plus the CSS attributes stored on the source node.

use anyhow::Result;
use crossterm::style::Color;

use crate::{
    html::css_parse::{FontWeight, TextAlign},
    layout::{BoxKind, LayoutBox, Rect},
    tdom::node::NodeArena,
    tui::graph_api::{Block, TermLayout, char_width, render_image_at},
};

/// Paint every box in `boxes` into `term_layout`.
///
/// Boxes are independent: each paints into its own `rect` regardless of
/// sibling order, so painters may run concurrently in the future if needed.
pub fn paint_layout_tree(
    boxes: &[LayoutBox],
    arena: &NodeArena,
    term_layout: &mut TermLayout,
) -> Result<()> {
    let mut max_y = 0usize;
    for b in boxes {
        paint_box(b, arena, term_layout, &mut max_y, None)?;
    }
    if max_y as u32 > term_layout.rendered_rows {
        term_layout.rendered_rows = max_y as u32;
    }
    Ok(())
}

fn paint_box(
    lb: &LayoutBox,
    arena: &NodeArena,
    term_layout: &mut TermLayout,
    max_y: &mut usize,
    parent_bg: Option<Color>,
) -> Result<()> {
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
        } => {
            paint_block(
                lb,
                arena,
                term_layout,
                fill_bg,
                spacing_after,
                my_bg,
                parent_bg,
            )?;
        }
        BoxKind::Image { cols, rows } => {
            paint_image(lb, term_layout, cols, rows)?;
        }
    }
    // The effective bg visible at this box's rect becomes the parent bg for
    // any descendant. If the current box has its own bg, that wins; otherwise
    // we inherit. This is what lets `<aside bg=cyan><p>30col</p></aside>`
    // render the text with cyan bg even though `<p>` itself has no bg.
    let effective_bg = my_bg.or(parent_bg);
    for child in &lb.children {
        paint_box(child, arena, term_layout, max_y, effective_bg)?;
    }
    Ok(())
}

fn paint_block(
    lb: &LayoutBox,
    arena: &NodeArena,
    term_layout: &mut TermLayout,
    fill_bg: bool,
    spacing_after: usize,
    my_bg: Option<Color>,
    parent_bg: Option<Color>,
) -> Result<()> {
    let node = &arena[lb.node_id];
    let fg = node.css_attr.get_color();
    let bold = matches!(node.css_attr.get_font_weight(), FontWeight::Bold);
    let align = node.css_attr.get_text_align();

    // Effective bg: own bg wins; otherwise inherit the parent's bg so child
    // text appears on the rect's background rather than the terminal default.
    let effective_bg = my_bg.or(parent_bg);

    let r = lb.rect;
    let cols = term_layout.columns as usize;
    let cell_start = cell_index(term_layout, r.x, r.y);

    if fill_bg {
        // Paint the background across every row of the rect that is *not* a
        // trailing spacing row. `<aside>` opts into `fill_bg` unconditionally
        // so it reads as a solid stripe even without an explicit
        // `background-color`; other blocks only fill when CSS specifies a bg.
        // For boxes with `height > 1 + spacing_after` (e.g. `<aside>` wrapping
        // nested children) the bg covers all content rows; the trailing
        // `spacing_after` rows reserved by `<header>`/`<footer>` stay blank.
        let bg_to_use = effective_bg;
        let rows_to_fill = r.height.saturating_sub(spacing_after);
        let cells = term_layout.layout.as_mut_slice();
        for row_offset in 0..rows_to_fill {
            let row_cell_start = cell_start + row_offset * cols;
            for i in 0..r.width {
                let idx = row_cell_start + i;
                if idx < cells.len() {
                    cells[idx] = Block::new(" ".to_string(), fg, bg_to_use, bold);
                }
            }
        }
    }

    if let Some(text) = &lb.text {
        if !text.is_empty() {
            let text_width = text.chars().map(char_width).sum::<usize>().min(r.width);
            let offset = match align {
                TextAlign::Left => 0,
                TextAlign::Center => r.width.saturating_sub(text_width) / 2,
                TextAlign::Right => r.width.saturating_sub(text_width),
            };
            paint_text_row(
                term_layout,
                cell_start + offset,
                text,
                fg,
                effective_bg,
                bold,
            );
        }
    }

    Ok(())
}

fn paint_text_row(
    term_layout: &mut TermLayout,
    start: usize,
    text: &str,
    fg: Option<Color>,
    bg: Option<Color>,
    bold: bool,
) {
    let cells = term_layout.layout.as_mut_slice();
    let max = cells.len();
    let mut idx = start;
    for c in text.chars() {
        let w = char_width(c);
        if idx >= max {
            break;
        }
        if w == 2 && idx + 1 >= max {
            break;
        }
        cells[idx] = Block::new(c.to_string(), fg, bg, bold);
        if w == 2 {
            cells[idx + 1] = Block::new(String::new(), fg, bg, bold);
        }
        idx += w;
    }
}

fn paint_image(
    lb: &LayoutBox,
    term_layout: &mut TermLayout,
    cols: usize,
    rows: usize,
) -> Result<()> {
    let Some(path) = lb.image_path.as_ref() else {
        return Ok(());
    };
    let r = lb.rect;
    render_image_at(term_layout, path, (r.x, r.y), (cols as u32, rows as u32)).or_else(|e| {
        log::debug!("Image paint skipped for {}: {}", path, e);
        Ok(())
    })
}

fn cell_index(term_layout: &TermLayout, x: usize, y: usize) -> usize {
    y.saturating_mul(term_layout.columns as usize)
        .saturating_add(x)
}

/// Build a `Rect::x = 0, width = ...` offset for the given containing block.
#[allow(dead_code)]
pub fn text_offset(rect: Rect, align: TextAlign, text_width: usize) -> usize {
    let w = rect.width.min(text_width);
    match align {
        TextAlign::Left => 0,
        TextAlign::Center => rect.width.saturating_sub(w) / 2,
        TextAlign::Right => rect.width.saturating_sub(w),
    }
}
