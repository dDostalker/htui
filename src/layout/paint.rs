//! Paint phase.
//!
//! Walks a [`LayoutBox`] forest produced by [`tree::layout_tree`] and writes
//! styled cells into the [`TermLayout`](crate::tui::graph_api::TermLayout)
//! canvas. Reads no DOM state: every paint decision is derived from the box's
//! own `rect`, `kind`, and `box_edges` plus the CSS attributes stored on the
//! source node.

use anyhow::Result;
use crossterm::style::Color;

use crate::{
    html::css_parse::TextAlign,
    layout::{BorderStyle, BoxKind, InlineRun, LayoutBox},
    tdom::node::NodeArena,
    tui::graph_api::{Block, FontStyle, TermLayout, block_style, char_width, render_image_at},
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
    chrome: bool,
    my_bg: Option<Color>,
    parent_bg: Option<Color>,
) -> Result<()> {
    let node = &arena[lb.node_id];
    let fg = node.css_attr.get_color();
    let style = block_style(&node.css_attr);
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
        //
        // CSS rule: the background-color paints the **padding-box** (the
        // border-box minus the border thickness), not the margin area. We
        // therefore offset `bg_start_x` by `border.left.thickness()` and
        // reduce `bg_width` by `border.horizontal()`. Without this, a block
        // with `margin: 1px 6px; background-color: black` would paint the
        // margin ring as well, which is invisible in graphical browsers
        // but would stand out in the terminal as an extra coloured band
        // on every side of the block.
        let border_horizontal = lb.box_edges.border.left.thickness()
            + lb.box_edges.border.right.thickness();
        let border_top = lb.box_edges.border.top.thickness();
        let border_left = lb.box_edges.border.left.thickness();
        let has_padding = lb.box_edges.padding.top
            + lb.box_edges.padding.right
            + lb.box_edges.padding.bottom
            + lb.box_edges.padding.left
            > 0;
        let text_clamped = my_bg.is_some()
            && !chrome
            && !has_padding
            && lb.font_size_cells > 1;
        let text_row_offset: usize = if text_clamped {
            (lb.font_size_cells.saturating_sub(1)) / 2
        } else {
            0
        };
        let (bg_start_x, bg_width) = if text_clamped {
            if let Some(runs) = &lb.runs {
                let inner_h = lb.box_edges.inner_horizontal();
                let content_x_offset = lb.box_edges.border.left.thickness()
                    + lb.box_edges.padding.left;
                let content_width = r.width.saturating_sub(inner_h).max(1);
                let text_width: usize = runs
                    .iter()
                    .map(|run| run.width_at(lb.font_size_cells))
                    .sum();
                let text_width = text_width.min(content_width);
                let offset = match align {
                    TextAlign::Left => 0,
                    TextAlign::Center => content_width.saturating_sub(text_width) / 2,
                    TextAlign::Right => content_width.saturating_sub(text_width),
                };
                let visible_width = text_width.max(1);
                (r.x + content_x_offset + offset, visible_width)
            } else {
                (r.x, r.width)
            }
        } else {
            // Default: bg covers the padding-box (border-box minus border).
            (r.x + border_left, r.width.saturating_sub(border_horizontal))
        };
        let bg_to_use = effective_bg;
        let rows_to_fill = r.height
            .saturating_sub(spacing_after)
            .saturating_sub(border_top);
        let cells = term_layout.layout.as_mut_slice();
        for row_offset in 0..rows_to_fill {
            if text_clamped && row_offset != text_row_offset {
                continue;
            }
            let row_cell_start = cell_start + (row_offset + border_top) * cols;
            for i in 0..bg_width {
                let idx = row_cell_start + (bg_start_x - r.x) + i;
                if idx < cells.len() {
                    cells[idx] = Block::new(" ".to_string(), fg, bg_to_use, style);
                }
            }
        }
    }

    // Border glyphs overlay the border cells (after fill). When no border is
    // configured this is a no-op. Borders use the same fg / bg as the box;
    // individual border colors are layered on top when set.
    paint_borders(lb, term_layout, fg, effective_bg);

    if let Some(runs) = &lb.runs {
        // Inline content sits in the content-box (rect shrunk by border +
        // padding), not the border-box, so text-align and run positioning
        // work against the inner width.
        let inner_h = lb.box_edges.inner_horizontal();
        let border_top = lb.box_edges.border.top.thickness();
        let padding_top = lb.box_edges.padding.top;
        let content_x_offset = lb.box_edges.border.left.thickness()
            + lb.box_edges.padding.left;
        let content_width = r.width.saturating_sub(inner_h).max(1);
        // When font-size > 1 the box's rect is taller (height = font_size
        // cells) so the text baseline visually sits on the middle row of the
        // rect, with empty cells of the run's background above and below.
        // The extra leading rows are also filled with `effective_bg` by the
        // background loop above, so the box reads as a tall band rather than
        // a 1-row strip.
        let vertical_offset = if lb.font_size_cells > 1 {
            (lb.font_size_cells.saturating_sub(1) / 2) * cols
        } else {
            0
        };
        let content_row = cell_start
            + (border_top.saturating_add(padding_top)) * cols
            + content_x_offset
            + vertical_offset;
        paint_runs(
            term_layout,
            runs,
            content_row,
            content_width,
            align,
            effective_bg,
            lb.font_size_cells,
        );
    }

    Ok(())
}

