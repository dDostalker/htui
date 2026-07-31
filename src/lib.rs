pub mod html;
pub mod layout;
pub mod logging;
pub mod tdom;
pub mod tui;

use std::io::{Write, stdout};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};

pub use html::html_parse::Parser;
pub use layout::layout::TermMap;

use crate::{
    html::css_parse::{self, CssRule},
    tdom::node::{NodeArena, NodeId},
    tui::graph_api::{RenderCmd, TermLayout, trigger_refresh},
};

fn parse_and_apply(html: &str, css_rules: &CssRule) -> (NodeId, NodeArena) {
    let mut html_parser = html::html_parse::Parser::new(html);
    let (root_id, mut arena, _embedded) = html_parser.parse();
    css_parse::css_into_node(&mut arena, root_id, css_rules);
    (root_id, arena)
}

fn build_css_rules(html: &str, css: &str) -> CssRule {
    let mut html_parser = html::html_parse::Parser::new(html);
    let (_root_id, _arena, embedded_css) = html_parser.parse();

    let combined = if css.is_empty() {
        embedded_css
    } else {
        format!("{}\n{}", embedded_css, css)
    };

    if combined.trim().is_empty() {
        return CssRule::new();
    }

    let mut css_parser = css_parse::Parser::new(&combined);
    css_parser.parse()
}

// ============================================================
// Render thread
// ============================================================

fn render_thread(
    rx: mpsc::Receiver<RenderCmd>,
    html: Arc<str>,
    css_rules: Arc<CssRule>,
    max_scroll: Arc<AtomicU32>,
) {
    let mut term_layout: Option<TermLayout> = None;

    loop {
        let mut cmd = match rx.recv() {
            Ok(c) => c,
            Err(e) => {
                log::error!("Render channel disconnected: {}", e);
                break;
            }
        };
        while let Ok(next) = rx.try_recv() {
            cmd = next;
            if matches!(cmd, RenderCmd::Quit) {
                break;
            }
        }

        match cmd {
            RenderCmd::Quit => break,

            RenderCmd::Rerender { scroll_y } => {
                let Some(tl) = term_layout.as_mut() else {
                    continue;
                };
                let prev = tl.scroll_y;
                tl.scroll_y = scroll_y;
                tl.clamp_scroll();
                // Defense in depth: drop rerenders whose resolved scroll
                // position is unchanged after clamping. Most such events are
                // already filtered in the input thread, but if a `Rerender`
                // slips past (e.g. races between a Redraw updating the
                // max_scroll ceiling and a queued scroll event) we still
                // avoid re-emitting Clear+render.
                if tl.scroll_y == prev {
                    continue;
                }
                if let Err(e) = execute!(stdout(), Clear(ClearType::All), MoveTo(0, 0)) {
                    log::error!("Failed to clear screen: {}", e);
                }
                tl.render();
                if let Err(e) = stdout().flush() {
                    log::error!("Failed to flush stdout: {}", e);
                }
            }

            RenderCmd::Redraw { scroll_y, zoom } => {
                let (root_id, arena) = parse_and_apply(&html, &css_rules);

                let term_map = match TermMap::get_ter_map() {
                    Ok(m) => m,
                    Err(e) => {
                        log::error!("Failed to get terminal size: {}", e);
                        continue;
                    }
                };

                // Reuse the existing canvas when its dimensions still match
                // the terminal. Only allocate a fresh Vec on the first render
                // or after a terminal resize, when the buffer size must change.
                let size_changed = match term_layout.as_ref() {
                    Some(tl) => tl.columns != term_map.columns || tl.rows != term_map.rows,
                    None => true,
                };
                if size_changed {
                    term_layout = Some(TermLayout::new(&term_map, zoom));
                }
                let tl = term_layout
                    .as_mut()
                    .expect("TermLayout just allocated above or reused from prior render");

                if let Err(e) = tl.draw(&arena, root_id, &term_map) {
                    log::error!("Layout failed: {}", e);
                    continue;
                }

                // Publish the new upper bound of `scroll_y` so the input
                // loop can drop events that would no-op against it.
                // u32::MAX is used as the startup placeholder (no upper
                // bound known yet), and the first successful layout will
                // overwrite it with the real value.
                max_scroll.store(tl.max_scroll(), Ordering::Relaxed);

                tl.scroll_y = scroll_y;
                tl.clamp_scroll();

                if let Err(e) = execute!(stdout(), Clear(ClearType::All), MoveTo(0, 0)) {
                    log::error!("Failed to clear screen: {}", e);
                }
                tl.render();
                if let Err(e) = stdout().flush() {
                    log::error!("Failed to flush stdout: {}", e);
                }
            }
        }
    }
}

// debug panic hook
// this fun only used in debug to hook render panic

