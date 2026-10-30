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
    graphics::{FontStyle, block_style},
    html::css_parse::{PseudoElement, TextTransform, WhiteSpace, parse_color_str},
    tdom::node::{Node, NodeArena, NodeId, NodeType},
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
    spacing: (usize, isize),
) -> Vec<InlineRun> {
    let transform = node.css_attr.get_text_transform();
    let white_space = node.css_attr.get_white_space(&node.node_type);
    let mut runs = Vec::new();
    // Generated content: a `::before` run precedes the element's children,
    // an `::after` run follows them (CSS `content`). The pseudo-element's
    // own declarations (color / styles / spacing) cascade into
    // key-prefixed slots (`color::before`); `content: none` / `normal`
    // produce nothing.
    if let Some(run) = pseudo_element_run(arena, node_id, PseudoElement::Before, fg, bg, style) {
        runs.push(run);
    }
    walk_runs(
        arena,
        node_id,
        node,
        fg,
        bg,
        style,
        transform,
        white_space,
        None,
        spacing,
        &mut runs,
    );
    if let Some(run) = pseudo_element_run(arena, node_id, PseudoElement::After, fg, bg, style) {
        runs.push(run);
    }
    runs
}

/// Build the generated inline run of one pseudo-element, or `None` when the
/// element has no matching `content` declaration (or it is `none` /
/// `normal` / unquoteable).
fn pseudo_element_run(
    arena: &NodeArena,
    node_id: NodeId,
    pe: PseudoElement,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
) -> Option<InlineRun> {
    let (suffix, content_key) = match pe {
        PseudoElement::Before => ("::before", "content::before"),
        PseudoElement::After => ("::after", "content::after"),
        // Slider pseudo-elements are styling-only (no generated content run).
        PseudoElement::SliderThumb | PseudoElement::SliderTrack | PseudoElement::SliderProgress => {
            return None;
        }
    };
    let node = &arena[node_id];
    let raw = node.css_attr.0.get(content_key)?;
    let raw = raw.as_ref();
    if raw.eq_ignore_ascii_case("none") || raw.eq_ignore_ascii_case("normal") {
        return None;
    }
    // `content` accepts a double- or single-quoted string; anything else
    // (attr() / counters / url()) is unsupported and generates nothing.
    let bytes = raw.trim();
    let text = if bytes.len() >= 2
        && ((bytes.starts_with('"') && bytes.ends_with('"'))
            || (bytes.starts_with('\'') && bytes.ends_with('\'')))
    {
        // Unescape \" / \\ minimally.
        let inner = &bytes[1..bytes.len() - 1];
        Some(inner.replace("\\\"", "\"").replace("\\\\", "\\"))
    } else {
        None
    }?;
    if text.is_empty() {
        return None;
    }
    let prefixed = |prop: &str| format!("{prop}{suffix}");
    // The pseudo-element's own colour / style / decoration override the
    // inherited look (declared on the origin element under the prefixed key).
    let pfg = node
        .css_attr
        .0
        .get(prefixed("color").as_str())
        .and_then(|v| parse_color_str(v))
        .or(fg);
    let pbg = node
        .css_attr
        .0
        .get(prefixed("background-color").as_str())
        .and_then(|v| parse_color_str(v))
        .or(bg);
    let mut pstyle = style;
    if node
        .css_attr
        .0
        .get(prefixed("font-weight").as_str())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("bold"))
    {
        pstyle |= FontStyle::BOLD;
    }
    if node
        .css_attr
        .0
        .get(prefixed("font-style").as_str())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("italic"))
    {
        pstyle |= FontStyle::ITALIC;
    }
    if node
        .css_attr
        .0
        .get(prefixed("text-decoration").as_str())
        .is_some_and(|v| v.contains("underline"))
    {
        pstyle |= FontStyle::UNDERLINED;
    }
    // Pseudo-element spacing uses the origin's declared value with neutral
    // contexts (px / ch terms resolve identically; only em/vw/vh would
    // need the layout context, and those degrade to the default grid).
    let spacing = (0usize, 0isize);
    Some(InlineRun::owned_spaced(
        text, pstyle, pfg, pbg, None, spacing.0, spacing.1,
    ))
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
    if child.text_is_painted(white_space)
        && let Some(s) = child.data.as_deref()
        && !s.is_empty()
    {
        let text = apply_text_transform(s, transform);
        return Some(InlineRun::owned(text, style, fg, bg, None));
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
    spacing: (usize, isize),
    runs: &mut Vec<InlineRun>,
) {
    // A closed `<details>` shows only its first `<summary>` child, so its own
    // inline content (text / spans directly inside the element) stays hidden
    // too — the same filter the box generation and block layout apply.
    for &child_id in visible_children(arena, node_id).iter() {
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
        // `display: grid` / `inline-grid` establish their own grid items, so
        // the element itself contributes no inline runs to its parent. This
        // keeps atomic inline-grid boxes from being painted twice (once as
        // an item box and again as text folded into the parent's runs).
        if child.css_attr.display().is_some_and(|d| d.is_grid()) {
            continue;
        }
        // A block-level child is dispatched as its own `LayoutBox` by the
        // layout registry — folding it into the parent's runs would
        // double-render it (the documented contract of `collect_runs`).
        // `<a>` has always enforced this; the same rule now covers every
        // inline tag the author blockified (`<em style="display:block">`).
        if child.css_attr.display().is_some_and(|d| d.is_block_level()) {
            continue;
        }
        match &child.node_type {
            NodeType::Text => {
                // Keep every non-empty text node (including whitespace-only
                // runs): the wrap phase collapses / preserves whitespace
                // according to `white-space`, so a whitespace-only node between
                // two words correctly collapses to a single space instead of
                // vanishing.
                if let Some(s) = child.data.as_deref()
                    && !s.is_empty()
                {
                    let text = apply_text_transform(s, transform);
                    runs.push(InlineRun::owned_spaced(
                        text, style, fg, bg, owner, spacing.0, spacing.1,
                    ));
                }
            }
            NodeType::Br => {
                // `<br>` forces a line break in every `white-space` mode.
                runs.push(InlineRun::forced_break(style, fg, bg));
            }
            NodeType::Q => {
                // User-agent default: `<q>` wraps its content in generated
                // quote marks (`q::before { content: open-quote }` /
                // `::after { content: close-quote }`). The pair alternates
                // with nesting depth like the CSS `quotes` property: even
                // depth uses double quotes, odd depth single quotes. The
                // quotes are not CSS declarations, so they are generated here
                // rather than carried in the UA table.
                let (q_fg, q_bg, q_style) = merge_inline_ua(child, fg, bg, style);
                let (open, close) = q_quote_pair(arena, child_id);
                runs.push(InlineRun::owned_spaced(
                    open.to_string(),
                    q_style,
                    q_fg,
                    q_bg,
                    owner,
                    spacing.0,
                    spacing.1,
                ));
                walk_inline_children(
                    arena,
                    child_id,
                    child,
                    q_fg,
                    q_bg,
                    q_style,
                    white_space,
                    owner,
                    spacing,
                    runs,
                );
                runs.push(InlineRun::owned_spaced(
                    close.to_string(),
                    q_style,
                    q_fg,
                    q_bg,
                    owner,
                    spacing.0,
                    spacing.1,
                ));
            }
            NodeType::Abbr => {
                // User-agent default: `abbr[title]` carries a dotted underline
                // as a tooltip affordance. The terminal has no dotted
                // underline primitive, so the approximation is a solid
                // underline. Conditional on the `title` attribute, so it is
                // applied here instead of the shared UA table (see its doc
                // comment); an explicit `text-decoration` (author or
                // inherited) wins.
                let (a_fg, a_bg, mut a_style) = merge_inline_ua(child, fg, bg, style);
                if child.attributes.get("title").is_some()
                    && !child.css_attr.is_set("text-decoration")
                {
                    a_style |= FontStyle::UNDERLINED;
                }
                walk_inline_children(
                    arena,
                    child_id,
                    child,
                    a_fg,
                    a_bg,
                    a_style,
                    white_space,
                    owner,
                    spacing,
                    runs,
                );
            }
            // Every remaining inline container (`<span>` / `<a>` / `<em>` /
            // `<strong>` / `<b>` / `<i>` / `<cite>` / `<dfn>` / `<var>` /
            // `<label>` / `<u>` / `<ins>` / `<s>` / `<del>` / `<mark>` /
            // `<code>` / `<kbd>` / `<samp>` / `<data>` / `<time>` /
            // `<output>`): merge the authored CSS with the user-agent
            // defaults from the shared [`NodeType::inline_ua_declarations`]
            // table — the same table the block-level layout path injects
            // into `css_attr` — instead of per-type hardcoded colours and
            // flag guards. `<a>` / `<label>` runs additionally carry the
            // element's node id so the hit map gives their text its own
            // region (the click-delegation target).
            NodeType::Span
            | NodeType::A
            | NodeType::Em
            | NodeType::Strong
            | NodeType::B
            | NodeType::I
            | NodeType::Cite
            | NodeType::Dfn
            | NodeType::Var
            | NodeType::Label
            | NodeType::U
            | NodeType::Ins
            | NodeType::S
            | NodeType::Del
            | NodeType::Mark
            | NodeType::Code
            | NodeType::Kbd
            | NodeType::Samp
            | NodeType::Data
            | NodeType::Time
            | NodeType::Output => {
                let (child_fg, child_bg, child_style) = merge_inline_ua(child, fg, bg, style);
                let owner = if matches!(child.node_type, NodeType::A | NodeType::Label) {
                    Some(child_id)
                } else {
                    owner
                };
                walk_inline_children(
                    arena,
                    child_id,
                    child,
                    child_fg,
                    child_bg,
                    child_style,
                    white_space,
                    owner,
                    spacing,
                    runs,
                );
            }
            // Block-level nested elements live in their own LayoutBox;
            // unknown inline tags are flattened by the HTML parser so they
            // never reach here as an unknown NodeType.
            _ => {}
        }
    }
}

