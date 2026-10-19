//! Element wrapper construction: one builder serves every node kind.

use boa_engine::{
    Context, JsResult, JsValue, NativeFunction, js_string, object::ObjectInitializer,
    property::Attribute,
};

use crate::tdom::node::{NodeId, NodeType};

use super::{
    attrs::{
        native_get_attribute, native_get_id, native_remove_attribute, native_set_attribute,
        native_set_id,
    },
    content::{
        native_get_data, native_get_inner_html, native_get_node_value, native_get_text,
        native_get_value, native_set_data, native_set_inner_html, native_set_node_value,
        native_set_text, native_set_value,
    },
    events::{native_add_event_listener, native_blur, native_focus, native_remove_event_listener},
    global_attrs::{
        native_get_class, native_get_class_list, native_get_dataset, native_get_hidden,
        native_set_class, native_set_hidden,
    },
    js_fn,
    mutation::{
        native_append_child, native_clone_node, native_insert_before, native_remove_child,
        native_replace_child,
    },
    node_id_to_f64,
    style::native_get_style,
    traverse::{
        native_contains, native_get_base_uri, native_get_child_nodes, native_get_first_child,
        native_get_last_child, native_get_next_sibling, native_get_node_name, native_get_node_type,
        native_get_owner_document, native_get_parent, native_get_parent_element,
        native_get_prev_sibling, native_get_tag_name, native_has_child_nodes, native_is_equal_node,
        native_is_same_node, native_query_selector, native_query_selector_all,
    },
    with_state,
};

