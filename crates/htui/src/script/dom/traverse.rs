//! Node-tree traversal natives: parent / sibling / child accessors, live
//! NodeList, containment / equality checks and `querySelector` search.

use boa_engine::{Context, JsNativeError, JsResult, JsValue, NativeFunction, js_string};

use crate::{
    html::css_parse::{CssSelector, selector_chain_matches},
    tdom::node::{NodeArena, NodeId, document_base_href},
};

use super::{f64_to_node_id, js_fn, node_id_to_f64, node_wrapper, this_node_id, with_state};

pub(super) fn native_get_parent(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let parent = st.arena.get(node_id).and_then(|n| n.parent);
    match parent {
        Some(pid) => node_wrapper(context, pid),
        None => Ok(JsValue::null()),
    }
}

// ============================================================
// Node interface natives (shared by every node kind)
// ============================================================

pub(super) fn native_get_node_type(
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
    Ok(JsValue::from(node.node_type_constant() as u32))
}

pub(super) fn native_get_node_name(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let Some(node) = st.arena.get(node_id) else {
        return Ok(js_string!().into());
    };
    Ok(js_string!(node.node_name()).into())
}

pub(super) fn native_get_tag_name(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let name = st
        .arena
        .get(node_id)
        .and_then(|n| n.node_type.tag_name())
        .map(str::to_uppercase)
        .unwrap_or_default();
    Ok(js_string!(name).into())
}

pub(super) fn native_get_parent_element(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let parent = st.arena.get(node_id).and_then(|n| n.parent).filter(|&pid| {
        st.arena
            .get(pid)
            .map(|p| p.node_type_constant() == 1)
            .unwrap_or(false)
    });
    match parent {
        Some(pid) => node_wrapper(context, pid),
        None => Ok(JsValue::null()),
    }
}

pub(super) fn native_get_child_nodes(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    call_live_list(context, node_id)
}

/// Build a live NodeList proxy over `node_id`'s children. True liveness:
/// the returned object's `length` / indexed access / enumeration read the
/// arena on every access, so a held reference reflects later DOM mutations.
fn call_live_list(context: &mut Context, node_id: NodeId) -> JsResult<JsValue> {
    let node_key = node_id_to_f64(node_id);
    let get_len = js_fn(
        context,
        NativeFunction::from_copy_closure_with_captures(nodelist_len, node_key),
    );
    let get_item = js_fn(
        context,
        NativeFunction::from_copy_closure_with_captures(nodelist_item, node_key),
    );
    let global = context.global_object();
    let helper = global
        .get(js_string!("__htuiLiveList"), context)?
        .to_object(context)?;
    helper.call(
        &JsValue::undefined(),
        &[get_len.into(), get_item.into()],
        context,
    )
}

/// Live `NodeList.length` (reads the arena on every call).
fn nodelist_len(
    _this: &JsValue,
    _args: &[JsValue],
    node_key: &f64,
    _context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = f64_to_node_id(*node_key);
    let len = with_state()
        .map(|rc| {
            rc.borrow()
                .arena
                .get(node_id)
                .map(|n| n.children.len())
                .unwrap_or(0)
        })
        .unwrap_or(0);
    Ok(JsValue::from(len as u32))
}

/// Live `NodeList[i]` (reads the arena on every call).
fn nodelist_item(
    _this: &JsValue,
    args: &[JsValue],
    node_key: &f64,
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = f64_to_node_id(*node_key);
    let idx = args
        .first()
        .and_then(|v| v.to_number(context).ok())
        .unwrap_or(-1.0);
    if idx < 0.0 {
        return Ok(JsValue::null());
    }
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let child = st
        .arena
        .get(node_id)
        .and_then(|n| n.children.get(idx as usize))
        .copied();
    match child {
        Some(cid) => node_wrapper(context, cid),
        None => Ok(JsValue::null()),
    }
}

pub(super) fn native_get_first_child(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let child = st
        .arena
        .get(node_id)
        .and_then(|n| n.children.first())
        .copied();
    match child {
        Some(cid) => node_wrapper(context, cid),
        None => Ok(JsValue::null()),
    }
}

pub(super) fn native_get_last_child(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let child = st
        .arena
        .get(node_id)
        .and_then(|n| n.children.last())
        .copied();
    match child {
        Some(cid) => node_wrapper(context, cid),
        None => Ok(JsValue::null()),
    }
}

