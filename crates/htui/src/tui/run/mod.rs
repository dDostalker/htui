//! `render` / `render_with_events` entry-point machinery.
//!
//! Owns the render thread HTML → layout → paint → ANSI diff output，
//! the hit-test thread geometry snapshots → shared `HitMap`,
//!  the incremental region-repaint helpers,
//!  and the input loop that translates keys mouse
//! events into `RenderCmd`s.

mod overlay;
mod pipeline;
mod region;
mod selection;
mod state;
mod thread;

use self::pipeline::combined_css_source;
use self::thread::{fold_script_event, hit_test_thread, render_thread};

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
    events::{Geometry, HitMap},
    html::{css_parse::DynamicState, html_parse::Parser},
    layout::layout::{MAX_TERM_COLS, MAX_TERM_ROWS, TermMap},
    resources,
    script::{ScriptMsg, ScriptResponse, run_script_thread},
    tdom::node::{NodeArena, NodeId},
    tui::graph_api::{
        DirtyInfo, InputState, KEY_GATE_TIMEOUT, RenderCmd, apply_focus_requests,
        reapply_console_modes, trigger_refresh,
    },
};

/// Whether the terminal is currently in raw mode / alternate screen
static TERMINAL_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Maximum number of pointer-move messages that may wait for the script
/// thread. Extra moves are dropped.
const MAX_IN_FLIGHT_MOVES: usize = 8;

pub(crate) fn run(html: &str, css: &str, session: &mut Session) {
    // register log hook
    install_panic_hook();

    // crossterm init
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
    //
    TERMINAL_ACTIVE.store(true, Ordering::SeqCst);

    // load source
    let document = session.with_registry(|| {
        let mut parser = Parser::new(html);
        parser.parse_with_scripts()
    });
    let css_source: Arc<str> = Arc::from(combined_css_source(&document.css, css));
    let config = *session.config();
    let registry = session.registry_handle();
    let bindings = if config.events {
        Some(session.take_bindings())
    } else {
        None
    };

    // Shared scroll upper bound
    let max_scroll = Arc::new(AtomicU32::new(u32::MAX));

    // The render thread sends layout snapshots to the hit-test thread.
    // The hit-test thread publishes clickable regions in this shared map.
    let (geo_tx, geo_rx) = mpsc::channel::<Arc<Geometry>>();
    let hit_map: Arc<Mutex<Arc<HitMap>>> = Arc::new(Mutex::new(Arc::new(HitMap::new())));

    // Input-loop requests go to the script thread; its responses come back
    // on the second channel. Start that thread only when event bindings exist.
    let (script_tx, script_rx) = mpsc::channel::<ScriptMsg>();
    let (script_out_tx, script_out_rx) = mpsc::channel::<ScriptResponse>();
    // start js thread
    let script_thread: Option<std::thread::JoinHandle<()>> = bindings.map(|bindings| {
        let script_document = document.clone();
        let hit_map_thread = Arc::clone(&hit_map);
        let script_registry = Arc::clone(&registry);
        std::thread::spawn(move || {
            resources::with_registry(script_registry, || {
                run_script_thread(
                    script_document,
                    bindings,
                    script_rx,
                    script_out_tx,
                    hit_map_thread,
                )
            });
        })
    });

    // Wait for the engine to come up — the first frame needs its live arena.
    // `live_dom` is the input loop's latest DOM snapshot for read-only queries;
    // it is refreshed whenever JS mutates the arena.
    let mut live_dom: Option<(Arc<NodeArena>, NodeId)> = None;
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

    // Text of the current page selection, refreshed by the render thread and
    // read by the input loop on `Ctrl+C` (shared through `Arc<Mutex<_>>`).
    let selection_text: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    let (tx, rx) = mpsc::channel::<RenderCmd>();
    // Frame-completion heartbeat: the render thread bumps it after every
    // processed command batch; the input loop's watchdog below uses it to
    // distinguish a *stuck* render thread from an idle one.
    let render_beat: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
    let render_handle = std::thread::spawn({
        let render_document = document;
        let css_clone = Arc::clone(&css_source);
        let max_scroll_clone = Arc::clone(&max_scroll);
        let selection_text_clone = Arc::clone(&selection_text);
        let render_beat_clone = Arc::clone(&render_beat);
        let render_registry = Arc::clone(&registry);
        let geo_tx = events_active.then_some(geo_tx);
        move || {
            // The render thread resolves `<img src>` / `<script src>` / `<link
            // href>` through the session's registry for its whole lifetime.
            resources::with_registry(render_registry, || {
                render_thread(
                    rx,
                    render_document,
                    css_clone,
                    max_scroll_clone,
                    geo_tx,
                    selection_text_clone,
                    render_beat_clone,
                    config,
                );
            });
        }
    });

    let mut state = InputState::new();
    state.zoom = config.zoom;
    state.copy_flow = config.copy_flow;
    state.copy_block = config.copy_block;
    state.selection_text = Arc::clone(&selection_text);
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
    // Pointer moves sent to the script thread but not answered yet.
    // Any script response can free a slot, so the count never rises above
    // `MAX_IN_FLIGHT_MOVES`.
    let mut moves_in_flight: usize = 0;
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
                Some(Duration::from_millis(100)),
            )
        } else {
            trigger_refresh(
                &mut state,
                rows,
                max_scroll_val,
                None,
                None,
                &mut msgs,
                Some(Duration::from_millis(100)),
            )
        };

        // A resize invalidates every canvas coordinate the input loop caches:
        // without this a press whose release the console dropped during the
        // resize would stick and turn all later motion into a drag (hover /
        // click events go silent), and a stale text selection / open dropdown
        // overlay would paint wrong cells.
        if size_changed {
            state.invalidate_viewport(&mut cmds);
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
        for msg in msgs {
            if let ScriptMsg::Move { .. } = msg {
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
            while !resolved {
                match script_out_rx.recv_timeout(KEY_GATE_TIMEOUT) {
                    Ok(ScriptResponse::Event(ev)) => {
                        let verdict = ev.key_gate.filter(|(id, _)| *id == gate.id);
                        fold_script_event(&state, &mut cmds, &mut hovered, &mut live_dom, ev);
                        moves_in_flight = moves_in_flight.saturating_sub(1);
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

        // Fold the script thread's replies into this frame: render commands
        // pushed by host functions, hover updates, and — when JS mutated the
        // DOM — a fresh arena snapshot + dirty set for a (region) repaint.
        for resp in script_out_rx.try_iter() {
            match resp {
                ScriptResponse::Event(ev) => {
                    fold_script_event(&state, &mut cmds, &mut hovered, &mut live_dom, ev);
                    moves_in_flight = moves_in_flight.saturating_sub(1);
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
