//! Session / config tests — the `Session` entry point and its `Config`
//! settings (zoom, theme, frame gap). Pure in-memory — no terminal required.

use std::time::Duration;

use crossterm::style::Color;

use htui::{Config, Session, Theme};

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

#[test]
fn config_defaults_match_the_renderer_historic_values() {
    init();
    let cfg = Config::default();
    assert_eq!(cfg.zoom, 1.0);
    assert_eq!(cfg.frame_gap, Duration::from_millis(16));
    let theme = cfg.theme;
    assert_eq!(theme.control_bg, Color::DarkGrey);
    assert_eq!(
        theme.selection_bg,
        Color::Rgb {
            r: 0x3b,
            g: 0x42,
            b: 0x61
        }
    );
    assert_eq!(
        theme.overlay_bg,
        Color::Rgb {
            r: 0x24,
            g: 0x28,
            b: 0x3b
        }
    );
    assert_eq!(
        theme.overlay_fg,
        Color::Rgb {
            r: 0xc0,
            g: 0xca,
            b: 0xf5
        }
    );
    assert_eq!(
        theme.group_fg,
        Color::Rgb {
            r: 0x7a,
            g: 0x83,
            b: 0xa8
        }
    );
}

#[test]
fn session_config_setters_and_mutators_roundtrip() {
    init();
    let mut session = Session::new();
    assert_eq!(session.zoom(), 1.0, "default zoom");

    session.set_zoom(2.5);
    assert_eq!(session.zoom(), 2.5);
    assert_eq!(session.config().zoom, 2.5);

    // `config_mut` edits a single setting in place.
    session.config_mut().frame_gap = Duration::from_millis(8);
    assert_eq!(session.config().frame_gap, Duration::from_millis(8));

    // `set_config` replaces the whole settings object.
    let mut custom = Config::default();
    custom.theme = Theme {
        control_bg: Color::Blue,
        ..Theme::default()
    };
    session.set_config(custom);
    assert_eq!(session.config().theme.control_bg, Color::Blue);
    assert_eq!(session.config().theme.selection_bg, Theme::default().selection_bg);
}
