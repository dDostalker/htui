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

pub fn trigger_refresh(scroll_y: &mut u32, zoom: &mut f32, rows: u16) -> Option<RenderCmd> {
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
                    *scroll_y = scroll_y.saturating_sub(1);
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    *scroll_y = scroll_y.saturating_add(1);
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                KeyCode::PageUp => {
                    let step = (rows as u32).saturating_sub(1).max(1);
                    *scroll_y = scroll_y.saturating_sub(step);
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                KeyCode::PageDown => {
                    let step = (rows as u32).saturating_sub(1).max(1);
                    *scroll_y = scroll_y.saturating_add(step);
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                KeyCode::Home => {
                    *scroll_y = 0;
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                KeyCode::End => {
                    *scroll_y = u32::MAX;
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
