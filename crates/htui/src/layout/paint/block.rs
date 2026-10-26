use crossterm::style::Color;

use crate::{
    graphics::{FontStyle, block_style},
    html::css_parse::TextAlign,
    layout::{LayoutBox, Rect},
    tdom::node::NodeArena,
};

use super::{PaintCanvas, border::paint_borders, intersect_rects, text::paint_runs};

pub(crate) fn paint_block(
    lb: &LayoutBox,
    arena: &NodeArena,
    canvas: &mut PaintCanvas<'_>,
    my_bg: Option<Color>,
    parent_bg: Option<Color>,
    clip: Rect,
) {
    let node = &arena[lb.node_id];
    let fg = node.css_attr.get_color();
    let style = block_style(&node.css_attr);
    // The resolved alignment comes from the paint style, not the cascade:
    // user-agent defaults the layout phase resolved (`<th>`'s `center`) are
    // not stored in the node's `css_attr`, so the paint style is the single
    // source the paint phase may consult.
    let align = lb.paint_style.text_align;

    // Effective bg: own bg wins; otherwise inherit the parent's bg so child
    // text appears on the rect's background rather than the terminal default.
    let effective_bg = my_bg.or(parent_bg);

    let cols = canvas.columns as usize;
    // A box can be wider than the terminal when a parent formatting context
    // (flex, replaced element) hands it a rectangle that overflows the
    // viewport. The canvas is a flat 1-D buffer, so writing past the right
    // edge of a row would wrap into the next row's cells and corrupt the
    // siblings below it — the misalignment seen on very small terminals.
    // Clamp the rect to the canvas width so overflow is clipped at the
    // terminal edge (mirroring browser viewport clipping) instead of
    // wrapping into the next row.
    let r = lb.rect;
    if r.x >= cols || r.width == 0 {
        return;
    }
    let r = Rect::new(r.x, r.y, r.width.min(cols - r.x), r.height);
    // Inherited clip (ancestors' border-boxes) narrows the paint region
    // further: when a box overflows its parent, the cells beyond the clip are
    // dropped instead of painting over the sibling below.
    let r = match intersect_rects(r, clip) {
        Some(r) => r,
        None => return,
    };
    let cell_start = canvas.cell_index(r.x, r.y);

    // `fill_bg` is a layout-time flag: whether the base cascade gave the
    // element a background. A `:hover` re-cascade (the render thread's `Hover`
    // path) can add a background to an element that had none at layout time
    // without re-laying-out, so the fill must also run when the *current*
    // cascade carries one — otherwise the padding cells of e.g. a bordered
    // `.btn:hover` keep the parent background and the highlight looks hollow.
    let fill_bg = lb.paint_style.fill_bg || my_bg.is_some();
    let spacing_after = lb.paint_style.spacing_after;
    let box_edges = &lb.paint_style.box_edges;
    let font = lb.paint_style.font_size_cells.max(1);
    let line_h = lb.paint_style.line_height_cells.max(1);
    // Row of the glyph block within its line box. With `line-height > font`
    // the extra rows are split above and below the glyphs; otherwise the
    // glyphs keep the historical centering within the font-sized line box.
    let glyph_row_within_line = if line_h > font {
        (line_h - font) / 2
    } else {
        font.saturating_sub(1) / 2
    };

    if fill_bg {
        // Paint the background across every row of the rect that is *not* a
        // trailing spacing row. Only blocks with an explicit `background-color`
        // fill here; the trailing `spacing_after` rows reserved by `<hr>` stay
        // blank.
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
        let text_clamped = my_bg.is_some() && !has_padding && lb.paint_style.font_size_cells > 1;
        let text_row_offset: usize = if text_clamped {
            glyph_row_within_line
        } else {
            0
        };
        let (bg_start_x, bg_width) = if text_clamped {
            if let Some(runs) = &lb.paint_style.runs {
                let inner_h = box_edges.inner_horizontal();
                let content_x_offset = box_edges.border.left.thickness() + box_edges.padding.left;
                let content_width = r.width.saturating_sub(inner_h).max(1);
                // `runs` is `Vec<Vec<InlineRun>>` (one entry per visual
                // line after wrapping). The widest visual line drives
                // the text-align offset; the bg only needs to cover
                // that widest line, so flatten across all lines and
                // pick the max per-line width.
                let widest_line_width = runs
                    .iter()
                    .map(|line| {
                        line.iter()
                            .map(|run| run.width_at(lb.paint_style.font_size_cells))
                            .sum::<usize>()
                    })
                    .max()
                    .unwrap_or(0);
                let text_width = widest_line_width.min(content_width);
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
                if idx < canvas.layout.len() && canvas.in_region(idx) {
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

    // Inline content sits in the content-box (rect shrunk by border +
    // padding), not the border-box, so text-align and run positioning
    // work against the inner width.
    if lb.paint_style.column_layout.is_some() {
        paint_column_layout(
            lb,
            canvas,
            clip,
            box_edges,
            align,
            effective_bg,
            r,
            cell_start,
            style,
            glyph_row_within_line,
        );
    } else if let Some(runs) = &lb.paint_style.runs {
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
        let vertical_offset = glyph_row_within_line * cols;
        let content_row = cell_start
            + (border_top.saturating_add(padding_top)) * cols
            + content_x_offset
            + vertical_offset;
        // The runs of a clipping box (`overflow: hidden` / `auto` / `scroll`
        // / `clip`) are additionally bounded by the content-box (the rect
        // minus border + padding): wrapped text that overflows the allocated
        // content area must not paint onto the padding / border rows (which
        // would break the frame glyphs). The inherited clip still applies on
        // top. A `visible` box skips that bound — overflowing lines paint
        // beyond the box, bounded only by the inherited clip / canvas.
        let border_bottom = box_edges.border.bottom.thickness();
        let padding_bottom = box_edges.padding.bottom;
        let content_clip = Rect::new(
            r.x.saturating_add(content_x_offset),
            r.y.saturating_add(border_top).saturating_add(padding_top),
            content_width,
            r.height
                .saturating_sub(border_top)
                .saturating_sub(padding_top)
                .saturating_sub(border_bottom)
                .saturating_sub(padding_bottom)
                .max(1),
        );
        let overflow_visible =
            node.css_attr.get_overflow() != crate::html::css_parse::Overflow::Clip;
        if overflow_visible {
            paint_runs(
                canvas,
                runs,
                content_row,
                content_width,
                align,
                effective_bg,
                lb.paint_style.font_size_cells,
                lb.paint_style.line_height_cells,
                clip,
                true,
                lb.paint_style.text_indent,
            );
        } else if let Some(runs_clip) = intersect_rects(content_clip, clip) {
            paint_runs(
                canvas,
                runs,
                content_row,
                content_width,
                align,
                effective_bg,
                lb.paint_style.font_size_cells,
                lb.paint_style.line_height_cells,
                runs_clip,
                false,
                lb.paint_style.text_indent,
            );
        }
    }

    // Border glyphs overlay the border cells (after fill and content). When
    // no border is configured this is a no-op. Borders use the same fg / bg
    // as the box; individual border colors are layered on top when set. The
    // clamped rect is passed down so edges never extend past the terminal
    // width. Painting last guarantees a frame cell keeps its glyph even if
    // wrapped content reached it.
    paint_borders(lb, r, canvas, fg, effective_bg);
}

/// Paint a multi-column box's fragmented inline content and column rules.
///
/// Each column's wrapped lines are painted at its own column x with the
/// column width (the text may be `text-align`-ed inside the column); column
/// rules (`column-rule`) are vertical `│` glyphs centred in each gap, painted
/// with the rule colour over the box's effective background. All writes stay
/// inside the box's content area and are clipped by the inherited `clip`.
#[allow(clippy::too_many_arguments)]
fn paint_column_layout(
    lb: &LayoutBox,
    canvas: &mut PaintCanvas<'_>,
    clip: Rect,
    box_edges: &crate::html::box_edges::BoxEdges,
    align: TextAlign,
    effective_bg: Option<Color>,
    r: Rect,
    cell_start: usize,
    style: FontStyle,
    glyph_row_within_line: usize,
) {
    let Some(columns_layout) = &lb.paint_style.column_layout else {
        return;
    };
    let cols = canvas.columns as usize;
    let font = lb.paint_style.font_size_cells.max(1);
    let line_h = lb.paint_style.line_height_cells.max(1);
    let border_top = box_edges.border.top.thickness();
    let padding_top = box_edges.padding.top;
    let content_top_abs = r.y.saturating_add(border_top).saturating_add(padding_top);
    let content_right = r.x.saturating_add(r.width);
    let vertical_offset = glyph_row_within_line * cols;
    let content_height = columns_layout.height;

    // Each column paints its wrapped lines inside the box's content area.
    for (col, lines) in columns_layout.lines.iter().enumerate() {
        let col_x = columns_layout.col_x[col];
        if col_x >= content_right {
            continue;
        }
        let col_width = (content_right - col_x).min(columns_layout.col_width);
        let col_clip = Rect::new(col_x, content_top_abs, col_width, content_height);
        if let Some(col_clip) = intersect_rects(col_clip, clip) {
            let col_cell_start = cell_start
                .saturating_add(col_x.saturating_sub(r.x))
                .saturating_add(border_top.saturating_add(padding_top).saturating_mul(cols))
                .saturating_add(vertical_offset);
            paint_runs(
                canvas,
                lines,
                col_cell_start,
                col_width,
                align,
                effective_bg,
                font,
                line_h,
                col_clip,
                // Multicol fragmentation is intrinsic clipping: each column
                // bounds its own lines regardless of the box's overflow.
                false,
                0,
            );
        }
    }

    // Column rules: vertical `│` glyphs centred in each gap, spanning the
    // content height (clipped by the box's content area and the inherited
    // clip). The colour may be `None` (no declared rule / element colour) —
    // like borders, the cell then keeps the terminal default foreground.
    if !columns_layout.rule_x.is_empty() {
        let rule_color = columns_layout.rule_color;
        let row_top = content_top_abs;
        // Bottom (exclusive) of the content area: the rect minus the bottom
        // border and padding. The top border is already reflected in
        // `content_top_abs`, so it is not subtracted here.
        let content_area_bottom = r.y.saturating_add(
            r.height
                .saturating_sub(box_edges.border.bottom.thickness())
                .saturating_sub(box_edges.padding.bottom),
        );
        let row_bottom = content_top_abs
            .saturating_add(content_height)
            .min(content_area_bottom);
        for &rule_x in &columns_layout.rule_x {
            if rule_x >= content_right {
                continue;
            }
            for y in row_top..row_bottom {
                if y < clip.y || y >= clip.y.saturating_add(clip.height) {
                    continue;
                }
                let idx = canvas.cell_index(rule_x, y);
                if idx < canvas.layout.len() && canvas.in_region(idx) {
                    let cell = &mut canvas.layout[idx];
                    cell.word.clear();
                    cell.word.push('│');
                    cell.fcolor = rule_color;
                    cell.bcolor = effective_bg;
                    cell.style = style;
                }
            }
        }
    }
}
