use std::fmt::Display;

use crossterm::{
    cursor::MoveTo,
    event::{self, Event, KeyCode},
    execute,
    style::{Color, Stylize},
};
use std::io::stdout;

/// 终端画布
#[derive(Debug, Default)]
pub struct TermLayout {
    pub layout: Vec<String>,
    /// 当前绘画指针位置
    pub point: u64,
    /// 当前显示的字符数量
    pub len: u64,
    pub columns: u16,
}
/// 画布打印
impl Display for TermLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, row) in self.layout.iter().enumerate() {
            if i == self.layout.len() - 1 {
                break;
            }
            let x = i as u16 % self.columns;
            let y = i as u16 / self.columns;
            if row != " " {
                execute!(stdout(), MoveTo(x, y)).ok();
                write!(f, "{}", row)?;
            }
        }
        write!(f, "")
    }
}

impl TermLayout {
    /// 替换指定画布文字
    pub fn paint_string(&mut self, s: &str, color: Option<Color>) -> u64 {
        let str_len = s.chars().count();
        
        let mut idx = self.point as usize;
        for c in s.chars() {
            if idx >= self.layout.len() {
                break;
            }
            let content = if let Some(color) = color {
                format!("{}", c.with(color))
            } else {
                c.to_string()
            };
            self.layout[idx] = content;
            idx += 1;
        }

        self.point += str_len as u64;
        let len = str_len as u64;
        self.len += len;
        len
    }
}
///判断当前是否应该刷新
pub fn trigger_refresh() -> bool {
    // 阻塞等待事件
    if let Ok(Event::Key(key)) = event::read() {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return false,
            _ => {}
        }
    }
    true
}
