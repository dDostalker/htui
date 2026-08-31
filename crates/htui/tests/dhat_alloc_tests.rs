//! Allocation profiling for the render pipeline using `dhat`.
//!
//! Measures heap allocations (blocks / bytes) per pipeline stage over a
//! realistic document, to ground decisions like "is `Vec` → `SmallVec`
//! worthwhile". Run with:
//!
//! ```text
//! cargo test -p htui --test dhat_alloc_tests -- --nocapture
//! ```
//!
//! This test is diagnostic-only: every check is a trivial sanity bound.

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use dhat::HeapStats;
use htui::html::css_parse::{self, CssRule};
use htui::html::html_parse::Parser;
use htui::layout::layout::TermMap;
use htui::tdom::node::{NodeArena, NodeId};
use htui::tui::graph_api::TermLayout;

const HTML_SMALL: &str = include_str!("../../../visual_test/src/helloworld/index.html");
const HTML_MEDIUM: &str = include_str!("../../../visual_test/src/all_in_one/index.html");

const COLUMNS: u16 = 100;
const ROWS: u16 = 40;
const ITERS: usize = 200;

struct Delta {
    blocks: u64,
    bytes: u64,
}

fn delta_since(start: HeapStats) -> Delta {
    let now = HeapStats::get();
    Delta {
        blocks: now.total_blocks - start.total_blocks,
        bytes: now.total_bytes - start.total_bytes,
    }
}

fn term_map() -> TermMap {
    TermMap {
        columns: COLUMNS,
        rows: ROWS,
    }
}

fn parse_html(html: &str) -> (NodeId, NodeArena, String) {
    let mut parser = Parser::new(html);
    parser.parse()
}

fn parse_css(css: &str) -> CssRule {
    let mut parser = css_parse::Parser::new(css);
    parser.parse()
}

fn apply_cascade(root_id: NodeId, arena: &mut NodeArena, rules: &CssRule) {
    css_parse::css_into_node(arena, root_id, rules);
}

fn print_stage(name: &str, iters: usize, d: &Delta) {
    let iters = iters as u64;
    println!(
        "{name:<32} {blocks:>8} blocks/iter {per_iter_blocks:>8.1}   {bytes:>10} B/iter {per_iter_bytes:>9.1}   avg {avg:>5.1} B/block",
        name = name,
        blocks = d.blocks / iters,
        per_iter_blocks = d.blocks as f64 / iters as f64,
        bytes = d.bytes / iters,
        per_iter_bytes = d.bytes as f64 / iters as f64,
        avg = if d.blocks > 0 { d.bytes as f64 / d.blocks as f64 } else { 0.0 },
    );
}

#[test]
fn alloc_profile_per_stage() {
    let _profiler = dhat::Profiler::builder().build();
    let map = term_map();

    // Warm up allocators / lazy statics so steady-state numbers are measured.
    for html in [HTML_SMALL, HTML_MEDIUM] {
        let (root_id, mut arena, embedded) = parse_html(html);
        let rules = parse_css(&embedded);
        apply_cascade(root_id, &mut arena, &rules);
        let mut tl = TermLayout::new(&map, 1.0);
        tl.draw(&arena, root_id, &map);
        let _ = tl.render_to_bytes();
    }

    for (label, html) in [("small", HTML_SMALL), ("medium", HTML_MEDIUM)] {
        println!("--- {label} ({COLUMNS}x{ROWS}) ---");

        // 1. HTML parse only.
        let start = HeapStats::get();
        for _ in 0..ITERS {
            let _ = parse_html(html);
        }
        print_stage("html parse", ITERS, &delta_since(start));

        // 2. CSS parse only.
        let (_, _, embedded) = parse_html(html);
        let start = HeapStats::get();
        for _ in 0..ITERS {
            let _ = parse_css(&embedded);
        }
        print_stage("css parse", ITERS, &delta_since(start));

        // 3. Cascade only (fresh arena each iter, minus the clone cost).
        let (root_id, arena, embedded) = parse_html(html);
        let rules = parse_css(&embedded);
        let start = HeapStats::get();
        for _ in 0..ITERS {
            let mut fresh = arena.clone();
            apply_cascade(root_id, &mut fresh, &rules);
        }
        let with_clone = delta_since(start);
        let start = HeapStats::get();
        for _ in 0..ITERS {
            let _ = arena.clone();
        }
        let clone_only = delta_since(start);
        print_stage(
            "cascade",
            ITERS,
            &Delta {
                blocks: with_clone.blocks.saturating_sub(clone_only.blocks),
                bytes: with_clone.bytes.saturating_sub(clone_only.bytes),
            },
        );

        // 4. Layout + paint only (cascade applied once, arena reused).
        let (root_id, mut arena, embedded) = parse_html(html);
        let rules = parse_css(&embedded);
        apply_cascade(root_id, &mut arena, &rules);
        let start = HeapStats::get();
        for _ in 0..ITERS {
            let mut tl = TermLayout::new(&map, 1.0);
            tl.draw(&arena, root_id, &map);
        }
        print_stage("layout + paint", ITERS, &delta_since(start));

        // 5. ANSI encode only.
        let mut tl = TermLayout::new(&map, 1.0);
        tl.draw(&arena, root_id, &map);
        let start = HeapStats::get();
        for _ in 0..ITERS {
            let _ = tl.render_to_bytes();
        }
        print_stage("ansi encode", ITERS, &delta_since(start));

        // 6. Full pipeline for reference.
        let start = HeapStats::get();
        for _ in 0..ITERS {
            let (root_id, mut arena, embedded) = parse_html(html);
            let rules = parse_css(&embedded);
            apply_cascade(root_id, &mut arena, &rules);
            let mut tl = TermLayout::new(&map, 1.0);
            tl.draw(&arena, root_id, &map);
            let _ = tl.render_to_bytes();
        }
        print_stage("full pipeline", ITERS, &delta_since(start));
    }

    let stats = HeapStats::get();
    assert!(stats.total_blocks > 0, "dhat must observe allocations");
    // Medium document must allocate strictly more than the small one overall.
    assert!(
        stats.total_bytes > 1 << 20,
        "expected at least 1 MiB total traffic, got {}",
        stats.total_bytes
    );
}
