//! Inline-text collection shared by the block flow and the flex algorithm.
//!
//! [`collect_runs`] walks a node's direct children and produces a flat list of
//! [`InlineRun`]s — the painted representation of inline text within a block.
//! Both the block layout ([`super::inner_formatting_context::flow`]) and the
//! flex layout ([`super::inner_formatting_context::flex`]) call into this
//! routine, so it lives in `layoutbox` rather than inside either formatting
//! context's implementation.

use crossterm::style::Color;

use super::InlineRun;
use crate::{
    html::css_parse::TextTransform,
    tdom::node::{Node, NodeArena, NodeType},
    tui::graph_api::{FontStyle, block_style},
};

/// Collect inline runs from a block's inline tree, preserving `<span>` style
/// boundaries.
///
/// Walks `node`'s direct children in document order. Each `Text` leaf becomes
/// one run that inherits the currently-active style. `<span>`, `<a>` and
/// `<em>` boundaries push a new style onto the stack: the element's own
/// `font-weight` / `font-style` / `text-decoration` flags are OR-ed into the
/// inherited `FontStyle`, and its own `color` / `background-color` override
/// the inherited `fg` / `bg` when present. `<a>` additionally applies
/// browser-style user-agent defaults (blue foreground, underline) whenever
/// the matching property has not been declared on the link itself or on any
/// ancestor; `<em>` applies an italic default for the same reason — both
/// match CSS cascade precedence for UA defaults. Block-level children are
/// skipped — they are dispatched as their own `LayoutBox` by the layout
/// registry, so folding them into the parent's runs would either
/// double-render them or wrap them in an unintended inline context.
pub fn collect_runs(
    arena: &NodeArena,
    node: &Node,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
) -> Vec<InlineRun> {
    let transform = node.css_attr.get_text_transform();
    let mut runs = Vec::new();
    walk_runs(arena, node, fg, bg, style, transform, &mut runs);
    runs
}

fn walk_runs(
    arena: &NodeArena,
    node: &Node,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
    transform: TextTransform,
    runs: &mut Vec<InlineRun>,
) {
    for child_id in node.child_ids() {
        let child = &arena[child_id];
        match &child.node_type {
            NodeType::Text(s) => {
                if !s.is_empty() {
                    let text = apply_text_transform(s, transform);
                    runs.push(InlineRun::new(text, style, fg, bg));
                }
            }
            NodeType::Span => {
                // Span inherits parent's fg/bg unless explicit overrides exist.
                let span_fg = child.css_attr.get_color().or(fg);
                let span_bg = child.css_attr.get_bg_color().or(bg);
                let span_style = style | block_style(&child.css_attr);
                let span_transform = child.css_attr.get_text_transform();
                walk_runs(arena, child, span_fg, span_bg, span_style, span_transform, runs);
            }
            NodeType::A => {
                // A block-level anchor is painted by its own `LayoutBox`
                // and must not have its text folded into the parent's
                // inline runs. A direct anchor child of a flex
                // container is likewise a flex item. Either way, the
                // anchor does not belong here.
                if child
                    .css_attr
                    .display()
                    .is_some_and(|d| d.is_block_level())
                    || node.css_attr.display().is_some_and(|d| d.is_flex())
                {
                    continue;
                }

                // User-agent defaults: unstyled links are blue and
                // underlined, matching browser behaviour. Both defaults
                // yield to anything the author (or an ancestor CSS rule)
                // has already declared for the matching property, so
                // `a { text-decoration: none }` or `a { color: green }`
                // take effect normally.
                let a_fg = if child.css_attr.is_set("color") {
                    child.css_attr.get_color().or(fg)
                } else {
                    // Standard blue (#0000FF) — must match the block-<a>
                    // UA default, which inserts the CSS name `blue` into
                    // `css_attr` and resolves it through `parse_color_str`.
                    Some(Color::Rgb {
                        r: 0x00,
                        g: 0x00,
                        b: 0xFF,
                    })
                };
                let a_bg = child.css_attr.get_bg_color().or(bg);
                let mut a_style = style | block_style(&child.css_attr);
                if !child.css_attr.is_set("text-decoration") {
                    a_style |= FontStyle::UNDERLINED;
                }
                let a_transform = child.css_attr.get_text_transform();
                walk_runs(arena, child, a_fg, a_bg, a_style, a_transform, runs);
            }
            NodeType::Em => {
                // User-agent default: `<em>` renders italic. Anything the
                // author (or an ancestor rule) declared for `font-style`
                // takes precedence — `em { font-style: normal }` or
                // `em { font-style: oblique }` are honoured.
                let em_fg = child.css_attr.get_color().or(fg);
                let em_bg = child.css_attr.get_bg_color().or(bg);
                let mut em_style = style | block_style(&child.css_attr);
                if !child.css_attr.is_set("font-style") {
                    em_style |= FontStyle::ITALIC;
                }
                let em_transform = child.css_attr.get_text_transform();
                walk_runs(arena, child, em_fg, em_bg, em_style, em_transform, runs);
            }
            // Block-level nested elements live in their own LayoutBox;
            // unknown inline tags are flattened by the HTML parser so they
            // never reach here as an unknown NodeType.
            _ => {}
        }
    }
}

/// Apply a [`TextTransform`] to a text node's content.
///
/// `capitalize` uppercases the first character of each whitespace / punct
/// delimited word; `to_uppercase` / `to_lowercase` delegate to the Unicode
/// case mappings (a single char may expand, e.g. `ß` → `SS`).
fn apply_text_transform(text: &str, transform: TextTransform) -> String {
    match transform {
        TextTransform::None => text.to_string(),
        TextTransform::Uppercase => text.to_uppercase(),
        TextTransform::Lowercase => text.to_lowercase(),
        TextTransform::Capitalize => {
            let mut out = String::with_capacity(text.len());
            let mut new_word = true;
            for ch in text.chars() {
                if ch.is_alphanumeric() {
                    if new_word {
                        out.extend(ch.to_uppercase());
                    } else {
                        out.push(ch);
                    }
                    new_word = false;
                } else {
                    out.push(ch);
                    new_word = true;
                }
            }
            out
        }
    }
}
