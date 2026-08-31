//! Document-level natives: factory methods (`createElement`,
//! `createTextNode`, …), lookups (`getElementById`) and the `documentElement`
//! / `body` / `head` / `doctype` getters.

use boa_engine::{Context, JsResult, JsValue};

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
