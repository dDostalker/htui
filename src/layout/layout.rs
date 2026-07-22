use anyhow::{Ok, Result};
use crossterm::terminal;

use crate::{
    layout::DEFAULT_LAYOUT,
    tdom::node::{Node, NodeArena, NodeId, NodeType},
    tui::graph_api::TermLayout,
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
    pub fn new(map: &TermMap) -> Result<TermLayout> {
        let layout = vec![" ".to_string(); map.columns as usize * map.rows as usize];
        Ok(TermLayout { layout, columns: map.columns, ..Default::default() })
    }
}

impl TermLayout {
    /// 将 DOM 树绘制到终端画布
    pub fn draw(
        &mut self,
        arena: &NodeArena,
        root_id: NodeId,
        term_map: &TermMap,
    ) -> Result<()> {
        let root = &arena[root_id];
        assert_eq!(root.node_type, NodeType::Global);
        let children: Vec<NodeId> = root.children.clone();
        for child_id in children {
            self._draw(&arena[child_id], arena, term_map)?;
        }
        Ok(())
    }

    fn _draw(
        &mut self,
        node: &Node,
        arena: &NodeArena,
        term_map: &TermMap,
    ) -> Result<()> {
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
    fn test_get_ter_map() {
        let map = TermMap::get_ter_map().unwrap();
        dbg!(&map);
    }
}
