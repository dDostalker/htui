//! End-to-end pipeline benchmarks: parse + cascade + draw + ANSI encode.
//!
//! Mirrors the work the render thread does on every `RenderCmd::Redraw`
//! today. Useful as a top-line number a single number for regression
//! tracking; use the per-stage benches (`parse`, `cascade`, `paint`,
//! `render_to_bytes`) to attribute slowdowns to a specific stage.

#[path = "common/mod.rs"]
mod common;

use criterion::{Criterion, criterion_group, criterion_main};
use htui::tui::graph_api::TermLayout;

fn bench_full_redraw_small(c: &mut Criterion) {
    let html = common::HTML_SMALL;
    let map = common::bench_term_map();

    c.bench_function("full_redraw_small", |b| {
        b.iter(|| {
            // 1. Parse HTML.
            let (root_id, mut arena, embedded) = common::parse_html(html);
            // 2. Parse embedded CSS.
            let rules = common::parse_css(&embedded);
            // 3. Apply cascade.
            common::apply_cascade(root_id, &mut arena, &rules);
            // 4. Layout + paint.
            let mut tl = TermLayout::new(&map, 1.0);
            tl.draw(&arena, root_id, &map);
            // 5. ANSI encode.
            let _ = tl.render_to_bytes();
        });
    });
}

fn bench_full_redraw_medium(c: &mut Criterion) {
    let html = common::HTML_MEDIUM;
    let map = common::bench_term_map();

    c.bench_function("full_redraw_medium", |b| {
        b.iter(|| {
            let (root_id, mut arena, embedded) = common::parse_html(html);
            let rules = common::parse_css(&embedded);
            common::apply_cascade(root_id, &mut arena, &rules);
            let mut tl = TermLayout::new(&map, 1.0);
            tl.draw(&arena, root_id, &map);
            let _ = tl.render_to_bytes();
        });
    });
}

criterion_group!(
    full_redraw_benches,
    bench_full_redraw_small,
    bench_full_redraw_medium,
);
criterion_main!(full_redraw_benches);