/// Merge an inline element's authored CSS with its user-agent defaults from
/// the shared [`NodeType::inline_ua_declarations`] table — the same table the
/// block-level layout path injects into `css_attr`
/// (`apply_a_ua_defaults` / `apply_inline_ua_defaults` in
/// `inner_formatting_context::flow`), so the inline-runs path and the
/// block path derive the browser defaults from one source instead of two
/// independently maintained copies.
///
/// Per-property precedence mirrors the cascade of a browser's UA stylesheet:
/// author declaration > UA default > inherited value.
fn merge_inline_ua(
    child: &Node,
    parent_fg: Option<Color>,
    parent_bg: Option<Color>,
    parent_style: FontStyle,
) -> (Option<Color>, Option<Color>, FontStyle) {
    let css = &child.css_attr;
    // Authored declarations first.
    let mut fg = css.get_color();
    let mut bg = css.get_bg_color();
    let mut style = parent_style | block_style(css);
    // UA defaults fill the gaps and never override an authored value —
    // the same `!is_set` guards the block path's injectors use.
    for &(prop, value) in child.node_type.inline_ua_declarations() {
        match prop {
            "color" if fg.is_none() => fg = parse_color_str(value),
            "background-color" if bg.is_none() => bg = parse_color_str(value),
            "font-style" if !css.is_set("font-style") => style |= FontStyle::ITALIC,
            "font-weight" if !css.is_set("font-weight") => style |= FontStyle::BOLD,
            "text-decoration" if !css.is_set("text-decoration") => {
                style |= match value {
                    "underline" => FontStyle::UNDERLINED,
                    "line-through" => FontStyle::CROSSED_OUT,
                    _ => FontStyle::empty(),
                };
            }
            _ => {}
        }
    }
    // Neither an author rule nor a UA default: keep the inherited colour.
    (fg.or(parent_fg), bg.or(parent_bg), style)
}

