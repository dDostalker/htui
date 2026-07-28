use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseEventKind};

const SCROLL_LINES_PER_TICK: u32 = 3;

#[derive(Debug, Clone)]
pub enum RenderCmd {
    Rerender { scroll_y: u32 },
    Redraw { scroll_y: u32, zoom: f32 },
    Quit,
}
const ZOOM_STEP: f32 = 0.1;
const ZOOM_MIN: f32 = 0.2;
const ZOOM_MAX: f32 = 5.0;

/// Wait for the next input event and translate it into a [`RenderCmd`].
///
/// `timeout` controls how long to block on `event::poll`:
/// - `None` blocks indefinitely (legacy behaviour, suitable for tests or
///   one-shot tools that don't need to react to the render thread dying).
/// - `Some(duration)` returns `None` when no event arrives within the
///   duration, giving the caller a chance to check other state (e.g.
///   whether the render thread is still alive).
///
/// `max_scroll` is the largest `scroll_y` the canvas currently allows
/// (i.e. `rendered_rows - viewport_rows`). It is supplied by the render
/// thread after each layout pass. Scroll events that would push past the
/// boundary in either direction are dropped here at the source instead of
/// being forwarded as a no-op `Rerender` — the render thread already had a
/// guard for that, but every dropped command here is one fewer channel
/// send and, more importantly, one fewer frame that risks a visible flash
/// at the scroll bounds.
pub fn trigger_refresh(
    scroll_y: &mut u32,
    zoom: &mut f32,
    rows: u16,
    max_scroll: u32,
    timeout: Option<Duration>,
) -> Option<RenderCmd> {
    if let Some(t) = timeout {
        // `poll` returns `Ok(false)` on timeout and `Err` on a system error;
        // both cases should be surfaced as `None` so the outer loop can
        // re-check the render thread instead of blocking forever.
        if !event::poll(t).unwrap_or(false) {
            return None;
        }
    }
    match event::read() {
        Ok(Event::Key(key)) => {
            if key.kind == KeyEventKind::Release {
                return None;
            }

            if key.modifiers.contains(event::KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('=') | KeyCode::Char('+') => {
                        *zoom = (*zoom + ZOOM_STEP).min(ZOOM_MAX);
                        return Some(RenderCmd::Redraw {
                            scroll_y: *scroll_y,
                            zoom: *zoom,
                        });
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') => {
                        *zoom = (*zoom - ZOOM_STEP).max(ZOOM_MIN);
                        return Some(RenderCmd::Redraw {
                            scroll_y: *scroll_y,
                            zoom: *zoom,
                        });
                    }
                    KeyCode::Char('0') => {
                        *zoom = 1.0;
                        return Some(RenderCmd::Redraw {
                            scroll_y: *scroll_y,
                            zoom: *zoom,
                        });
                    }
                    _ => {}
                }
            }

            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => return Some(RenderCmd::Quit),

                KeyCode::Up | KeyCode::Char('k') => {
                    // Already at the top boundary — drop the event instead
                    // of forwarding a no-op scroll to the render thread.
                    if *scroll_y == 0 {
                        return None;
                    }
                    *scroll_y -= 1;
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    // Already at the bottom boundary — drop the event.
                    if *scroll_y >= max_scroll {
                        return None;
                    }
                    *scroll_y += 1;
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                KeyCode::PageUp => {
                    if *scroll_y == 0 {
                        return None;
                    }
                    let step = (rows as u32).saturating_sub(1).max(1);
                    *scroll_y = scroll_y.saturating_sub(step);
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                KeyCode::PageDown => {
                    if *scroll_y >= max_scroll {
                        return None;
                    }
                    let step = (rows as u32).saturating_sub(1).max(1);
                    *scroll_y = scroll_y.saturating_add(step);
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                KeyCode::Home => {
                    // No-op move — drop to avoid a wasted rerender.
                    if *scroll_y == 0 {
                        return None;
                    }
                    *scroll_y = 0;
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                KeyCode::End => {
                    // No-op move (already at the bottom) — drop.
                    if *scroll_y >= max_scroll {
                        return None;
                    }
                    // Use the actual upper bound rather than u32::MAX so the
                    // render thread receives a value it doesn't have to clamp.
                    *scroll_y = max_scroll;
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                _ => {}
            }
            None
        }

        Ok(Event::Mouse(mouse)) => {
            let ctrl = mouse.modifiers.contains(event::KeyModifiers::CONTROL);
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    if ctrl {
                        *zoom = (*zoom - ZOOM_STEP).max(ZOOM_MIN);
                        Some(RenderCmd::Redraw {
                            scroll_y: *scroll_y,
                            zoom: *zoom,
                        })
                    } else {
                        if *scroll_y == 0 {
                            return None;
                        }
                        *scroll_y = scroll_y.saturating_sub(SCROLL_LINES_PER_TICK);
                        Some(RenderCmd::Rerender {
                            scroll_y: *scroll_y,
                        })
                    }
                }
                MouseEventKind::ScrollDown => {
                    if ctrl {
                        *zoom = (*zoom + ZOOM_STEP).min(ZOOM_MAX);
                        Some(RenderCmd::Redraw {
                            scroll_y: *scroll_y,
                            zoom: *zoom,
                        })
                    } else {
                        if *scroll_y >= max_scroll {
                            return None;
                        }
                        *scroll_y = scroll_y.saturating_add(SCROLL_LINES_PER_TICK);
                        Some(RenderCmd::Rerender {
                            scroll_y: *scroll_y,
                        })
                    }
                }
                _ => None,
            }
        }

        Ok(Event::Resize(_, _new_rows)) => Some(RenderCmd::Redraw {
            scroll_y: *scroll_y,
            zoom: *zoom,
        }),

        _ => None,
    }
}