/// Render the four border edges of a block-level box.
///
/// Border characters are the Unicode box-drawing glyphs (─ │ ┌ ┐ └ ┘) so the
/// rect reads as a continuous frame in the terminal. Each side picks its
/// color from `border.color` if set, otherwise falls back to the box's text
/// color. Borders with width > 1 currently render as a single row/column —
/// terminals cannot express thicker edges without mis-aligning the inner
/// content area, so any width > 1 is treated as 1.
fn paint_borders(
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
                    term_layout.layout[idx] =
                        Block::new(horiz.to_string(), top_color, bg, style);
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
                    term_layout.layout[idx] =
                        Block::new(vert.to_string(), left_color, bg, style);
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
                    term_layout.layout[idx] =
                        Block::new(vert.to_string(), right_color, bg, style);
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

/// Render each run in `runs` with its own style, colour and background.
///
/// Width accounting uses [`InlineRun::width`] (which sums `char_width` per
/// char) so a wide CJK glyph at the end of one run does not collide with
/// the first cell of the next run. The block's `effective_bg` is applied
/// to runs whose own background is `None`, inheriting the surrounding
/// rect's fill (e.g. an `<aside>`'s `darkblue` strip showing through
/// unstyled text).
///
/// `font_size_cells` is the effective font-size of the containing box. When
/// `font_size_cells > 1`, each character cell is expanded so the character
/// occupies that many terminal columns (the trailing columns are filled with
/// the run's background, mirroring the empty space a larger font would leave
/// around its glyphs).
fn paint_runs(
    term_layout: &mut TermLayout,
    runs: &[InlineRun],
    cell_start: usize,
    block_width: usize,
    align: TextAlign,
    effective_bg: Option<Color>,
    font_size_cells: usize,
) {
    let total_width: usize = runs.iter().map(|r| r.width_at(font_size_cells)).sum();
    let total_width = total_width.min(block_width);
    let offset = match align {
        TextAlign::Left => 0,
        TextAlign::Center => block_width.saturating_sub(total_width) / 2,
        TextAlign::Right => block_width.saturating_sub(total_width),
    };
    // The text sits inside the content area (rect shrunk by border + padding);
    // `cell_start` is the first column of the content row and `block_width`
    // is the content-box width. Clip the painted text to this range so a long
    // inline run does not bleed into subsequent rows (or off the right edge
    // of the canvas) and overwrite neighbouring boxes.
    let row_end = cell_start.saturating_add(block_width);
    let mut index = cell_start + offset;
    for run in runs {
        if run.is_empty() {
            continue;
        }
        let run_bg = run.bg.or(effective_bg);
        paint_text_row(
            term_layout,
            index,
            &run.text,
            run.fg,
            run_bg,
            run.style,
            font_size_cells,
            row_end,
        );
        index = index.saturating_add(run.width_at(font_size_cells));
    }
}

fn paint_text_row(
    term_layout: &mut TermLayout,
    start: usize,
    text: &str,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
    font_size_cells: usize,
    row_end: usize,
) {
    let cells = term_layout.layout.as_mut_slice();
    let max = cells.len();
    let mut idx = start;
    let font_size = font_size_cells.max(1);
    for c in text.chars() {
        let w = char_width(c);
        // Stop when the next char + its background padding would step past
        // the content-box right edge or the canvas boundary. Without this,
        // a long `<p>` would spill its overflow into the rows below and
        // corrupt the cells of subsequent boxes.
        let slot_width = w.max(font_size);
        if idx.saturating_add(slot_width) > row_end {
            break;
        }
        if idx >= max {
            break;
        }
        if w == 2 && idx + 1 >= max {
            break;
        }
        cells[idx] = Block::new(c.to_string(), fg, bg, style);
        if w == 2 {
            cells[idx + 1] = Block::new(String::new(), fg, bg, style);
        }
        // Fill the remaining columns the "font" would occupy beyond the glyph
        // itself. Each character cell is expanded to `font_size` columns
        // wide — the glyph itself takes `w` columns (1 for ASCII, 2 for wide
        // CJK), and the rest are filled with space cells carrying the same
        // fg / bg / style so the terminal paints the background across the
        // whole slot. An empty word would be skipped by the renderer
        // (`render_to_bytes` drops cells with `word.is_empty()` so the
        // second half of a wide CJK glyph doesn't double-print), which would
        // erase the padded cell's bg. Wide glyphs still skip the
        // second-column overwrite via the `w == 2 && pad == 1` guard so the
        // paired empty cell stays intact.
        for pad in 1..font_size {
            let target = idx + pad;
            if target >= row_end {
                break;
            }
            if target >= max {
                break;
            }
            if w == 2 && pad == 1 {
                continue;
            }
            cells[target] = Block::new(" ".to_string(), fg, bg, style);
        }
        // Advance to the next character: the slot is `font_size` columns
        // wide (or `w` columns wide, whichever is larger — `w > font_size`
        // for narrow fonts rendering wide glyphs).
        idx += slot_width;
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