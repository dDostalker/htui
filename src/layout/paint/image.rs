use crate::{
    layout::{LayoutBox, ReplacedContent},
    tui::graph_api::render_image_at_slice,
};

use super::PaintCanvas;

pub(crate) fn paint_image(lb: &LayoutBox, canvas: &mut PaintCanvas<'_>, cols: usize, rows: usize) {
    let ReplacedContent::Image { path, .. } = match &lb.kind {
        crate::layout::BoxKind::Replaced(content) => content,
        _ => return,
    };
    let r = lb.rect;
    if let Err(error) = render_image_at_slice(
        canvas.layout,
        canvas.columns,
        path.as_str(),
        (r.x, r.y),
        (cols as u32, rows as u32),
    ) {
        log::debug!("Image paint skipped for {}: {}", path, error);
    }
}