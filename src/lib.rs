pub mod html;
pub mod layout;
pub mod tdom;
pub mod tui;

use std::io::{stdout, Write};

use anyhow::Result;
    use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
// 重新导出常用的类型以简化导入
pub use html::html_parse::Parser;
pub use layout::layout::TermMap;

use crate::{
    html::{css_parse, html_parse},
    tdom::node::Node,
    tui::graph_api::{TermLayout, trigger_refresh},
};
/// 总体的渲染解析
    fn parse(html: &mut html_parse::Parser, css: &mut css_parse::Parser) -> Result<Node> {
    let css = css.parse().unwrap();
    let mut node = html.parse().unwrap();
    css_parse::css_into_node(&mut node, &css).unwrap();
    Ok(node)
}
/// 渲染
pub fn render(html: &str, css: &str) {
    execute!(stdout(), EnterAlternateScreen, EnableMouseCapture, Hide).unwrap();
    loop {
        execute!(
            stdout(),
            Clear(ClearType::All), // 清空整个屏幕
            MoveTo(0, 0)           // 光标移动到左上角
        )
        .unwrap();
        // 解析
        let mut html_parser = html_parse::Parser::new(html);
        let mut css_parser = css_parse::Parser::new(css);
        let node = parse(&mut html_parser, &mut css_parser).unwrap();
        let term_map = TermMap::get_ter_map().unwrap();
        let mut term_layout = TermLayout::new(&term_map).unwrap();
        term_layout.draw(&node, &term_map).unwrap();
        // 渲染页面输出
        print!("{}", term_layout);
        stdout().flush().unwrap();
        if !trigger_refresh() {
            break;
        }
    }
    execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture, Show).unwrap();
}
