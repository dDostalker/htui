//! Element wrapper construction: one builder serves every node kind.

use boa_engine::{
    Context, JsResult, JsValue, NativeFunction, js_string, object::ObjectInitializer,
    property::Attribute,
};

use crate::tdom::node::{NodeId, NodeType};

use super::{
    attrs::{
        native_get_attribute, native_get_attributes, native_get_id, native_has_attribute,
        native_remove_attribute, native_set_attribute, native_set_id, native_toggle_attribute,
    },
    content::{
        native_append_data, native_delete_data, native_get_data, native_get_data_length,
        native_get_inner_html, native_get_node_value, native_get_outer_html, native_get_text,
        native_get_value, native_insert_data, native_replace_data, native_set_data,
        native_set_inner_html, native_set_node_value, native_set_text, native_set_value,
        native_split_text, native_substring_data,
    },
    events::{
        native_add_event_listener, native_blur, native_click, native_dispatch_event, native_focus,
        native_remove_event_listener,
    },
    global_attrs::{
        native_get_class, native_get_class_list, native_get_dataset, native_get_hidden,
        native_set_class, native_set_hidden,
    },
    js_fn,
    mutation::{
        native_after, native_append, native_append_child, native_before, native_clone_node,
        native_insert_adjacent_element, native_insert_adjacent_html, native_insert_adjacent_text,
        native_insert_before, native_normalize, native_prepend, native_remove, native_remove_child,
        native_replace_child, native_replace_children, native_replace_with, native_set_outer_html,
    },
    node_id_to_f64,
    style::native_get_style,
    traverse::{
        native_closest, native_compare_document_position, native_contains, native_get_base_uri,
        native_get_child_nodes, native_get_elements_by_class_name, native_get_elements_by_tag_name,
        native_get_first_child, native_get_is_connected, native_get_last_child,
        native_get_local_name, native_get_namespace_uri, native_get_next_element_sibling,
        native_get_next_sibling, native_get_node_name, native_get_node_type,
        native_get_owner_document, native_get_parent, native_get_parent_element, native_get_prefix,
        native_get_prev_element_sibling, native_get_prev_sibling, native_get_root_node,
        native_get_tag_name, native_has_child_nodes, native_is_equal_node, native_is_same_node,
        native_matches, native_query_selector, native_query_selector_all,
    },
    view::{
        native_get_bounding_client_rect, native_get_client_height, native_get_client_left,
        native_get_client_top, native_get_client_width, native_get_scroll_height,
        native_get_scroll_left, native_get_scroll_top, native_get_scroll_width,
        native_scroll_into_view, native_set_scroll_top,
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

    let (is_element, is_text_like, is_text, is_parent_node) = {
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
            node.node_type == NodeType::Text,
            // ParentNode mixin domain: elements, the document and fragments.
            matches!(
                node.node_type,
                NodeType::Document | NodeType::DocumentFragment
            ) || node.node_type_constant() == 1,
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
    let get_data_length = js_fn(context, NativeFunction::from_fn_ptr(native_get_data_length));
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
    let get_outer_html = js_fn(context, NativeFunction::from_fn_ptr(native_get_outer_html));
    let set_outer_html = js_fn(context, NativeFunction::from_fn_ptr(native_set_outer_html));
    let get_attributes = js_fn(context, NativeFunction::from_fn_ptr(native_get_attributes));
    let get_hidden = js_fn(context, NativeFunction::from_fn_ptr(native_get_hidden));
    let set_hidden = js_fn(context, NativeFunction::from_fn_ptr(native_set_hidden));
    let get_dataset = js_fn(context, NativeFunction::from_fn_ptr(native_get_dataset));
    let get_prev_elem_sibling = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_prev_element_sibling),
    );
    let get_next_elem_sibling = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_next_element_sibling),
    );
    let get_is_connected = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_is_connected),
    );
    let get_namespace_uri = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_namespace_uri),
    );
    let get_prefix = js_fn(context, NativeFunction::from_fn_ptr(native_get_prefix));
    let get_local_name = js_fn(context, NativeFunction::from_fn_ptr(native_get_local_name));
    let get_client_width = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_client_width),
    );
    let get_client_height = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_client_height),
    );
    let get_client_top = js_fn(context, NativeFunction::from_fn_ptr(native_get_client_top));
    let get_client_left = js_fn(context, NativeFunction::from_fn_ptr(native_get_client_left));
    let get_scroll_top = js_fn(context, NativeFunction::from_fn_ptr(native_get_scroll_top));
    let set_scroll_top = js_fn(context, NativeFunction::from_fn_ptr(native_set_scroll_top));
    let get_scroll_left = js_fn(context, NativeFunction::from_fn_ptr(native_get_scroll_left));
    let get_scroll_height = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_scroll_height),
    );
    let get_scroll_width = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_scroll_width),
    );

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
    init.function(
        NativeFunction::from_fn_ptr(native_dispatch_event),
        js_string!("dispatchEvent"),
        1,
    );

    // Node connectivity / structure helpers apply to every node kind.
    init.accessor(
        js_string!("isConnected"),
        Some(get_is_connected),
        None,
        Attribute::all(),
    );
    init.function(
        NativeFunction::from_fn_ptr(native_get_root_node),
        js_string!("getRootNode"),
        0,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_normalize),
        js_string!("normalize"),
        0,
    );

    // `previousElementSibling` / `nextElementSibling` belong to the Element
    // and CharacterData mixins — exposed on element and text-like wrappers
    // only, never on the document root or fragments.
    if is_element || is_text_like {
        init.accessor(
            js_string!("previousElementSibling"),
            Some(get_prev_elem_sibling),
            None,
            Attribute::all(),
        );
        init.accessor(
            js_string!("nextElementSibling"),
            Some(get_next_elem_sibling),
            None,
            Attribute::all(),
        );
    }

    if is_element {
        init.accessor(js_string!("tagName"), Some(get_tag), None, Attribute::all());
        init.accessor(
            js_string!("namespaceURI"),
            Some(get_namespace_uri),
            None,
            Attribute::all(),
        );
        init.accessor(
            js_string!("prefix"),
            Some(get_prefix),
            None,
            Attribute::all(),
        );
        init.accessor(
            js_string!("localName"),
            Some(get_local_name),
            None,
            Attribute::all(),
        );
        init.function(
            NativeFunction::from_fn_ptr(native_compare_document_position),
            js_string!("compareDocumentPosition"),
            1,
        );
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
            js_string!("outerHTML"),
            Some(get_outer_html),
            Some(set_outer_html),
            Attribute::all(),
        );
        init.accessor(
            js_string!("attributes"),
            Some(get_attributes),
            None,
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
            NativeFunction::from_fn_ptr(native_has_attribute),
            js_string!("hasAttribute"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_toggle_attribute),
            js_string!("toggleAttribute"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_matches),
            js_string!("matches"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_closest),
            js_string!("closest"),
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
        init.function(
            NativeFunction::from_fn_ptr(native_click),
            js_string!("click"),
            0,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_scroll_into_view),
            js_string!("scrollIntoView"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_get_bounding_client_rect),
            js_string!("getBoundingClientRect"),
            0,
        );
        init.accessor(
            js_string!("clientWidth"),
            Some(get_client_width),
            None,
            Attribute::all(),
        );
        init.accessor(
            js_string!("clientHeight"),
            Some(get_client_height),
            None,
            Attribute::all(),
        );
        init.accessor(
            js_string!("clientTop"),
            Some(get_client_top),
            None,
            Attribute::all(),
        );
        init.accessor(
            js_string!("clientLeft"),
            Some(get_client_left),
            None,
            Attribute::all(),
        );
        init.accessor(
            js_string!("scrollTop"),
            Some(get_scroll_top),
            Some(set_scroll_top),
            Attribute::all(),
        );
        init.accessor(
            js_string!("scrollLeft"),
            Some(get_scroll_left),
            None,
            Attribute::all(),
        );
        init.accessor(
            js_string!("scrollHeight"),
            Some(get_scroll_height),
            None,
            Attribute::all(),
        );
        init.accessor(
            js_string!("scrollWidth"),
            Some(get_scroll_width),
            None,
            Attribute::all(),
        );
        init.function(
            NativeFunction::from_fn_ptr(native_insert_adjacent_element),
            js_string!("insertAdjacentElement"),
            2,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_insert_adjacent_text),
            js_string!("insertAdjacentText"),
            2,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_insert_adjacent_html),
            js_string!("insertAdjacentHTML"),
            2,
        );
    }
    if is_text_like {
        init.accessor(
            js_string!("data"),
            Some(get_data),
            Some(set_data),
            Attribute::all(),
        );
        init.accessor(
            js_string!("length"),
            Some(get_data_length),
            None,
            Attribute::all(),
        );
        init.function(
            NativeFunction::from_fn_ptr(native_substring_data),
            js_string!("substringData"),
            2,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_append_data),
            js_string!("appendData"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_insert_data),
            js_string!("insertData"),
            2,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_delete_data),
            js_string!("deleteData"),
            2,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_replace_data),
            js_string!("replaceData"),
            3,
        );
    }
    if is_text {
        init.function(
            NativeFunction::from_fn_ptr(native_split_text),
            js_string!("splitText"),
            1,
        );
    }
    // ChildNode mixin: element and CharacterData wrappers (the DocumentType
    // exclusion of the spec is unreachable — doctype wrappers are never
    // built here).
    if is_element || is_text_like {
        init.function(
            NativeFunction::from_fn_ptr(native_remove),
            js_string!("remove"),
            0,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_before),
            js_string!("before"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_after),
            js_string!("after"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_replace_with),
            js_string!("replaceWith"),
            1,
        );
    }
    // ParentNode mixin: elements, the document and document fragments.
    if is_parent_node {
        init.function(
            NativeFunction::from_fn_ptr(native_append),
            js_string!("append"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_prepend),
            js_string!("prepend"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_replace_children),
            js_string!("replaceChildren"),
            0,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_get_elements_by_tag_name),
            js_string!("getElementsByTagName"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_get_elements_by_class_name),
            js_string!("getElementsByClassName"),
            1,
        );
    }
    let obj = init.build();
    Ok(obj.into())
}
