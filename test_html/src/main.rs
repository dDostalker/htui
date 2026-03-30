use anyhow::Result;
use env_logger::Builder;
use htui::render;
use log::{debug, error, info, warn};

fn main() -> Result<()> {
    let html = include_str!("css.html");
    let css = "p { color: red; }
        h1 { color: blue; }";

    debug!(
        "准备渲染 HTML 长度:{} 字节，CSS 长度:{} 字节",
        html.len(),
        css.len()
    );
    render(html, css);
    info!("渲染完成");
    Ok(())
}
