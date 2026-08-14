//! `<select>` field painting — a single-line `[label ▾]` frame with an
//! accent border while the dropdown is open.

use crossterm::style::Color;

use crate::{
    html::css_parse::parse_color_str,
    layout::layoutbox::geometry::select_options,
    layout::{LayoutBox, Rect},
    tdom::node::NodeArena,
    tui::graph_api::{FontStyle, char_width},
};

use super::{PaintCanvas, intersect_rects};

/// Frame colour for an open (focused) dropdown (Tokyo Night accent cyan).
const OPEN_FRAME: &str = "#2ac3de";

pub(crate) fn paint_select(
    lb: &LayoutBox,
    arena: &NodeArena,
    canvas: &mut PaintCanvas<'_>,
    clip: Rect,
) {
    let r = lb.rect;
    let canvas_cols = canvas.columns as usize;
    if r.x >= canvas_cols || r.width == 0 || r.height == 0 {
        return;
    }
    let Some(visible) = intersect_rects(Rect::new(r.x, r.y, r.width, r.height), clip) else {
        return;
    };

    let node = &arena[lb.node_id];
    let fg = node.css_attr.get_color();
    let bg = node.css_attr.get_bg_color();
    let open = canvas.open_select == Some(lb.node_id);
    let frame = if open {
        parse_color_str(OPEN_FRAME).or(fg)
    } else {
        fg
    };

    // Current selection: the node's `value` attribute, falling back to the
    // first option.
    let options = select_options(arena, lb.node_id);
    let value = node.attributes.get("value").cloned().unwrap_or_default();
    let label = options
        .iter()
        .find(|o| o.value == value)
        .map(|o| o.label.clone())
        .unwrap_or_else(|| options.first().map(|o| o.label.clone()).unwrap_or_default());

    let inner_width = r.width.saturating_sub(2);
    // The chevron occupies the last content cell; the label is truncated to
    // fit in front of it.
    let chevron_w = char_width('▾').max(1);
    let max_label_w = inner_width.saturating_sub(chevron_w).max(1);

    let mut shown = label;
    while shown.chars().map(char_width).sum::<usize>() > max_label_w && !shown.is_empty() {
        shown.pop();
    }

    put(canvas, cell_index(canvas, r.x, r.y), Some('['), frame, bg);
    put(
        canvas,
        cell_index(canvas, r.x.saturating_add(r.width).saturating_sub(1), r.y),
        Some(']'),
        frame,
        bg,
    );

    let inner_x = r.x.saturating_add(1);
    let mut col = 0usize;
    for ch in shown.chars() {
        let w = char_width(ch).max(1);
        for sub in 0..w {
            let abs_col = inner_x.saturating_add(col).saturating_add(sub);
            if abs_col >= visible.x && abs_col < visible.x.saturating_add(visible.width) {
                put(
                    canvas,
                    cell_index(canvas, abs_col, r.y),
                    if sub == 0 { Some(ch) } else { None },
                    fg,
                    bg,
                );
            }
        }
        col += w;
    }
    // Chevron sits in the last content cell, right before `]`.
    let chevron_col = inner_x
        .saturating_add(inner_width)
        .saturating_sub(chevron_w);
    if chevron_col >= visible.x && chevron_col < visible.x.saturating_add(visible.width) {
        put(
            canvas,
            cell_index(canvas, chevron_col, r.y),
            Some('▾'),
            fg,
            bg,
        );
    }
}

fn cell_index(canvas: &PaintCanvas<'_>, x: usize, y: usize) -> usize {
    canvas.cell_index(x, y)
}

fn put(
    canvas: &mut PaintCanvas<'_>,
    idx: usize,
    word: Option<char>,
    fg: Option<Color>,
    bg: Option<Color>,
) {
    if idx >= canvas.layout.len() || !canvas.in_region(idx) {
        return;
    }
    let cell = &mut canvas.layout[idx];
    cell.word.clear();
    if let Some(c) = word {
        cell.word.push(c);
    }
    cell.fcolor = fg;
    cell.bcolor = bg;
    cell.style = FontStyle::empty();
}
