use anyhow::Result;
use htui::render;

fn main() -> Result<()> {
    let html = r#"<h1>Hello</h1>
    <h2>World</h2>
    <img src="./test_html/src/pic.png">
    "#;
    let css = "p { color: red; }
        h1 { color: blue; }";
    render(html, css);
    Ok(())
}
