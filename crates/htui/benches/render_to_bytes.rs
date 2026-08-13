//! Benchmarks covering the ANSI escape-sequence emission stage.
//!
//! `render_to_bytes` walks the visible viewport, batches runs of cells
//! with identical styling, and writes a single SGR sequence per batch.
//! The bench reuses one fully-painted `TermLayout` so the measured cost
//! reflects only the encode step.

#[path = "common/mod.rs"]
mod common;

use criterion::{criterion_group, criterion_main, Criterion};
use htui::tui::graph_api::TermLayout;

fn bench_render_to_bytes_small(c: &mut Criterion) {
    let (root_id, arena) = common::build_fixture(common::HTML_SMALL, "");
    let map = common::bench_term_map();
    let mut tl = TermLayout::new(&map, 1.0);
    tl.draw(&arena, root_id, &map);

    c.bench_function("render_to_bytes_small", |b| {
        b.iter(|| tl.render_to_bytes());
    });
}

fn bench_render_to_bytes_medium(c: &mut Criterion) {
    let (root_id, arena) = common::build_fixture(common::HTML_MEDIUM, "");
    let map = common::bench_term_map();
    let mut tl = TermLayout::new(&map, 1.0);
    tl.draw(&arena, root_id, &map);

    c.bench_function("render_to_bytes_medium", |b| {
        b.iter(|| tl.render_to_bytes());
    });
}

/// Pin the encode cost with a deep scroll so the batch loop hits many
/// rows that are still inside the canvas buffer. This is the cost the
/// user sees every time they press `j` / `k` once the document is taller
/// than the viewport.
fn bench_render_to_bytes_medium_scrolled(c: &mut Criterion) {
    let (root_id, arena) = common::build_fixture(common::HTML_MEDIUM, "");
    let map = common::bench_term_map();
    let mut tl = TermLayout::new(&map, 1.0);
    tl.draw(&arena, root_id, &map);
    tl.scroll_y = tl.rendered_rows / 2;

    c.bench_function("render_to_bytes_medium_scrolled", |b| {
        b.iter(|| tl.render_to_bytes());
    });
}

criterion_group!(
    render_to_bytes_benches,
    bench_render_to_bytes_small,
    bench_render_to_bytes_medium,
    bench_render_to_bytes_medium_scrolled,
);
criterion_main!(render_to_bytes_benches);
