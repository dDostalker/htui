//! Mutation natives: `appendChild` / `removeChild` / `insertBefore` /
//! `replaceChild` / `cloneNode` over the live arena.

use boa_engine::{Context, JsNativeError, JsResult, JsValue};

use crate::{script::ScriptState, tdom::node::NodeId};

use super::{mark_dirty, node_wrapper, this_node_id, with_state};

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
    Ok(JsValue::undefined())
}