#[cfg(debug_assertions)]
use std::panic;
#[cfg(debug_assertions)]
use std::sync::Once;
#[cfg(debug_assertions)]
use std::backtrace::Backtrace;

#[cfg(debug_assertions)]
fn install_debug_panic_hook() {
    static INSTALL_HOOK: Once = Once::new();

    INSTALL_HOOK.call_once(|| {
        let default_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            default_hook(info);

            let message = if let Some(message) = info.payload().downcast_ref::<&str>() {
                *message
            } else if let Some(message) = info.payload().downcast_ref::<String>() {
                message.as_str()
            } else {
                "non-string panic payload"
            };
            let location = info
                .location()
                .map(|location| {
                    format!(
                        "{}:{}:{}",
                        location.file(),
                        location.line(),
                        location.column()
                    )
                })
                .unwrap_or_else(|| "unknown location".to_owned());
            let thread = std::thread::current();
            let thread_name = thread.name().unwrap_or("unnamed");
            let backtrace = Backtrace::force_capture();

            log::error!(
                "Thread '{thread_name}' panicked at {location}: {message}\nBacktrace:\n{backtrace}"
            );
        }));
    });
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    if let Some(message) = payload.downcast_ref::<&str>() {
        message
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.as_str()
    } else {
        "non-string panic payload"
    }
}

// ============================================================
// Entry point
// ============================================================

pub fn render(html: &str, css: &str) {
    // install debug hook
    #[cfg(debug_assertions)]
    install_debug_panic_hook();
    
    if let Err(e) = enable_raw_mode() {
        log::error!("Failed to enable raw mode: {}", e);
        return;
    }
    if let Err(e) = execute!(stdout(), EnterAlternateScreen, EnableMouseCapture, Hide) {
        log::error!("Failed to enter alternate screen: {}", e);
        if let Err(e2) = disable_raw_mode() {
            log::error!("Failed to disable raw mode during cleanup: {}", e2);
        }
        return;
    }

    let css_rules = Arc::new(build_css_rules(html, css));

    let html: Arc<str> = Arc::from(html);
    // Shared scroll upper bound — initial value `u32::MAX` means "not yet
    // known" (the first layout hasn't run); the render thread overwrites it
    // on every successful `Redraw` with the real `tl.max_scroll()`.
    let max_scroll = Arc::new(AtomicU32::new(u32::MAX));
    let (tx, rx) = mpsc::channel::<RenderCmd>();

    let render_handle = std::thread::spawn({
        let html_clone = Arc::clone(&html);
        let css_clone = Arc::clone(&css_rules);
        let max_scroll_clone = Arc::clone(&max_scroll);
        move || {
            render_thread(rx, html_clone, css_clone, max_scroll_clone);
        }
    });

    let mut scroll_y: u32 = 0;
    let mut zoom: f32 = 1.0;
    let mut rows: u16 = match TermMap::get_ter_map() {
        Ok(m) => m.rows,
        Err(e) => {
            log::error!("Failed to get terminal size: {}", e);
            24
        }
    };

    if let Err(e) = tx.send(RenderCmd::Redraw { scroll_y, zoom }) {
        log::error!("Failed to send initial Redraw: {}", e);
        if let Err(e2) = tx.send(RenderCmd::Quit) {
            log::error!("Failed to send Quit during cleanup: {}", e2);
        }
    }

    loop {
        // Detect a dead render thread (e.g. from a panic) promptly. Rust's
        // spawned-thread panic handler swallows the panic and kills just the
        // worker, so without this check the main loop would keep polling
        // events, fail to send, and only exit when the user presses a key.
        if render_handle.is_finished() {
            log::error!("Render thread has terminated unexpectedly");
            break;
        }

        match trigger_refresh(
            &mut scroll_y,
            &mut zoom,
            rows,
            max_scroll.load(Ordering::Relaxed),
            Some(Duration::from_millis(100)),
        ) {
            Some(RenderCmd::Quit) => {
                if let Err(e) = tx.send(RenderCmd::Quit) {
                    log::error!("Failed to send Quit: {}", e);
                }
                break;
            }
            Some(cmd) => {
                if let RenderCmd::Redraw { .. } = &cmd {
                    if let Ok(map) = TermMap::get_ter_map() {
                        rows = map.rows;
                    }
                }
                if let Err(e) = tx.send(cmd) {
                    log::error!("Failed to send render command: {}", e);
                    break;
                }
            }
            None => {}
        }
    }

    if let Err(payload) = render_handle.join() {
        log::error!("Render thread panicked: {}", panic_message(payload.as_ref()));
    }

    if let Err(e) = execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture, Show) {
        log::error!("Failed to leave alternate screen: {}", e);
    }
    if let Err(e) = disable_raw_mode() {
        log::error!("Failed to disable raw mode: {}", e);
    }
}
