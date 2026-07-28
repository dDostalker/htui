use crossterm::style::Color;

use crate::{
    layout::InlineRun,
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
    let mut runs = Vec::new();
    walk_runs(arena, node, fg, bg, style, &mut runs);
    runs
}

fn walk_runs(
    arena: &NodeArena,
    node: &Node,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
    runs: &mut Vec<InlineRun>,
) {
    for child_id in node.child_ids() {
        let child = &arena[child_id];
        match &child.node_type {
            NodeType::Text(s) => {
                if !s.is_empty() {
                    runs.push(InlineRun::new(s.clone(), style, fg, bg));
                }
            }
            NodeType::Span => {
                // Span inherits parent's fg/bg unless explicit overrides exist.
                let span_fg = child.css_attr.get_color().or(fg);
                let span_bg = child.css_attr.get_bg_color().or(bg);
                let span_style = style | block_style(&child.css_attr);
                walk_runs(arena, child, span_fg, span_bg, span_style, runs);
            }
            NodeType::A => {
                // User-agent defaults: unstyled links are blue and
                // underlined, matching browser behaviour. Both defaults
                // yield to anything the author (or an ancestor CSS rule)
                // has already declared for the matching property, so
                // `a { text-decoration: none }` or `a { color: green }`
                // take effect normally.
                let a_fg = if child.css_attr.is_set("color") {
                    child.css_attr.get_color().or(fg)
                } else {
                    Some(Color::Blue)
                };
                let a_bg = child.css_attr.get_bg_color().or(bg);
                let mut a_style = style | block_style(&child.css_attr);
                if !child.css_attr.is_set("text-decoration") {
                    a_style |= FontStyle::UNDERLINED;
                }
                walk_runs(arena, child, a_fg, a_bg, a_style, runs);
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
                walk_runs(arena, child, em_fg, em_bg, em_style, runs);
            }
            // Block-level nested elements live in their own LayoutBox;
            // unknown inline tags are flattened by the HTML parser so they
            // never reach here as an unknown NodeType.
            _ => {}
        }
    }
}
