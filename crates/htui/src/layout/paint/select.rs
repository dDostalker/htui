//! `<select>` field painting — a background-filled single-line field showing
//! the selected option's label with a trailing chevron. No bracket frame; a
//! `border:` declaration opts into a CSS frame via the shared control chrome.

use crate::{
    graphics::char_width,
    html::css_parse::parse_color_str,
    layout::layoutbox::geometry::select_options,
    layout::{LayoutBox, Rect},
    tdom::node::NodeArena,
};

use super::{
    PaintCanvas, cell_index, control_background_at, control_clip, paint_control_chrome, put,
};

/// Frame colour for an open (focused) dropdown (Tokyo Night accent cyan).
const OPEN_FRAME: &str = "#2ac3de";

pub(crate) fn paint_select(
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
    let fg = node.css_attr.get_color();
    let open = canvas.open_select == Some(lb.node_id);

    let (content, background) = paint_control_chrome(lb, arena, canvas, r, fg);
    // While the dropdown is open the label is drawn with the accent colour
    // (a border, when declared, keeps its own colour from the chrome).
    let label_fg = if open {
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

    let inner_width = content.width;
    // The chevron occupies the last content cell; the label is truncated to
    // fit in front of it.
    let chevron_w = char_width('▾').max(1);
    let max_label_w = inner_width.saturating_sub(chevron_w).max(1);

    let mut shown = label;
    while shown.chars().map(char_width).sum::<usize>() > max_label_w && !shown.is_empty() {
        shown.pop();
    }

    let inner_x = content.x;
    let mut col = 0usize;
    for ch in shown.chars() {
        let w = char_width(ch).max(1);
        for sub in 0..w {
            let abs_col = inner_x.saturating_add(col).saturating_add(sub);
            if abs_col < r.x || abs_col >= r.x.saturating_add(r.width) {
                continue;
            }
            put(
                canvas,
                cell_index(canvas, abs_col, content.y),
                if sub == 0 { Some(ch) } else { None },
                label_fg,
                control_background_at(&background, abs_col, content.y, r),
            );
        }
        col += w;
    }
    // Chevron sits in the last content cell.
    let chevron_col = inner_x
        .saturating_add(inner_width)
        .saturating_sub(chevron_w);
    if chevron_col >= r.x && chevron_col < r.x.saturating_add(r.width) {
        put(
            canvas,
            cell_index(canvas, chevron_col, content.y),
            Some('▾'),
            label_fg,
            control_background_at(&background, chevron_col, content.y, r),
        );
    }
}
