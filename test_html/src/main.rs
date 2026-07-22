use anyhow::Result;
use htui::render;
use log::info;

fn main() -> Result<()> {
    // 将工作目录切换到 crate 根目录，确保 HTML 中的相对路径
    // （如 <img src="src/pic.png">）能正确解析
    std::env::set_current_dir(env!("CARGO_MANIFEST_DIR"))?;

    let args: Vec<String> = std::env::args().collect();
    let test_name = if args.len() > 1 {
        args[1].as_str()
    } else {
        "all_in_one"
    };

    let (html, css) = match test_name {
        "all_in_one" => (
            include_str!("all_in_one.html"),
            "h1 { color: cyan; } h2 { color: yellow; } h3 { color: green; } \
             p { color: grey; } .highlight { color: magenta; }",
        ),
        "headings" => (
            include_str!("headings.html"),
            "h1 { color: cyan; } h2 { color: green; } h3 { color: yellow; } \
             h4 { color: blue; } h5 { color: magenta; }",
        ),
        "image" => (include_str!("image.html"), ""),
        "colors" => (
            include_str!("colors.html"),
            ".red { color: red; } .green { color: green; } .blue { color: blue; } \
             .yellow { color: yellow; } .cyan { color: cyan; } \
             .magenta { color: magenta; } .white { color: white; } .grey { color: grey; }",
        ),
        "nested" => (
            include_str!("nested.html"),
            "h1 { color: yellow; } p { color: white; } \
             .deep { color: cyan; }",
        ),
        "text" => (
            include_str!("text.html"),
            "p { color: white; } h1 { color: cyan; } h2 { color: yellow; }",
        ),
        "css" => (
            include_str!("css.html"),
            // css.html 已内嵌 <style> 块，无需额外 CSS
            "",
        ),
        "helloworld" => (
            include_str!("helloworld.html"),
            "h1 { color: cyan; } p { color: white; }",
        ),
        _ => {
            eprintln!(
                "Unknown test: '{}'\n\
                 Available tests:\n  \
                 all_in_one   - Comprehensive demo (default)\n  \
                 headings     - H1-H5 heading levels\n  \
                 image        - Image rendering\n  \
                 colors       - Color rendering\n  \
                 nested       - Deeply nested structures\n  \
                 text         - Text content & layout\n  \
                 css          - CSS layout demo\n  \
                 helloworld   - Simple hello world",
                test_name
            );
            return Ok(());
        }
    };

    info!(
        "Running visual test: '{}' (HTML: {} bytes, CSS: {} bytes)",
        test_name,
        html.len(),
        css.len()
    );
    render(html, css);
    info!("Test '{}' completed", test_name);
    Ok(())
}
