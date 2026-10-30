//! `render` / `render_with_events` entry-point machinery.
//!
//! Owns the render thread HTML → layout → paint → ANSI diff output，
//! the hit-test thread geometry snapshots → shared `HitMap`,
//!  the incremental region-repaint helpers,
//!  and the input loop that translates keys mouse
//! events into `RenderCmd`s.

#[cfg(debug_assertions)]
mod hot_reload;
mod overlay;
mod pipeline;
mod region;
mod selection;
mod state;
mod thread;

use self::thread::{fold_script_event, hit_test_thread, render_thread};

use std::collections::HashMap;
use std::io::stdout;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::{
    cursor::{Hide, Show},
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size,
    },
};

use crate::Session;
use crate::{
    events::{HitMap, SharedGeometry, SharedHitMap},
    html::{css_parse::DynamicState, css_parse::Viewport, html_parse::Parser},
    layout::layout::{MAX_TERM_COLS, MAX_TERM_ROWS, TermMap},
    resources,
    script::SharedUiState,
    script::{ScriptMsg, ScriptResponse, run_script_thread},
    source::SharedSourceText,
    tdom::node::{NodeId, SharedNodeArena},
    tui::graph_api::{
        ANIM_POLL_MS, DirtyInfo, InputState, KEY_GATE_TIMEOUT, RenderCmd, ScrollAnim, ScrollTarget,
        apply_focus_requests, reapply_console_modes, scroll_into_view_target, trigger_refresh,
    },
};

/// The input loop's latest read-only DOM snapshot.
type LiveDocument = (SharedNodeArena, NodeId);

/// Channel used to publish immutable layout snapshots.
type GeometrySender = Option<mpsc::Sender<SharedGeometry>>;

/// Shared slot the render thread publishes the latest frame's geometry into;
/// the input loop reads its arena as the DOM-read snapshot.
pub(super) type LiveGeoSlot = Arc<Mutex<Option<SharedGeometry>>>;

/// Whether the terminal is currently in raw mode / alternate screen
static TERMINAL_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Maximum number of pointer-move messages that may wait for the script
/// thread. Extra moves are dropped.
const MAX_IN_FLIGHT_MOVES: usize = 8;

/// Debug source paths retained by `include_html!` / `include_css!` /
/// `include_js!`. Release sources carry no paths.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct HotReloadPaths<'a> {
    pub html: Option<&'a str>,
    pub css: Option<&'a str>,
    pub js: Option<&'a str>,
}

