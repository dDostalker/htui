pub mod events;
pub mod html;
pub mod layout;
pub mod logging;
pub mod tdom;
pub mod tui;

use std::io::{BufWriter, Write, stdout};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use crossterm::{
    cursor::{Hide, Show},
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};

pub use html::html_parse::Parser;
pub use layout::layout::TermMap;

use crate::{
    events::{EventBindings, Geometry, HitMap},
    html::css_parse::{self, CssRule, css_into_node_with_hover, reset_css_attr},
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::{InputState, RenderCmd, TermLayout, trigger_refresh},
};

fn parse_and_apply(html: &str, css_rules: &CssRule) -> (NodeId, NodeArena) {
    let mut html_parser = html::html_parse::Parser::new(html);
    let (root_id, mut arena, _embedded) = html_parser.parse();
    css_parse::css_into_node(&mut arena, root_id, css_rules);
    (root_id, arena)
}

/// Flush one rendered frame from the session's `BufWriter`, skipping the
/// syscall entirely when the frame wrote nothing (an unchanged viewport).
fn flush_frame<W: Write>(out: &mut BufWriter<W>) {
    if out.buffer().is_empty() {
        return;
    }
    if let Err(e) = out.flush() {
        log::error!("Failed to flush stdout: {}", e);
    }
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

/// Extract the document `<title>` text from the parsed DOM, if present.
///
/// Browsers use the first `<title>` element and ignore later ones; this walks
/// the arena in document order and returns the concatenated text of the first
/// `<title>` found. Returns `None` when the document has no `<title>` or it
/// contains no non-whitespace text.
fn extract_title(arena: &NodeArena, root_id: NodeId) -> Option<String> {
    let mut stack = vec![root_id];
    while let Some(id) = stack.pop() {
        let node = &arena[id];
        if node.node_type == NodeType::Title {
            let text: String = node
                .children
                .iter()
                .filter_map(|&child_id| match &arena[child_id].node_type {
                    NodeType::Text(s) => Some(s.as_str()),
                    _ => None,
                })
                .collect();
            let text = text.trim();
            return if text.is_empty() {
                None
            } else {
                Some(text.to_string())
            };
        }
        // Reverse so the stack visits children in document order (LIFO).
        stack.extend(node.children.iter().rev().copied());
    }
    None
}

// ============================================================
// Render thread
// ============================================================

/// Per-session state owned by the render thread.
///
/// Holds everything that outlives a single frame: the session document, the
/// stylesheet, the reused canvas, the cached geometry and the scroll bound
/// shared with the input loop.
struct RenderState {
    html: Arc<str>,
    css_rules: Arc<CssRule>,
    term_layout: Option<TermLayout>,
    /// Terminal title: install the document `<title>` once per session (or
    /// per replaced document). crossterm offers no title getter, so the
    /// previous title is intentionally left untouched on quit.
    title_set: bool,
    /// Cached geometry of the latest layout pass: kept so `RenderCmd::Hover`
    /// can re-style the document for CSS `:hover` rules and repaint without
    /// re-parsing or re-laying out.
    geometry: Option<Arc<Geometry>>,
    max_scroll: Arc<AtomicU32>,
}

impl RenderState {
    /// Parse the current session document and cascade its styles.
    fn parse_and_apply(&self) -> (NodeId, NodeArena) {
        parse_and_apply(&self.html, &self.css_rules)
    }

    /// Re-parse, re-layout, re-paint, publish the geometry and flush a frame.
    ///
    /// Shared by `Redraw` (same document) and `RenderHtml` (new document).
    /// `out` is generic so tests can drive the pipeline against an in-memory
    /// writer.
    fn redraw<W: Write>(
        &mut self,
        out: &mut BufWriter<W>,
        geo_tx: &Option<mpsc::Sender<Arc<Geometry>>>,
        term_map: &TermMap,
        scroll_y: u32,
        zoom: f32,
    ) {
        let (root_id, arena) = self.parse_and_apply();

        // Reuse the existing canvas when its dimensions still match the
        // terminal. Only allocate a fresh Vec on the first render or after a
        // terminal resize, when the buffer size must change.
        let size_changed = match self.term_layout.as_ref() {
            Some(tl) => tl.columns != term_map.columns || tl.rows != term_map.rows,
            None => true,
        };
        if size_changed {
            self.term_layout = Some(TermLayout::new(term_map, zoom));
        }
        let tl = self
            .term_layout
            .as_mut()
            .expect("TermLayout just allocated above or reused from prior render");

        let boxes = match tl.draw(&arena, root_id, term_map) {
            Ok(b) => b,
            Err(e) => {
                log::error!("Layout failed: {}", e);
                return;
            }
        };

        // Install the document `<title>` as the terminal window title once
        // per session. The DOM is re-parsed on every `Redraw` but the title
        // is static, so this only runs on the first successful layout.
        if !self.title_set {
            if let Some(title) = extract_title(&arena, root_id) {
                execute!(stdout(), crossterm::terminal::SetTitle(title)).unwrap_or_else(|e| {
                    log::error!("Failed to set terminal title: {}", e)
                });
            }
            self.title_set = true;
        }

        // Publish the layout snapshot: cache it for `:hover` re-styling and
        // forward a copy to the hit-test thread when one is listening (the
        // event system is active).
        let geo = Arc::new(Geometry {
            arena,
            boxes,
            root_id,
        });
        self.geometry = Some(Arc::clone(&geo));
        if let Some(tx) = geo_tx
            && let Err(e) = tx.send(geo)
        {
            // No hit-test thread is listening (plain `render`), so the
            // snapshot has no consumer — benign.
            log::debug!("Hit-test channel closed: {}", e);
        }

        // Publish the new upper bound of `scroll_y` so the input loop can
        // drop events that would no-op against it. u32::MAX is used as the
        // startup placeholder (no upper bound known yet), and the first
        // successful layout will overwrite it with the real value.
        self.max_scroll.store(tl.max_scroll(), Ordering::Relaxed);

        tl.scroll_y = scroll_y;
        tl.clamp_scroll();

        // The incremental renderer emits only the changed cells (and a
        // one-time full clear on the first frame / after resize), so redraws
        // need no per-frame Clear. `flush_frame` is the single I/O per frame,
        // skipped entirely when nothing changed.
        tl.render(out);
        flush_frame(out);
    }

    /// Replace the session document and fully re-render it, keeping the
    /// current scroll position / zoom.
    ///
    /// Backs `RenderCmd::RenderHtml` — the dynamic-content entry point that
    /// lets event handlers rebuild the document (e.g. a counter that
    /// re-renders on every click).
    fn render_html<W: Write>(
        &mut self,
        out: &mut BufWriter<W>,
        geo_tx: &Option<mpsc::Sender<Arc<Geometry>>>,
        term_map: &TermMap,
        html: Arc<str>,
    ) {
        let (scroll_y, zoom) = match &self.term_layout {
            Some(tl) => (tl.scroll_y, tl.zoom),
            None => (0, 1.0),
        };
        self.html = html;
        // The new document may carry a different `<title>`.
        self.title_set = false;
        self.redraw(out, geo_tx, term_map, scroll_y, zoom);
    }
}

fn render_thread(
    rx: mpsc::Receiver<RenderCmd>,
    html: Arc<str>,
    css_rules: Arc<CssRule>,
    max_scroll: Arc<AtomicU32>,
    geo_tx: Option<mpsc::Sender<Arc<Geometry>>>,
) {
    // One persistent write buffer for the whole session: every frame writes
    // into it and is flushed exactly once, so a frame costs a single write
    // syscall instead of one per render call.
    let mut out = BufWriter::new(stdout());
    let mut state = RenderState {
        html,
        css_rules,
        term_layout: None,
        title_set: false,
        geometry: None,
        max_scroll,
    };

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
                let Some(tl) = state.term_layout.as_mut() else {
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
                // avoid the diff pass.
                if tl.scroll_y == prev {
                    continue;
                }
                // The incremental renderer diffs the shifted viewport against
                // the previous frame, so scrolling needs no full-screen Clear.
                tl.render(&mut out);
                flush_frame(&mut out);
            }

            RenderCmd::Redraw { scroll_y, zoom } => {
                let term_map = match TermMap::get_ter_map() {
                    Ok(m) => m,
                    Err(e) => {
                        log::error!("Failed to get terminal size: {}", e);
                        continue;
                    }
                };
                state.redraw(&mut out, &geo_tx, &term_map, scroll_y, zoom);
            }

            RenderCmd::RenderHtml { html } => {
                let term_map = match TermMap::get_ter_map() {
                    Ok(m) => m,
                    Err(e) => {
                        log::error!("Failed to get terminal size: {}", e);
                        continue;
                    }
                };
                state.render_html(&mut out, &geo_tx, &term_map, html);
            }

            RenderCmd::Hover { node_id } => {
                let Some(tl) = state.term_layout.as_mut() else {
                    continue;
                };
                let Some(geo) = state.geometry.as_ref() else {
                    continue;
                };
                // Re-run the cascade with the hovered node so `:hover` rules
                // apply, then repaint. The arena is cloned — the shared
                // snapshot stays immutable for the hit-test thread, and a
                // slotmap clone keeps the same keys, so the cached boxes
                // remain valid. With no `:hover` rules this repaints to the
                // same pixels (browser-like no-op).
                let mut arena = geo.arena.clone();
                reset_css_attr(&mut arena, geo.root_id);
                css_into_node_with_hover(&mut arena, geo.root_id, &state.css_rules, node_id);
                tl.repaint_from(&arena, &geo.boxes);
                tl.render(&mut out);
                flush_frame(&mut out);
            }
        }
    }
}

