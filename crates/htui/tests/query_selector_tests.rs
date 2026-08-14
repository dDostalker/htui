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
    ScriptEngine::new(html, EventBindings::default()).expect("engine initializes")
}

fn eval_str(engine: &mut ScriptEngine, expr: &str) -> String {
    let v = engine
        .eval_source(&format!("String({expr})"))
        .expect("eval must succeed");
    v.as_string()
        .map(|s| s.to_std_string_lossy())
        .unwrap_or_default()
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
