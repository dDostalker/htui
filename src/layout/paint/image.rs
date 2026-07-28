use crate::{
    layout::LayoutBox,
    tui::graph_api::render_image_at_slice,
};

use super::PaintCanvas;

pub(super) fn paint_image(lb: &LayoutBox, canvas: &mut PaintCanvas<'_>, cols: usize, rows: usize) {
    let Some(path) = lb.image_path.as_ref() else {
        return;
    };
    let r = lb.rect;
    if let Err(error) = render_image_at_slice(
        canvas.layout,
        canvas.columns,
        path,
        (r.x, r.y),
        (cols as u32, rows as u32),
    ) {
        log::debug!("Image paint skipped for {}: {}", path, error);
    }
}
