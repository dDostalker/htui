//! Content natives: `value` / `textContent` / `innerHTML` / `nodeValue` /
//! `data` accessors and their shared write paths, plus the CharacterData
//! methods (`substringData` / `appendData` / `insertData` / `deleteData` /
//! `replaceData`, `length` and `Text.splitText`).

use boa_engine::{Context, JsNativeError, JsResult, JsValue, js_string};

use crate::tdom::node::{Node, NodeId, NodeType};

use super::{
    ScriptState, collect_text, mark_dirty, node_wrapper, this_node_id, with_state,
    wrapper_is_cached,
};

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
            // The node is leaving the arena for good — drop its attribute
            // handler slots so the registry cannot grow without bound.
            crate::script::bindings::clear_node_attribute_listeners(context, id)?;
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

// ============================================================
// CharacterData methods (substringData / insertData / …)
// ============================================================

/// Character-offset check shared by the CharacterData methods. Offsets count
/// Unicode scalar values (chars) rather than UTF-16 code units — the natural
/// unit for a char-grid renderer. `offset > length` is the DOM's
/// IndexSizeError, surfaced as a JS `RangeError`.
fn checked_char_offset(data: &str, offset: u32) -> JsResult<usize> {
    let len = data.chars().count();
    let offset = offset as usize;
    if offset > len {
        return Err(JsNativeError::range()
            .with_message(format!(
                "IndexSizeError: offset {offset} exceeds data length {len}"
            ))
            .into());
    }
    Ok(offset)
}

/// Replace `count` characters starting at `offset` with `insert` (the shared
/// core of insert / append / delete / replace: an empty `insert` deletes and
/// a zero `count` inserts).
fn replace_char_range(data: &str, offset: usize, count: u32, insert: &str) -> String {
    let mut chars: Vec<char> = data.chars().collect();
    let end = (offset + count as usize).min(chars.len());
    chars.splice(offset..end, insert.chars());
    chars.into_iter().collect()
}

/// Unsigned-argument read with the DOM's `unsigned long` conversion (negative
/// input wraps modulo 2³², matching browser behavior for e.g.
/// `substringData(-1, 1)`).
fn uint_arg(args: &[JsValue], index: usize, context: &mut Context) -> JsResult<u32> {
    match args.get(index) {
        Some(v) => v.to_u32(context),
        None => Ok(0),
    }
}

/// String-argument read (missing argument is the empty string, matching the
/// loose call style the other content natives tolerate).
fn string_arg(args: &[JsValue], index: usize, context: &mut Context) -> JsResult<String> {
    match args.get(index) {
        Some(v) => Ok(v.to_string(context)?.to_std_string_lossy()),
        None => Ok(String::new()),
    }
}

/// Read the `data` of a text-like node (empty for anything else).
fn text_data(arena: &crate::tdom::node::NodeArena, node_id: NodeId) -> String {
    arena
        .get(node_id)
        .and_then(|n| n.data.clone())
        .unwrap_or_default()
}

/// Write back the `data` of a text-like node, short-circuiting when the value
/// did not change (no dirty, no re-render — the same contract as the `data`
/// setter).
fn store_data(st: &mut ScriptState, node_id: NodeId, new: String) {
    let Some(node) = st.arena.get_mut(node_id) else {
        return;
    };
    let changed = node.data.as_deref() != Some(new.as_str());
    node.data = Some(new);
    if changed {
        mark_dirty(st, node_id);
    }
}

/// `CharacterData.substringData(offset, count)` — the `count` characters
/// starting at `offset`.
pub(super) fn native_substring_data(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let offset = uint_arg(args, 0, context)?;
    let count = uint_arg(args, 1, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let data = text_data(&st.arena, node_id);
    let offset = checked_char_offset(&data, offset)?;
    Ok(js_string!(
        data.chars()
            .skip(offset)
            .take(count as usize)
            .collect::<String>()
    )
    .into())
}

/// `CharacterData.appendData(text)` — add `text` at the end of `data`.
pub(super) fn native_append_data(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let text = string_arg(args, 0, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let data = text_data(&st.arena, node_id);
    let new = format!("{data}{text}");
    store_data(&mut st, node_id, new);
    Ok(JsValue::undefined())
}

/// `CharacterData.insertData(offset, text)` — insert `text` at `offset`.
pub(super) fn native_insert_data(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let offset = uint_arg(args, 0, context)?;
    let text = string_arg(args, 1, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let data = text_data(&st.arena, node_id);
    let offset = checked_char_offset(&data, offset)?;
    let new = replace_char_range(&data, offset, 0, &text);
    store_data(&mut st, node_id, new);
    Ok(JsValue::undefined())
}

/// `CharacterData.deleteData(offset, count)` — remove `count` characters
/// starting at `offset` (count clamps to the remaining length).
pub(super) fn native_delete_data(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let offset = uint_arg(args, 0, context)?;
    let count = uint_arg(args, 1, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let data = text_data(&st.arena, node_id);
    let offset = checked_char_offset(&data, offset)?;
    let new = replace_char_range(&data, offset, count, "");
    store_data(&mut st, node_id, new);
    Ok(JsValue::undefined())
}

/// `CharacterData.replaceData(offset, count, text)` — swap the `count`
/// characters starting at `offset` for `text`.
pub(super) fn native_replace_data(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let offset = uint_arg(args, 0, context)?;
    let count = uint_arg(args, 1, context)?;
    let text = string_arg(args, 2, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let data = text_data(&st.arena, node_id);
    let offset = checked_char_offset(&data, offset)?;
    let new = replace_char_range(&data, offset, count, &text);
    store_data(&mut st, node_id, new);
    Ok(JsValue::undefined())
}

/// `CharacterData.length` — the number of characters in `data` (the same
/// char-counting contract as the CharacterData methods above).
pub(super) fn native_get_data_length(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let len = with_state()
        .map(|rc| {
            let st = rc.borrow();
            text_data(&st.arena, node_id).chars().count()
        })
        .unwrap_or(0);
    Ok(JsValue::from(len as u32))
}

/// `Text.splitText(offset)` — split the node at `offset` characters: this
/// node keeps the leading text and a fresh Text node holding the tail is
/// inserted as the next sibling (detached when the node has no parent).
/// Text nodes only — `Comment` has no `splitText` (browser behavior).
pub(super) fn native_split_text(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let offset = uint_arg(args, 0, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let mut st = rc.borrow_mut();
    let data = text_data(&st.arena, node_id);
    let offset = checked_char_offset(&data, offset)?;
    let head: String = data.chars().take(offset).collect();
    let tail: String = data.chars().skip(offset).collect();
    let parent = st.arena.get(node_id).and_then(|n| n.parent);
    store_data(&mut st, node_id, head);
    let tail_id = st.arena.insert(Node::text(&tail));
    if let Some(pid) = parent {
        if let Some(pnode) = st.arena.get_mut(pid) {
            let pos = pnode
                .children
                .iter()
                .position(|&c| c == node_id)
                .map_or(pnode.children.len(), |p| p + 1);
            pnode.children.insert(pos, tail_id);
        }
        if let Some(t) = st.arena.get_mut(tail_id) {
            t.parent = Some(pid);
        }
        mark_dirty(&mut st, pid);
    }
    drop(st);
    node_wrapper(context, tail_id)
}
