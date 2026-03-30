use anyhow::Result;
use htui::render;

fn main() -> Result<()> {
    let html = include_str!("css.html");
    let css = "p { color: red; }
        h1 { color: blue; }";
    render(html, css);
    Ok(())
}
