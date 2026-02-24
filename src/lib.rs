pub mod html;
pub mod layout;
pub mod tdom;
pub mod tui;

// 重新导出常用的类型以简化导入
pub use html::html_parse::Parser;
pub use layout::layout::{TermLayout, TermMap};