pub(super) fn native_get_prev_sibling(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let prev = st
        .arena
        .get(node_id)
        .and_then(|n| n.parent)
        .and_then(|pid| {
            let parent = &st.arena[pid];
            let mut prev = None;
            for &cid in &parent.children {
                if cid == node_id {
                    break;
                }
                prev = Some(cid);
            }
            prev
        });
    match prev {
        Some(cid) => node_wrapper(context, cid),
        None => Ok(JsValue::null()),
    }
}

pub(super) fn native_get_next_sibling(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let next = st
        .arena
        .get(node_id)
        .and_then(|n| n.parent)
        .and_then(|pid| {
            let parent = &st.arena[pid];
            let mut found = false;
            let mut next = None;
            for &cid in &parent.children {
                if found {
                    next = Some(cid);
                    break;
                }
                if cid == node_id {
                    found = true;
                }
            }
            next
        });
    match next {
        Some(cid) => node_wrapper(context, cid),
        None => Ok(JsValue::null()),
    }
}

pub(super) fn native_get_owner_document(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    if node_id == st.root_id {
        return Ok(JsValue::null());
    }
    node_wrapper(context, st.root_id)
}

/// `baseURI` — the document base URL established by the first `<base href>`,
/// or an empty string when the document declares no `<base>`. Mirrors the DOM
/// property that reports the base URL relative references (`<img src>`,
/// `<script src>`, `fetch`, …) resolve against.
pub(super) fn native_get_base_uri(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let uri = document_base_href(&st.arena, st.root_id).unwrap_or_default();
    Ok(js_string!(uri).into())
}

pub(super) fn native_has_child_nodes(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let has = with_state()
        .map(|rc| {
            rc.borrow()
                .arena
                .get(node_id)
                .map(|n| !n.children.is_empty())
                .unwrap_or(false)
        })
        .unwrap_or(false);
    Ok(JsValue::from(has))
}

pub(super) fn native_contains(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(other_id) = this_node_id(args.first().unwrap_or(&JsValue::undefined()), context).ok()
    else {
        return Ok(JsValue::from(false));
    };
    let Some(rc) = with_state() else {
        return Ok(JsValue::from(false));
    };
    let st = rc.borrow();
    // A node contains itself and all of its descendants.
    let mut cur = Some(other_id);
    while let Some(id) = cur {
        if id == node_id {
            return Ok(JsValue::from(true));
        }
        cur = st.arena.get(id).and_then(|n| n.parent);
    }
    Ok(JsValue::from(false))
}

pub(super) fn native_is_same_node(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let other = this_node_id(args.first().unwrap_or(&JsValue::undefined()), context).ok();
    Ok(JsValue::from(other == Some(node_id)))
}

pub(super) fn native_is_equal_node(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(other_id) = this_node_id(args.first().unwrap_or(&JsValue::undefined()), context).ok()
    else {
        return Ok(JsValue::from(false));
    };
    let Some(rc) = with_state() else {
        return Ok(JsValue::from(false));
    };
    let st = rc.borrow();
    Ok(JsValue::from(nodes_equal(&st.arena, node_id, other_id)))
}

/// Structural equality: same node kind / data / attributes (order-insensitive,
/// matching the DOM's set semantics) and equal children recursively.
fn nodes_equal(arena: &NodeArena, a: NodeId, b: NodeId) -> bool {
    let (na, nb) = match (arena.get(a), arena.get(b)) {
        (Some(x), Some(y)) => (x, y),
        _ => return false,
    };
    if na.node_type != nb.node_type || na.data != nb.data {
        return false;
    }
    if na.attributes.len() != nb.attributes.len() {
        return false;
    }
    for (k, v) in &na.attributes {
        if nb.attributes.get(k) != Some(v) {
            return false;
        }
    }
    na.children.len() == nb.children.len()
        && na
            .children
            .iter()
            .zip(&nb.children)
            .all(|(&ca, &cb)| nodes_equal(arena, ca, cb))
}

// ============================================================
// Element natives
// ============================================================

/// Parse a CSS selector string for `querySelector` / `querySelectorAll`,
/// throwing a JS `SyntaxError` on invalid input (browser behavior).
fn parse_query_selector(args: &[JsValue], context: &mut Context) -> JsResult<CssSelector> {
    let raw = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    raw.parse::<CssSelector>().map_err(|e| {
        JsNativeError::syntax()
            .with_message(format!("invalid selector `{raw}`: {e}"))
            .into()
    })
}

