use anyhow::{Ok, Result};
use crossterm::terminal;

use crate::{
    layout::DEFAULT_LAYOUT,
    tdom::node::{Node, NodeType},
    tui::graph_api::TermLayout,
};

/// 终端映射信息
#[derive(Debug)]
pub struct TermMap {
    /// 终端列数（字符宽度）
    pub columns: u16,
    /// 终端行数（字符高度）
    pub rows: u16,
}

impl TermMap {
    pub fn get_ter_map() -> Result<TermMap> {
        let (columns, rows) = terminal::size()?;
        Ok(TermMap { columns, rows })
    }
}

///创建画布以及画布信息部分
impl TermLayout {
    pub fn new(map: &TermMap) -> Result<TermLayout> {
        let layout = vec![" ".to_string(); map.columns as usize * map.rows as usize];

        Ok(TermLayout {
            layout,
            columns: map.columns,
            ..Default::default()
        })
    }
}

/// 画布内容描绘
impl TermLayout {
    pub fn draw(&mut self, tdom: &Node, term_map: &TermMap) -> Result<()> {
        assert_eq!(tdom.node_type, NodeType::Global);
        let children = tdom.children();
        for child in children {
            self._draw(child, term_map)?;
        }
        Ok(())
    }
    /// 绘制画布 递归
    fn _draw(&mut self, tdom: &Node, term_map: &TermMap) -> Result<()> {
        for draw_fn in DEFAULT_LAYOUT {
            draw_fn(tdom, self, term_map)?;
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
