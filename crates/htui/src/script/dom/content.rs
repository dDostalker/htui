//! Content natives: `value` / `textContent` / `innerHTML` / `nodeValue` /
//! `data` accessors and their shared write paths.

use boa_engine::{Context, JsResult, JsValue, js_string};

use crate::tdom::node::{Node, NodeId, NodeType};

use super::{ScriptState, collect_text, mark_dirty, this_node_id, with_state, wrapper_is_cached};

/// `el.value` getter — the DOM `value` attribute (the field's committed
/// content; the active edit buffer while focused is engine-side state).
/// `<output>` and `<textarea>` are special-cased: their `value` IDL reflects
/// the text content (a textarea's DOM value lives in its child text nodes,
/// not a `value` attribute).
pub(super) fn native_get_value(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let value = st
        .arena
        .get(node_id)
        .map(|n| {
            if matches!(n.node_type, NodeType::Output | NodeType::Textarea) {
                n.text_content(&st.arena)
            } else {
                n.attributes.get("value").cloned().unwrap_or_default()
            }
        })
        .unwrap_or_default();
    Ok(js_string!(value).into())
}

/// `el.value` setter — write the DOM `value` attribute, marking the node dirty
/// so the next frame re-renders it. Equal values short-circuit (no re-render),
/// mirroring the `textContent` fast path. `<output>` and `<textarea>` are
/// special-cased: setting `value` replaces the element's text content (the
/// output / textarea IDL contract).
pub(super) fn native_set_value(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let value = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    if st
        .arena
        .get(node_id)
        .is_some_and(|n| matches!(n.node_type, NodeType::Output | NodeType::Textarea))
    {
        return set_text_content(&mut st, node_id, &value);
    }
    let existing = st
        .arena
        .get(node_id)
        .and_then(|n| n.attributes.get("value"))
        .cloned()
        .unwrap_or_default();
    if existing == value {
        return Ok(JsValue::undefined());
    }
    if let Some(node) = st.arena.get_mut(node_id) {
        node.attributes.insert("value".to_string(), value);
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}

pub(super) fn native_get_text(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let text = collect_text(&st.arena, node_id);
    Ok(js_string!(text).into())
}

pub(super) fn native_set_text(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let text = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    set_text_content(&mut st, node_id, &text)
}

/// Shared `textContent` write: replace the node's text, short-circuiting on
/// equal values and reusing a sole text child to avoid arena growth. Also the
/// backing store of the `<output>` `value` setter.
fn set_text_content(st: &mut ScriptState, node_id: NodeId, text: &str) -> JsResult<JsValue> {
    // Text / Comment nodes: `textContent` writes their `data` directly.
    let is_text_like = matches!(
        st.arena.get(node_id).map(|n| n.node_type),
        Some(NodeType::Text | NodeType::Comment)
    );
    if is_text_like {
        let existing = st
            .arena
            .get(node_id)
            .and_then(|n| n.data.clone())
            .unwrap_or_default();
        if existing == text {
            return Ok(JsValue::undefined());
        }
        if let Some(n) = st.arena.get_mut(node_id) {
            n.data = Some(text.to_string());
        }
        mark_dirty(st, node_id);
        return Ok(JsValue::undefined());
    }

    // Fast path: the node already holds exactly one text child — reuse it so
    // repeated `textContent` updates don't grow the arena (each grown node
    // makes the per-frame `RenderArena` clone progressively slower). When the
    // new text equals the existing one, skip entirely so no re-render is
    // scheduled (monitors that refresh identical values avoid full relayouts).
    let reuse_child = st
        .arena
        .get(node_id)
        .filter(|n| n.children.len() == 1)
        .and_then(|n| n.children.first().copied())
        .filter(|&c| matches!(st.arena.get(c).map(|n| n.node_type), Some(NodeType::Text)));
    if let Some(child_id) = reuse_child {
        let existing = st
            .arena
            .get(child_id)
            .and_then(|c| c.data.clone())
            .unwrap_or_default();
        if existing == text {
            return Ok(JsValue::undefined());
        }
        if let Some(c) = st.arena.get_mut(child_id) {
            c.data = Some(text.to_string());
        }
        mark_dirty(st, node_id);
        return Ok(JsValue::undefined());
    }

    // Structural change: detach all children and insert a fresh text node.
    let Some(node) = st.arena.get_mut(node_id) else {
        return Ok(JsValue::undefined());
    };
    let old_children = std::mem::take(&mut node.children);
    for child in old_children {
        if let Some(c) = st.arena.get_mut(child) {
            c.parent = None;
        }
    }
    let text_id = st.arena.insert(Node::text(text));
    if let Some(parent) = st.arena.get_mut(node_id) {
        parent.children.push(text_id);
    }
    if let Some(t) = st.arena.get_mut(text_id) {
        t.parent = Some(node_id);
    }
    mark_dirty(st, node_id);
    Ok(JsValue::undefined())
}

pub(super) fn native_get_inner_html(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let html = crate::html::html_parse::serialize_inner_html(&st.arena, node_id);
    Ok(js_string!(html).into())
}

pub(super) fn native_set_inner_html(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let html = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    {
        let mut st = rc.borrow_mut();
        set_inner_html(&mut st, node_id, &html, context)?;
    }
    // The fresh fragment's `on*` attributes wire their handlers now (browser
    // semantics) — compile the inserted subtree before returning to JS.
    crate::script::bindings::compile_attribute_listeners_for_subtree(context, &rc, node_id)?;
    Ok(JsValue::undefined())
}

/// Shared `innerHTML` write: parse `html` as a fragment in the context of
/// `node_id` and replace the node's children. `<style>` CSS is accumulated on
/// the state so the next render applies it; `<script>` blocks are skipped
/// (never executed, matching the browser's `innerHTML` semantics). Replaced
/// subtrees JS can no longer reach are freed from the arena (see
/// [`sweep_replaced_subtrees`]).
fn set_inner_html(
    st: &mut ScriptState,
    node_id: NodeId,
    html: &str,
    context: &mut Context,
) -> JsResult<JsValue> {
    // `innerHTML` is an Element-only property; non-elements are left alone.
    let is_element = st
        .arena
        .get(node_id)
        .is_some_and(|n| n.node_type_constant() == 1);
    if !is_element {
        return Ok(JsValue::undefined());
    }
    // Capture the children about to be replaced so the unreachable ones can
    // be freed after the new fragment is in place — without this, every
    // timer-driven `innerHTML` refresh (clock / monitor pages) leaks the
    // full replaced subtree into the slotmap.
    let replaced = st
        .arena
        .get(node_id)
        .map(|n| n.children.clone())
        .unwrap_or_default();
    let mut css_sources = Vec::new();
    crate::html::html_parse::parse_fragment_into(&mut st.arena, node_id, html, &mut css_sources);
    if !css_sources.is_empty() {
        // Host-keyed replacement semantics: rewriting a host's `innerHTML`
        // replaces its injected styles (the browser drops the host's old
        // `<style>` elements), so a timer that re-injects the same stylesheet
        // every tick is idempotent instead of accumulating an unbounded
        // history on the render side.
        st.dirty
            .injected_css
            .insert(node_id, css_sources.join("\n"));
    }
    sweep_replaced_subtrees(st, &replaced, context)?;
    mark_dirty(st, node_id);
    Ok(JsValue::undefined())
}

/// Free the replaced subtrees that JS can no longer reach.
///
/// Browser semantics: nodes dropped by an `innerHTML` write stay alive while
/// a script holds a reference to them, and are collected once unreachable.
/// The wrapper cache (`__htuiElements`, GC-rooted) is the conservative
/// reachability oracle: a wrapper exists exactly when JS has touched the
/// node, so a wrapped node — and its whole subtree, which stays attached to
/// it like a detached browser node — is kept, while unwrapped subtrees are
/// removed from the arena. Listeners imply a wrapper (they are installed on
/// one), so no listener registry cleanup is needed here.
fn sweep_replaced_subtrees(
    st: &mut ScriptState,
    roots: &[NodeId],
    context: &mut Context,
) -> JsResult<()> {
    for &root in roots {
        // Collect top-down; a wrapped node stops the descent (its whole
        // subtree stays attached to it).
        let mut doomed: Vec<NodeId> = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if wrapper_is_cached(context, id)? {
                continue;
            }
            doomed.push(id);
            if let Some(node) = st.arena.get(id) {
                stack.extend(node.children.iter().copied());
            }
        }
        for id in doomed {
            st.dirty.injected_css.swap_remove(&id);
            st.arena.remove(id);
        }
    }
    Ok(())
}

pub(super) fn native_get_node_value(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let Some(node) = st.arena.get(node_id) else {
        return Ok(JsValue::null());
    };
    match node.node_type {
        NodeType::Text | NodeType::Comment => {
            Ok(js_string!(node.data.as_deref().unwrap_or("")).into())
        }
        _ => Ok(JsValue::null()),
    }
}

pub(super) fn native_set_node_value(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let value = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let Some(node) = st.arena.get_mut(node_id) else {
        return Ok(JsValue::undefined());
    };
    if matches!(node.node_type, NodeType::Text | NodeType::Comment) {
        node.data = Some(value);
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}

pub(super) fn native_get_data(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let data = st
        .arena
        .get(node_id)
        .and_then(|n| n.data.clone())
        .unwrap_or_default();
    Ok(js_string!(data).into())
}

pub(super) fn native_set_data(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let value = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    if let Some(n) = st.arena.get_mut(node_id) {
        if n.data.as_deref() == Some(value.as_str()) {
            return Ok(JsValue::undefined());
        }
        n.data = Some(value);
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}
