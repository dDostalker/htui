use anyhow::{Ok, Result};
use crossterm::terminal;

use crate::{
    layout::DEFAULT_LAYOUT,
    tdom::node::{Node, NodeArena, NodeId, NodeType},
    tui::graph_api::{Block, TermLayout},
};

#[derive(Debug)]
pub struct TermMap {
    pub columns: u16,
    pub rows: u16,
}

impl TermMap {
    pub fn get_ter_map() -> Result<TermMap> {
        let (columns, rows) = terminal::size()?;
        log::info!("终端尺寸: {}x{}", columns, rows);
        let columns = if columns == 0 { 80 } else { columns };
        let rows = if rows == 0 { 24 } else { rows };
        Ok(TermMap { columns, rows })
    }
}

impl TermLayout {
    pub fn new(map: &TermMap, zoom: f32) -> Result<TermLayout> {
        let max_cells = map.columns as usize * map.rows as usize * 20;
        let layout = vec![Block::new(" ".to_string(), None, None); max_cells];
        Ok(TermLayout {
            layout,
            columns: map.columns,
            rows: map.rows,
            scroll_y: 0,
            point: 0,
            len: 0,
            rendered_rows: 0,
            zoom,
        })
    }
}

impl TermLayout {
    /// Draw the DOM tree onto the terminal canvas (virtual, may exceed terminal size).
    pub fn draw(&mut self, arena: &NodeArena, root_id: NodeId, term_map: &TermMap) -> Result<()> {
        let root = &arena[root_id];
        assert_eq!(root.node_type, NodeType::Global);
        let children: Vec<NodeId> = root.children.clone();
        for child_id in children {
            if self.is_full() {
                break;
            }
            self._draw(&arena[child_id], arena, term_map)?;
        }
        // 绘制完成后更新 rendered_rows
        self.rendered_rows = self.point / self.columns as u32;
        Ok(())
    }

    fn _draw(&mut self, node: &Node, arena: &NodeArena, term_map: &TermMap) -> Result<()> {
        if self.is_full() {
            return Ok(());
        }
        for draw_fn in DEFAULT_LAYOUT {
            draw_fn(node, self, term_map, arena)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_term_map_mock() {
        let map = TermMap {
            columns: 80,
            rows: 24,
        };
        assert_eq!(map.columns, 80);
        assert_eq!(map.rows, 24);
    }
}
