//! Mutation natives: `appendChild` / `removeChild` / `insertBefore` /
//! `replaceChild` / `cloneNode` over the live arena, plus the ChildNode
//! (`remove` / `before` / `after` / `replaceWith`) and ParentNode
//! (`append` / `prepend`) mixins.

use boa_engine::{Context, JsNativeError, JsResult, JsValue, js_string};

use crate::{
    script::ScriptState,
    tdom::node::{Node, NodeId, NodeType},
};

use super::{f64_to_node_id, mark_dirty, node_wrapper, this_node_id, with_state};

/// Wire `on*` attribute handlers for a subtree that just entered the
/// document (`appendChild` / `insertBefore` / `replaceChild`). Compilation
/// failures are logged, never surfaced — a broken handler body must not
/// abort the mutation that installed it.
fn wire_inserted_handlers(
    context: &mut Context,
    rc: &std::rc::Rc<std::cell::RefCell<ScriptState>>,
    root_id: NodeId,
) {
    if let Err(e) =
        crate::script::bindings::compile_attribute_listeners_for_subtree(context, rc, root_id)
    {
        log::warn!("Failed to wire on* attributes after insertion: {e}");
    }
}

/// Reject an insertion that would create a DOM cycle, with the browser's
/// `HierarchyRequestError` semantics: `parent_id` must not be the node
/// itself nor a descendant of it.
///
/// Without this check `el.appendChild(el)` produces a parent cycle in the
/// arena, and every ancestor-chain walk — `contains`, hit-map path building,
/// `<q>` quote pairing during layout, subtree serialization — loops forever
/// (or recurses until the stack overflows). The mutation natives are the only
/// place a cycle can be created, so guarding them restores the acyclicity
/// invariant every downstream walk relies on.
fn reject_cycle(
    st: &ScriptState,
    parent_id: NodeId,
    node_id: NodeId,
    method: &str,
) -> JsResult<()> {
    let mut current = Some(parent_id);
    while let Some(id) = current {
        if id == node_id {
            return Err(JsNativeError::error()
                .with_message(format!(
                    "Failed to execute '{method}' on 'Node': The new child element contains the parent."
                ))
                .into());
        }
        current = st.arena[id].parent;
    }
    Ok(())
}

pub(super) fn native_insert_before(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let parent_id = this_node_id(this, context)?;
    let child_id = this_node_id(args.first().unwrap_or(&JsValue::undefined()), context)?;
    // `null` (or a missing) reference child appends at the end.
    let reference_id = this_node_id(args.get(1).unwrap_or(&JsValue::undefined()), context).ok();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    reject_cycle(&st, parent_id, child_id, "insertBefore")?;
    let position = reference_id.and_then(|ref_id| {
        st.arena
            .get(parent_id)
            .and_then(|p| p.children.iter().position(|&c| c == ref_id))
    });
    // Detach `child_id` from any current parent.
    let old_parent = st.arena.get(child_id).and_then(|c| c.parent);
    if let Some(op) = old_parent
        && let Some(opn) = st.arena.get_mut(op)
    {
        opn.children.retain(|c| *c != child_id);
    }
    if let Some(child) = st.arena.get_mut(child_id) {
        child.parent = Some(parent_id);
    }
    if let Some(parent) = st.arena.get_mut(parent_id) {
        match position {
            Some(pos) => parent.children.insert(pos, child_id),
            None => parent.children.push(child_id),
        }
        mark_dirty(&mut st, parent_id);
        mark_dirty(&mut st, child_id);
    }
    drop(st);
    wire_inserted_handlers(context, &rc, child_id);
    node_wrapper(context, child_id)
}

