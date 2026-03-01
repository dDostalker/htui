pub mod html;
pub mod layout;
pub mod tdom;
pub mod tui;

// 重新导出常用的类型以简化导入
pub use html::html_parse::Parser;
pub use layout::layout::TermMap;

use crate::html::{css_parse, html_parse};

fn parse(html: &mut html_parse::Parser, css: &mut css_parse::Parser) {
    let css = css.parse().unwrap();
    let mut html = html.parse().unwrap();
    css_parse::css_into_node(&mut html, &css).unwrap();
}
