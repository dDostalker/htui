use anyhow::Result;
use crossterm::terminal;

use crate::{
    layout::{
        BoxConstraint, Cursor, paint::paint_layout_tree, tree::layout_tree,
    },
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::{Block, FontStyle, TermLayout},
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
        let layout = vec![Block::new(" ".to_string(), None, None, FontStyle::empty()); max_cells];
        Ok(TermLayout {
            layout,
            columns: map.columns,
            rows: map.rows,
            scroll_y: 0,
            rendered_rows: 0,
            zoom,
        })
    }
}

impl TermLayout {
    /// Draw the DOM tree onto the terminal canvas (virtual, may exceed terminal size).
    ///
    /// Two-phase pipeline:
    /// 1. [`layout_tree`] walks the DOM through `DEFAULT_LAYOUT` and builds a
    ///    `Vec<LayoutBox>` rooted at `root_id`. Each block claims a row range
    ///    inside the containing block.
    /// 2. [`paint_layout_tree`] writes the resulting rects into `self.layout`
    ///    using each box's own coordinates — no shared cursor.
    pub fn draw(&mut self, arena: &NodeArena, root_id: NodeId, term_map: &TermMap) -> Result<()> {
        let root = &arena[root_id];
        assert_eq!(root.node_type, NodeType::Global);
        let containing = BoxConstraint::root(term_map);
        let mut cursor = Cursor::new();
        let boxes = layout_tree(arena, root_id, containing, &mut cursor)?;
        paint_layout_tree(&boxes, arena, self)?;
        Ok(())
    }
}
