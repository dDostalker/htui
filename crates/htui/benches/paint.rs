//! Benchmarks covering the layout + paint pipeline (`TermLayout::draw`).
//!
//! `draw` is the heaviest individual stage on the hot path — it walks the
//! DOM through the layout registry and writes every styled cell into the
//! canvas. Two variants are measured:
//!
//! * `paint_alloc` — allocates a fresh `TermLayout` each iteration (worst
//!   case: what the render thread does on every `Redraw`).
//! * `paint_reuse` — reuses a single `TermLayout` (best case: what the
//!   planned HTML cache will deliver once 1.2 lands).

#[path = "common/mod.rs"]
mod common;

use criterion::{Criterion, criterion_group, criterion_main};
use htui::tui::graph_api::TermLayout;

/// Allocate a fresh `TermLayout` per iteration. Captures the cost of
/// `reset_canvas` and the canvas initialisation (`vec![Block::default(); n]`
/// allocates one `String` per cell here).
fn bench_paint_alloc_small(c: &mut Criterion) {
    let (root_id, arena) = common::build_fixture(common::HTML_SMALL, "");
    let map = common::bench_term_map();

    c.bench_function("paint_alloc_small", |b| {
        b.iter(|| {
            let mut tl = TermLayout::new(&map, 1.0);
            tl.draw(&arena, root_id, &map);
        });
    });
}

fn bench_paint_alloc_medium(c: &mut Criterion) {
    let (root_id, arena) = common::build_fixture(common::HTML_MEDIUM, "");
    let map = common::bench_term_map();

    c.bench_function("paint_alloc_medium", |b| {
        b.iter(|| {
            let mut tl = TermLayout::new(&map, 1.0);
            tl.draw(&arena, root_id, &map);
        });
    });
}

/// Reuse a single `TermLayout` across iterations. Isolate the cost of
/// `reset_canvas` + the actual paint walk from the initial allocation.
fn bench_paint_reuse_medium(c: &mut Criterion) {
    let (root_id, arena) = common::build_fixture(common::HTML_MEDIUM, "");
    let map = common::bench_term_map();
    let mut tl = TermLayout::new(&map, 1.0);

    c.bench_function("paint_reuse_medium", |b| {
        b.iter(|| {
            tl.draw(&arena, root_id, &map);
        });
    });
}

criterion_group!(
    paint_benches,
    bench_paint_alloc_small,
    bench_paint_alloc_medium,
    bench_paint_reuse_medium,
);
criterion_main!(paint_benches);
