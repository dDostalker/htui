//! Tests for the DOM node object model: `nodeType` / `nodeName` / `tagName`,
//! live `childNodes` (Proxy-backed), sibling / parent traversal, structural
//! mutation (`insertBefore` / `replaceChild` / `cloneNode`), Text / Comment /
//! DocumentFragment nodes and the `document` accessors. Pure in-memory — no
//! terminal required.

use htui::events::EventBindings;
use htui::script::ScriptEngine;
use htui::tdom::node::NodeType;

/// Build a script engine over `html` (its `<script>` blocks run immediately).
fn engine(html: &str) -> ScriptEngine {
    ScriptEngine::new(html, EventBindings::default()).expect("engine initializes")
}

/// Run a statement / declaration script.
fn eval(engine: &mut ScriptEngine, code: &str) {
    engine.eval_source(code).expect("eval must succeed");
}

/// Evaluate an expression as a string (`String(expr)`).
fn eval_str(engine: &mut ScriptEngine, expr: &str) -> String {
    let v = engine
        .eval_source(&format!("String({expr})"))
        .expect("eval must succeed");
    v.as_string()
        .map(|s| s.to_std_string_lossy())
        .unwrap_or_default()
}

fn find_id(arena: &htui::tdom::node::NodeArena, id: &str) -> htui::tdom::node::NodeId {
    arena
        .iter()
        .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id))
        .map(|(nid, _)| nid)
        .expect("node with id")
}