/// Worker of the dedicated hit-test thread.
///
/// Consumes geometry snapshots published by the render thread and replaces
/// the shared hit map with a freshly built region list. Exits when the render
/// thread drops the channel (i.e. on quit).
fn hit_test_thread(rx: mpsc::Receiver<Arc<Geometry>>, hit_map: Arc<Mutex<Arc<HitMap>>>) {
    while let Ok(geo) = rx.recv() {
        let map = Arc::new(HitMap::build(&geo));
        match hit_map.lock() {
            Ok(mut guard) => *guard = map,
            Err(e) => log::error!("Hit map lock poisoned: {}", e),
        }
    }
    log::debug!("Hit-test thread exiting");
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

/// Render an HTML document in the terminal with keyboard navigation, mouse
/// scrolling and zoom.
///
/// This is the plain entry point without event handling; use
/// [`render_with_events`] to enable click / hover interaction.
pub fn render(html: &str, css: &str) {
    run(html, css, None);
}

/// Render an HTML document with JS-like event handling enabled.
///
/// In addition to everything [`render`] does, elements that declare `onclick`
/// / `onmouseover` / `onmouseout` attributes become hit regions, a dedicated
/// hit-test thread records their positions, and events bubble through the DOM
/// ancestor chain to the handlers registered in `bindings`. Hovering produces
/// no visual highlight (matching a browser without CSS `:hover` rules) —
/// only the `onmouseover` / `onmouseout` events fire. See the
/// [`events`](crate::events) module for the event model.
pub fn render_with_events(html: &str, css: &str, bindings: EventBindings) {
    run(html, css, Some(bindings));
}

fn run(html: &str, css: &str, mut bindings: Option<EventBindings>) {
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

    // Geometry snapshot channel (render thread → hit-test thread) and the
    // published hit map (hit-test thread → input loop). The hit-test thread
    // is only spawned when event handling is requested.
    let (geo_tx, geo_rx) = mpsc::channel::<Arc<Geometry>>();
    let hit_map: Arc<Mutex<Arc<HitMap>>> = Arc::new(Mutex::new(Arc::new(HitMap::new())));
    if bindings.is_some() {
        let hit_map_thread = Arc::clone(&hit_map);
        std::thread::spawn(move || hit_test_thread(geo_rx, hit_map_thread));
    }

    let (tx, rx) = mpsc::channel::<RenderCmd>();
    let render_handle = std::thread::spawn({
        let html_clone = Arc::clone(&html);
        let css_clone = Arc::clone(&css_rules);
        let max_scroll_clone = Arc::clone(&max_scroll);
        let geo_tx = bindings.is_some().then_some(geo_tx);
        move || {
            render_thread(rx, html_clone, css_clone, max_scroll_clone, geo_tx);
        }
    });

    let mut state = InputState::new();
    let mut rows: u16 = match TermMap::get_ter_map() {
        Ok(m) => m.rows,
        Err(e) => {
            log::error!("Failed to get terminal size: {}", e);
            24
        }
    };

    if let Err(e) = tx.send(RenderCmd::Redraw {
        scroll_y: state.scroll_y,
        zoom: state.zoom,
    }) {
        log::error!("Failed to send initial Redraw: {}", e);
        if let Err(e2) = tx.send(RenderCmd::Quit) {
            log::error!("Failed to send Quit during cleanup: {}", e2);
        }
    }

    let mut quit = false;
    while !quit {
        // Detect a dead render thread (e.g. from a panic) promptly. Rust's
        // spawned-thread panic handler swallows the panic and kills just the
        // worker, so without this check the main loop would keep polling
        // events, fail to send, and only exit when the user presses a key.
        if render_handle.is_finished() {
            log::error!("Render thread has terminated unexpectedly");
            break;
        }

        let cmds = if bindings.is_some() {
            let map = match hit_map.lock() {
                Ok(guard) => Arc::clone(&guard),
                Err(e) => {
                    log::error!("Hit map lock poisoned: {}", e);
                    break;
                }
            };
            trigger_refresh(
                &mut state,
                rows,
                max_scroll.load(Ordering::Relaxed),
                Some(&map),
                bindings.as_mut(),
                Some(Duration::from_millis(100)),
            )
        } else {
            trigger_refresh(
                &mut state,
                rows,
                max_scroll.load(Ordering::Relaxed),
                None,
                None,
                Some(Duration::from_millis(100)),
            )
        };

        for cmd in cmds {
            if matches!(cmd, RenderCmd::Quit) {
                if let Err(e) = tx.send(RenderCmd::Quit) {
                    log::error!("Failed to send Quit: {}", e);
                }
                quit = true;
                break;
            }
            if let RenderCmd::Redraw { .. } = &cmd
                && let Ok(map) = TermMap::get_ter_map()
            {
                rows = map.rows;
            }
            if let Err(e) = tx.send(cmd) {
                log::error!("Failed to send render command: {}", e);
                quit = true;
                break;
            }
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
