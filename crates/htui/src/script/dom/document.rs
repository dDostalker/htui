//! Document-level natives: factory methods (`createElement`,
//! `createTextNode`, …), lookups (`getElementById`) and the `documentElement`
//! / `body` / `head` / `doctype` getters.

use boa_engine::{Context, JsResult, JsValue, js_string};

use crate::tdom::node::{Node, NodeType};

use super::{node_wrapper, with_state};

pub(super) fn native_get_element_by_id(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let id = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?;
    let Some(id) = id else {
        return Ok(JsValue::null());
    };
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let found = st
        .arena
        .iter()
        .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id.as_str()))
        .map(|(nid, _)| nid);
    match found {
        Some(node_id) => node_wrapper(context, node_id),
        None => Ok(JsValue::null()),
    }
}

pub(super) fn native_create_text_node(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let data = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let mut st = rc.borrow_mut();
    // Detached text node (renders nothing until appended).
    let text_id = st.arena.insert(Node::text(&data));
    drop(st);
    node_wrapper(context, text_id)
}

pub(super) fn native_create_comment(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let data = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let mut st = rc.borrow_mut();
    let comment_id = st.arena.insert(Node::comment(&data));
    drop(st);
    node_wrapper(context, comment_id)
}

pub(super) fn native_create_document_fragment(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let mut st = rc.borrow_mut();
    let fragment_id = st.arena.insert(Node::fragment());
    drop(st);
    node_wrapper(context, fragment_id)
}

/// Find the first `<html>` descendant of the document root (`documentElement`).
pub(super) fn native_get_document_element(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    find_descendant_by_type(context, NodeType::Html)
}

/// Find the `<body>` element (`document.body`).
pub(super) fn native_get_document_body(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    find_descendant_by_type(context, NodeType::Body)
}

/// Find the `<head>` element (`document.head`).
pub(super) fn native_get_document_head(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    find_descendant_by_type(context, NodeType::Head)
}

/// Find the `<!DOCTYPE>` node (`document.doctype`).
pub(super) fn native_get_document_doctype(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    find_descendant_by_type(context, NodeType::DocumentType)
}

/// Depth-first search for the first node of `node_type` under the root.
fn find_descendant_by_type(context: &mut Context, node_type: NodeType) -> JsResult<JsValue> {
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let found = Node::descendants(&st.arena, st.root_id)
        .find(|(_, n)| n.node_type == node_type)
        .map(|(id, _)| id);
    match found {
        Some(id) => node_wrapper(context, id),
        None => Ok(JsValue::null()),
    }
}

/// `document.title` getter — the concatenated text of the first `<title>`
/// element (empty string when the document has none).
pub(super) fn native_get_document_title(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let title = Node::descendants(&st.arena, st.root_id)
        .find(|(_, n)| n.node_type == NodeType::Title)
        .map(|(id, _)| {
            Node::descendants(&st.arena, id)
                .filter(|(_, n)| n.node_type == NodeType::Text)
                .map(|(_, n)| n.data.as_deref().unwrap_or_default())
                .collect::<String>()
        })
        .unwrap_or_default();
    Ok(js_string!(title).into())
}

/// `document.title` setter — write the first `<title>`'s text; when the
/// document has no `<title>`, create one under `<head>` (or directly under
/// the root when there is no head). The render thread re-reads the title on
/// the next dirty repaint, so the terminal title follows.
pub(super) fn native_set_document_title(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let value = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let title_id = Node::descendants(&st.arena, st.root_id)
        .find(|(_, n)| n.node_type == NodeType::Title)
        .map(|(id, _)| id);
    if let Some(id) = title_id {
        let text_id = st
            .arena
            .get(id)
            .map(|n| n.children.clone())
            .unwrap_or_default()
            .into_iter()
            .find(|&c| st.arena.get(c).map(|n| n.node_type) == Some(NodeType::Text));
        match text_id {
            Some(text_id) => {
                if let Some(t) = st.arena.get_mut(text_id) {
                    t.data = Some(value);
                }
            }
            None => {
                let text_id = st.arena.insert(Node::text(&value));
                if let Some(title) = st.arena.get_mut(id) {
                    title.children.push(text_id);
                }
                if let Some(t) = st.arena.get_mut(text_id) {
                    t.parent = Some(id);
                }
            }
        }
        super::mark_dirty(&mut st, id);
        return Ok(JsValue::undefined());
    }
    // No <title> yet: create one under the first <head> (fallback: the root).
    let host = Node::descendants(&st.arena, st.root_id)
        .find(|(_, n)| n.node_type == NodeType::Head)
        .map(|(id, _)| id)
        .unwrap_or(st.root_id);
    let title_id = st.arena.insert(Node::from(NodeType::Title));
    let text_id = st.arena.insert(Node::text(&value));
    if let Some(title) = st.arena.get_mut(title_id) {
        title.children.push(text_id);
    }
    if let Some(t) = st.arena.get_mut(text_id) {
        t.parent = Some(title_id);
    }
    if let Some(h) = st.arena.get_mut(host) {
        h.children.push(title_id);
    }
    if let Some(t) = st.arena.get_mut(title_id) {
        t.parent = Some(host);
    }
    super::mark_dirty(&mut st, host);
    Ok(JsValue::undefined())
}

pub(super) fn native_create_element(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let tag = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?;
    let tag = tag.unwrap_or_else(|| "div".to_string());
    let node_type = crate::html::html_parse::tag_name_to_node_type(&tag).unwrap_or(NodeType::Div);
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let mut st = rc.borrow_mut();
    let node_id = st.arena.insert(Node::from(node_type));
    // Not attached to the document yet.
    drop(st);
    node_wrapper(context, node_id)
}

// ============================================================
// Document metadata (constant snapshots of the environment)
// ============================================================

/// `document.characterSet` / `charset` / `inputEncoding` — the parser
/// always decodes UTF-8.
pub(super) fn native_get_character_set(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    Ok(js_string!("UTF-8").into())
}

/// `document.contentType` — always `text/html`.
pub(super) fn native_get_content_type(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    Ok(js_string!("text/html").into())
}

/// `document.compatMode` — the engine always lays out in standards mode.
pub(super) fn native_get_compat_mode(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    Ok(js_string!("CSS1Compat").into())
}

/// `document.URL` / `document.documentURI` — the `<base href>` when the
/// document declares one, otherwise an empty string (there is no network
/// origin in htui).
pub(super) fn native_get_document_uri(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let uri = crate::tdom::node::document_base_href(&st.arena, st.root_id).unwrap_or_default();
    Ok(js_string!(uri).into())
}

/// `document.hasFocus()` — always `true`: a terminal UI owns the foreground
/// for its whole lifetime (the documented approximation; focus of a
/// specific field is separate, see `focus()`).
pub(super) fn native_has_focus(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::from(true))
}

/// `document.visibilityState` — always `"visible"` (a terminal app cannot
/// be backgrounded by the page).
pub(super) fn native_get_visibility_state(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    Ok(js_string!("visible").into())
}

/// `document.hidden` — always `false` (see [`native_get_visibility_state`]).
pub(super) fn native_get_hidden(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::from(false))
}
