use crate::{
    graphics::{ClipRect, render_image_at_slice},
    layout::{LayoutBox, Rect, ReplacedContent},
};

use super::{PaintCanvas, intersect_rects};

pub(crate) fn paint_image(
    lb: &LayoutBox,
    canvas: &mut PaintCanvas<'_>,
    cols: usize,
    rows: usize,
    clip: Rect,
) {
    let path = match &lb.kind {
        crate::layout::BoxKind::Replaced(ReplacedContent::Image { path, .. }) => path,
        _ => return,
    };
    let r = lb.rect;
    let canvas_cols = canvas.columns as usize;
    if r.x >= canvas_cols {
        // The whole image sits past the terminal's right edge; painting it
        // would wrap into the next row of the flat canvas buffer.
        return;
    }
    // Only the intersection with the inherited clip is drawn: an image whose
    // rect overflows an ancestor (flex-shrunk on a tiny terminal) must not
    // spill over the box below. The image is decoded at its FULL box size and
    // the visible region is stamped as a crop, so the overflow is clipped at
    // native scale instead of squeezing the whole image into the visible
    // region.
    let Some(visible) = intersect_rects(Rect::new(r.x, r.y, cols, rows), clip) else {
        return;
    };
    if let Err(error) = render_image_at_slice(
        canvas.image_cache,
        canvas.layout,
        canvas.columns,
        path.as_str(),
        (r.x, r.y),
        (cols as u32, rows as u32),
        Some(ClipRect {
            x: visible.x,
            y: visible.y,
            width: visible.width,
            height: visible.height,
        }),
    ) {
        log::debug!("Image paint skipped for {}: {}", path, error);
    }
}
