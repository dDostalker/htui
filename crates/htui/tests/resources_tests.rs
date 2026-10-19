//! Tests for the virtual resource layer (`htui::resources`): `mem://`
//! registration, prefix normalisation, load / unregister semantics, and the
//! thread-scoped [`with_registry`] install.

use std::sync::Arc;

use htui::resources::{ResourceRegistry, with_registry};

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

#[test]
fn register_rejects_empty_paths() {
    init();
    let reg = ResourceRegistry::new();
    let err = reg
        .register("", b"x".to_vec())
        .expect_err("empty path must error");
    assert!(
        err.to_string().contains("must not be empty"),
        "error should explain the empty-path rule, got {err}"
    );
}

#[test]
fn register_then_load_roundtrips_bytes() {
    init();
    let reg = ResourceRegistry::new();
    let key = format!("mem://test/roundtrip_{}", std::process::id());
    reg.register(&key, b"hello resource".to_vec())
        .expect("register mem resource");
    let loaded = reg.load(&key).expect("mem load succeeds");
    assert_eq!(&*loaded, b"hello resource");
    reg.unregister(&key);
}

#[test]
fn bare_paths_and_prefixed_paths_are_equivalent() {
    init();
    // Register with the `mem://` prefix, load without it (and vice versa) —
    // both must hit the same entry.
    let reg = ResourceRegistry::new();
    let tag = std::process::id();
    let bare = format!("assets/{tag}.txt");
    reg.register(&bare, b"prefixed".to_vec())
        .expect("register bare path");
    let prefixed = format!("mem://{bare}");
    let loaded = reg.load(&prefixed).expect("load with prefix");
    assert_eq!(&*loaded, b"prefixed");

    let prefixed2 = format!("mem://other/{tag}.txt");
    reg.register(&prefixed2, b"bare".to_vec())
        .expect("register prefixed path");
    let bare2 = format!("other/{tag}.txt");
    let loaded2 = reg.load(&bare2).expect("load bare path");
    assert_eq!(&*loaded2, b"bare");

    reg.unregister(&bare);
    reg.unregister(&prefixed2);
}

#[test]
fn register_replaces_existing_bytes() {
    init();
    let reg = ResourceRegistry::new();
    let key = format!("mem://test/replace_{}", std::process::id());
    reg.register(&key, b"first".to_vec())
        .expect("register once");
    reg.register(&key, b"second".to_vec())
        .expect("register again");
    let loaded = reg.load(&key).expect("mem load succeeds");
    assert_eq!(&*loaded, b"second", "re-registration must replace");
    reg.unregister(&key);
}

#[test]
fn unregister_returns_whether_key_existed() {
    init();
    let reg = ResourceRegistry::new();
    let key = format!("mem://test/unregister_{}", std::process::id());
    reg.register(&key, b"x".to_vec())
        .expect("register mem resource");
    // Before unregistering, loading succeeds.
    assert!(reg.load(&key).is_ok());
    assert!(reg.unregister(&key), "existing key unregisters");
    assert!(
        reg.load(&key).is_err(),
        "after unregistering, loading must fail"
    );
    assert!(!reg.unregister(&key), "missing key unregisters nothing");
}

#[test]
fn load_missing_mem_key_errors() {
    init();
    let reg = ResourceRegistry::new();
    let err = reg
        .load("mem://does/not/exist")
        .expect_err("unregistered mem:// key must error");
    assert!(
        err.to_string().contains("not registered"),
        "error should mention the missing registration, got {err}"
    );
}

