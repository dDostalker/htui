//! Tests for the virtual resource layer (`htui::resources`): `mem://`
//! registration, prefix normalisation, and load / unregister semantics.

use htui::resources;

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

/// Unique `mem://` key so parallel tests never collide on the process-wide
/// registry.
fn mem_key(tag: &str) -> String {
    format!("mem://test/{tag}_{}", std::process::id())
}

#[test]
fn register_rejects_empty_paths() {
    init();
    let err = resources::register("", b"x".to_vec()).expect_err("empty path must error");
    assert!(
        err.to_string().contains("must not be empty"),
        "error should explain the empty-path rule, got {err}"
    );
}

#[test]
fn register_then_load_roundtrips_bytes() {
    init();
    let key = mem_key("roundtrip");
    resources::register(&key, b"hello resource".to_vec())
        .expect("register mem resource");
    let loaded = resources::load(&key).expect("mem load succeeds");
    assert_eq!(&*loaded, b"hello resource");
    resources::unregister(&key);
}

#[test]
fn bare_paths_and_prefixed_paths_are_equivalent() {
    init();
    // Register with the `mem://` prefix, load without it (and vice versa) —
    // both must hit the same entry.
    let bare = format!("assets/{tag}.txt", tag = std::process::id());
    resources::register(&bare, b"prefixed".to_vec()).expect("register bare path");
    let prefixed = format!("mem://{bare}");
    let loaded = resources::load(&prefixed).expect("load with prefix");
    assert_eq!(&*loaded, b"prefixed");

    let prefixed2 = format!("mem://other/{tag}.txt", tag = std::process::id());
    resources::register(&prefixed2, b"bare".to_vec()).expect("register prefixed path");
    let bare2 = format!("other/{tag}.txt", tag = std::process::id());
    let loaded2 = resources::load(&bare2).expect("load bare path");
    assert_eq!(&*loaded2, b"bare");

    resources::unregister(&bare);
    resources::unregister(&prefixed2);
}

#[test]
fn register_replaces_existing_bytes() {
    init();
    let key = mem_key("replace");
    resources::register(&key, b"first".to_vec()).expect("register once");
    resources::register(&key, b"second".to_vec()).expect("register again");
    let loaded = resources::load(&key).expect("mem load succeeds");
    assert_eq!(&*loaded, b"second", "re-registration must replace");
    resources::unregister(&key);
}

#[test]
fn unregister_returns_whether_key_existed() {
    init();
    let key = mem_key("unregister");
    resources::register(&key, b"x".to_vec()).expect("register mem resource");
    // Before unregistering, loading succeeds.
    assert!(resources::load(&key).is_ok());
    assert!(resources::unregister(&key), "existing key unregisters");
    assert!(
        resources::load(&key).is_err(),
        "after unregistering, loading must fail"
    );
    assert!(!resources::unregister(&key), "missing key unregisters nothing");
}

#[test]
fn load_missing_mem_key_errors() {
    init();
    let err = resources::load("mem://does/not/exist")
        .expect_err("unregistered mem:// key must error");
    assert!(
        err.to_string().contains("not registered"),
        "error should mention the missing registration, got {err}"
    );
}

// ============================================================
// resolve_url — `<base href>` document-relative resolution
// ============================================================

#[test]
fn resolve_url_without_base_prepends_mem_prefix() {
    assert_eq!(
        resources::resolve_url(None, "app.js"),
        "mem://app.js",
        "no base: bare path gains the mem:// prefix"
    );
    assert_eq!(
        resources::resolve_url(None, "mem://app.js"),
        "mem://app.js",
        "no base: prefixed path passes through"
    );
    assert_eq!(
        resources::resolve_url(Some(""), "app.js"),
        "mem://app.js",
        "empty base is treated as absent"
    );
}

#[test]
fn resolve_url_keeps_absolute_urls_untouched() {
    assert_eq!(
        resources::resolve_url(Some("mem://dir/"), "mem://x/y.png"),
        "mem://x/y.png",
        "absolute mem:// URLs ignore the base"
    );
    assert_eq!(
        resources::resolve_url(Some("mem://dir/"), "https://example.com/a.png"),
        "https://example.com/a.png",
        "external schemes pass through (the registry will report them as missing)"
    );
}

#[test]
fn resolve_url_joins_relative_paths_against_base() {
    assert_eq!(
        resources::resolve_url(Some("mem://dir/"), "pic.png"),
        "mem://dir/pic.png",
        "base with trailing slash"
    );
    assert_eq!(
        resources::resolve_url(Some("mem://dir"), "pic.png"),
        "mem://dir/pic.png",
        "base without trailing slash is treated as a directory"
    );
    assert_eq!(
        resources::resolve_url(Some("dir/"), "pic.png"),
        "mem://dir/pic.png",
        "bare base gains the mem:// prefix"
    );
    assert_eq!(
        resources::resolve_url(Some("mem://a/b/"), "c/d.png"),
        "mem://a/b/c/d.png",
        "nested relative paths join under the base"
    );
}

#[test]
fn resolve_url_root_relative_resolves_to_mem_root() {
    assert_eq!(
        resources::resolve_url(Some("mem://dir/"), "/root.png"),
        "mem://root.png",
        "root-relative reference ignores the base directory"
    );
}
