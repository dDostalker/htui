use std::fmt::Display;

use anyhow::Result;
use crossterm::terminal;

/// 终端映射信息
#[derive(Debug)]
pub struct TermMap {
    /// 终端列数（字符宽度）
    pub columns: u16,
    /// 终端行数（字符高度）
    pub rows: u16,
}
/// 终端画布
pub struct TermLayout(Vec<Vec<char>>);

impl TermLayout {
    fn new(map: &TermMap) -> Result<TermLayout> {
        let mut layout = vec![vec![' '; map.columns as usize]; map.rows as usize];

        Ok(TermLayout(layout))
    }
    /// 获取当前终端中界面信息
    fn get_ter_map() -> Result<TermMap> {
        let (columns, rows) = terminal::size()?;
        let block_count = (columns as usize) * (rows as usize);

        Ok(TermMap { columns, rows })
    }
}

impl Display for TermLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for row in &self.0 {
            for cell in row {
                write!(f, "{}", cell)?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_ter_map() {
        let map = TermLayout::get_ter_map().unwrap();
        dbg!(&map);
    }
}
