//! `querySelector` / `querySelectorAll` tests.
//!
//! `document.querySelector(sel)` / `document.querySelectorAll(sel)` return the
//! first / all elements matching a CSS selector, in document order; the same
//! methods on an element scope the search to its subtree. Invalid selectors
//! throw a JS `SyntaxError` like a browser.

use htui::events::EventBindings;
use htui::script::ScriptEngine;

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

/// Evaluate a JS snippet in a fresh engine for `html`, returning the engine.
fn engine(html: &str) -> ScriptEngine {
    ScriptEngine::new(html, "", EventBindings::default()).expect("engine initializes")
}

fn eval_str(engine: &mut ScriptEngine, expr: &str) -> String {
    let v = engine
        .eval_source(&format!("String({expr})"))
        .expect("eval must succeed");
    v.as_string()
        .map(|s| s.to_std_string_lossy())
        .unwrap_or_default()
}

/// Run a statement / declaration script.
fn eval(engine: &mut ScriptEngine, code: &str) {
    engine.eval_source(code).expect("eval must succeed");
}

/// Run a `<script>` body and evaluate `expr` to a string.
fn run(html_body: &str, script: &str, expr: &str) -> String {
    let mut e = engine(&format!("{html_body}<script>{script}</script>"));
    eval_str(&mut e, expr)
}

const DOC: &str = r#"<div class="panel">
      <p id="intro" class="lead">hello</p>
      <p class="lead">second</p>
      <span>deep <em>em</em></span>
    </div>
    <ul>
      <li>one</li>
      <li>two</li>
      <li>three</li>
    </ul>"#;

#[test]
fn test_query_selector_tag_and_id_and_class() {
    init();
    assert_eq!(
        run(
            DOC,
            "globalThis.__t = document.querySelector('p').textContent;",
            "__t"
        ),
        "hello",
        "tag selector returns the first match in document order"
    );
    assert_eq!(
        run(
            DOC,
            "globalThis.__i = document.querySelector('#intro').textContent;",
            "__i"
        ),
        "hello",
        "id selector"
    );
    assert_eq!(
        run(
            DOC,
            "globalThis.__c = document.querySelector('.lead').textContent;",
            "__c"
        ),
        "hello",
        "class selector returns the first `.lead`"
    );
}

#[test]
fn test_query_selector_combinators() {
    init();
    assert_eq!(
        run(
            DOC,
            "globalThis.__d = document.querySelector('div span em').textContent;",
            "__d"
        ),
        "em",
        "descendant chain"
    );
    assert_eq!(
        run(
            DOC,
            "globalThis.__c = document.querySelector('div > p').id;",
            "__c"
        ),
        "intro",
        "child combinator"
    );
    // `~` walks every previous sibling (text nodes between elements do not
    // block it), so the span matches the earlier `<p>`s.
    assert_eq!(
        run(
            DOC,
            "globalThis.__s = document.querySelector('p ~ span') === null ? 'none' : 'hit';",
            "__s"
        ),
        "hit",
        "subsequent sibling combinator"
    );
    // `+` requires the immediately-preceding element sibling (no element
    // between them); use adjacent tags with no whitespace text between.
    let html = "<i>a</i><b>b</b><div>d</div>";
    assert_eq!(
        run(
            html,
            "globalThis.__a = document.querySelector('i + b').textContent;",
            "__a"
        ),
        "b",
        "adjacent sibling combinator"
    );
    assert_eq!(
        run(
            html,
            "globalThis.__u = document.querySelector('*').nodeType;",
            "__u"
        ),
        "1",
        "universal selector matches an element node"
    );
}

#[test]
fn test_query_selector_nth_child() {
    init();
    // No whitespace text between the list items, so the sibling indices are
    // clean (the engine counts text nodes toward `:nth-child`, like the CSS
    // stylesheet matcher).
    let html = "<ul><li>a</li><li>b</li><li>c</li></ul>";
    assert_eq!(
        run(
            html,
            "globalThis.__n = document.querySelector('li:nth-child(2)').textContent;",
            "__n"
        ),
        "b",
        ":nth-child(2) picks the second li"
    );
}

#[test]
fn test_query_selector_no_match_returns_null() {
    init();
    assert_eq!(
        run(
            DOC,
            "globalThis.__n = document.querySelector('article') === null;",
            "__n"
        ),
        "true",
        "no match returns null"
    );
}

#[test]
fn test_query_selector_invalid_selector_throws() {
    init();
    // Browsers throw SyntaxError for invalid selectors.
    let mut e = engine("<p>x</p>");
    let _ = e
        .eval_source("try { document.querySelector('..'); globalThis.__err = 'none'; } catch (err) { globalThis.__err = err.name; }")
        .expect("eval");
    assert_eq!(
        eval_str(&mut e, "__err"),
        "SyntaxError",
        "invalid selector surfaces as a JS SyntaxError"
    );
}

