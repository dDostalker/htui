use crate::{
    layout::{LayoutBox, Rect, ReplacedContent},
    tui::graph_api::render_image_at_slice,
};

use super::{PaintCanvas, intersect_rects};

pub(crate) fn paint_image(
    lb: &LayoutBox,
    canvas: &mut PaintCanvas<'_>,
    cols: usize,
    rows: usize,
    clip: Rect,
) {
    let ReplacedContent::Image { path, .. } = match &lb.kind {
        crate::layout::BoxKind::Replaced(content) => content,
        _ => return,
    };
    let r = lb.rect;
    let canvas_cols = canvas.columns as usize;
    if r.x >= canvas_cols {
        // The whole image sits past the terminal's right edge; painting it
        // would wrap into the next row of the flat canvas buffer.
        return;
    }
    // Only the intersection with the inherited clip is drawn: an image
    // whose rect overflows an ancestor (flex-shrunk on a tiny terminal) is
    // rendered into the visible region instead of spilling over the box
    // below.
    let Some(visible) = intersect_rects(Rect::new(r.x, r.y, cols, rows), clip) else {
        return;
    };
    if let Err(error) = render_image_at_slice(
        canvas.layout,
        canvas.columns,
        path.as_str(),
        (visible.x, visible.y),
        (visible.width as u32, visible.height as u32),
    ) {
        log::debug!("Image paint skipped for {}: {}", path, error);
    }
}