#[test]
fn node_type_node_name_tag_name() {
    let mut e = engine(r#"<!DOCTYPE html><html><body><div id="d">hi</div></body></html>"#);
    assert_eq!(eval_str(&mut e, "document.nodeType"), "9");
    assert_eq!(eval_str(&mut e, "document.nodeName"), "#document");
    assert_eq!(eval_str(&mut e, "document.documentElement.tagName"), "HTML");
    assert_eq!(
        eval_str(&mut e, "document.getElementById('d').nodeType"),
        "1"
    );
    assert_eq!(
        eval_str(&mut e, "document.getElementById('d').nodeName"),
        "DIV"
    );
    assert_eq!(
        eval_str(&mut e, "document.getElementById('d').tagName"),
        "DIV"
    );
    // Text node: nodeType 3 / #text, content exposed via data / nodeValue.
    eval(&mut e, "var t = document.getElementById('d').childNodes[0]");
    assert_eq!(eval_str(&mut e, "t.nodeType"), "3");
    assert_eq!(eval_str(&mut e, "t.nodeName"), "#text");
    assert_eq!(eval_str(&mut e, "t.data"), "hi");
    assert_eq!(eval_str(&mut e, "t.nodeValue"), "hi");
    assert_eq!(eval_str(&mut e, "t.textContent"), "hi");
    assert_eq!(eval_str(&mut e, "document.doctype.nodeType"), "10");
    assert_eq!(eval_str(&mut e, "document.doctype.nodeName"), "html");
}

#[test]
fn parser_keeps_whitespace_text_comments_and_doctype() {
    // The parser preserves whitespace-only text nodes, comments and the
    // doctype so `childNodes` stays faithful to the source.
    let e = engine(
        "<!DOCTYPE html>\n\
         <div id=\"d\">\n  <span>x</span><!-- note -->\n</div>",
    );
    let arena = e.arena();
    assert!(
        arena
            .iter()
            .any(|(_, n)| n.node_type == NodeType::DocumentType),
        "doctype node must be kept"
    );
    let d = find_id(&arena, "d");
    let kids: Vec<NodeType> = arena[d]
        .children
        .iter()
        .map(|&c| arena[c].node_type)
        .collect();
    assert!(
        kids.contains(&NodeType::Text),
        "whitespace text nodes kept: {kids:?}"
    );
    assert!(
        kids.contains(&NodeType::Comment),
        "comment node kept: {kids:?}"
    );
    // The leading whitespace text node holds the exact source bytes.
    let first = arena[d].children[0];
    assert_eq!(arena[first].node_type, NodeType::Text);
    assert_eq!(arena[first].data.as_deref(), Some("\n  "));
    // The comment node carries its content without the delimiters.
    let comment = arena[d]
        .children
        .iter()
        .find(|&&c| arena[c].node_type == NodeType::Comment)
        .copied()
        .expect("comment child");
    assert_eq!(arena[comment].data.as_deref(), Some(" note "));
}

#[test]
fn child_nodes_is_a_true_live_list() {
    // D2: a held `childNodes` reference reflects later mutations — `length`
    // and newly appended indices are read live from the arena.
    let mut e = engine(r#"<div id="d"><span>a</span></div>"#);
    eval(&mut e, "var nl = document.getElementById('d').childNodes");
    assert_eq!(eval_str(&mut e, "nl.length"), "1");
    eval(
        &mut e,
        "document.getElementById('d').appendChild(document.createElement('p'))",
    );
    assert_eq!(
        eval_str(&mut e, "nl.length"),
        "2",
        "held list must see the append"
    );
    assert_eq!(
        eval_str(&mut e, "nl[1].tagName"),
        "P",
        "new index must resolve"
    );
    // Item identity is stable (wrapper cache).
    assert_eq!(eval_str(&mut e, "nl[0] === nl[0]"), "true");
    // `in` and iteration reflect the live list.
    assert_eq!(eval_str(&mut e, "'1' in nl"), "true");
    eval(
        &mut e,
        "var out = []; nl.forEach(function (n) { out.push(n.tagName); }); globalThis.__out = out",
    );
    assert_eq!(eval_str(&mut e, "globalThis.__out.join(',')"), "SPAN,P");
}

#[test]
fn parent_and_sibling_traversal() {
    let mut e = engine(r#"<div id="d"><span id="a">A</span><span id="b">B</span></div>"#);
    eval(&mut e, "var a = document.getElementById('a')");
    let d = "document.getElementById('d')";
    assert_eq!(eval_str(&mut e, "a.parentNode.id"), "d");
    assert_eq!(eval_str(&mut e, "a.parentElement.id"), "d");
    assert_eq!(eval_str(&mut e, "a.nextSibling.id"), "b");
    assert_eq!(
        eval_str(&mut e, "document.getElementById('b').previousSibling.id"),
        "a"
    );
    assert_eq!(eval_str(&mut e, &format!("{d}.firstChild.id")), "a");
    assert_eq!(eval_str(&mut e, &format!("{d}.lastChild.id")), "b");
    assert_eq!(eval_str(&mut e, "a.firstChild.nodeType"), "3");
    assert_eq!(eval_str(&mut e, "a.lastChild.data"), "A");
    assert_eq!(eval_str(&mut e, "a.hasChildNodes()"), "true");
    assert_eq!(eval_str(&mut e, "a.nextSibling.nextSibling"), "null");
}

#[test]
fn insert_before_and_replace_child() {
    let mut e = engine(r#"<div id="d"><span id="a">A</span><span id="b">B</span></div>"#);
    eval(
        &mut e,
        "var d = document.getElementById('d'); var b = document.getElementById('b'); \
         var p = document.createElement('p'); p.id = 'c'; d.insertBefore(p, b)",
    );
    assert_eq!(eval_str(&mut e, "d.childNodes.length"), "3");
    assert_eq!(eval_str(&mut e, "d.childNodes[1].id"), "c");
    assert_eq!(eval_str(&mut e, "d.childNodes[2].id"), "b");
    // insertBefore with a null reference appends.
    eval(
        &mut e,
        "var q = document.createElement('q'); q.id = 'z'; d.insertBefore(q, null)",
    );
    assert_eq!(eval_str(&mut e, "d.lastChild.id"), "z");
    // replaceChild swaps in place and returns the removed child.
    eval(
        &mut e,
        "var r = d.replaceChild(q, document.getElementById('a'))",
    );
    assert_eq!(eval_str(&mut e, "r.id"), "a");
    assert_eq!(eval_str(&mut e, "d.childNodes[0].id"), "z");
    assert_eq!(
        eval_str(&mut e, "document.getElementById('a').parentNode"),
        "null"
    );
}

#[test]
fn clone_node_deep_is_detached_and_structural() {
    let mut e = engine(r#"<div id="d"><span class="s">text</span></div>"#);
    eval(
        &mut e,
        "var c = document.getElementById('d').cloneNode(true)",
    );
    assert_eq!(eval_str(&mut e, "c.nodeType"), "1");
    assert_eq!(
        eval_str(&mut e, "c.parentNode === null"),
        "true",
        "clone is detached"
    );
    assert_eq!(eval_str(&mut e, "c.childNodes.length"), "1");
    assert_eq!(eval_str(&mut e, "c.childNodes[0].tagName"), "SPAN");
    assert_eq!(eval_str(&mut e, "c.childNodes[0].textContent"), "text");
    // Structural equality vs identity.
    assert_eq!(
        eval_str(&mut e, "c.isEqualNode(document.getElementById('d'))"),
        "true"
    );
    assert_eq!(
        eval_str(&mut e, "c.isSameNode(document.getElementById('d'))"),
        "false"
    );
    // The clone is independent: mutating it leaves the original intact.
    eval(&mut e, "c.childNodes[0].textContent = 'changed'");
    assert_eq!(
        eval_str(&mut e, "document.getElementById('d').textContent"),
        "text"
    );
    // Shallow clone drops children.
    eval(
        &mut e,
        "var s = document.getElementById('d').cloneNode(false)",
    );
    assert_eq!(eval_str(&mut e, "s.childNodes.length"), "0");
}

#[test]
fn create_text_comment_and_fragment_nodes() {
    let mut e = engine(r#"<div id="d"></div>"#);
    eval(&mut e, "var t = document.createTextNode('hi')");
    assert_eq!(eval_str(&mut e, "t.nodeType"), "3");
    assert_eq!(eval_str(&mut e, "t.data"), "hi");
    assert_eq!(eval_str(&mut e, "t.nodeValue"), "hi");
    assert_eq!(
        eval_str(&mut e, "t.parentNode === null"),
        "true",
        "detached until appended"
    );
    eval(&mut e, "document.getElementById('d').appendChild(t)");
    assert_eq!(
        eval_str(&mut e, "document.getElementById('d').textContent"),
        "hi"
    );
    assert_eq!(
        eval_str(&mut e, "document.getElementById('d').childNodes.length"),
        "1"
    );
    // nodeValue writes back into data.
    eval(&mut e, "t.nodeValue = 'yo'");
    assert_eq!(eval_str(&mut e, "t.data"), "yo");

    eval(&mut e, "var c = document.createComment('note')");
    assert_eq!(eval_str(&mut e, "c.nodeType"), "8");
    assert_eq!(eval_str(&mut e, "c.nodeName"), "#comment");
    assert_eq!(eval_str(&mut e, "c.data"), "note");
    // Comments are DOM nodes that render nothing.
    eval(&mut e, "document.getElementById('d').appendChild(c)");
    assert_eq!(
        eval_str(&mut e, "document.getElementById('d').childNodes.length"),
        "2"
    );

    eval(&mut e, "var f = document.createDocumentFragment()");
    assert_eq!(eval_str(&mut e, "f.nodeType"), "11");
    assert_eq!(eval_str(&mut e, "f.nodeName"), "#document-fragment");
    assert_eq!(eval_str(&mut e, "f.childNodes.length"), "0");
}

#[test]
fn contains_same_node_and_document_accessors() {
    let mut e = engine(
        r#"<!DOCTYPE html><html><head><title>t</title></head><body id="b"><div id="d"><span id="s">x</span></div></body></html>"#,
    );
    assert_eq!(eval_str(&mut e, "document.documentElement.tagName"), "HTML");
    assert_eq!(eval_str(&mut e, "document.body.id"), "b");
    assert_eq!(eval_str(&mut e, "document.head.tagName"), "HEAD");
    assert_eq!(eval_str(&mut e, "document.doctype.nodeName"), "html");
    assert_eq!(
        eval_str(
            &mut e,
            "document.getElementById('d').contains(document.getElementById('s'))"
        ),
        "true"
    );
    assert_eq!(
        eval_str(
            &mut e,
            "document.getElementById('d').contains(document.getElementById('d'))"
        ),
        "true"
    );
    assert_eq!(
        eval_str(
            &mut e,
            "document.getElementById('s').contains(document.getElementById('d'))"
        ),
        "false"
    );
    assert_eq!(
        eval_str(
            &mut e,
            "document.getElementById('s').contains(document.createTextNode('x'))"
        ),
        "false"
    );
}

#[test]
fn text_content_writes_through_the_new_text_model() {
    let mut e = engine(r#"<div id="d">old</div>"#);
    eval(&mut e, "document.getElementById('d').textContent = 'new'");
    assert_eq!(
        eval_str(&mut e, "document.getElementById('d').textContent"),
        "new"
    );
    assert_eq!(
        eval_str(&mut e, "document.getElementById('d').childNodes.length"),
        "1"
    );
    // The reused text child keeps its node identity; the value lives in `data`.
    eval(&mut e, "var t = document.getElementById('d').childNodes[0]");
    eval(&mut e, "t.data = 'edited'");
    assert_eq!(
        eval_str(&mut e, "document.getElementById('d').textContent"),
        "edited"
    );
}

#[test]
fn whitespace_and_comments_do_not_change_rendering() {
    // The parser now keeps whitespace text nodes and comments; layout must
    // skip them so an indented document renders exactly like a compact one.
    use htui::layout::layout::TermMap;
    use htui::layout::{BoxConstraint, Cursor, layout_tree};

    let compact = engine(r#"<div><p>hi</p></div>"#);
    let indented = engine("<div>\n  <p>hi</p>\n</div>");
    let term_map = TermMap {
        columns: 80,
        rows: 24,
    };
    let render = |e: &ScriptEngine| -> String {
        let arena = e.arena();
        let root_id = e.root_id();
        let containing = BoxConstraint::root(&term_map);
        let mut cursor = Cursor::new();
        let boxes = layout_tree(
            &arena,
            root_id,
            containing,
            &mut cursor,
            term_map.columns as usize,
            term_map.rows as usize,
        );
        let mut out = String::new();
        fn walk(boxes: &[htui::layout::LayoutBox], out: &mut String) {
            for b in boxes {
                if let Some(runs) = &b.paint_style.runs {
                    for line in runs {
                        let text: String = line.iter().map(|r| r.text.as_str()).collect();
                        out.push_str(text.as_str());
                        out.push('|');
                    }
                }
                walk(&b.children, out);
            }
        }
        walk(&boxes, &mut out);
        out
    };
    assert_eq!(
        render(&indented),
        render(&compact),
        "whitespace text nodes and comments must not change painted output"
    );
}

#[test]
fn style_camel_case_flex_and_order_props() {
    // `el.style` derives its camelCase accessors from `SUPPORTED_PROPERTIES`,
    // so flex/order/positioning longhands must round-trip through the style
    // attribute instead of silently becoming plain JS expando properties.
    let mut e = engine(r#"<div id="d">x</div>"#);
    eval(&mut e, "var el = document.getElementById('d')");

    eval(&mut e, "el.style.flexGrow = '30'");
    assert_eq!(eval_str(&mut e, "el.style.flexGrow"), "30");
    assert!(eval_str(&mut e, "el.getAttribute('style')").contains("flex-grow: 30"));

    eval(&mut e, "el.style.flexShrink = '0'");
    assert_eq!(eval_str(&mut e, "el.style.flexShrink"), "0");

    eval(&mut e, "el.style.flexBasis = '40%'");
    assert_eq!(eval_str(&mut e, "el.style.flexBasis"), "40%");

    eval(&mut e, "el.style.order = '-1'");
    assert_eq!(eval_str(&mut e, "el.style.order"), "-1");
    assert!(eval_str(&mut e, "el.getAttribute('style')").contains("order: -1"));

    // Unset properties read as the empty string, matching CSSStyleDeclaration.
    assert_eq!(eval_str(&mut e, "el.style.zIndex"), "");

    eval(&mut e, "el.style.zIndex = '10'");
    assert_eq!(eval_str(&mut e, "el.style.zIndex"), "10");
    assert!(eval_str(&mut e, "el.getAttribute('style')").contains("z-index: 10"));

    eval(&mut e, "el.style.alignSelf = 'flex-end'");
    assert_eq!(eval_str(&mut e, "el.style.alignSelf"), "flex-end");
    assert!(eval_str(&mut e, "el.getAttribute('style')").contains("align-self: flex-end"));
}