#[test]
fn with_registry_installs_the_registry_for_the_scope() {
    init();
    let reg = Arc::new(ResourceRegistry::new());
    reg.register("scoped.css", b"p { color: red; }".to_vec())
        .expect("register");

    // Outside the scope the internal bridge has nothing installed: a `<link
    // href>` resource fails to load (the parser keeps the css source empty).
    let (_, _, css_outside) = htui::html::html_parse::Parser::new(
        "<link rel=\"stylesheet\" href=\"scoped.css\"><p>x</p>",
    )
    .parse();
    assert!(
        !css_outside.contains("color: red"),
        "no registry installed outside with_registry"
    );

    // Inside the scope the resource resolves.
    let (_, _, css_inside) = with_registry(Arc::clone(&reg), || {
        htui::html::html_parse::Parser::new("<link rel=\"stylesheet\" href=\"scoped.css\"><p>x</p>")
            .parse()
    });
    assert!(
        css_inside.contains("color: red"),
        "scoped link must load the registered stylesheet"
    );
}

#[test]
fn with_registry_scopes_restore_the_previous_registry() {
    init();
    let outer = Arc::new(ResourceRegistry::new());
    outer
        .register("outer.css", b"a { color: blue; }".to_vec())
        .expect("register");
    let inner = Arc::new(ResourceRegistry::new());
    inner
        .register("inner.css", b"b { color: green; }".to_vec())
        .expect("register");

    let parse = |href: &str| {
        let html = format!("<link rel=\"stylesheet\" href=\"{href}\"><p>x</p>");
        let (_, _, css) = htui::html::html_parse::Parser::new(&html).parse();
        css
    };

    with_registry(Arc::clone(&outer), || {
        // Inner scope reads its own registry, not the outer's.
        let inner_css = with_registry(Arc::clone(&inner), || parse("inner.css"));
        assert!(
            inner_css.contains("color: green"),
            "inner scope must resolve its own registry"
        );
        // Back in the outer scope the inner registry is gone, the outer's is
        // restored.
        let outer_css = parse("outer.css");
        assert!(
            outer_css.contains("color: blue"),
            "outer scope must restore after the inner scope leaves"
        );
        let inner_gone = !parse("inner.css").contains("color: green");
        assert!(inner_gone, "inner registry must uninstall on scope exit");
    });
}

// ============================================================
// resolve_url — `<base href>` document-relative resolution
// ============================================================

#[test]
fn resolve_url_without_base_prepends_mem_prefix() {
    assert_eq!(
        htui::resources::resolve_url(None, "app.js"),
        "mem://app.js",
        "no base: bare path gains the mem:// prefix"
    );
    assert_eq!(
        htui::resources::resolve_url(None, "mem://app.js"),
        "mem://app.js",
        "no base: prefixed path passes through"
    );
    assert_eq!(
        htui::resources::resolve_url(Some(""), "app.js"),
        "mem://app.js",
        "empty base is treated as absent"
    );
}

#[test]
fn resolve_url_keeps_absolute_urls_untouched() {
    assert_eq!(
        htui::resources::resolve_url(Some("mem://dir/"), "mem://x/y.png"),
        "mem://x/y.png",
        "absolute mem:// URLs ignore the base"
    );
    assert_eq!(
        htui::resources::resolve_url(Some("mem://dir/"), "https://example.com/a.png"),
        "https://example.com/a.png",
        "external schemes pass through (the registry will report them as missing)"
    );
}

#[test]
fn resolve_url_joins_relative_paths_against_base() {
    assert_eq!(
        htui::resources::resolve_url(Some("mem://dir/"), "pic.png"),
        "mem://dir/pic.png",
        "base with trailing slash"
    );
    assert_eq!(
        htui::resources::resolve_url(Some("mem://dir"), "pic.png"),
        "mem://dir/pic.png",
        "base without trailing slash is treated as a directory"
    );
    assert_eq!(
        htui::resources::resolve_url(Some("dir/"), "pic.png"),
        "mem://dir/pic.png",
        "bare base gains the mem:// prefix"
    );
    assert_eq!(
        htui::resources::resolve_url(Some("mem://a/b/"), "c/d.png"),
        "mem://a/b/c/d.png",
        "nested relative paths join under the base"
    );
}

#[test]
fn resolve_url_root_relative_resolves_to_mem_root() {
    assert_eq!(
        htui::resources::resolve_url(Some("mem://dir/"), "/root.png"),
        "mem://root.png",
        "root-relative reference ignores the base directory"
    );
}
