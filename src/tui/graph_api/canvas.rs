use super::cell::Block;

const MAX_SCROLL_MULTIPLIER: usize = 20;

#[derive(Debug)]
pub struct TermLayout {
    pub layout: Vec<Block>,
    pub columns: u16,
    pub rows: u16,
    pub scroll_y: u32,
    pub rendered_rows: u32,
    pub zoom: f32,
}

impl TermLayout {
    pub fn max_cells(&self) -> usize {
        self.columns as usize * self.rows as usize * MAX_SCROLL_MULTIPLIER
    }

    pub fn effective_columns(&self) -> u16 {
        (self.columns as f32 / self.zoom).round() as u16
    }

    pub fn effective_rows(&self) -> u16 {
        (self.rows as f32 / self.zoom).round() as u16
    }

    pub fn max_scroll(&self) -> u32 {
        self.rendered_rows.saturating_sub(self.rows as u32)
    }

    pub fn clamp_scroll(&mut self) {
        let max = self.max_scroll();
        if self.scroll_y > max {
            self.scroll_y = max;
        }
    }
}
