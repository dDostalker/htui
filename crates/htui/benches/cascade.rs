//! Benchmarks covering the CSS cascade (selector matching + inheritance).
//!
//! `apply_css_rule` walks the entire DOM for every rule, so the worst-case
//! cost is `O(rules × nodes)`. These benches pin that down before/after
//! any future index-based filtering.

#[path = "common/mod.rs"]
mod common;

use criterion::{Criterion, criterion_group, criterion_main};

/// Full pipeline for the small fixture: HTML parse + CSS parse + cascade.
/// Useful as a sanity check that the cascade dominates the parse cost.
fn bench_cascade_full_small(c: &mut Criterion) {
    let html = common::HTML_SMALL;
    c.bench_function("cascade_full_small", |b| {
        b.iter(|| {
            let (root_id, mut arena, embedded) = common::parse_html(html);
            let rules = common::parse_css(&embedded);
            common::apply_cascade(root_id, &mut arena, &rules);
        });
    });
}

/// Full pipeline for the medium fixture.
fn bench_cascade_full_medium(c: &mut Criterion) {
    let html = common::HTML_MEDIUM;
    c.bench_function("cascade_full_medium", |b| {
        b.iter(|| {
            let (root_id, mut arena, embedded) = common::parse_html(html);
            let rules = common::parse_css(&embedded);
            common::apply_cascade(root_id, &mut arena, &rules);
        });
    });
}

/// Cascade application only on the medium fixture. The arena is cloned
/// each iteration so the cascade starts from a clean slate; the measured
/// cost is the selector walk + inheritance propagation, not the parse.
fn bench_cascade_apply_only_medium(c: &mut Criterion) {
    let (root_id, arena, rules) = common::build_plain_arena(common::HTML_MEDIUM);
    c.bench_function("cascade_apply_only_medium", |b| {
        b.iter(|| {
            let mut arena = arena.clone();
            common::apply_cascade(root_id, &mut arena, &rules);
        });
    });
}

criterion_group!(
    cascade_benches,
    bench_cascade_full_small,
    bench_cascade_full_medium,
    bench_cascade_apply_only_medium,
);
criterion_main!(cascade_benches);