/// Build a fresh wrapper object for `node_id`. The surface depends on the
/// node kind: every node exposes the common Node interface (`nodeType` /
/// `nodeName` / `textContent` / `parentNode` / `childNodes` / `appendChild`
/// / …), elements add attribute accessors, and Text / Comment nodes add
/// `data`.
pub(super) fn make_node_wrapper(context: &mut Context, node_id: NodeId) -> JsResult<JsValue> {
    let node_key = node_id_to_f64(node_id);
    let hidden = Attribute::READONLY | Attribute::NON_ENUMERABLE;

    let (is_element, is_text_like) = {
        let Some(rc) = with_state() else {
            return Ok(JsValue::null());
        };
        let st = rc.borrow();
        let Some(node) = st.arena.get(node_id) else {
            return Ok(JsValue::null());
        };
        (
            node.node_type_constant() == 1,
            matches!(node.node_type, NodeType::Text | NodeType::Comment),
        )
    };

    // Accessor functions must be built before the `ObjectInitializer` takes
    // its `&mut Context` borrow.
    let get_node_type = js_fn(context, NativeFunction::from_fn_ptr(native_get_node_type));
    let get_node_name = js_fn(context, NativeFunction::from_fn_ptr(native_get_node_name));
    let get_node_value = js_fn(context, NativeFunction::from_fn_ptr(native_get_node_value));
    let set_node_value = js_fn(context, NativeFunction::from_fn_ptr(native_set_node_value));
    let get_text = js_fn(context, NativeFunction::from_fn_ptr(native_get_text));
    let set_text = js_fn(context, NativeFunction::from_fn_ptr(native_set_text));
    let get_parent = js_fn(context, NativeFunction::from_fn_ptr(native_get_parent));
    let get_parent_element = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_parent_element),
    );
    let get_child_nodes = js_fn(context, NativeFunction::from_fn_ptr(native_get_child_nodes));
    let get_first_child = js_fn(context, NativeFunction::from_fn_ptr(native_get_first_child));
    let get_last_child = js_fn(context, NativeFunction::from_fn_ptr(native_get_last_child));
    let get_prev_sibling = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_prev_sibling),
    );
    let get_next_sibling = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_next_sibling),
    );
    let get_owner_doc = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_owner_document),
    );
    let get_base_uri = js_fn(context, NativeFunction::from_fn_ptr(native_get_base_uri));
    let get_data = js_fn(context, NativeFunction::from_fn_ptr(native_get_data));
    let set_data = js_fn(context, NativeFunction::from_fn_ptr(native_set_data));
    let get_tag = js_fn(context, NativeFunction::from_fn_ptr(native_get_tag_name));
    let get_class = js_fn(context, NativeFunction::from_fn_ptr(native_get_class));
    let set_class = js_fn(context, NativeFunction::from_fn_ptr(native_set_class));
    let get_class_list = js_fn(context, NativeFunction::from_fn_ptr(native_get_class_list));
    let get_id = js_fn(context, NativeFunction::from_fn_ptr(native_get_id));
    let set_id = js_fn(context, NativeFunction::from_fn_ptr(native_set_id));
    let get_style = js_fn(context, NativeFunction::from_fn_ptr(native_get_style));
    let get_value = js_fn(context, NativeFunction::from_fn_ptr(native_get_value));
    let set_value = js_fn(context, NativeFunction::from_fn_ptr(native_set_value));
    let get_inner_html = js_fn(context, NativeFunction::from_fn_ptr(native_get_inner_html));
    let set_inner_html = js_fn(context, NativeFunction::from_fn_ptr(native_set_inner_html));
    let get_hidden = js_fn(context, NativeFunction::from_fn_ptr(native_get_hidden));
    let set_hidden = js_fn(context, NativeFunction::from_fn_ptr(native_set_hidden));
    let get_dataset = js_fn(context, NativeFunction::from_fn_ptr(native_get_dataset));

    let mut init = ObjectInitializer::new(context);
    init.property(js_string!("_nodeId"), node_key, hidden);

    // Common Node interface.
    init.accessor(
        js_string!("nodeType"),
        Some(get_node_type),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("nodeName"),
        Some(get_node_name),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("textContent"),
        Some(get_text),
        Some(set_text),
        Attribute::all(),
    );
    init.accessor(
        js_string!("nodeValue"),
        Some(get_node_value),
        Some(set_node_value),
        Attribute::all(),
    );
    init.accessor(
        js_string!("parentNode"),
        Some(get_parent),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("parentElement"),
        Some(get_parent_element),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("childNodes"),
        Some(get_child_nodes),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("firstChild"),
        Some(get_first_child),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("lastChild"),
        Some(get_last_child),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("previousSibling"),
        Some(get_prev_sibling),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("nextSibling"),
        Some(get_next_sibling),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("ownerDocument"),
        Some(get_owner_doc),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("baseURI"),
        Some(get_base_uri),
        None,
        Attribute::all(),
    );

    init.function(
        NativeFunction::from_fn_ptr(native_has_child_nodes),
        js_string!("hasChildNodes"),
        0,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_append_child),
        js_string!("appendChild"),
        1,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_remove_child),
        js_string!("removeChild"),
        1,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_insert_before),
        js_string!("insertBefore"),
        2,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_replace_child),
        js_string!("replaceChild"),
        2,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_clone_node),
        js_string!("cloneNode"),
        1,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_contains),
        js_string!("contains"),
        1,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_is_same_node),
        js_string!("isSameNode"),
        1,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_is_equal_node),
        js_string!("isEqualNode"),
        1,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_add_event_listener),
        js_string!("addEventListener"),
        2,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_remove_event_listener),
        js_string!("removeEventListener"),
        2,
    );

    if is_element {
        init.accessor(js_string!("tagName"), Some(get_tag), None, Attribute::all());
        init.accessor(
            js_string!("className"),
            Some(get_class),
            Some(set_class),
            Attribute::all(),
        );
        init.accessor(
            js_string!("classList"),
            Some(get_class_list),
            None,
            Attribute::all(),
        );
        init.accessor(
            js_string!("id"),
            Some(get_id),
            Some(set_id),
            Attribute::all(),
        );
        init.accessor(
            js_string!("value"),
            Some(get_value),
            Some(set_value),
            Attribute::all(),
        );
        init.accessor(js_string!("style"), Some(get_style), None, Attribute::all());
        init.accessor(
            js_string!("innerHTML"),
            Some(get_inner_html),
            Some(set_inner_html),
            Attribute::all(),
        );
        init.accessor(
            js_string!("hidden"),
            Some(get_hidden),
            Some(set_hidden),
            Attribute::all(),
        );
        init.accessor(
            js_string!("dataset"),
            Some(get_dataset),
            None,
            Attribute::all(),
        );
        init.function(
            NativeFunction::from_fn_ptr(native_get_attribute),
            js_string!("getAttribute"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_set_attribute),
            js_string!("setAttribute"),
            2,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_remove_attribute),
            js_string!("removeAttribute"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_query_selector),
            js_string!("querySelector"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_query_selector_all),
            js_string!("querySelectorAll"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_focus),
            js_string!("focus"),
            0,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_blur),
            js_string!("blur"),
            0,
        );
    }
    if is_text_like {
        init.accessor(
            js_string!("data"),
            Some(get_data),
            Some(set_data),
            Attribute::all(),
        );
    }
    let obj = init.build();
    Ok(obj.into())
}
