//! Session / config tests — the `Session` entry point and its `Config`
//! settings (zoom, events, frame gap). Pure in-memory — no terminal required.

use std::time::Duration;

use htui::{Config, Session};

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

#[test]
fn config_defaults_are_interactive() {
    init();
    let cfg = Config::default();
    assert_eq!(cfg.zoom, 1.0);
    assert!(cfg.events, "events default to on");
    assert_eq!(cfg.frame_gap, Duration::from_millis(16));
    assert!(cfg.smooth_scroll, "smooth scrolling defaults to on");
}

#[test]
fn session_config_setters_and_mutators_roundtrip() {
    init();
    let mut session = Session::new();
    assert_eq!(session.zoom(), 1.0, "default zoom");
    assert!(session.config().events, "default events on");

    session.set_zoom(2.5);
    assert_eq!(session.zoom(), 2.5);
    assert_eq!(session.config().zoom, 2.5);

    // `config_mut` edits a single setting in place.
    session.config_mut().frame_gap = Duration::from_millis(8);
    assert_eq!(session.config().frame_gap, Duration::from_millis(8));
    session.config_mut().events = false;
    assert!(!session.config().events);

    // `set_config` replaces the whole settings object.
    session.set_config(Config::default());
    assert!(session.config().events, "set_config restores defaults");
    assert_eq!(session.config().zoom, 1.0);
}