/// Document-order (pre-order DFS) list of every node under `scope` matching
/// `selector`. The scope element itself is never considered — matching
/// `document.querySelector` (the root's children) and `element.querySelector`
/// (the element's subtree) semantics.
fn query_selector_nodes(arena: &NodeArena, scope: NodeId, selector: &CssSelector) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = arena[scope].children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if selector_chain_matches(arena, id, selector) {
            out.push(id);
        }
        stack.extend(arena[id].children.iter().rev().copied());
    }
    out
}

/// `document.querySelector(sel)` / `element.querySelector(sel)` — the first
/// matching element in document order, or `null`. On an element the search is
/// scoped to its subtree; `:hover` never matches (no pointer context).
pub(super) fn native_query_selector(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let selector = parse_query_selector(args, context)?;
    let scope = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let first = query_selector_nodes(&st.arena, scope, &selector)
        .into_iter()
        .next();
    drop(st);
    match first {
        Some(node_id) => node_wrapper(context, node_id),
        None => Ok(JsValue::null()),
    }
}

/// `document.querySelectorAll(sel)` / `element.querySelectorAll(sel)` — every
/// matching element in document order, returned as a static array of element
/// wrappers (indexable / `.length` / `.forEach`, like a browser NodeList).
pub(super) fn native_query_selector_all(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let selector = parse_query_selector(args, context)?;
    let scope = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let st = rc.borrow();
    let ids = query_selector_nodes(&st.arena, scope, &selector);
    drop(st);
    let arr = boa_engine::object::builtins::JsArray::new(context);
    for id in ids {
        arr.push(node_wrapper(context, id)?, context)?;
    }
    Ok(arr.into())
}

/// `element.matches(sel)` — whether this element itself matches the selector.
/// Invalid selectors throw a JS `SyntaxError` like `querySelector`; `:hover`
/// never matches (no pointer context).
pub(super) fn native_matches(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let selector = parse_query_selector(args, context)?;
    let node_id = this_node_id(this, context)?;
    let matched = with_state()
        .map(|rc| selector_chain_matches(&rc.borrow().arena, node_id, &selector))
        .unwrap_or(false);
    Ok(JsValue::from(matched))
}

/// `element.closest(sel)` — the nearest ancestor-or-self matching the
/// selector, or `null`. The walk is the DOM ancestor chain; a removed
/// (stale) node simply finds nothing.
pub(super) fn native_closest(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let selector = parse_query_selector(args, context)?;
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let mut cur = Some(node_id);
    while let Some(id) = cur {
        if selector_chain_matches(&st.arena, id, &selector) {
            drop(st);
            return node_wrapper(context, id);
        }
        cur = st.arena.get(id).and_then(|n| n.parent);
    }
    Ok(JsValue::null())
}

// ============================================================
// Element-sibling accessors (Element / CharacterData mixin)
// ============================================================

/// The nearest sibling of `node_id` on `side` (before or after it in the
/// parent's child list) that is an element node, ignoring text and comment
/// siblings. `None` when detached or when every sibling on that side is
/// non-element.
fn adjacent_element_sibling(arena: &NodeArena, node_id: NodeId, after: bool) -> Option<NodeId> {
    let parent_id = arena.get(node_id)?.parent?;
    let children = &arena[parent_id].children;
    let pos = children.iter().position(|&c| c == node_id)?;
    let range: Box<dyn Iterator<Item = &NodeId>> = if after {
        Box::new(children[pos + 1..].iter())
    } else {
        Box::new(children[..pos].iter().rev())
    };
    range
        .copied()
        .find(|&c| arena.get(c).is_some_and(|n| n.node_type_constant() == 1))
}

pub(super) fn native_get_prev_element_sibling(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let prev = with_state().and_then(|rc| {
        let st = rc.borrow();
        adjacent_element_sibling(&st.arena, node_id, false)
    });
    match prev {
        Some(id) => node_wrapper(context, id),
        None => Ok(JsValue::null()),
    }
}

pub(super) fn native_get_next_element_sibling(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let next = with_state().and_then(|rc| {
        let st = rc.borrow();
        adjacent_element_sibling(&st.arena, node_id, true)
    });
    match next {
        Some(id) => node_wrapper(context, id),
        None => Ok(JsValue::null()),
    }
}

// ============================================================
// Tree search / connectivity (getElementsByTagName* / Class,
// isConnected / getRootNode)
// ============================================================