/// Whether the node's inline content contains a `<br>` — the cheap boolean
/// scan `natural_intrinsic_size` uses instead of building the full run list:
/// when the measured text width is zero, a forced break is the only thing
/// that can still reserve an intrinsic row.
///
/// Mirrors [`walk_runs`]'s recursion exactly — keep the inline-container
/// list in sync with its generic arm: inline containers only,
/// author-blockified subtrees paint via their own `LayoutBox`, and a flex
/// container folds only raw text into its own runs (a `<br>` child of one
/// is not a run).
pub(crate) fn inline_tree_has_forced_break(arena: &NodeArena, node_id: NodeId) -> bool {
    let node = &arena[node_id];
    if node.css_attr.display().is_some_and(|d| d.is_flex()) {
        return false;
    }
    for &child_id in visible_children(arena, node_id).iter() {
        let child = &arena[child_id];
        if child.css_attr.display().is_some_and(|d| d.is_block_level()) {
            continue;
        }
        if child.node_type == NodeType::Br {
            return true;
        }
        if matches!(
            child.node_type,
            NodeType::Span
                | NodeType::A
                | NodeType::Em
                | NodeType::Strong
                | NodeType::B
                | NodeType::I
                | NodeType::Cite
                | NodeType::Dfn
                | NodeType::Var
                | NodeType::Label
                | NodeType::U
                | NodeType::Ins
                | NodeType::S
                | NodeType::Del
                | NodeType::Mark
                | NodeType::Code
                | NodeType::Kbd
                | NodeType::Samp
                | NodeType::Data
                | NodeType::Time
                | NodeType::Output
                | NodeType::Q
                | NodeType::Abbr
        ) && inline_tree_has_forced_break(arena, child_id)
        {
            return true;
        }
    }
    false
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
    spacing: (usize, isize),
    runs: &mut Vec<InlineRun>,
) {
    let transform = child.css_attr.get_text_transform();
    let ws = child.white_space_or(parent_ws);
    walk_runs(
        arena, child_id, child, fg, bg, style, transform, ws, owner, spacing, runs,
    );
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
    if depth.is_multiple_of(2) {
        ("\u{201C}", "\u{201D}")
    } else {
        ("\u{2018}", "\u{2019}")
    }
}

/// Apply a [`TextTransform`] to a text node's content.
///
/// `capitalize` uppercases the first character of each whitespace / punct
/// delimited word; `to_uppercase` / `to_lowercase` delegate to the Unicode
/// case mappings (a single char may expand, e.g. `ß` → `SS`). `None` borrows
/// the input untouched — this runs for every text node on every layout pass.
fn apply_text_transform(text: &str, transform: TextTransform) -> std::borrow::Cow<'_, str> {
    match transform {
        TextTransform::None => std::borrow::Cow::Borrowed(text),
        TextTransform::Uppercase => std::borrow::Cow::Owned(text.to_uppercase()),
        TextTransform::Lowercase => std::borrow::Cow::Owned(text.to_lowercase()),
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
            std::borrow::Cow::Owned(out)
        }
    }
}