#[test]
fn test_query_selector_all_document_order() {
    init();
    assert_eq!(
        run(
            DOC,
            "var ls = document.querySelectorAll('li');
             globalThis.__len = ls.length;
             globalThis.__joined = Array.prototype.map.call(ls, function (l) { return l.textContent; }).join(',');
             globalThis.__first = ls[0].textContent;",
            "__len + '|' + __joined + '|' + __first"
        ),
        "3|one,two,three|one",
        "querySelectorAll returns every match in document order"
    );
    assert_eq!(
        run(
            DOC,
            "globalThis.__n = document.querySelectorAll('.lead').length;",
            "__n"
        ),
        "2",
        "compound class selector matches both leads"
    );
    assert_eq!(
        run(
            DOC,
            "globalThis.__n = document.querySelectorAll('*').length;",
            "__n"
        ),
        "12",
        "universal selector enumerates every element (incl. html/head/body)"
    );
}

#[test]
fn test_element_query_selector_scoped() {
    init();
    // Element-scoped queries search the subtree and never return the element
    // itself.
    let html = r#"<div id="outer"><p id="inner">inside</p></div><p id="out">outside</p>"#;
    assert_eq!(
        run(
            html,
            "var outer = document.getElementById('outer');
             globalThis.__scoped = outer.querySelector('p').id;
             globalThis.__self = outer.querySelector('#outer') === null;
             globalThis.__outside = outer.querySelector('#out') === null;
             globalThis.__all = outer.querySelectorAll('p').length;",
            "__scoped + '|' + __self + '|' + __outside + '|' + __all"
        ),
        "inner|true|true|1",
        "element.querySelector scopes to the subtree, excluding the element itself"
    );
    // The document query still sees everything.
    assert_eq!(
        run(
            html,
            "globalThis.__all = document.querySelectorAll('p').length;",
            "__all"
        ),
        "2",
        "document.querySelectorAll spans the whole document"
    );
}

#[test]
fn test_query_selector_returns_live_wrapper() {
    init();
    // The returned wrapper is the same live element object: mutating it is
    // visible through getElementById and the DOM.
    let mut e = engine(
        r#"<p id="a">one</p>
        <script>
          var el = document.querySelector('p');
          el.textContent = 'changed';
          globalThis.__via = document.getElementById('a').textContent;
        </script>"#,
    );
    assert_eq!(
        eval_str(&mut e, "__via"),
        "changed",
        "querySelector returns the live element wrapper"
    );
}

#[test]
fn test_query_selector_matches_new_elements() {
    init();
    // The engine runs its own arena, so a selector matches whatever the DOM
    // holds at query time — including non-`<style>`/`<script>` elements.
    let mut e = engine(
        r#"<div id="d"><span>s</span></div>
        <script>
          var s = document.querySelector('div span');
          globalThis.__t = s.textContent;
          var em = document.createElement('em');
          em.textContent = 'made';
          document.getElementById('d').appendChild(em);
          globalThis.__after = document.querySelector('#d em').textContent;
        </script>"#,
    );
    assert_eq!(eval_str(&mut e, "__t"), "s", "selector reads existing DOM");
    assert_eq!(
        eval_str(&mut e, "__after"),
        "made",
        "selector sees elements created by JS"
    );
}

#[test]
fn test_matches_and_closest() {
    init();
    let mut e = engine(
        r#"<div class="panel" id="p"><ul id="u"><li class="lead">one</li><li>two</li></ul></div>"#,
    );
    eval(&mut e, "var lead = document.querySelector('li.lead')");
    eval(&mut e, "var plain = document.querySelectorAll('li')[1]");
    // `matches` on the element itself.
    assert_eq!(eval_str(&mut e, "String(lead.matches('li.lead'))"), "true");
    assert_eq!(eval_str(&mut e, "String(lead.matches('.panel'))"), "false");
    // `closest` walks ancestor-or-self.
    assert_eq!(
        eval_str(&mut e, "lead.closest('ul').id"),
        "u",
        "closest finds the nearest matching ancestor"
    );
    assert_eq!(
        eval_str(&mut e, "lead.closest('.panel li').tagName"),
        "LI",
        "closest matches the element itself when the chain fits"
    );
    assert_eq!(
        eval_str(&mut e, "String(lead.closest('table'))"),
        "null",
        "no match returns null"
    );
    assert_eq!(
        eval_str(&mut e, "String(plain.matches(':not(.lead)'))"),
        "true",
        "structural pseudo-classes work through matches()"
    );
    // Invalid selector throws SyntaxError, like querySelector.
    eval(&mut e, "globalThis.__err = ''");
    eval(
        &mut e,
        "try { lead.matches('>>') } catch (x) { globalThis.__err = x.name }",
    );
    assert_eq!(eval_str(&mut e, "__err"), "SyntaxError");
}

