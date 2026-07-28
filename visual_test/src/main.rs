//! htui visual test runner.
//!
//! Usage:
//!   cargo run -p visual_test -- [test_name]
//!
//! Logs are written to `%TEMP%/htui-visual-test.log` via
//! `htui::logging::init_file_logger`. If the file cannot be opened a warning
//! is printed to stderr and logging is silently disabled for the run.
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
    let log_path = std::env::temp_dir().join("htui-visual-test.log");
    if let Err(e) = htui::logging::init_file_logger(&log_path, None) {
        eprintln!("warning: file logger init failed ({e}); logs will be dropped");
    }

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
        "all_in_one" => test_case!(all_in_one, no_css),
        "headings" => test_case!(headings, no_css),
        "image" => test_case!(image, no_css),
        "colors" => test_case!(colors, no_css),
        "nested" => test_case!(nested, no_css),
        "text" => test_case!(text, no_css),
        "css" => test_case!(css, no_css),
        "header_footer" => test_case!(header_footer, no_css),
        "aside" => test_case!(aside, no_css),
        "helloworld" => test_case!(helloworld, no_css),
        "font_styles" => test_case!(font_styles, no_css),
        "list" => test_case!(list, no_css),
        "table" => test_case!(table, no_css),
        "box_model" => test_case!(box_model, no_css),
        "font_size" => test_case!(font_size, no_css),
        "flex" => test_case!(flex, no_css),
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
                "font_styles",
                "list",
                "table",
                "box_model",
                "font_size",
                "flex",
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
