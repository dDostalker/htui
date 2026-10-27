//! Render configuration, shared by the session and the rendering pipeline.
//! This module is a dependency leaf: it only uses `crossterm` and `std`, so
//! any layer can import it without creating cycles.

use std::time::Duration;

/// Render settings for a [`Session`](crate::session::Session).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    /// Initial zoom factor (default 1.0). The user can still zoom
    /// interactively during a render; this only seeds the start value.
    pub zoom: f32,
    /// Whether JS-like event handling (click / hover / keyboard events, the
    /// script thread) is active. Default `true` — every render is interactive.
    /// Set to `false` for a static, non-interactive document (no hit-testing,
    /// no script thread; a blocking host function can never stall anything
    /// because there is no event loop).
    pub events: bool,
    /// Minimum time between two render-thread frames. A fast wheel scroll can
    /// queue a full-viewport diff per tick; without a cap the render thread
    /// would try to draw at the event rate and saturate the terminal's output
    /// pipe, which looks exactly like an app freeze. Default 16 ms (~60 fps).
    pub frame_gap: Duration,
    /// Whether the flow (left-drag) page text selection and its `Ctrl+C` copy
    /// are enabled (browser-like reading-order selection). Default `true`.
    pub copy_flow: bool,
    /// Whether the block (right-drag) rectangle selection and its `Ctrl+C`
    /// copy are enabled (vim `Ctrl+V` / tmux style). Default `true`.
    pub copy_block: bool,
    /// Whether wheel and page-jump scrolling glide to the target offset with
    /// an ease-out animation instead of jumping instantly (browser smooth
    /// scrolling). Default `true`.
    pub smooth_scroll: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            events: true,
            frame_gap: Duration::from_millis(16),
            copy_flow: true,
            copy_block: true,
            smooth_scroll: true,
        }
    }
}
