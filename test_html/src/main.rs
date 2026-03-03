use anyhow::Result;
use htui::render;

fn main() -> Result<()> {
    let html = "<h1>Hello</h1>
    <h2>World</h2>
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
    let css = "p { color: red; }
        h1 { color: blue; }";
    render(html, css);
    Ok(())
}
