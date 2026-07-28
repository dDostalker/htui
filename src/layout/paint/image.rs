use crate::{
    layout::LayoutBox,
    tui::graph_api::{TermLayout, render_image_at},
};

pub(super) fn paint_image(lb: &LayoutBox, term_layout: &mut TermLayout, cols: usize, rows: usize) {
    let Some(path) = lb.image_path.as_ref() else {
        return;
    };
    let r = lb.rect;
    if let Err(error) = render_image_at(term_layout, path, (r.x, r.y), (cols as u32, rows as u32)) {
        log::debug!("Image paint skipped for {}: {}", path, error);
    }
}
