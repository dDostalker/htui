//! htui visual test runner.
//!
//! Usage:
//!   cargo run -p visual_test -- [test_name]
//!
//! Each test lives in its own folder under `src/`:
//!   src/{name}/index.html    — HTML document
//!   src/{name}/style.css     — stylesheet (optional)

use anyhow::Result;
use htui::render;
use log::info;

macro_rules! test_case {
    ($name:ident) => {
        (
            include_str!(concat!(stringify!($name), "/index.html")),
            include_str!(concat!(stringify!($name), "/style.css")),
        )
    };
    ($name:ident, no_css) => {
        (include_str!(concat!(stringify!($name), "/index.html")), "")
    };
}

fn main() -> Result<()> {
    env_logger::init();

    // Set working dir to this crate root so relative paths in HTML
    // (e.g., <img src="src/image/pic.png">) resolve correctly.
    std::env::set_current_dir(env!("CARGO_MANIFEST_DIR"))?;

    let args: Vec<String> = std::env::args().collect();
    let test_name = if args.len() > 1 {
        args[1].as_str()
    } else {
        "all_in_one"
    };

    let (html, css) = match test_name {
        "all_in_one" => test_case!(all_in_one),
        "headings" => test_case!(headings),
        "image" => test_case!(image, no_css),
        "colors" => test_case!(colors),
        "nested" => test_case!(nested),
        "text" => test_case!(text),
        "css" => test_case!(css, no_css),
        "header_footer" => test_case!(header_footer, no_css),
        "aside" => test_case!(aside, no_css),
        "helloworld" => test_case!(helloworld),
        _ => {
            let available = [
                "all_in_one",
                "headings",
                "image",
                "colors",
                "nested",
                "text",
                "css",
                "header_footer",
                "aside",
                "helloworld",
            ];
            eprintln!(
                "Unknown test: '{test_name}'\nAvailable tests: {}",
                available.join(", ")
            );
            return Ok(());
        }
    };

    info!(
        "Running visual test: '{test_name}' (HTML: {} bytes, CSS: {} bytes)",
        html.len(),
        css.len()
    );
    render(html, css);
    info!("Test '{test_name}' completed");
    Ok(())
}
