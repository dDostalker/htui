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
use super::generation::visible_children;
use crate::{
    html::css_parse::{TextTransform, WhiteSpace},
    tdom::node::{Node, NodeArena, NodeId, NodeType},
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
    node_id: NodeId,
    node: &Node,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
) -> Vec<InlineRun> {
    let transform = node.css_attr.get_text_transform();
    let white_space = node.css_attr.get_white_space(&node.node_type);
    let mut runs = Vec::new();
    walk_runs(arena, node_id, node, fg, bg, style, transform, white_space, None, &mut runs);
    runs
}

/// Collect a single raw text node's inline run (whitespace-collapsed and
/// text-transformed), or `None` for empty / whitespace-only text.
///
/// Used by the flex container to promote its direct text children into
/// anonymous flex items, mirroring the `NodeType::Text` branch of
/// [`walk_runs`] without walking an element's children.
pub(crate) fn text_node_run(
    arena: &NodeArena,
    text_node_id: NodeId,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
    transform: TextTransform,
    white_space: WhiteSpace,
) -> Option<InlineRun> {
    let child = &arena[text_node_id];
    if child.node_type != NodeType::Text {
        return None;
    }
    if child.text_is_painted(white_space) {
        if let Some(s) = child.data.as_deref()
            && !s.is_empty()
        {
            let text = apply_text_transform(s, transform);
            return Some(InlineRun::owned(text, style, fg, bg, None));
        }
    }
    None
}