#[test]
fn test_has_attribute_and_toggle_attribute() {
    init();
    let mut e = engine(r#"<input id="i" value="v"><input id="j" value="" checked>"#);
    eval(&mut e, "var i = document.getElementById('i')");
    eval(&mut e, "var j = document.getElementById('j')");
    assert_eq!(eval_str(&mut e, "String(i.hasAttribute('value'))"), "true");
    // An empty value still counts as present.
    assert_eq!(
        eval_str(&mut e, "String(j.hasAttribute('value'))"),
        "true",
        "empty-valued attribute is present"
    );
    assert_eq!(
        eval_str(&mut e, "String(i.hasAttribute('checked'))"),
        "false"
    );
    // toggleAttribute flips presence and reports the result.
    assert_eq!(
        eval_str(&mut e, "String(i.toggleAttribute('disabled'))"),
        "true"
    );
    assert_eq!(
        eval_str(&mut e, "String(i.hasAttribute('disabled'))"),
        "true"
    );
    assert_eq!(
        eval_str(&mut e, "String(i.toggleAttribute('disabled'))"),
        "false"
    );
    assert_eq!(
        eval_str(&mut e, "String(i.hasAttribute('disabled'))"),
        "false"
    );
    // `force` drives add / remove unconditionally.
    eval(&mut e, "j.toggleAttribute('checked', true)");
    assert_eq!(
        eval_str(&mut e, "String(j.hasAttribute('checked'))"),
        "true"
    );
    eval(&mut e, "j.toggleAttribute('checked', true)");
    assert_eq!(
        eval_str(&mut e, "String(j.hasAttribute('checked'))"),
        "true"
    );
}

#[test]
fn test_element_sibling_accessors() {
    init();
    let mut e = engine(
        r#"<div>text<b id="b">b</b> tail<i id="i">i</i><!-- c --><span id="s">s</span></div>"#,
    );
    eval(&mut e, "var b = document.getElementById('b')");
    eval(&mut e, "var i = document.getElementById('i')");
    eval(&mut e, "var s = document.getElementById('s')");
    // Element siblings skip text / comment nodes between them.
    assert_eq!(eval_str(&mut e, "String(b.previousElementSibling)"), "null");
    assert_eq!(eval_str(&mut e, "i.previousElementSibling.id"), "b");
    assert_eq!(eval_str(&mut e, "s.previousElementSibling.id"), "i");
    assert_eq!(eval_str(&mut e, "b.nextElementSibling.id"), "i");
    assert_eq!(eval_str(&mut e, "s.nextElementSibling"), "null");
    // Contrast with the raw node siblings (text / comment nodes included).
    assert_eq!(eval_str(&mut e, "i.previousSibling.nodeType"), "3");
    assert_eq!(eval_str(&mut e, "s.previousSibling.nodeType"), "8");
}

#[test]
fn test_structural_pseudo_classes_via_query_selector() {
    init();
    let mut e = engine(
        r#"<ul id="u"><li>one</li><li>two</li><li>three</li></ul>
           <form>
             <input id="cb" type="checkbox" checked>
             <input id="tx">
             <button id="bd" disabled>b</button>
           </form>"#,
    );
    assert_eq!(
        eval_str(&mut e, "document.querySelectorAll('li:first-child').length"),
        "1"
    );
    assert_eq!(
        eval_str(
            &mut e,
            "document.querySelector('li:last-child').textContent"
        ),
        "three"
    );
    assert_eq!(
        eval_str(
            &mut e,
            "document.querySelectorAll('li:nth-last-child(2)').length"
        ),
        "1",
        "second-to-last li matches"
    );
    assert_eq!(
        eval_str(
            &mut e,
            "document.querySelector('li:nth-last-child(2)').textContent"
        ),
        "two"
    );
    assert_eq!(
        eval_str(&mut e, "document.querySelectorAll('li:not(.x)').length"),
        "3",
        ":not matches every li (none carry .x)"
    );
    assert_eq!(
        eval_str(&mut e, "document.querySelectorAll(':checked').length"),
        "1",
        "the checked checkbox is :checked"
    );
    assert_eq!(
        eval_str(&mut e, "document.querySelector(':checked').id"),
        "cb"
    );
    assert_eq!(
        eval_str(&mut e, "document.querySelectorAll(':disabled').length"),
        "1",
        "the disabled button is :disabled"
    );
    assert_eq!(
        eval_str(&mut e, "document.querySelectorAll('input:enabled').length"),
        "2",
        "both inputs are :enabled (checked state does not disable)"
    );
    // `<html>` is the root element.
    assert_eq!(
        eval_str(&mut e, "document.querySelector(':root').tagName"),
        "HTML"
    );
    // A `<li>` with no element children but whitespace text is not :empty;
    // build a truly empty one to compare.
    eval(
        &mut e,
        "var blank = document.createElement('div'); document.body.appendChild(blank)",
    );
    assert_eq!(
        eval_str(
            &mut e,
            "String(document.querySelector('div:empty') === blank)"
        ),
        "true",
        "a freshly created div with no children is :empty"
    );
}
