//! `<button>` painting — a background-filled push-button with its label
//! centred (browser style). No bracket frame: a `border:` declaration opts
//! into a CSS frame via the shared control chrome.

use crossterm::style::Color;

use crate::{
    graphics::char_width,
    layout::{LayoutBox, Rect},
    tdom::node::{NodeArena, NodeType},
};

use super::{
    PaintCanvas, cell_index, control_background_at, control_clip, paint_control_chrome, put,
};

pub(crate) fn paint_button(
    lb: &LayoutBox,
    arena: &NodeArena,
    canvas: &mut PaintCanvas<'_>,
    clip: Rect,
) {
    // Control chrome and replaced glyphs are painted directly by cell; reduce
    // the control rect to the inherited clip so a partially visible field
    // cannot overwrite a parent border or sibling cell.
    let Some(r) = control_clip(lb, canvas, clip) else {
        return;
    };

    let node = &arena[lb.node_id];
    let disabled = node.attributes.contains_key("disabled");
    // `<button>` draws its text content; an `<input type=button|submit|reset>`
    // draws its `value` attribute (with the browser default label when the
    // attribute is absent) through the same painter.
    let text = if node.node_type == NodeType::Input {
        node.input_button_label().unwrap_or_default()
    } else {
        node.text_content(arena)
    };
    let fg = node.css_attr.get_color().unwrap_or(Color::White);
    let text_fg = if disabled { Color::DarkGrey } else { fg };

    // The label is centred inside the chrome's content area, like a browser
    // button.
    let (content, background) = paint_control_chrome(lb, arena, canvas, r, Some(fg));

    let text_w: usize = text.chars().map(|c| char_width(c).max(1)).sum();
    let leading = content.width.saturating_sub(text_w) / 2;
    let mut col = 0usize;
    for c in text.chars() {
        let w = char_width(c).max(1);
        let base = content.x.saturating_add(leading).saturating_add(col);
        if base >= content.x.saturating_add(content.width) {
            break;
        }
        for sub in 0..w {
            let abs = base.saturating_add(sub);
            if abs < r.x || abs >= r.x.saturating_add(r.width) {
                continue;
            }
            put(
                canvas,
                cell_index(canvas, abs, content.y),
                if sub == 0 { Some(c) } else { None },
                Some(text_fg),
                control_background_at(&background, abs, content.y, r),
            );
        }
        col += w;
    }
}
