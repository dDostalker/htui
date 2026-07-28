//! Shared fixtures for the bench suite.
//!
//! Each helper returns a pre-parsed `(root_id, arena)` plus a `CssRule` so
//! individual bench files can target a single pipeline stage without paying
//! the cost of upstream stages during measurement.
//!
//! Resources are pulled from the `visual_test` crate so the bench numbers
//! reflect a realistic document (mixed headings, paragraphs, styles, lists
//! and images) rather than synthetic noise.
//!
//! Each bench target is compiled as its own crate, so individual helpers
//! may appear "unused" from a single target's perspective — disable the
//! warning at the module level here.
#![allow(dead_code)]

use htui::html::css_parse::{self, CssRule};
use htui::html::html_parse::Parser;
use htui::layout::layout::TermMap;
use htui::tdom::node::{NodeArena, NodeId};

// ----------------------------------------------------------
// HTML / CSS fixtures
// ----------------------------------------------------------

/// Small HTML fixture (~22 lines, single heading + paragraph).
pub const HTML_SMALL: &str = include_str!("../../visual_test/src/helloworld/index.html");

/// Medium HTML fixture (~132 lines, covers headings, paragraphs, lists,
/// table, box model, colors — the canonical `all_in_one` stress test).
pub const HTML_MEDIUM: &str = include_str!("../../visual_test/src/all_in_one/index.html");

// ----------------------------------------------------------
// Parsing helpers
// ----------------------------------------------------------

/// Parse HTML using the shared [`Parser`].
pub fn parse_html(html: &str) -> (NodeId, NodeArena, String) {
    let mut parser = Parser::new(html);
    parser.parse()
}

/// Parse CSS using the shared [`css_parse::Parser`].
pub fn parse_css(css: &str) -> CssRule {
    let mut parser = css_parse::Parser::new(css);
    parser.parse()
}

/// Build a plain arena (HTML parsed, CSS cascade **not** applied) plus the
/// parsed `CssRule` ready for [`css_into_node`](css_parse::css_into_node).
///
/// Cascade benches clone the arena each iteration so the `css_into_node`
/// call measures the cascade itself, not the upstream parse.
pub fn build_plain_arena(html: &str) -> (NodeId, NodeArena, CssRule) {
    let (root_id, arena, embedded_css) = parse_html(html);
    let rules = if embedded_css.trim().is_empty() {
        CssRule::new()
    } else {
        parse_css(&embedded_css)
    };
    (root_id, arena, rules)
}

/// Build a fully resolved fixture: HTML parsed **and** CSS cascade applied.
///
/// Use this when a bench only cares about stages downstream of the cascade
/// (layout, paint, ANSI encode).
pub fn build_fixture(html: &str, extra_css: &str) -> (NodeId, NodeArena) {
    let (root_id, mut arena, embedded_css) = parse_html(html);
    let combined = if extra_css.is_empty() {
        embedded_css
    } else {
        format!("{embedded_css}\n{extra_css}")
    };
    let rules = if combined.trim().is_empty() {
        CssRule::new()
    } else {
        parse_css(&combined)
    };
    apply_cascade(root_id, &mut arena, &rules);
    (root_id, arena)
}

/// Apply the CSS cascade to a fresh arena.
///
/// Convenience wrapper for full-pipeline benches that want to skip the
/// parse-css step in the hot loop while still measuring the cascade.
pub fn apply_cascade(root_id: NodeId, arena: &mut NodeArena, rules: &CssRule) {
    css_parse::css_into_node(arena, root_id, rules);
}

/// Build a 100×40 [`TermMap`] for paint benches that don't need a real
/// terminal.
pub fn bench_term_map() -> TermMap {
    TermMap {
        columns: 100,
        rows: 40,
    }
}
