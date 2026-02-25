use std::fmt::Display;

use crossterm::style::{Color, Stylize};

/// 终端画布
#[derive(Debug, Default)]
pub struct TermLayout {
    pub layout: Vec<char>,
    /// 当前绘画指针位置
    pub point: u64,
    /// 当前显示的字符数量
    pub len: u64,
}
/// 画布打印
impl Display for TermLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for row in &self.layout {
            write!(f, "{}", row)?;
        }
        write!(f, "")
    }
}

impl TermLayout {
    /// 替换指定画布文字
    pub fn paint_string(&mut self, s: &str, color: Option<Color>) -> u64 {
        let str_len = s.chars().count();
        for _ in 0..str_len {
            self.layout.remove(self.point as usize);
        }

        let content = if let Some(color) = color {
            format!("{}", s.with(color))
        } else {
            s.to_string()
        };

        for c in content.chars() {
            self.layout.insert(self.point as usize, c);
            self.point += 1;
        }
        let len = s.chars().count() as u64;
        self.len += len;
        len
    }
}
