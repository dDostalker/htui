//! Benchmarks covering HTML and CSS parsing.
//!
//! These two stages are exercised on every `Redraw` today (see
//! `lib.rs::render_thread`), so any unnecessary work here is paid by every
//! viewport resize and zoom change.

#[path = "common/mod.rs"]
mod common;

use criterion::{criterion_group, criterion_main, Criterion};

fn bench_html_parse_small(c: &mut Criterion) {
    let html = common::HTML_SMALL;
    c.bench_function("html_parse_small", |b| {
        b.iter(|| common::parse_html(html));
    });
}

fn bench_html_parse_medium(c: &mut Criterion) {
    let html = common::HTML_MEDIUM;
    c.bench_function("html_parse_medium", |b| {
        b.iter(|| common::parse_html(html));
    });
}

fn bench_css_parse_medium(c: &mut Criterion) {
    // CSS is embedded in the medium HTML — extract it once and re-parse
    // each iteration. This mirrors the production render path where the
    // embedded CSS is concatenated with any user-supplied stylesheet.
    let css = extract_embedded_css(common::HTML_MEDIUM);
    c.bench_function("css_parse_medium", |b| {
        b.iter(|| common::parse_css(&css));
    });
}

/// Run the HTML parser once and pull out the embedded `<style>` contents.
/// Only the embedded CSS string is interesting for the CSS parse bench —
/// the HTML structure is irrelevant.
fn extract_embedded_css(html: &str) -> String {
    let (_, _, css) = common::parse_html(html);
    css
}

criterion_group!(
    parse_benches,
    bench_html_parse_small,
    bench_html_parse_medium,
    bench_css_parse_medium,
);
criterion_main!(parse_benches);
