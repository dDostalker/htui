//! Render configuration and colour theme, shared by the session and the
//! rendering pipeline. This module is a dependency leaf: it only uses
//! `crossterm` and `std`, so any layer can import it without creating cycles.

use std::time::Duration;

use crossterm::style::Color;

/// Terminal colour theme applied by the renderer.
///
/// All values default to the Tokyo Night palette the renderer historically
/// used; override the fields you care about and keep the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Background of replaced controls (`<input>` / `<button>` / `<select>` /
    /// `<textarea>`) when the CSS `background-color` is unset — a neutral
    /// grey that reads as a field on both light and dark terminals.
    pub control_bg: Color,
    /// Highlight colour for the selected character range (page text
    /// selection and the focused `<input>` selection).
    pub selection_bg: Color,
    /// Background of the open `<select>` dropdown overlay.
    pub overlay_bg: Color,
    /// Foreground of the open `<select>` dropdown rows.
    pub overlay_fg: Color,
    /// Foreground of `<optgroup>` header rows — dimmer than the options so a
    /// group label reads as a non-option heading.
    pub group_fg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            control_bg: Color::DarkGrey,
            selection_bg: Color::Rgb {
                r: 0x3b,
                g: 0x42,
                b: 0x61,
            },
            overlay_bg: Color::Rgb {
                r: 0x24,
                g: 0x28,
                b: 0x3b,
            },
            overlay_fg: Color::Rgb {
                r: 0xc0,
                g: 0xca,
                b: 0xf5,
            },
            group_fg: Color::Rgb {
                r: 0x7a,
                g: 0x83,
                b: 0xa8,
            },
        }
    }
}

/// Render settings for a [`Session`](crate::session::Session).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    /// Initial zoom factor (default 1.0). The user can still zoom
    /// interactively during a render; this only seeds the start value.
    pub zoom: f32,
    /// Colour theme (default: Tokyo Night).
    pub theme: Theme,
    /// Minimum time between two render-thread frames. A fast wheel scroll can
    /// queue a full-viewport diff per tick; without a cap the render thread
    /// would try to draw at the event rate and saturate the terminal's output
    /// pipe, which looks exactly like an app freeze. Default 16 ms (~60 fps).
    pub frame_gap: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            theme: Theme::default(),
            frame_gap: Duration::from_millis(16),
        }
    }
}