fn walk_runs(
    arena: &NodeArena,
    node_id: NodeId,
    node: &Node,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
    transform: TextTransform,
    white_space: WhiteSpace,
    owner: Option<NodeId>,
    runs: &mut Vec<InlineRun>,
) {
    // A closed `<details>` shows only its first `<summary>` child, so its own
    // inline content (text / spans directly inside the element) stays hidden
    // too — the same filter the box generation and block layout apply.
    for child_id in visible_children(arena, node_id) {
        let child = &arena[child_id];
        // In a flex container every element child is a flex item (inline
        // elements are blockified), so only raw text folds into the
        // container's own inline runs. Block children are already skipped by
        // the `match` fallthrough below; this additionally keeps inline
        // children (`<span>` / `<em>` / …) out of the container's text so
        // they are not rendered twice (once above the flex items and again
        // as items).
        if node.css_attr.display().is_some_and(|d| d.is_flex())
            && !matches!(child.node_type, NodeType::Text)
        {
            continue;
        }
        match &child.node_type {
            NodeType::Text => {
                // Whitespace-only text nodes are kept in the DOM but only
                // become runs when the context preserves whitespace (see
                // `Node::text_is_painted`).
                if child.text_is_painted(white_space) {
                    if let Some(s) = child.data.as_deref()
                        && !s.is_empty()
                    {
                        let text = apply_text_transform(s, transform);
                        runs.push(InlineRun::owned(text, style, fg, bg, owner));
                    }
                }
            }
            NodeType::Span => {
                // Span inherits parent's fg/bg unless explicit overrides exist.
                let span_fg = child.css_attr.get_color().or(fg);
                let span_bg = child.css_attr.get_bg_color().or(bg);
                let span_style = style | block_style(&child.css_attr);
                let span_transform = child.css_attr.get_text_transform();
                let span_ws = child.white_space_or(white_space);
                walk_runs(arena, child_id, child, span_fg, span_bg, span_style, span_transform, span_ws, owner, runs);
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
                let a_ws = child.white_space_or(white_space);
                walk_runs(arena, child_id, child, a_fg, a_bg, a_style, a_transform, a_ws, owner, runs);
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
                let em_ws = child.white_space_or(white_space);
                walk_runs(arena, child_id, child, em_fg, em_bg, em_style, em_transform, em_ws, owner, runs);
            }
            NodeType::Strong | NodeType::B => {
                // User-agent default: bold. Anything the author (or an
                // ancestor rule) declared for `font-weight` takes precedence.
                let mut st = style | block_style(&child.css_attr);
                if !child.css_attr.is_set("font-weight") {
                    st |= FontStyle::BOLD;
                }
                walk_inline_children(arena, child_id, child, fg, bg, st, white_space, owner, runs);
            }
            NodeType::I | NodeType::Cite | NodeType::Dfn | NodeType::Var => {
                // User-agent default: italic (same precedence rule as `<em>`).
                let mut st = style | block_style(&child.css_attr);
                if !child.css_attr.is_set("font-style") {
                    st |= FontStyle::ITALIC;
                }
                walk_inline_children(arena, child_id, child, fg, bg, st, white_space, owner, runs);
            }
            NodeType::Label => {
                // `<label>` is a plain inline container (like `<span>`), but
                // its runs carry the label's node id so the hit map can give
                // the label text its own region — the click-delegation target
                // for `<label for>` / wrapped controls.
                let label_fg = child.css_attr.get_color().or(fg);
                let label_bg = child.css_attr.get_bg_color().or(bg);
                walk_inline_children(
                    arena,
                    child_id,
                    child,
                    label_fg,
                    label_bg,
                    style | block_style(&child.css_attr),
                    white_space,
                    Some(child_id),
                    runs,
                );
            }
            NodeType::U | NodeType::Ins => {
                // User-agent default: underline. An explicit
                // `text-decoration` (author or inherited) wins.
                let mut st = style | block_style(&child.css_attr);
                if !child.css_attr.is_set("text-decoration") {
                    st |= FontStyle::UNDERLINED;
                }
                walk_inline_children(arena, child_id, child, fg, bg, st, white_space, owner, runs);
            }
            NodeType::S | NodeType::Del => {
                // User-agent default: line-through. An explicit
                // `text-decoration` (author or inherited) wins.
                let mut st = style | block_style(&child.css_attr);
                if !child.css_attr.is_set("text-decoration") {
                    st |= FontStyle::CROSSED_OUT;
                }
                walk_inline_children(arena, child_id, child, fg, bg, st, white_space, owner, runs);
            }
            NodeType::Mark => {
                // User-agent default: yellow background + black foreground
                // (the highlight). Explicit declarations win.
                let child_fg = child.css_attr.get_color().or(fg);
                let child_bg = child.css_attr.get_bg_color().or(bg);
                let mark_fg = if child.css_attr.is_set("color") {
                    child_fg
                } else {
                    Some(Color::Black)
                };
                let mark_bg = if child.css_attr.is_set("background-color") {
                    child_bg
                } else {
                    Some(Color::Yellow)
                };
                let st = style | block_style(&child.css_attr);
                walk_inline_children(arena, child_id, child, mark_fg, mark_bg, st, white_space, owner, runs);
            }
            NodeType::Q => {
                // User-agent default: `<q>` wraps its content in generated
                // quote marks (`q::before { content: open-quote }` /
                // `::after { content: close-quote }`). The pair alternates
                // with nesting depth like the CSS `quotes` property: even
                // depth uses double quotes, odd depth single quotes.
                let q_fg = child.css_attr.get_color().or(fg);
                let q_bg = child.css_attr.get_bg_color().or(bg);
                let q_style = style | block_style(&child.css_attr);
                let (open, close) = q_quote_pair(arena, child_id);
                runs.push(InlineRun::owned(open.to_string(), q_style, q_fg, q_bg, owner));
                let q_transform = child.css_attr.get_text_transform();
                let q_ws = child.white_space_or(white_space);
                walk_runs(arena, child_id, child, q_fg, q_bg, q_style, q_transform, q_ws, owner, runs);
                runs.push(InlineRun::owned(close.to_string(), q_style, q_fg, q_bg, owner));
            }
            NodeType::Abbr => {
                // User-agent default: `abbr[title]` carries a dotted underline
                // as a tooltip affordance. The terminal has no dotted
                // underline primitive, so the approximation is a solid
                // underline; an explicit `text-decoration` (author or
                // inherited) wins.
                let fg = child.css_attr.get_color().or(fg);
                let bg = child.css_attr.get_bg_color().or(bg);
                let mut st = style | block_style(&child.css_attr);
                if child.attributes.get("title").is_some()
                    && !child.css_attr.is_set("text-decoration")
                {
                    st |= FontStyle::UNDERLINED;
                }
                walk_inline_children(arena, child_id, child, fg, bg, st, white_space, owner, runs);
            }
            NodeType::Code | NodeType::Kbd | NodeType::Samp => {
                // User-agent default `font-family: monospace` is already the
                // terminal's native font, so no style bit applies — the
                // element is kept for CSS / DOM addressing.
                let fg = child.css_attr.get_color().or(fg);
                let bg = child.css_attr.get_bg_color().or(bg);
                walk_inline_children(arena, child_id, child, fg, bg, style | block_style(&child.css_attr), white_space, owner, runs);
            }
            NodeType::Data | NodeType::Time | NodeType::Output => {
                // No UA visual default: plain inline containers like `<span>`.
                // The `value` / `datetime` attributes are machine-readable
                // only and carry no visual effect, matching browsers.
                let fg = child.css_attr.get_color().or(fg);
                let bg = child.css_attr.get_bg_color().or(bg);
                walk_inline_children(arena, child_id, child, fg, bg, style | block_style(&child.css_attr), white_space, owner, runs);
            }
            NodeType::Br => {
                // `<br>` forces a line break in every `white-space` mode.
                runs.push(InlineRun::forced_break(style, fg, bg));
            }
            // Block-level nested elements live in their own LayoutBox;
            // unknown inline tags are flattened by the HTML parser so they
            // never reach here as an unknown NodeType.
            _ => {}
        }
    }
}

/// Recurse into an inline element's children, carrying the resolved `fg` /
/// `bg` / `style` (already merged with the element's own CSS), the element's
/// own `text-transform` / `white-space` overrides, and the owning inline
/// element (`owner`) for hit-map attribution.
fn walk_inline_children(
    arena: &NodeArena,
    child_id: NodeId,
    child: &Node,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
    parent_ws: WhiteSpace,
    owner: Option<NodeId>,
    runs: &mut Vec<InlineRun>,
) {
    let transform = child.css_attr.get_text_transform();
    let ws = child.white_space_or(parent_ws);
    walk_runs(arena, child_id, child, fg, bg, style, transform, ws, owner, runs);
}

/// Quote pair for a `<q>` element based on its nesting depth, mirroring the
/// CSS `quotes` alternation: even depth uses double quotes (`“`/`”`), odd
/// depth single quotes (`‘`/`’`).
pub(crate) fn q_quote_pair(arena: &NodeArena, node_id: NodeId) -> (&'static str, &'static str) {
    let mut depth = 0usize;
    let mut current = arena[node_id].parent;
    while let Some(pid) = current {
        if arena[pid].node_type == NodeType::Q {
            depth = depth.saturating_add(1);
        }
        current = arena[pid].parent;
    }
    if depth % 2 == 0 {
        ("\u{201C}", "\u{201D}")
    } else {
        ("\u{2018}", "\u{2019}")
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