#[cfg_attr(not(debug_assertions), allow(dead_code))]
impl<'a> HotReloadPaths<'a> {
    #[cfg(debug_assertions)]
    fn watches(self) -> hot_reload::SourceWatches<'a> {
        [
            self.html.map(|path| (hot_reload::SourceKind::Html, path)),
            self.css.map(|path| (hot_reload::SourceKind::Css, path)),
            self.js.map(|path| (hot_reload::SourceKind::Js, path)),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    pub(crate) fn any(self) -> bool {
        self.html.is_some() || self.css.is_some() || self.js.is_some()
    }
}

pub(crate) fn run(html: &str, css: &str, js: &str, session: &mut Session) {
    run_inner(html, css, js, session, HotReloadPaths::default());
}

/// Start a debug render with hot reload for every source that has a path.
///
/// The watcher is part of the debug-only branch: release binaries never
/// reference this entry point.
#[cfg(debug_assertions)]
pub(crate) fn run_hot(
    paths: HotReloadPaths<'_>,
    html: &str,
    css: &str,
    js: &str,
    session: &mut Session,
) {
    run_inner(html, css, js, session, paths);
}

fn run_inner(
    html: &str,
    css: &str,
    js: &str,
    session: &mut Session,
    hot_reload_paths: HotReloadPaths<'_>,
) {
    // The paths are consumed only by the debug watcher below.
    #[cfg(not(debug_assertions))]
    let _ = hot_reload_paths;
    install_panic_hook();

    if let Err(e) = enable_raw_mode() {
        log::error!("Failed to enable raw mode: {}", e);
        return;
    }
    if let Err(e) = execute!(
        stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste,
        Hide
    ) {
        log::error!("Failed to enter alternate screen: {}", e);
        if let Err(e2) = disable_raw_mode() {
            log::error!("Failed to disable raw mode during cleanup: {}", e2);
        }
        return;
    }
    TERMINAL_ACTIVE.store(true, Ordering::SeqCst);

    // Keep the original HTML available for debug `mem://` re-parses. A
    // registry change cannot carry the document text, so this source is sent
    // back through the script engine to resolve every resource again.
    let html_source: SharedSourceText = Arc::from(html);
    let document = session.with_registry(|| {
        let mut parser = Parser::new(&html_source);
        parser.parse_with_scripts()
    });
    let session_css: SharedSourceText = Arc::from(css);
    let session_js: SharedSourceText = Arc::from(js);
    let config = *session.config();

    // A debug-only side channel reads stable source edits. HTML goes to the
    // script thread, JS re-evaluates in the script thread, and CSS is applied
    // by the render thread. Release builds compile this to no watcher.
    #[cfg(debug_assertions)]
    let hot_reload = (config.events && hot_reload_paths.any())
        .then(|| hot_reload::spawn(hot_reload_paths.watches()));
    #[cfg(debug_assertions)]
    let mut hot_css: Option<String> = None;

    let registry = session.registry_handle();
    let bindings = if config.events {
        Some(session.take_bindings())
    } else {
        None
    };

    let max_scroll = Arc::new(AtomicU32::new(u32::MAX));

    // The render thread sends layout snapshots to the hit-test thread.
    // The hit-test thread publishes clickable regions in this shared map.
    let (geo_tx, geo_rx) = mpsc::channel::<SharedGeometry>();
    let hit_map: SharedHitMap = Arc::new(Mutex::new(Arc::new(HitMap::new())));

    // Input-loop requests go to the script thread; its responses come back
    // on the second channel. Start that thread only when event bindings exist.
    let (script_tx, script_rx) = mpsc::channel::<ScriptMsg>();
    let (script_out_tx, script_out_rx) = mpsc::channel::<ScriptResponse>();
    // The viewport / scroll offset the input loop owns, shared with the
    // script thread for the `window.innerWidth` & co getters (relaxed
    // atomics — advisory values, no synchronization).
    let ui_state = Arc::new(SharedUiState::default());
    let script_thread: Option<std::thread::JoinHandle<()>> = bindings.map(|bindings| {
        let script_document = document.clone();
        let hit_map_thread = Arc::clone(&hit_map);
        let script_registry = Arc::clone(&registry);
        let ui_state_thread = Arc::clone(&ui_state);
        std::thread::spawn(move || {
            resources::with_registry(script_registry, || {
                run_script_thread(
                    script_document,
                    bindings,
                    Arc::clone(&session_js),
                    script_rx,
                    script_out_tx,
                    hit_map_thread,
                    ui_state_thread,
                )
            });
        })
    });

    // Wait for the engine to come up — the first frame needs its live arena.
    // `live_dom` is the input loop's latest DOM snapshot for read-only queries;
    // it is refreshed whenever JS mutates the arena.
    let mut live_dom: Option<LiveDocument> = None;
    if script_thread.is_some() {
        match script_out_rx.recv() {
            Ok(ScriptResponse::Ready { arena, root_id }) => live_dom = Some((arena, root_id)),
            Ok(ScriptResponse::Failed(err)) => {
                log::error!("Failed to initialize script engine: {err}");
            }
            Ok(ScriptResponse::Event(_)) => {
                log::error!("Script thread sent an event before Ready");
            }
            Err(e) => log::error!("Script thread init channel error: {e}"),
        }
    }
    let events_active = live_dom.is_some();
    if events_active {
        let hit_map_thread = Arc::clone(&hit_map);
        std::thread::spawn(move || hit_test_thread(geo_rx, hit_map_thread));
    }

    // Subscribe after the engine is ready so debug `mem://` invalidation only
    // runs when a script engine exists to perform the whole-document reparse.
    #[cfg(debug_assertions)]
    let mem_reload_rx = events_active.then(|| registry.subscribe_reloads());
    #[cfg(debug_assertions)]
    let mut mem_reload;

    // Text of the current page selection, refreshed by the render thread and
    // read by the input loop on `Ctrl+C` (shared through `Arc<Mutex<_>>`).
    let selection_text: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    // Scrollbar chrome starts hidden and is revealed by interaction. Sharing
    // the flag with the render thread makes the hide transition atomic with
    // the repaint that removes it.
    let scrollbars_visible: crate::tui::graph_api::SharedScrollbarVisibility =
        Arc::new(Mutex::new(HashMap::new()));

    let (tx, rx) = mpsc::channel::<RenderCmd>();
    // Frame-completion heartbeat: the render thread bumps it after every
    // processed command batch; the input loop's watchdog below uses it to
    // distinguish a *stuck* render thread from an idle one.
    let render_beat: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
    // The render thread publishes each frame's geometry here; the input loop
    // reads its arena instead of holding the script thread's snapshots.
    let live_geo: LiveGeoSlot = Arc::new(Mutex::new(None));
    let render_handle = std::thread::spawn({
        let render_document = document;
        let css_clone = Arc::clone(&session_css);
        let max_scroll_clone = Arc::clone(&max_scroll);
        let selection_text_clone = Arc::clone(&selection_text);
        let scrollbars_visible_clone = Arc::clone(&scrollbars_visible);
        let render_beat_clone = Arc::clone(&render_beat);
        let live_geo_clone = Arc::clone(&live_geo);
        let render_registry = Arc::clone(&registry);
        let geo_tx = events_active.then_some(geo_tx);
        move || {
            resources::with_registry(render_registry, || {
                render_thread(
                    rx,
                    render_document,
                    css_clone,
                    max_scroll_clone,
                    geo_tx,
                    selection_text_clone,
                    scrollbars_visible_clone,
                    render_beat_clone,
                    live_geo_clone,
                    config,
                );
            });
        }
    });

    let mut state = InputState::new();
    state.zoom = config.zoom;
    state.copy_flow = config.copy_flow;
    state.copy_block = config.copy_block;
    state.smooth_scroll = config.smooth_scroll;
    state.selection_text = Arc::clone(&selection_text);
    state.scrollbars_visible = Arc::clone(&scrollbars_visible);
    let (mut columns, mut rows): (u16, u16) = match TermMap::get_ter_map() {
        Ok(m) => (m.columns, m.rows),
        Err(e) => {
            log::error!("Failed to get terminal size: {}", e);
            (80, 24)
        }
    };

    // First frame: the engine's live arena (scripts already ran and may have
    // mutated the DOM) or a plain parse-based redraw.
    let first_cmd = match &live_dom {
        Some((arena, root_id)) => RenderCmd::RenderArena {
            arena: Arc::clone(arena),
            root_id: *root_id,
            scroll_y: state.scroll_y,
            zoom: state.zoom,
            dynamic: DynamicState::default(),
            dirty: DirtyInfo::default(),
        },
        None => RenderCmd::Redraw {
            scroll_y: state.scroll_y,
            zoom: state.zoom,
        },
    };
    if let Err(e) = tx.send(first_cmd) {
        log::error!("Failed to send initial render: {}", e);
        if let Err(e2) = tx.send(RenderCmd::Quit) {
            log::error!("Failed to send Quit during cleanup: {}", e2);
        }
    }

    // Current `:hover` target, reported by the script thread after every
    // pointer message; feeds `RenderArena.hovered` so re-cascades re-apply
    // the highlight.
    let mut hovered: Option<NodeId> = None;

    let mut quit = false;
    // Whether the previous loop pass engaged the script thread (a message
    // was sent to it, or one of its replies was folded). Replies sitting on
    // `script_out_rx` are only noticed between polls, so the poll timeout
    // gates every JS-driven frame: a timer / rAF page animating at ~60 Hz
    // would be throttled to the idle 100 ms cadence (~10 fps). While
    // engaged, the loop polls at the animation cadence instead — the same
    // mechanism a running scroll animation uses. One fast pass after the
    // engagement ends costs an idle page a single 16 ms wake.
    let mut script_activity = false;
    // Pointer moves sent to the script thread but not answered yet.
    // Any script response can free a slot, so the count never rises above
    // `MAX_IN_FLIGHT_MOVES`.
    let mut moves_in_flight: usize = 0;
    // Scroll offset at the previous loop pass: the input loop notifies the
    // script thread (`ScriptMsg::Scroll`) when it differs.
    let mut scroll_y_prev: u32 = 0;
    // Render-stall watchdog state: `(heartbeat at last command send, when it
    // was sent)`. The render thread bumps its heartbeat after each processed
    // batch, so a send followed by no heartbeat for a while means the render
    // thread is stuck (not merely idle — an idle render thread receives no
    // commands, so no send happens and this stays `None`).
    let mut render_stall: Option<(u64, Instant)> = None;
    while !quit {
        if let Some((beat_at_send, sent_at)) = render_stall {
            let beat = render_beat.load(Ordering::Relaxed);
            if beat != beat_at_send {
                // A frame completed — the render thread is alive.
                render_stall = None;
            } else if sent_at.elapsed() > Duration::from_secs(2) {
                log::warn!(
                    "渲染线程疑似卡死：{} 秒未完成任何帧（心跳停在 {})",
                    sent_at.elapsed().as_secs(),
                    beat_at_send
                );
                // Throttle: do not re-warn every iteration while stuck.
                render_stall = None;
            }
        }

        // Detect a dead render thread (e.g. from a panic) promptly. Rust's
        // spawned-thread panic handler swallows the panic and kills just the
        // worker, so without this check the main loop would keep polling
        // events, fail to send, and only exit when the user presses a key.
        if render_handle.is_finished() {
            log::error!("Render thread has terminated unexpectedly");
            break;
        }

        // A dead script thread must not be silently ignored either: without
        // this check the loop would keep sending messages into a void and
        // hover / events would freeze while scrolling still worked.
        if events_active && script_thread.as_ref().is_some_and(|h| h.is_finished()) {
            log::error!("Script thread has terminated unexpectedly");
            break;
        }

        let mut msgs: Vec<ScriptMsg> = Vec::new();
        // Coalesce mem:// invalidations into one reparse. Resource registration
        // may happen from a host callback thread while the input loop sleeps.
        #[cfg(debug_assertions)]
        if let Some(rx) = &mem_reload_rx {
            mem_reload = false;
            while rx.try_recv().is_ok() {
                mem_reload = true;
            }
            if mem_reload {
                log::info!("Hot reloading mem:// resources by re-parsing HTML");
                msgs.push(ScriptMsg::ReloadHtml {
                    html: Arc::clone(&html_source),
                });
            }
        }
        // Coalesce every watcher update that arrived while the input loop was
        // blocked / processing into the latest HTML. Rapid saves produce one
        // script-thread reload rather than a queue of obsolete documents.
        #[cfg(debug_assertions)]
        if let Some(watcher) = &hot_reload {
            let mut latest_html: Option<String> = None;
            let mut latest_js: Option<String> = None;
            while let Ok(event) = watcher.receiver().try_recv() {
                match event {
                    hot_reload::HotReloadEvent::Html(html) => latest_html = Some(html),
                    hot_reload::HotReloadEvent::Css(css) => hot_css = Some(css),
                    hot_reload::HotReloadEvent::Js(js) => latest_js = Some(js),
                }
            }
            if let Some(html) = latest_html {
                log::info!("Hot reloading HTML document");
                msgs.push(ScriptMsg::ReloadHtml {
                    html: Arc::from(html.as_str()),
                });
            }
            if let Some(js) = latest_js {
                log::info!("Hot reloading session JS");
                msgs.push(ScriptMsg::ReloadJs {
                    js: Arc::from(js.as_str()),
                });
            }
        }
        // Detect terminal size changes ourselves instead of relying solely on
        // `Event::Resize`: some resize paths never emit the event (a legacy
        // Windows console can resize its window without changing the buffer),
        // and after such a resize the render thread keeps drawing to a stale
        // viewport while page-step keys use a stale `rows` — together this
        // looks like input is frozen. Polling per iteration re-syncs both
        // within one loop pass and (on Windows) re-applies the console input
        // mode the resize may have reset. The size is normalized exactly like
        // `TermMap::get_ter_map` so both sides always agree.
        let mut size_changed = false;
        match size() {
            Ok((c, r)) => {
                // Clamp the same way `TermMap::get_ter_map` does: a transiently
                // inverted Windows `SMALL_RECT` during a resize can report
                // values like 65535 (i16 → u16 wrap), which would make the
                // render thread allocate a gigantic canvas and freeze the app.
                let c = c.clamp(1, MAX_TERM_COLS);
                let r = r.clamp(1, MAX_TERM_ROWS);
                size_changed = c != columns || r != rows;
                if size_changed {
                    log::info!("终端尺寸变化: {}x{} -> {}x{}", columns, rows, c, r);
                }
                columns = c;
                rows = r;
            }
            Err(e) => log::error!("Failed to get terminal size: {}", e),
        }
        // Re-apply the console input mode every iteration, not just when the
        // size *changed*: a Windows resize can reset it asynchronously (the
        // console finishes the resize after our re-apply), and once the size
        // stabilises nothing else would ever re-enable mouse capture / raw
        // mode — mouse and keyboard events stay dead until the next resize.
        // The calls are cheap and idempotent (no-op off Windows).
        reapply_console_modes();

        // Mirror the render thread's scroll clamp before computing any input:
        // a terminal grow (or a document reload) can shrink the content below
        // the current `scroll_y`. Without this the boundary checks below
        // disagree with the on-screen position — every wheel / arrow `Rerender`
        // gets clamped to the same value in the render thread and dropped as a
        // no-op, so scrolling appears dead after a resize.
        let max_scroll_val = max_scroll.load(Ordering::Relaxed);
        state.clamp_scroll_to(max_scroll_val);
        // The input loop's DOM-read snapshot is the latest *rendered* frame's
        // arena (published by the render thread above), not the script
        // thread's snapshot: holding the snapshot would keep the render thread
        // from unwrapping and cascading it in place — one full-tree copy per
        // batch instead of two. The reads run at most one frame behind.
        if let Ok(guard) = live_geo.lock()
            && let Some(geo) = guard.as_ref()
        {
            live_dom = Some((Arc::clone(&geo.arena), geo.root_id));
        }
        // Publish the UI state the script thread's view getters read
        // (`window.innerWidth` / `innerHeight` / `scrollY`). Cheap relaxed
        // stores every iteration; the values are advisory.
        ui_state.cols.store(u32::from(columns), Ordering::Relaxed);
        ui_state.rows.store(u32::from(rows), Ordering::Relaxed);
        ui_state.scroll_y.store(state.scroll_y, Ordering::Relaxed);
        // The page scroll offset changed since the previous iteration (wheel,
        // keys, animation tick, `scrollTo`): notify the script thread so
        // `scroll` listeners run. One frame behind the paint — the detection
        // runs before this iteration's animation tick — which matches how
        // browsers coalesce scroll events.
        if events_active && state.scroll_y != scroll_y_prev {
            msgs.push(ScriptMsg::Scroll);
        }
        scroll_y_prev = state.scroll_y;
        // While a scroll animation is active the loop wakes at the animation
        // frame rate instead of the idle poll cadence; eased positions are
        // wall-clock based, so a late wake self-corrects next frame. Recent
        // script-thread engagement (see `script_activity`) takes the same
        // fast path.
        let script_engaged = script_activity;
        script_activity = false;
        let poll_timeout = if state.scroll_anim.is_some() || script_engaged {
            Duration::from_millis(ANIM_POLL_MS)
        } else {
            Duration::from_millis(100)
        };
        let mut cmds = if events_active {
            let map = match hit_map.lock() {
                Ok(guard) => Arc::clone(&guard),
                Err(e) => {
                    log::error!("Hit map lock poisoned: {}", e);
                    break;
                }
            };
            // A pending viewport change has now reached the *rebuilt* hit map
            // (the resize render finished and the hit-test thread published
            // it), so re-validate the pointer hover against the fresh
            // geometry: the element under a stationary pointer may have
            // reflowed to something else, and a synthetic `Move` fires the
            // correct MouseOut / MouseOver (+ a fresh `Hover` highlight).
            if state.take_resize_revalidation()
                && let Some((col, row)) = state.last_mouse
            {
                let (x, y) = (col as u32, row as u32 + state.scroll_y);
                msgs.push(ScriptMsg::Move { x, y });
            }
            trigger_refresh(
                &mut state,
                rows,
                max_scroll_val,
                Some(&map),
                live_dom.as_ref().map(|(arena, _)| arena.as_ref()),
                &mut msgs,
                Some(poll_timeout),
            )
        } else {
            trigger_refresh(
                &mut state,
                rows,
                max_scroll_val,
                None,
                None,
                &mut msgs,
                Some(poll_timeout),
            )
        };

        // The input loop owns the timeout and supplies the repaint at hide
        // time; the shared atomic is already false when this frame renders.
        let hidden_scrollbars = state.take_expired_scrollbar_visibility();
        if !hidden_scrollbars.is_empty() {
            cmds.push(RenderCmd::Redraw {
                scroll_y: state.scroll_y,
                zoom: state.zoom,
            });
        }

        // A resize invalidates every canvas coordinate the input loop caches:
        // without this a press whose release the console dropped during the
        // resize would stick and turn all later motion into a drag (hover /
        // click events go silent), and a stale text selection / open dropdown
        // overlay would paint wrong cells.
        if size_changed {
            // Notify the script thread so `resize` listeners and `matchMedia`
            // re-evaluate against the new viewport (the message itself is
            // dataless; the getters read the shared UI state published above).
            if events_active {
                msgs.push(ScriptMsg::Resize);
            }
            state.invalidate_viewport(&mut cmds);
        }

        // Queue a CSS source replacement after any viewport invalidation, but
        // before a forced redraw so both changes merge into the same frame.
        #[cfg(debug_assertions)]
        if let Some(css) = hot_css.take() {
            log::info!("Hot reloading session CSS");
            cmds.push(RenderCmd::ReloadCss {
                css: Arc::from(css.as_str()),
            });
        }

        // Force a re-layout against the new viewport. When the script engine
        // owns the document this is converted into a `RenderArena` below (so
        // JS DOM mutations survive the re-layout); the render thread merges it
        // with any event-driven `Redraw` from the same batch.
        if size_changed {
            cmds.push(RenderCmd::Redraw {
                scroll_y: state.scroll_y,
                zoom: state.zoom,
            });
            // The pointer hover must be re-validated once the re-layout's
            // fresh hit map is published (consumed next iteration, above).
            state.request_resize_revalidation();
        }

        // Send engine work to the script thread. Move messages are capped so
        // a busy script thread cannot accumulate a large pointer backlog.
        script_activity |= !msgs.is_empty();
        for msg in msgs {
            if let ScriptMsg::Move { .. } | ScriptMsg::MouseMove { .. } = msg {
                if moves_in_flight >= MAX_IN_FLIGHT_MOVES {
                    log::debug!(
                        "Script thread busy: dropping pointer move ({moves_in_flight} in flight)"
                    );
                    continue;
                }
                moves_in_flight += 1;
            }
            if let Err(e) = script_tx.send(msg) {
                log::error!("Failed to send script message: {}", e);
                quit = true;
                break;
            }
        }

        // Gate a page-level built-in key (scroll / zoom / quit): the script
        // thread's `keydown` response reports whether JS called
        // `preventDefault()`. Clear verdict → the deferred commands are
        // dropped (the page owns that key); no verdict within
        // `KEY_GATE_TIMEOUT` → run the built-in anyway, so a busy or blocked
        // script thread never stalls scrolling / quitting (decoupling). The
        // wait also folds any responses that arrive meanwhile (hover, DOM
        // mutations from key handlers).
        if let Some(gate) = state.key_gate.take() {
            let mut gate_cmds = gate.cmds;
            let mut resolved = false;
            // The budget is an *absolute* deadline, not a per-`recv` timeout:
            // each unrelated response folded below (a hover move, a timer
            // tick) must consume the same window instead of restarting it —
            // a per-recv timeout let a busy script thread's backlog (hot
            // timers, slow per-event handlers) defer the built-in action
            // indefinitely, stalling scrolling / quitting for seconds.
            let deadline = Instant::now() + KEY_GATE_TIMEOUT;
            while !resolved {
                match script_out_rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                {
                    Ok(ScriptResponse::Event(ev)) => {
                        let verdict = ev.key_gate.filter(|(id, _)| *id == gate.id);
                        fold_script_event(
                            &mut state,
                            &mut cmds,
                            &mut hovered,
                            Viewport {
                                cols: columns as usize,
                                rows: rows as usize,
                            },
                            ev,
                        );
                        moves_in_flight = moves_in_flight.saturating_sub(1);
                        script_activity = true;
                        if let Some((_, prevented)) = verdict {
                            if !prevented {
                                log::debug!(
                                    "Key gate cleared for key id {} — applying built-in",
                                    gate.id
                                );
                                cmds.extend(std::mem::take(&mut gate_cmds));
                            }
                            resolved = true;
                        }
                    }
                    Ok(ScriptResponse::Ready { .. } | ScriptResponse::Failed(_)) => {
                        log::error!("Unexpected one-shot script response during the key gate");
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        // Script thread busy / blocked in a host function:
                        // fall back to the built-in so input stays responsive.
                        log::debug!(
                            "Key gate timed out for key id {} — running built-in",
                            gate.id
                        );
                        cmds.extend(std::mem::take(&mut gate_cmds));
                        resolved = true;
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        log::error!("Script thread disconnected during the key gate");
                        cmds.extend(std::mem::take(&mut gate_cmds));
                        resolved = true;
                    }
                }
            }
        }

        // Convert scroll-animation markers into the input loop's animation
        // state, then emit this frame's tick — before the script-response
        // fold, so a DOM-mutation arena appended below still supersedes the
        // tick's viewport command (the render batch folds the newer scroll
        // offset into a pending arena instead of dropping either). A
        // `ScrollSet` marker (JS `scrollTo` without `behavior: "smooth"`)
        // jumps directly: it cancels a running page glide, writes
        // `state.scroll_y` capped at the live ceiling and emits the
        // `Rerender` here.
        let now = Instant::now();
        // Resolve JS `scrollIntoView` markers against the current hit map
        // (the element's document rect lives there, not on the script side)
        // into concrete page-scroll markers the sweep below understands.
        cmds.retain_mut(|cmd| {
            if let RenderCmd::ScrollIntoView {
                node_id,
                block,
                smooth,
            } = cmd
            {
                let resolved = hit_map.lock().ok().and_then(|map| {
                    map.region_for(*node_id)
                        .map(|region| scroll_into_view_target(region.rect, *block, u32::from(rows)))
                });
                match resolved {
                    Some(to) if *smooth => {
                        *cmd = RenderCmd::ScrollAnimate {
                            target: ScrollTarget::Page,
                            to,
                        };
                    }
                    Some(to) => *cmd = RenderCmd::ScrollSet { to },
                    // The element produced no hit region (removed / never
                    // laid out): nothing to scroll to.
                    None => return false,
                }
            }
            true
        });
        // `ScrollSet` jumps emit their `Rerender` after the sweep (the
        // closure borrows `cmds`); the animation tick below merges into the
        // same batch (snapshot commands supersede, latest wins).
        let mut set_rerenders: Vec<RenderCmd> = Vec::new();
        cmds.retain_mut(|cmd| {
            if let RenderCmd::ScrollAnimate { target, to } = cmd {
                match &mut state.scroll_anim {
                    Some(anim) if anim.target() == *target => anim.retarget(*to, now),
                    _ => {
                        let from = match *target {
                            ScrollTarget::Page => state.scroll_y,
                            ScrollTarget::Container(id) => {
                                state.scroll_offsets.get(&id).copied().unwrap_or(0)
                            }
                        };
                        state.scroll_anim = Some(ScrollAnim::start(*target, from, *to, now));
                    }
                }
                false
            } else if let RenderCmd::ScrollSet { to } = cmd {
                let to = (*to).min(max_scroll_val);
                state.scroll_anim = None;
                if to != state.scroll_y {
                    state.scroll_y = to;
                    set_rerenders.push(RenderCmd::Rerender { scroll_y: to });
                }
                false
            } else {
                true
            }
        });
        cmds.splice(0..0, set_rerenders);
        if let Some(anim) = state.scroll_anim.take() {
            // A page glide is capped by the live scroll ceiling (the document
            // may have shrunk since the trigger); a container glide already
            // targets the container's own ceiling.
            let cap = match anim.target() {
                ScrollTarget::Page => max_scroll_val,
                ScrollTarget::Container(_) => anim.to(),
            };
            let pos = anim.position(now).min(cap);
            match anim.target() {
                ScrollTarget::Page => {
                    if pos != state.scroll_y {
                        state.scroll_y = pos;
                        cmds.push(RenderCmd::Rerender { scroll_y: pos });
                    }
                }
                ScrollTarget::Container(id) => {
                    state.show_scrollbar(id);
                    if state.scroll_offsets.get(&id) != Some(&pos) {
                        state.scroll_offsets.insert(id, pos);
                        cmds.push(RenderCmd::RenderScroll {
                            node_id: id,
                            scroll_y: pos,
                        });
                    }
                }
            }
            if !anim.finished(now) {
                state.scroll_anim = Some(anim);
            }
        }

        // Fold the script thread's replies into this frame: render commands
        // pushed by host functions, hover updates, and — when JS mutated the
        // DOM — a fresh arena snapshot + dirty set for a (region) repaint.
        for resp in script_out_rx.try_iter() {
            match resp {
                ScriptResponse::Event(ev) => {
                    fold_script_event(
                        &mut state,
                        &mut cmds,
                        &mut hovered,
                        Viewport {
                            cols: columns as usize,
                            rows: rows as usize,
                        },
                        ev,
                    );
                    moves_in_flight = moves_in_flight.saturating_sub(1);
                    script_activity = true;
                }
                ScriptResponse::Ready { .. } | ScriptResponse::Failed(_) => {
                    log::error!("Unexpected one-shot script response in the input loop");
                }
            }
        }

        // Command merging lives entirely on the render thread's drain
        // ([`DrainedBatch`]): snapshot commands supersede, a `Focus` behind a
        // pending `RenderArena` merges into its interaction state, a newer
        // `Hover` is deferred on top — so neither a DOM mutation nor a
        // highlight delta can be lost between batches.

        // JS `el.focus()` / `el.blur()` requests target input-loop state (the
        // edit session lives here) and route their `focus` / `blur` (+ `change`)
        // events back through the script-message path — apply them now and
        // never forward them to the render thread. `msgs` was already sent
        // above, so the focus transitions flush their own messages immediately.
        let cmds = {
            let mut focus_msgs: Vec<ScriptMsg> = Vec::new();
            let cmds = apply_focus_requests(
                &mut state,
                cmds,
                live_dom.as_ref().map(|(a, _)| a.as_ref()),
                &mut focus_msgs,
            );
            for msg in focus_msgs {
                if let Err(e) = script_tx.send(msg) {
                    log::error!("Failed to send focus message: {}", e);
                    quit = true;
                    break;
                }
            }
            cmds
        };

        let mut sent_any = false;
        for cmd in cmds {
            if matches!(cmd, RenderCmd::Quit) {
                if let Err(e) = tx.send(RenderCmd::Quit) {
                    log::error!("Failed to send Quit: {}", e);
                }
                quit = true;
                break;
            }
            sent_any = true;
            // Capture whether this was a resize / zoom `Redraw` *before* the
            // events-active rewrite below turns it into a `RenderArena` — the
            // terminal-height refresh must run in both paths or PageUp /
            // PageDown keep stepping with the pre-resize row count.
            let was_redraw = matches!(cmd, RenderCmd::Redraw { .. });
            // With the script engine active, the engine's live arena is the
            // source of truth: convert parse-based `Redraw` (zoom / resize)
            // into a `RenderArena` so JS DOM mutations survive re-renders.
            let mut cmd = match (cmd, events_active.then_some(())) {
                (RenderCmd::Redraw { scroll_y, zoom }, Some(())) => match &live_dom {
                    Some((arena, root_id)) => RenderCmd::RenderArena {
                        arena: Arc::clone(arena),
                        root_id: *root_id,
                        scroll_y,
                        zoom,
                        dynamic: DynamicState {
                            hovered,
                            focused: state.focused_input,
                            viewport: Viewport {
                                cols: columns as usize,
                                rows: rows as usize,
                            },
                        },
                        dirty: DirtyInfo::default(),
                    },
                    None => RenderCmd::Redraw { scroll_y, zoom },
                },
                (other, _) => other,
            };
            // Stamp the batch-final focus state onto every outgoing arena:
            // JS `el.focus()` / `el.blur()` requests are expanded *after* the
            // fold, so an arena folded earlier in this batch can still carry
            // the pre-focus `dynamic.focused`. The arena render must paint
            // the state this batch ends with — otherwise the expanded `Focus`
            // delta would be absorbed by a stale snapshot and the `:focus`
            // highlight would freeze on the previous field.
            cmd.refresh_dynamic_focus(state.focused_input);
            if was_redraw && let Ok(map) = TermMap::get_ter_map() {
                rows = map.rows;
            }
            if let Err(e) = tx.send(cmd) {
                log::error!("Failed to send render command: {}", e);
                quit = true;
                break;
            }
        }
        if !quit && sent_any {
            // Arm the render-stall watchdog: remember the heartbeat as of this
            // batch so the next loop iteration can tell "alive" from "stuck".
            render_stall = Some((render_beat.load(Ordering::Relaxed), Instant::now()));
        }
    }

    // Ask the script thread to stop. It is not joined — the process is about
    // to exit, and a host function blocked inside it must not hang shutdown
    // (it holds no terminal resources; it only talks through channels).
    if let Some(handle) = script_thread {
        if script_tx.send(ScriptMsg::Quit).is_err() {
            log::debug!("Script thread already gone; skipping Quit");
        }
        drop(handle);
    }

    // Wake the render thread before joining: the error break paths above (a
    // dead script thread, a poisoned hit-map lock, a failed channel send) exit
    // the loop without any `Quit` in flight, and `tx` stays alive on this
    // stack frame until after `join` — the render thread would block in
    // `rx.recv()` forever and the terminal restore below would never run.
    if let Err(e) = tx.send(RenderCmd::Quit) {
        // The render thread already terminated (e.g. panicked): `join` below
        // returns immediately, nothing is left to wake up.
        log::debug!("Render thread already gone; skipping Quit: {e}");
    }

    if let Err(payload) = render_handle.join() {
        log::error!(
            "Render thread panicked: {}",
            panic_message(payload.as_ref())
        );
    }

    // The engine no longer owns the terminal: a panic from here on (e.g.
    // inside the logging below) must not re-run the restore.
    TERMINAL_ACTIVE.store(false, Ordering::SeqCst);
    if let Err(e) = execute!(
        stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste,
        Show
    ) {
        log::error!("Failed to leave alternate screen: {}", e);
    }
    if let Err(e) = disable_raw_mode() {
        log::error!("Failed to disable raw mode: {}", e);
    }
}

#[cfg(debug_assertions)]
use std::backtrace::Backtrace;
use std::panic;
use std::sync::Once;

fn install_panic_hook() {
    static INSTALL_HOOK: Once = Once::new();

    INSTALL_HOOK.call_once(|| {
        let default_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            // A panic on any thread must not leave the user's terminal in
            // raw mode / alternate screen: the normal cleanup below `run`'s
            // main loop only executes on the regular exit path, and a panic
            // on the input-loop (main) thread unwinds past it. The flag is
            // swapped first so the normal-path cleanup never double-restores.
            if TERMINAL_ACTIVE.swap(false, Ordering::SeqCst) {
                if let Err(e) = disable_raw_mode() {
                    log::error!("Panic-hook raw-mode restore failed: {}", e);
                }
                if let Err(e) = execute!(
                    stdout(),
                    LeaveAlternateScreen,
                    DisableMouseCapture,
                    DisableBracketedPaste,
                    Show
                ) {
                    log::error!("Panic-hook screen restore failed: {}", e);
                }
            }

            default_hook(info);

            let message = panic_message(info.payload());
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
            #[cfg(debug_assertions)]
            let backtrace = Backtrace::force_capture();
            #[cfg(debug_assertions)]
            log::error!(
                "Thread '{thread_name}' panicked at {location}: {message}\nBacktrace:\n{backtrace}"
            );
            #[cfg(not(debug_assertions))]
            log::error!("Thread '{thread_name}' panicked at {location}: {message}");
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
