//! `<button>` painting — a single-row push-button frame (`[ text ]`).

use crossterm::style::Color;

use crate::{
    layout::{LayoutBox, Rect},
    tdom::node::{NodeArena, NodeType},
    tui::graph_api::{FontStyle, char_width},
};

use super::{PaintCanvas, clamp_rect_to_canvas, intersect_rects};

pub(crate) fn paint_button(
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
    let Some(r) = clamp_rect_to_canvas(r, canvas_cols) else {
        return;
    };
    let Some(visible) = intersect_rects(Rect::new(r.x, r.y, r.width, r.height), clip) else {
        return;
    };

    let node = &arena[lb.node_id];
    let disabled = node.attributes.contains_key("disabled");
    // `<button>` draws its text content; an `<input type=button|submit|reset>`
    // draws its `value` attribute (with the browser default label when the
    // attribute is absent) through the same `[ text ]` frame.
    let text = if node.node_type == NodeType::Input {
        node.input_button_label().unwrap_or_default()
    } else {
        node.text_content(arena)
    };
    let fg = node.css_attr.get_color().unwrap_or(Color::White);
    let bg = node.css_attr.get_bg_color();
    // A disabled button is dimmed (frame and text both grey).
    let frame = if disabled {
        Some(Color::DarkGrey)
    } else {
        Some(fg)
    };
    let text_fg = if disabled {
        Some(Color::DarkGrey)
    } else {
        Some(fg)
    };

    // `[ text ]`: frame brackets, one padding column each side, then the
    // text content clipped to the box.
    put(canvas, cell_index(canvas, r.x, r.y), Some('['), frame, bg);
    put(
        canvas,
        cell_index(canvas, r.x.saturating_add(1), r.y),
        Some(' '),
        text_fg,
        bg,
    );
    let mut col = 0usize;
    for c in text.chars() {
        let w = char_width(c).max(1);
        let base = r.x.saturating_add(2).saturating_add(col);
        if base >= r.x.saturating_add(r.width).saturating_sub(1) {
            break;
        }
        for sub in 0..w {
            let abs = base.saturating_add(sub);
            if abs < visible.x || abs >= visible.x.saturating_add(visible.width) {
                continue;
            }
            put(
                canvas,
                cell_index(canvas, abs, r.y),
                if sub == 0 { Some(c) } else { None },
                text_fg,
                bg,
            );
        }
        col += w;
    }
    let right = r.x.saturating_add(r.width).saturating_sub(1);
    if right > r.x.saturating_add(2).saturating_add(col) {
        put(
            canvas,
            cell_index(canvas, right.saturating_sub(1), r.y),
            Some(' '),
            text_fg,
            bg,
        );
    }
    put(canvas, cell_index(canvas, right, r.y), Some(']'), frame, bg);
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
