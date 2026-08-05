//! htui visual test runner.
//!
//! Usage:
//!   cargo run -p visual_test -- [test_name]
//!   cargo run -p visual_test -- --dump [test_name] [--cols N] [--rows N]
//!
//! `--dump` renders headlessly (no raw mode / alternate screen) and prints
//! the canvas as plain text rows — handy for previews in a pipe or log.
//!
//! Logs are written to `%TEMP%/htui-visual-test.log` via
//! `htui::logging::init_file_logger`. If the file cannot be opened a warning
//! is printed to stderr and logging is silently disabled for the run.
//!
//! Each test lives in its own folder under `src/`:
//!   src/{name}/index.html    — HTML document
//!   src/{name}/style.css     — stylesheet (optional)

use anyhow::Result;
use htui::events::EventBindings;
use htui::render;
use htui::render_with_events;
use log::info;

mod sysmon;

/// Headless preview: parse the document, run the full layout pipeline into a
/// [`TermLayout`] canvas, and print each non-empty row as plain text so the
/// structure can be inspected without an interactive terminal.
///
/// When `bg_map` is set, every cell that carries a background colour is shown
/// as `█` and every other cell as a space — this reveals filled regions
/// (panels, bars, header/footer backgrounds) that the plain-text view hides.
fn dump(html: &str, css: &str, cols: u16, rows: u16, bg_map: bool) {
    use htui::html::{css_parse, html_parse};
    use htui::layout::layout::TermMap;
    use htui::tui::graph_api::TermLayout;

    let mut html_parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = html_parser.parse();
    let combined = if css.is_empty() {
        embedded
    } else if embedded.is_empty() {
        css.to_string()
    } else {
        format!("{embedded}\n{css}")
    };
    if !combined.trim().is_empty() {
        let mut css_parser = css_parse::Parser::new(&combined);
        let rules = css_parser.parse();
        css_parse::css_into_node(&mut arena, root_id, &rules);
    }

    let term_map = TermMap { columns: cols, rows };
    let mut tl = TermLayout::new(&term_map, 1.0);
    if let Err(e) = tl.draw(&arena, root_id, &term_map) {
        eprintln!("layout error: {e}");
        std::process::exit(1);
    }

    let c = term_map.columns as usize;
    for (i, chunk) in tl.layout.chunks(c).enumerate() {
        let s: String = if bg_map {
            chunk
                .iter()
                .map(|b| if b.bcolor.is_some() { '█' } else { ' ' })
                .collect()
        } else {
            chunk.iter().map(|b| b.word.as_str()).collect()
        };
        let trimmed = s.trim_end();
        if !trimmed.is_empty() {
            println!("{i:02}│{trimmed}");
        }
    }
}

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

    // `--dump` enables the headless preview (see `dump`); `--dump bg` prints
    // a background-fill map instead of the text glyphs.
    let dump_mode = args.iter().any(|a| a == "--dump");
    // `--sample` (used with the `sysmon` test) prints the HTML string the
    // refresh handler would produce, so the sampled page can be inspected
    // without an interactive terminal.
    let sample_mode = args.iter().any(|a| a == "--sample");
    // `--watch` (used with the `sysmon` test) renders several consecutive
    // sampled frames, simulating the auto-refresh headlessly so the changing
    // values can be observed without an interactive terminal.
    let watch_mode = args.iter().any(|a| a == "--watch");
    let bg_map = dump_mode
        && args
            .iter()
            .position(|a| a == "--dump")
            .is_some_and(|pos| args.get(pos + 1).is_some_and(|v| v == "bg"));
    let positional: Vec<&str> = args
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with("--") && *a != "bg")
        .map(String::as_str)
        .collect();

    let mut cols: u16 = 104;
    let mut rows: u16 = 32;
    let mut count: usize = 5;
    let mut every_ms: u64 = 800;
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--cols" => {
                if let Some(v) = it.next()
                    && let Ok(n) = v.parse()
                {
                    cols = n;
                }
            }
            "--rows" => {
                if let Some(v) = it.next()
                    && let Ok(n) = v.parse()
                {
                    rows = n;
                }
            }
            "--count" => {
                if let Some(v) = it.next()
                    && let Ok(n) = v.parse()
                {
                    count = n;
                }
            }
            "--every" => {
                if let Some(v) = it.next()
                    && let Ok(n) = v.parse()
                {
                    every_ms = n;
                }
            }
            _ => {}
        }
    }

    let test_name = positional.first().copied().unwrap_or("showcase");

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
        "pre" => test_case!(pre, no_css),
        "table" => test_case!(table, no_css),
        "box_model" => test_case!(box_model, no_css),
        "font_size" => test_case!(font_size, no_css),
        "flex" => test_case!(flex, no_css),
        "flex_align_self" => test_case!(flex_align_self, no_css),
        "flex_align_content" => test_case!(flex_align_content, no_css),
        "flex_nested" => test_case!(flex_nested, no_css),
        "css_props" => test_case!(css_props, no_css),
        "events" => test_case!(events),
        "sysmon" => test_case!(sysmon),
        "viewport_units" => test_case!(viewport_units, no_css),
        "position" => test_case!(position, no_css),
        "input" => test_case!(input, no_css),
        "configurator" => test_case!(configurator, no_css),
        "showcase" => test_case!(showcase, no_css),
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
                "pre",
                "table",
                "box_model",
                "font_size",
                "flex",
                "flex_align_self",
                "flex_align_content",
                "flex_nested",
                "css_props",
                "events",
                "sysmon",
                "viewport_units",
                "position",
                "input",
                "configurator",
                "showcase",
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

    // The `sysmon` case runs through `render_with_events` with a dedicated
    // bindings set (see `sysmon::bindings`): a background thread samples the
    // machine via `sysinfo`, and JS `refresh()` (auto every 200 ms) writes the
    // values into the DOM with `textContent`; the buttons highlight on hover
    // through CSS `:hover`. `--dump` / `--sample` / `--watch` fall back to the
    // static preview (interaction needs a real terminal).
    if test_name == "sysmon" {
        if watch_mode {
            // Headless simulation of the auto-refresh: render `count` sampled
            // frames, `every_ms` apart, so the updating values are visible.
            for i in 1..=count {
                println!("\n===== frame {i}/{count} ===");
                dump(&sysmon::preview_page(), css, cols, rows, bg_map);
                if i < count {
                    std::thread::sleep(std::time::Duration::from_millis(every_ms));
                }
            }
            return Ok(());
        }
        if sample_mode {
            // Render the page the `refresh` handler would produce, headlessly.
            dump(&sysmon::preview_page(), css, cols, rows, bg_map);
            return Ok(());
        }
        if dump_mode {
            dump(html, css, cols, rows, bg_map);
        } else {
            render_with_events(html, css, sysmon::bindings());
        }
        info!("Test '{test_name}' completed");
        return Ok(());
    }

    // The `events` case runs through `render_with_events`. The demo is fully
    // script-driven: its `<script>` block defines the handler functions and
    // registers listeners (on* attributes + `addEventListener`), and
    // `console.log` / DOM mutation demonstrate the JS-DOM bridge. No host
    // functions are needed, so bindings stay empty. `--dump` falls back to
    // the static preview (interaction needs a real terminal).
    if test_name == "events" {
        if dump_mode {
            dump(html, css, cols, rows, bg_map);
        } else {
            render_with_events(html, css, EventBindings::default());
        }
        info!("Test '{test_name}' completed");
        return Ok(());
    }

    // The `input` case runs through `render_with_events` so click-to-focus
    // and keyboard editing are active (interaction needs a real terminal).
    // `--dump` falls back to the static preview.
    if test_name == "input" {
        if dump_mode {
            dump(html, css, cols, rows, bg_map);
        } else {
            render_with_events(html, css, EventBindings::default());
        }
        info!("Test '{test_name}' completed");
        return Ok(());
    }

    // The `configurator` case runs through `render_with_events` (interactive
    // settings form: text inputs + JS-built toggle / radio controls, live
    // preview, save/reset). `--dump` falls back to the static preview.
    if test_name == "configurator" {
        if dump_mode {
            dump(html, css, cols, rows, bg_map);
        } else {
            render_with_events(html, css, EventBindings::default());
        }
        info!("Test '{test_name}' completed");
        return Ok(());
    }

    if dump_mode {
        dump(html, css, cols, rows, bg_map);
        return Ok(());
    }
    render(html, css);
    info!("Test '{test_name}' completed");
    Ok(())
}
