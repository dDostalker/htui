use crossterm::style::Color;

use crate::{
    html::css_parse::TextAlign, layout::LayoutBox, tdom::node::NodeArena,
    tui::graph_api::block_style,
};

use super::{PaintCanvas, border::paint_borders, text::paint_runs};

pub(super) fn paint_block(
    lb: &LayoutBox,
    arena: &NodeArena,
    canvas: &mut PaintCanvas<'_>,
    my_bg: Option<Color>,
    parent_bg: Option<Color>,
) {
    let node = &arena[lb.node_id];
    let fg = node.css_attr.get_color();
    let style = block_style(&node.css_attr);
    let align = node.css_attr.get_text_align();

    // Effective bg: own bg wins; otherwise inherit the parent's bg so child
    // text appears on the rect's background rather than the terminal default.
    let effective_bg = my_bg.or(parent_bg);

    let r = lb.rect;
    let cols = canvas.columns as usize;
    let cell_start = canvas.cell_index(r.x, r.y);

    let fill_bg = lb.paint_style.fill_bg;
    let chrome = lb.paint_style.chrome;
    let spacing_after = lb.paint_style.spacing_after;
    let box_edges = &lb.paint_style.box_edges;

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
        let border_horizontal =
            box_edges.border.left.thickness() + box_edges.border.right.thickness();
        let border_top = box_edges.border.top.thickness();
        let border_left = box_edges.border.left.thickness();
        let has_padding = box_edges.padding.top
            + box_edges.padding.right
            + box_edges.padding.bottom
            + box_edges.padding.left
            > 0;
        let text_clamped = my_bg.is_some() && !chrome && !has_padding && lb.paint_style.font_size_cells > 1;
        let text_row_offset: usize = if text_clamped {
            (lb.paint_style.font_size_cells.saturating_sub(1)) / 2
        } else {
            0
        };
        let (bg_start_x, bg_width) = if text_clamped {
            if let Some(runs) = &lb.paint_style.runs {
                let inner_h = box_edges.inner_horizontal();
                let content_x_offset =
                    box_edges.border.left.thickness() + box_edges.padding.left;
                let content_width = r.width.saturating_sub(inner_h).max(1);
                let text_width: usize = runs
                    .iter()
                    .map(|run| run.width_at(lb.paint_style.font_size_cells))
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
        let rows_to_fill = r
            .height
            .saturating_sub(spacing_after)
            .saturating_sub(border_top);
        for row_offset in 0..rows_to_fill {
            if text_clamped && row_offset != text_row_offset {
                continue;
            }
            let row_cell_start = cell_start + (row_offset + border_top) * cols;
            for i in 0..bg_width {
                let idx = row_cell_start + (bg_start_x - r.x) + i;
                if idx < canvas.layout.len() {
                    // Mutate in place: `clear()` keeps the 4-byte buffer from
                    // `Block::default()`, `push(' ')` reuses it, then the
                    // colour fields flip. This avoids the per-cell
                    // alloc/free of `cells[idx] = Block::new(...)`.
                    let cell = &mut canvas.layout[idx];
                    cell.word.clear();
                    cell.word.push(' ');
                    cell.fcolor = fg;
                    cell.bcolor = bg_to_use;
                    cell.style = style;
                }
            }
        }
    }

    // Border glyphs overlay the border cells (after fill). When no border is
    // configured this is a no-op. Borders use the same fg / bg as the box;
    // individual border colors are layered on top when set.
    paint_borders(lb, canvas, fg, effective_bg);

    if let Some(runs) = &lb.paint_style.runs {
        // Inline content sits in the content-box (rect shrunk by border +
        // padding), not the border-box, so text-align and run positioning
        // work against the inner width.
        let inner_h = box_edges.inner_horizontal();
        let border_top = box_edges.border.top.thickness();
        let padding_top = box_edges.padding.top;
        let content_x_offset = box_edges.border.left.thickness() + box_edges.padding.left;
        let content_width = r.width.saturating_sub(inner_h).max(1);
        // When font-size > 1 the box's rect is taller (height = font_size
        // cells) so the text baseline visually sits on the middle row of the
        // rect, with empty cells of the run's background above and below.
        // The extra leading rows are also filled with `effective_bg` by the
        // background loop above, so the box reads as a tall band rather than
        // a 1-row strip.
        let vertical_offset = if lb.paint_style.font_size_cells > 1 {
            (lb.paint_style.font_size_cells.saturating_sub(1) / 2) * cols
        } else {
            0
        };
        let content_row = cell_start
            + (border_top.saturating_add(padding_top)) * cols
            + content_x_offset
            + vertical_offset;
        paint_runs(
            canvas,
            runs,
            content_row,
            content_width,
            align,
            effective_bg,
            lb.paint_style.font_size_cells,
        );
    }
}