/// Document-order (pre-order DFS) list of every element under `scope`
/// satisfying `pred`. Same walk as [`query_selector_nodes`]; scoped to the
/// caller's subtree.
fn collect_elements(
    arena: &NodeArena,
    scope: NodeId,
    pred: &dyn Fn(&crate::tdom::node::Node) -> bool,
) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = arena[scope].children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if let Some(node) = arena.get(id)
            && node.node_type_constant() == 1
            && pred(node)
        {
            out.push(id);
        }
        stack.extend(arena[id].children.iter().rev().copied());
    }
    out
}

/// Wrap a node-id list as a static JS array (indexable / `.length`, like the
/// `querySelectorAll` result).
fn ids_to_array(context: &mut Context, ids: Vec<NodeId>) -> JsResult<JsValue> {
    let arr = boa_engine::object::builtins::JsArray::new(context);
    for id in ids {
        arr.push(node_wrapper(context, id)?, context)?;
    }
    Ok(arr.into())
}

/// `getElementsByTagName(name)` — tag lookup scoped to the caller (document
/// or element). Tag names are ASCII-case-insensitive; `*` matches every
/// element. Returns a static array in document order (not a live
/// HTMLCollection).
pub(super) fn native_get_elements_by_tag_name(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let scope = this_node_id(this, context)?;
    let err = JsNativeError::typ().with_message("getElementsByTagName expects a string");
    let name = args
        .first()
        .and_then(|v| v.as_string())
        .map(|s| s.to_std_string_lossy().to_ascii_lowercase())
        .ok_or(err)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let st = rc.borrow();
    let wildcard = name == "*";
    let ids = collect_elements(&st.arena, scope, &|n| {
        wildcard || n.node_type.tag_name().is_some_and(|t| t == name)
    });
    drop(st);
    ids_to_array(context, ids)
}

/// `getElementsByClassName(classes)` — elements under the caller carrying
/// **every** whitespace-separated class token, in document order (static
/// array).
pub(super) fn native_get_elements_by_class_name(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let scope = this_node_id(this, context)?;
    let err = JsNativeError::typ().with_message("getElementsByClassName expects a string");
    let classes = args
        .first()
        .and_then(|v| v.as_string())
        .map(|s| s.to_std_string_lossy())
        .ok_or(err)?;
    let tokens: Vec<String> = classes.split_whitespace().map(str::to_string).collect();
    if tokens.is_empty() {
        return Ok(boa_engine::object::builtins::JsArray::new(context).into());
    }
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let st = rc.borrow();
    let ids = collect_elements(&st.arena, scope, &|n| tokens.iter().all(|t| n.has_class(t)));
    drop(st);
    ids_to_array(context, ids)
}

/// `node.isConnected` — whether the node is attached to the document root
/// (the chain of parents from the node reaches the session root). The
/// document itself reports `true`.
pub(super) fn native_get_is_connected(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let connected = with_state().is_some_and(|rc| {
        let st = rc.borrow();
        let mut cur = Some(node_id);
        while let Some(id) = cur {
            if id == st.root_id {
                return true;
            }
            cur = st.arena.get(id).and_then(|n| n.parent);
        }
        false
    });
    Ok(JsValue::from(connected))
}

/// `node.getRootNode()` — the top of the node's parent chain: the document
/// root for attached nodes, the outermost ancestor for detached subtrees.
pub(super) fn native_get_root_node(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let mut top = node_id;
    while let Some(parent) = st.arena.get(top).and_then(|n| n.parent) {
        top = parent;
    }
    drop(st);
    node_wrapper(context, top)
}

// ============================================================
// Element namespace / document-order comparison
// ============================================================

/// The HTML namespace URI every parsed element carries.
const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";

/// `element.namespaceURI` — the HTML namespace for every element (the
/// parser never produces foreign-content nodes).
pub(super) fn native_get_namespace_uri(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    this_node_id(this, context)?;
    Ok(js_string!(HTML_NAMESPACE).into())
}

/// `element.prefix` — `null` for HTML documents (no namespace prefix is
/// ever parsed).
pub(super) fn native_get_prefix(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    this_node_id(this, context)?;
    Ok(JsValue::null())
}

/// `element.localName` — the lowercase tag name.
pub(super) fn native_get_local_name(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let name = with_state()
        .and_then(|rc| {
            rc.borrow()
                .arena
                .get(node_id)
                .and_then(|n| n.node_type.tag_name().map(str::to_string))
        })
        .unwrap_or_default();
    Ok(js_string!(name.as_str()).into())
}

