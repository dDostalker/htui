use crossterm::style::Color;

use crate::{
    layout::{BorderStyle, LayoutBox},
    tui::graph_api::{Block, FontStyle, TermLayout},
};

use super::cell_index;

/// Render the four border edges of a block-level box.
///
/// Border characters are the Unicode box-drawing glyphs (─ │ ┌ ┐ └ ┘) so the
/// rect reads as a continuous frame in the terminal. Each side picks its
/// color from `border.color` if set, otherwise falls back to the box's text
/// color. Borders with width > 1 currently render as a single row/column —
/// terminals cannot express thicker edges without mis-aligning the inner
/// content area, so any width > 1 is treated as 1.
pub(super) fn paint_borders(
    lb: &LayoutBox,
    term_layout: &mut TermLayout,
    fg: Option<Color>,
    bg: Option<Color>,
) {
    let sides = &lb.box_edges.border;
    if !sides.any_visible() {
        return;
    }
    let r = lb.rect;
    if r.width == 0 || r.height == 0 {
        return;
    }

    let cols = term_layout.columns as usize;
    let cell_start = cell_index(term_layout, r.x, r.y);
    let max_cells = term_layout.layout.len();

    // Effective border thickness per side, clamped to the rect size and
    // collapsed to 1 — see function-level note.
    let top_t = sides.top.thickness().clamp(0, r.height).min(1);
    let bottom_t = sides.bottom.thickness().clamp(0, r.height).min(1);
    let left_t = sides.left.thickness().clamp(0, r.width).min(1);
    let right_t = sides.right.thickness().clamp(0, r.width).min(1);

    // Edge colors: each side has its own color, falling back to the box fg.
    let top_color = sides.top.color.or(fg);
    let bottom_color = sides.bottom.color.or(fg);
    let left_color = sides.left.color.or(fg);
    let right_color = sides.right.color.or(fg);

    // Borders use no font style flags; their glyphs are the box-drawing set.
    let style = FontStyle::empty();

    let right_col_start = r.width.saturating_sub(right_t);

    // Top edge: rows 0..top_t, columns from left_t to right_col_start.
    if sides.top.style == BorderStyle::Solid && top_t > 0 {
        let horiz = "─";
        for row_offset in 0..top_t {
            let row_start = cell_start + row_offset * cols;
            for col in left_t..right_col_start {
                let idx = row_start + col;
                if idx < max_cells {
                    term_layout.layout[idx] = Block::new(horiz.to_string(), top_color, bg, style);
                }
            }
        }
    }

    // Bottom edge: rows (height-bottom_t)..height, columns left_t..right_col_start.
    if sides.bottom.style == BorderStyle::Solid && bottom_t > 0 {
        let horiz = "─";
        let bottom_row = cell_start + (r.height - bottom_t) * cols;
        for row_offset in 0..bottom_t {
            let row_start = bottom_row + row_offset * cols;
            for col in left_t..right_col_start {
                let idx = row_start + col;
                if idx < max_cells {
                    term_layout.layout[idx] =
                        Block::new(horiz.to_string(), bottom_color, bg, style);
                }
            }
        }
    }

    // Left edge: rows top_t..(height-bottom_t), columns 0..left_t.
    if sides.left.style == BorderStyle::Solid && left_t > 0 {
        let vert = "│";
        for row_offset in top_t..r.height.saturating_sub(bottom_t) {
            let row_start = cell_start + row_offset * cols;
            for col_offset in 0..left_t {
                let idx = row_start + col_offset;
                if idx < max_cells {
                    term_layout.layout[idx] = Block::new(vert.to_string(), left_color, bg, style);
                }
            }
        }
    }

    // Right edge: rows top_t..(height-bottom_t), columns right_col_start..width.
    if sides.right.style == BorderStyle::Solid && right_t > 0 {
        let vert = "│";
        for row_offset in top_t..r.height.saturating_sub(bottom_t) {
            let row_start = cell_start + row_offset * cols;
            for col_offset in 0..right_t {
                let idx = row_start + right_col_start + col_offset;
                if idx < max_cells {
                    term_layout.layout[idx] = Block::new(vert.to_string(), right_color, bg, style);
                }
            }
        }
    }

    // Corners: overwrite the 4 corner cells so the edges join cleanly.
    let corner_chars = ["┌", "┐", "└", "┘"];
    let corner_positions: [(usize, usize); 4] = [
        (0, 0),
        (right_col_start, 0),
        (0, r.height - bottom_t),
        (right_col_start, r.height - bottom_t),
    ];
    let corner_colors = [
        top_color.or(left_color),
        top_color.or(right_color),
        bottom_color.or(left_color),
        bottom_color.or(right_color),
    ];
    for i in 0..4 {
        let ch = corner_chars[i];
        let (cx, cy) = corner_positions[i];
        let color = corner_colors[i];
        let idx = cell_start + cy * cols + cx;
        if idx < max_cells {
            term_layout.layout[idx] = Block::new(ch.to_string(), color, bg, style);
        }
    }

    // Corners / edges fully painted above; nothing more to do.
}
