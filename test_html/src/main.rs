use anyhow::Result;
use htui::render;

fn main() -> Result<()> {
    let html = "<p>Hello</p>
    <p>World</p>
    <p>Hello</p>
    <p>World</p>
    <p>Hello</p>
    <p>World</p>
    <p>Hello</p>
    <p>World</p>
    <p>Hello</p>
    <p>World</p>
    <p>Hello</p>
    ";
    let css = "p { color: red; }";
    render(html, css);
    Ok(())
}