/// `Node.compareDocumentPosition(other)` bitmasks (the DOM's
/// `DOCUMENT_POSITION_*` values, not exposed as constants — there is no
/// `Node` constructor):
/// `1` disconnected · `2` preceding · `4` following · `8` contains ·
/// `0x10` contained-by · `0x20` implementation-specific.
pub(super) fn native_compare_document_position(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let undefined = JsValue::undefined();
    let other = args.first().unwrap_or(&undefined);
    let Some(other_obj) = other.as_object() else {
        return Err(JsNativeError::typ()
            .with_message("compareDocumentPosition expects a Node")
            .into());
    };
    let id_val = other_obj.get(js_string!("_nodeId"), context)?;
    let Some(n) = id_val.as_number() else {
        return Err(JsNativeError::typ()
            .with_message("compareDocumentPosition expects a Node")
            .into());
    };
    let other_id = f64_to_node_id(n);

    const DISCONNECTED: u32 = 0x01;
    const PRECEDING: u32 = 0x02;
    const FOLLOWING: u32 = 0x04;
    const CONTAINS: u32 = 0x08;
    const CONTAINED_BY: u32 = 0x10;
    const IMPLEMENTATION_SPECIFIC: u32 = 0x20;

    let Some(rc) = with_state() else {
        return Ok(JsValue::from(DISCONNECTED | IMPLEMENTATION_SPECIFIC));
    };
    let st = rc.borrow();
    let arena = &st.arena;
    // Stale ids compare as disconnected.
    if arena.get(node_id).is_none() || arena.get(other_id).is_none() {
        return Ok(JsValue::from(DISCONNECTED | IMPLEMENTATION_SPECIFIC));
    }
    // Comparing a node with itself is 0 (no bits).
    if node_id == other_id {
        return Ok(JsValue::from(0u32));
    }
    let ancestors = |from: NodeId| -> Vec<NodeId> {
        let mut chain = vec![from];
        let mut cur = arena.get(from).and_then(|n| n.parent);
        while let Some(id) = cur {
            chain.push(id);
            cur = arena.get(id).and_then(|n| n.parent);
        }
        chain
    };
    let mine = ancestors(node_id);
    let theirs = ancestors(other_id);
    // Connected = the two root chains share a top node.
    let connected = match (mine.last(), theirs.last()) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    };
    if !connected {
        return Ok(JsValue::from(DISCONNECTED | IMPLEMENTATION_SPECIFIC));
    }
    if mine
        .iter()
        .position(|&id| id == other_id)
        .is_some_and(|d| d > 0)
    {
        // `other` is an ancestor of this node.
        return Ok(JsValue::from(CONTAINED_BY | PRECEDING));
    }
    if theirs
        .iter()
        .position(|&id| id == node_id)
        .is_some_and(|d| d > 0)
    {
        // `other` is a descendant of this node.
        return Ok(JsValue::from(CONTAINS | FOLLOWING));
    }
    // Siblings under a common ancestor: pre-order document order decides.
    // Walk the common ancestor's subtree, stopping as soon as both nodes
    // have been numbered.
    let common = mine
        .iter()
        .rev()
        .zip(theirs.iter().rev())
        .find(|(a, b)| a == b)
        .map(|(a, _)| *a);
    let Some(common) = common else {
        return Ok(JsValue::from(DISCONNECTED | IMPLEMENTATION_SPECIFIC));
    };
    let mut order: std::collections::HashMap<NodeId, usize> = std::collections::HashMap::new();
    let mut stack = vec![common];
    while let Some(id) = stack.pop() {
        let done = order.contains_key(&node_id) && order.contains_key(&other_id);
        if done {
            break;
        }
        if !order.contains_key(&id) {
            order.insert(id, order.len());
        }
        if let Some(n) = arena.get(id) {
            stack.extend(n.children.iter().rev().copied());
        }
    }
    match (order.get(&node_id), order.get(&other_id)) {
        // The result describes `other` relative to `this`: `other` numbered
        // later (follows in document order) → FOLLOWING; earlier → PRECEDING.
        (Some(a), Some(b)) if a < b => Ok(JsValue::from(FOLLOWING)),
        (Some(_), Some(_)) => Ok(JsValue::from(PRECEDING)),
        // Defensive fallback (should not trigger for a well-formed DOM).
        _ => Ok(JsValue::from(DISCONNECTED | IMPLEMENTATION_SPECIFIC)),
    }
}