pub(super) fn native_replace_child(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let parent_id = this_node_id(this, context)?;
    let new_child = this_node_id(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let old_child = this_node_id(args.get(1).unwrap_or(&JsValue::undefined()), context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let mut st = rc.borrow_mut();
    reject_cycle(&st, parent_id, new_child, "replaceChild")?;
    // Detach `new_child` from its current parent (if any).
    let old_parent = st.arena.get(new_child).and_then(|c| c.parent);
    if let Some(op) = old_parent
        && let Some(opn) = st.arena.get_mut(op)
    {
        opn.children.retain(|c| *c != new_child);
    }
    if let Some(parent) = st.arena.get_mut(parent_id) {
        match parent.children.iter().position(|&c| c == old_child) {
            Some(pos) => parent.children[pos] = new_child,
            None => parent.children.push(new_child),
        }
    }
    if let Some(c) = st.arena.get_mut(new_child) {
        c.parent = Some(parent_id);
    }
    if let Some(c) = st.arena.get_mut(old_child) {
        c.parent = None;
    }
    mark_dirty(&mut st, parent_id);
    mark_dirty(&mut st, new_child);
    mark_dirty(&mut st, old_child);
    drop(st);
    wire_inserted_handlers(context, &rc, new_child);
    if let Err(e) =
        crate::script::bindings::clear_attribute_handlers_for_subtree(context, &rc, old_child)
    {
        log::warn!("Failed to clear on* handlers after replaceChild: {e}");
    }
    node_wrapper(context, old_child)
}

pub(super) fn native_clone_node(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let deep = args.first().map(|v| v.to_boolean()).unwrap_or(false);
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let mut st = rc.borrow_mut();
    let Some(clone_id) = clone_subtree(&mut st, node_id, deep) else {
        return Ok(JsValue::null());
    };
    drop(st);
    node_wrapper(context, clone_id)
}

/// Deep (or shallow) copy a node into the arena as a detached subtree.
///
/// `parent` is left `None`, so the clone renders nothing until appended;
/// event listeners are not copied (they are keyed by `NodeId` in the JS
/// registry, and a clone gets fresh ids) — matching browser semantics.
fn clone_subtree(st: &mut ScriptState, node_id: NodeId, deep: bool) -> Option<NodeId> {
    let source = st.arena.get(node_id)?.clone();
    let mut clone = source.clone();
    clone.parent = None;
    clone.children.clear();
    let clone_id = st.arena.insert(clone);
    if deep {
        for &child in &source.children {
            let child_clone = clone_subtree(st, child, true)?;
            st.arena[child_clone].parent = Some(clone_id);
            st.arena[clone_id].children.push(child_clone);
        }
    }
    Some(clone_id)
}

pub(super) fn native_append_child(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let parent_id = this_node_id(this, context)?;
    let child_id = this_node_id(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    reject_cycle(&st, parent_id, child_id, "appendChild")?;
    // Detach from any existing parent.
    let old_parent = st.arena.get(child_id).and_then(|c| c.parent);
    if let Some(old_parent) = old_parent
        && let Some(op) = st.arena.get_mut(old_parent)
    {
        op.children.retain(|c| *c != child_id);
    }
    if let Some(child) = st.arena.get_mut(child_id) {
        child.parent = Some(parent_id);
    }
    if let Some(parent) = st.arena.get_mut(parent_id) {
        parent.children.push(child_id);
        mark_dirty(&mut st, parent_id);
        mark_dirty(&mut st, child_id);
    }
    drop(st);
    wire_inserted_handlers(context, &rc, child_id);
    Ok(JsValue::undefined())
}

pub(super) fn native_remove_child(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let parent_id = this_node_id(this, context)?;
    let child_id = this_node_id(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    if let Some(child) = st.arena.get_mut(child_id) {
        child.parent = None;
    }
    if let Some(parent) = st.arena.get_mut(parent_id) {
        parent.children.retain(|c| *c != child_id);
        mark_dirty(&mut st, parent_id);
        mark_dirty(&mut st, child_id);
    }
    drop(st);
    if let Err(e) =
        crate::script::bindings::clear_attribute_handlers_for_subtree(context, &rc, child_id)
    {
        log::warn!("Failed to clear on* handlers after removeChild: {e}");
    }
    Ok(JsValue::undefined())
}

// ============================================================
// ChildNode / ParentNode mixins
// ============================================================

/// One position in a parent's child list where a mixin insertion may land.
enum InsertPos {
    /// Insert at the end of the child list.
    End,
    /// Insert before `ref_id`; `None` appends at the end.
    Before(Option<NodeId>),
    /// Insert at the given index (clamped to the list length) — how
    /// `prepend(a, b, …)` keeps its argument order.
    At(usize),
}

/// Detach `child_id` from any current parent and insert it under
/// `parent_id` at `position` — the shared insertion core of the ChildNode /
/// ParentNode mixins (mirrors the `appendChild` mutation pattern: detach,
/// relink, mark both ends dirty). A stale child id (freed by an `innerHTML`
/// sweep) must not enter the child list, so the whole insertion is skipped.
fn detach_and_insert(
    st: &mut ScriptState,
    parent_id: NodeId,
    child_id: NodeId,
    position: InsertPos,
) {
    if !st.arena.contains_key(child_id) {
        return;
    }
    let old_parent = st.arena.get(child_id).and_then(|c| c.parent);
    if let Some(op) = old_parent
        && let Some(opn) = st.arena.get_mut(op)
    {
        opn.children.retain(|c| *c != child_id);
    }
    if let Some(child) = st.arena.get_mut(child_id) {
        child.parent = Some(parent_id);
    }
    if let Some(parent) = st.arena.get_mut(parent_id) {
        match position {
            InsertPos::End => parent.children.push(child_id),
            InsertPos::Before(Some(ref_id)) => {
                match parent.children.iter().position(|&c| c == ref_id) {
                    Some(p) => parent.children.insert(p, child_id),
                    None => parent.children.push(child_id),
                }
            }
            InsertPos::Before(None) => parent.children.push(child_id),
            InsertPos::At(idx) => {
                let idx = idx.min(parent.children.len());
                parent.children.insert(idx, child_id);
            }
        }
        mark_dirty(st, parent_id);
        mark_dirty(st, child_id);
    }
}

/// Convert one mixin argument into a node id: node wrappers pass through
/// (their `_nodeId`), primitives (strings, numbers, `null`, …) become fresh
/// Text nodes via their string conversion — the browser's ChildNode /
/// ParentNode union conversion (`append(null)` inserts the text "null").
/// Any other object (no `_nodeId`) is a `TypeError`, like the browser's
/// "parameter is not of type 'Node'".
fn arg_to_node_id(
    st: &mut ScriptState,
    value: &JsValue,
    context: &mut Context,
) -> JsResult<NodeId> {
    if let Some(obj) = value.as_object() {
        let id = obj.get(js_string!("_nodeId"), context)?;
        let Some(n) = id.as_number() else {
            return Err(JsNativeError::typ()
                .with_message("Failed to execute: parameter is not of type 'Node'")
                .into());
        };
        return Ok(f64_to_node_id(n));
    }
    let text = value.to_string(context)?.to_std_string_lossy();
    Ok(st.arena.insert(Node::text(&text)))
}

/// Convert every mixin argument up front (ChildNode / ParentNode methods are
/// all-or-nothing in browsers: a `TypeError` on argument 2 must not leave
/// argument 1 inserted), then cycle-check every node argument together.
fn convert_args(
    st: &mut ScriptState,
    args: &[JsValue],
    context: &mut Context,
    parent_id: NodeId,
    method: &str,
) -> JsResult<Vec<NodeId>> {
    let mut ids = Vec::with_capacity(args.len());
    for arg in args {
        ids.push(arg_to_node_id(st, arg, context)?);
    }
    for &id in &ids {
        reject_cycle(st, parent_id, id, method)?;
    }
    Ok(ids)
}

/// `ChildNode.remove()` — detach this node from its parent (a no-op when it
/// has none, matching the browser). The subtree keeps its arena nodes and
/// JS wrappers; only its `on*` attribute handlers are unwired, exactly like
/// `removeChild`.
pub(super) fn native_remove(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let detached_from = {
        let mut st = rc.borrow_mut();
        let parent = st.arena.get(node_id).and_then(|n| n.parent);
        if let Some(pid) = parent {
            if let Some(p) = st.arena.get_mut(pid) {
                p.children.retain(|c| *c != node_id);
            }
            if let Some(n) = st.arena.get_mut(node_id) {
                n.parent = None;
            }
            mark_dirty(&mut st, pid);
            mark_dirty(&mut st, node_id);
        }
        parent
    };
    if detached_from.is_some()
        && let Err(e) =
            crate::script::bindings::clear_attribute_handlers_for_subtree(context, &rc, node_id)
    {
        log::warn!("Failed to clear on* handlers after remove: {e}");
    }
    Ok(JsValue::undefined())
}

/// `ParentNode.append(…nodes)` — insert every argument at the end of this
/// node's child list (strings become Text nodes).
pub(super) fn native_append(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let parent_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let inserted = {
        let mut st = rc.borrow_mut();
        let inserted = convert_args(&mut st, args, context, parent_id, "append")?;
        for child_id in &inserted {
            detach_and_insert(&mut st, parent_id, *child_id, InsertPos::End);
        }
        inserted
    };
    for id in inserted {
        wire_inserted_handlers(context, &rc, id);
    }
    Ok(JsValue::undefined())
}

/// `ParentNode.prepend(…nodes)` — insert every argument at the start of this
/// node's child list, preserving argument order (`prepend(a, b)` yields
/// `[a, b, …]`).
pub(super) fn native_prepend(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let parent_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let inserted = {
        let mut st = rc.borrow_mut();
        let inserted = convert_args(&mut st, args, context, parent_id, "prepend")?;
        for (i, child_id) in inserted.iter().enumerate() {
            detach_and_insert(&mut st, parent_id, *child_id, InsertPos::At(i));
        }
        inserted
    };
    for id in inserted {
        wire_inserted_handlers(context, &rc, id);
    }
    Ok(JsValue::undefined())
}

/// Resolve the parent that a ChildNode mixin (`before` / `after` /
/// `replaceWith`) would insert into, or `None` when the node is detached
/// (the whole call degrades to a no-op, matching the browser).
fn child_node_parent(st: &ScriptState, node_id: NodeId) -> Option<NodeId> {
    st.arena.get(node_id).and_then(|n| n.parent)
}

/// `ChildNode.before(…nodes)` — insert every argument immediately before
/// this node (a no-op when detached).
pub(super) fn native_before(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let inserted = {
        let mut st = rc.borrow_mut();
        let Some(parent_id) = child_node_parent(&st, node_id) else {
            return Ok(JsValue::undefined());
        };
        let inserted = convert_args(&mut st, args, context, parent_id, "before")?;
        for child_id in &inserted {
            detach_and_insert(
                &mut st,
                parent_id,
                *child_id,
                InsertPos::Before(Some(node_id)),
            );
        }
        inserted
    };
    for id in inserted {
        wire_inserted_handlers(context, &rc, id);
    }
    Ok(JsValue::undefined())
}

/// `ChildNode.after(…nodes)` — insert every argument immediately after this
/// node (a no-op when detached). The insertion reference is this node's
/// next sibling, captured once so `after(a, b)` keeps argument order.
pub(super) fn native_after(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let inserted = {
        let mut st = rc.borrow_mut();
        let Some(parent_id) = child_node_parent(&st, node_id) else {
            return Ok(JsValue::undefined());
        };
        let inserted = convert_args(&mut st, args, context, parent_id, "after")?;
        if inserted.is_empty() {
            return Ok(JsValue::undefined());
        }
        let next = {
            let parent = &st.arena[parent_id];
            let pos = parent.children.iter().position(|&c| c == node_id);
            pos.and_then(|p| parent.children.get(p + 1).copied())
        };
        for child_id in &inserted {
            detach_and_insert(&mut st, parent_id, *child_id, InsertPos::Before(next));
        }
        inserted
    };
    for id in inserted {
        wire_inserted_handlers(context, &rc, id);
    }
    Ok(JsValue::undefined())
}

/// `ChildNode.replaceWith(…nodes)` — swap this node for the arguments
/// (strings become Text nodes; a no-op when detached). Every argument is
/// inserted before this node in order, then this node is removed — so the
/// arguments land exactly in the original position.
pub(super) fn native_replace_with(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let inserted = {
        let mut st = rc.borrow_mut();
        let Some(parent_id) = child_node_parent(&st, node_id) else {
            return Ok(JsValue::undefined());
        };
        let inserted = convert_args(&mut st, args, context, parent_id, "replaceWith")?;
        for child_id in &inserted {
            detach_and_insert(
                &mut st,
                parent_id,
                *child_id,
                InsertPos::Before(Some(node_id)),
            );
        }
        // Remove this node now that the replacements are in place.
        if let Some(p) = st.arena.get_mut(parent_id) {
            p.children.retain(|c| *c != node_id);
        }
        if let Some(n) = st.arena.get_mut(node_id) {
            n.parent = None;
        }
        mark_dirty(&mut st, parent_id);
        mark_dirty(&mut st, node_id);
        inserted
    };
    if let Err(e) =
        crate::script::bindings::clear_attribute_handlers_for_subtree(context, &rc, node_id)
    {
        log::warn!("Failed to clear on* handlers after replaceWith: {e}");
    }
    for id in inserted {
        wire_inserted_handlers(context, &rc, id);
    }
    Ok(JsValue::undefined())
}

/// `node.normalize()` — merge every run of adjacent Text children into its
/// first member and drop empty Text children (browser semantics). Comments
/// break a run and are never touched. Only the parent's child list changes;
/// the merged-away / empty text nodes stay in the arena detached (JS may
/// still hold wrappers, like `removeChild`).
pub(super) fn native_normalize(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let Some(children) = st.arena.get(node_id).map(|n| n.children.clone()) else {
        return Ok(JsValue::undefined());
    };
    let mut rebuilt: Vec<NodeId> = Vec::with_capacity(children.len());
    let mut pending_text: Option<NodeId> = None;
    let mut changed = false;
    for cid in children {
        // Read the child's shape first; the mutation arms below need
        // disjoint borrows (run head vs current child).
        let shape = st.arena.get(cid).map(|c| {
            (
                c.node_type == NodeType::Text,
                c.data.clone().unwrap_or_default(),
            )
        });
        match shape {
            Some((true, data)) if data.is_empty() => {
                // Empty text child: dropped from the list (arena node kept,
                // detached — a held JS wrapper must not go stale).
                if let Some(c) = st.arena.get_mut(cid) {
                    c.parent = None;
                }
                changed = true;
            }
            Some((true, data)) => match pending_text {
                Some(target) => {
                    // Merge into the run head.
                    let merged = st.arena.get_mut(target).map(|t| {
                        let joined = format!("{}{}", t.data.take().unwrap_or_default(), data);
                        t.data = Some(joined);
                    });
                    if merged.is_some() {
                        if let Some(c) = st.arena.get_mut(cid) {
                            c.parent = None;
                        }
                        changed = true;
                    }
                }
                None => {
                    pending_text = Some(cid);
                    rebuilt.push(cid);
                }
            },
            _ => {
                // Non-text child (element / comment / …) ends the run.
                pending_text = None;
                rebuilt.push(cid);
            }
        }
    }
    if changed {
        if let Some(node) = st.arena.get_mut(node_id) {
            node.children = rebuilt;
        }
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}

/// `ParentNode.replaceChildren(…nodes)` — swap the whole child list for the
/// arguments (strings become Text nodes; no arguments clears the list).
/// Removed subtrees are detached and their `on*` handlers unwired, exactly
/// like `removeChild`.
pub(super) fn native_replace_children(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let parent_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let (removed, inserted) = {
        let mut st = rc.borrow_mut();
        let inserted = convert_args(&mut st, args, context, parent_id, "replaceChildren")?;
        let removed: Vec<NodeId> = st
            .arena
            .get(parent_id)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        for child_id in &removed {
            if let Some(c) = st.arena.get_mut(*child_id) {
                c.parent = None;
            }
        }
        if let Some(parent) = st.arena.get_mut(parent_id) {
            parent.children.clear();
        }
        for child_id in &inserted {
            detach_and_insert(&mut st, parent_id, *child_id, InsertPos::End);
        }
        if !removed.is_empty() || !inserted.is_empty() {
            mark_dirty(&mut st, parent_id);
        }
        (removed, inserted)
    };
    for id in removed {
        if let Err(e) =
            crate::script::bindings::clear_attribute_handlers_for_subtree(context, &rc, id)
        {
            log::warn!("Failed to clear on* handlers after replaceChildren: {e}");
        }
    }
    for id in inserted {
        wire_inserted_handlers(context, &rc, id);
    }
    Ok(JsValue::undefined())
}

// ============================================================
// insertAdjacent* (beforebegin / afterbegin / beforeend / afterend)
// ============================================================

/// Parse the position argument of the `insertAdjacent*` family. An unknown
/// position is the browser's `SyntaxError`.
fn adjacent_position(args: &[JsValue], context: &mut Context) -> JsResult<&'static str> {
    let raw = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default()
        .to_ascii_lowercase();
    match raw.as_str() {
        "beforebegin" => Ok("beforebegin"),
        "afterbegin" => Ok("afterbegin"),
        "beforeend" => Ok("beforeend"),
        "afterend" => Ok("afterend"),
        _ => Err(JsNativeError::syntax()
            .with_message(format!(
                "invalid position `{raw}`: expected beforebegin/afterbegin/beforeend/afterend"
            ))
            .into()),
    }
}

/// The insertion [`InsertPos`] for `position` relative to `node_id` under
/// `parent_id`. `afterend` anchors on this node's next sibling, captured
/// once so repeated calls keep their own order.
fn adjacent_insert_pos(
    st: &ScriptState,
    position: &str,
    parent_id: NodeId,
    node_id: NodeId,
) -> InsertPos {
    match position {
        "beforebegin" => InsertPos::Before(Some(node_id)),
        "afterbegin" => InsertPos::At(0),
        "beforeend" => InsertPos::End,
        _ => {
            let next = {
                let parent = &st.arena[parent_id];
                let p = parent.children.iter().position(|&c| c == node_id);
                p.and_then(|i| parent.children.get(i + 1).copied())
            };
            InsertPos::Before(next)
        }
    }
}

/// `element.insertAdjacentElement(where, el)` — insert a node relative to
/// this element. `beforebegin` / `afterend` need a parent; without one they
/// are a no-op returning `null` (detached target, browser behavior).
/// Returns the inserted element's wrapper.
pub(super) fn native_insert_adjacent_element(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let position = adjacent_position(args, context)?;
    let undefined = JsValue::undefined();
    let arg = args.get(1).unwrap_or(&undefined);
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let inserted = {
        let mut st = rc.borrow_mut();
        let node_arg = arg
            .as_object()
            .and_then(|obj| obj.get(js_string!("_nodeId"), context).ok())
            .and_then(|v| v.as_number());
        let Some(n) = node_arg else {
            return Err(JsNativeError::typ()
                .with_message("insertAdjacentElement expects a Node")
                .into());
        };
        let child_id = f64_to_node_id(n);
        let parent_id = match position {
            "beforebegin" | "afterend" => st.arena.get(node_id).and_then(|n| n.parent),
            _ => Some(node_id),
        };
        let Some(parent_id) = parent_id else {
            return Ok(JsValue::null());
        };
        reject_cycle(&st, parent_id, child_id, "insertAdjacentElement")?;
        let pos = adjacent_insert_pos(&st, position, parent_id, node_id);
        detach_and_insert(&mut st, parent_id, child_id, pos);
        child_id
    };
    wire_inserted_handlers(context, &rc, inserted);
    node_wrapper(context, inserted)
}

/// `element.insertAdjacentText(where, text)` — insert a fresh Text node
/// relative to this element (same position semantics as
/// [`native_insert_adjacent_element`]).
pub(super) fn native_insert_adjacent_text(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let position = adjacent_position(args, context)?;
    let text = args
        .get(1)
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    {
        let mut st = rc.borrow_mut();
        let parent_id = match position {
            "beforebegin" | "afterend" => st.arena.get(node_id).and_then(|n| n.parent),
            _ => Some(node_id),
        };
        let Some(parent_id) = parent_id else {
            return Ok(JsValue::undefined());
        };
        let text_id = st.arena.insert(Node::text(&text));
        let pos = adjacent_insert_pos(&st, position, parent_id, node_id);
        detach_and_insert(&mut st, parent_id, text_id, pos);
    }
    Ok(JsValue::undefined())
}

/// `element.insertAdjacentHTML(where, html)` — parse `html` as a fragment
/// and insert the result relative to this element. `afterbegin` /
/// `beforeend` parse straight into this element; `beforebegin` / `afterend`
/// parse into the parent and the fresh nodes are moved next to this
/// element. `<style>` CSS found in the fragment is injected keyed by this
/// element (the same host-keyed semantics as `innerHTML`); `<script>`
/// blocks are never executed.
pub(super) fn native_insert_adjacent_html(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let position = adjacent_position(args, context)?;
    let html = args
        .get(1)
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let inserted = {
        let mut st = rc.borrow_mut();
        let parent_id = match position {
            "beforebegin" | "afterend" => st.arena.get(node_id).and_then(|n| n.parent),
            _ => Some(node_id),
        };
        let Some(parent_id) = parent_id else {
            return Ok(JsValue::undefined());
        };
        // `parse_fragment_into` has replacement (innerHTML) semantics, so
        // the fragment is parsed into a scratch node typed like this
        // element (keeping the parser's context tag) and its children are
        // then moved to the target position; the scratch node is freed.
        let scratch = st
            .arena
            .get(node_id)
            .map(|n| Node::from(n.node_type))
            .map(|mut s| {
                s.parent = None;
                st.arena.insert(s)
            });
        let Some(scratch) = scratch else {
            return Ok(JsValue::undefined());
        };
        let mut css_sources = Vec::new();
        crate::html::html_parse::parse_fragment_into(
            &mut st.arena,
            scratch,
            &html,
            &mut css_sources,
        );
        if !css_sources.is_empty() {
            st.dirty
                .injected_css
                .insert(node_id, css_sources.join("\n"));
        }
        let fresh: Vec<NodeId> = st
            .arena
            .get(scratch)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        match position {
            "afterbegin" => {
                // Reverse insert-at-0 keeps document order.
                for id in fresh.iter().rev() {
                    detach_and_insert(&mut st, node_id, *id, InsertPos::At(0));
                }
            }
            "beforeend" => {
                for id in &fresh {
                    detach_and_insert(&mut st, node_id, *id, InsertPos::End);
                }
            }
            "beforebegin" => {
                for id in &fresh {
                    detach_and_insert(&mut st, parent_id, *id, InsertPos::Before(Some(node_id)));
                }
            }
            _ => {
                for id in &fresh {
                    let pos = adjacent_insert_pos(&st, "afterend", parent_id, node_id);
                    detach_and_insert(&mut st, parent_id, *id, pos);
                }
            }
        }
        // The scratch node held the parse result; nobody can reference it
        // (it was never wrapped), so it leaves the arena for good.
        st.arena.remove(scratch);
        if !fresh.is_empty() {
            mark_dirty(&mut st, parent_id);
        }
        fresh
    };
    for id in inserted {
        wire_inserted_handlers(context, &rc, id);
    }
    Ok(JsValue::undefined())
}
