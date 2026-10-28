//! Worker threads: the render thread (frame loop over `RenderCmd` batches)
//! and the hit-test thread (shared `HitMap` maintenance), plus the script
//! reply folding shared with the input loop.

use std::io::{BufWriter, stdout};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::events::{HitMap, SharedGeometry, SharedHitMap};
use crate::html::css_parse::Viewport;
use crate::html::css_parse::{DynamicState, PseudoClass, css_pseudo_rule_set};
use crate::html::html_parse::ParsedDocument;
use crate::layout::layout::TermMap;
use crate::source::SharedSourceText;
use crate::tdom::node::NodeId;
use crate::tui::graph_api::{DrainedBatch, InputState, RenderCmd, SharedScrollbarVisibility};

use super::GeometrySender;
use super::LiveDocument;
use super::pipeline::{combined_css_source, flush_frame, parse_css_source};
use super::state::RenderState;

pub(super) fn render_thread(
    rx: mpsc::Receiver<RenderCmd>,
    document: ParsedDocument,
    session_css: SharedSourceText,
    max_scroll: Arc<AtomicU32>,
    geo_tx: GeometrySender,
    selection_text: Arc<Mutex<String>>,
    scrollbars_visible: SharedScrollbarVisibility,
    render_beat: Arc<AtomicU64>,
    config: Config,
) {
    // One persistent write buffer for the whole session: every frame writes
    // into it and is flushed exactly once, so a frame costs a single write
    // syscall instead of one per render call.
    let mut out = BufWriter::new(stdout());
    let document_css = Arc::clone(&document.css);
    let css_rules = Arc::new(parse_css_source(&combined_css_source(
        &document_css,
        &session_css,
    )));
    let pseudo_rules = css_pseudo_rule_set(&css_rules);
    let mut state = RenderState {
        document,
        session_css,
        document_css,
        extra_css: String::new(),
        css_rules,
        pseudo_rules,
        term_layout: None,
        title_set: false,
        geometry: None,
        has_pinned: false,
        dynamic: DynamicState::default(),
        text_selection: None,
        selection_text,
        open_select: None,
        scrollbars_visible,
        max_scroll,
    };

    let mut last_frame_at: Option<Instant> = None;
    'frame: loop {
        let first = match rx.recv() {
            Ok(c) => c,
            Err(e) => {
                log::error!("Render channel disconnected: {}", e);
                break;
            }
        };
        // Rate-limit frames: when commands arrive faster than the console can
        // drain a frame (a fast wheel scroll can push a full-viewport diff at
        // every tick), the render thread would otherwise queue frame after
        // frame, saturate the output pipe and stall on the write — which
        // looks exactly like the whole app froze. Waiting out the remainder of
        // `MIN_FRAME_GAP` lets the `try_recv` drain below coalesce the burst
        // into one frame and bounds the ANSI output rate.
        if let Some(t) = last_frame_at {
            let since = t.elapsed();
            if since < config.frame_gap {
                std::thread::sleep(config.frame_gap - since);
            }
        }
        // Commands merge per class (see [`DrainedBatch`]): snapshot
        // commands supersede, stateful ones run in order, incremental ones
        // defer around the snapshot. A `Hover` behind a pending `RenderArena`
        // is deferred on top of the arena render; a `Focus` behind one merges
        // into the arena's interaction state — neither can drop the DOM
        // mutations the snapshot carries.
        let mut batch = DrainedBatch::new();
        batch.absorb(first);
        while !batch.quit_requested() {
            match rx.try_recv() {
                Ok(next) => batch.absorb(next),
                Err(_) => break,
            }
        }
        for cmd in batch.drain_inputs() {
            if let RenderCmd::RenderInput {
                node_id,
                value,
                cursor,
                focused,
                selection,
            } = cmd
            {
                state.handle_input(&mut out, node_id, value, cursor, focused, selection);
            }
        }
        for cmd in batch.drain_scrolls() {
            if let RenderCmd::RenderScroll { node_id, scroll_y } = cmd {
                let term_map = match TermMap::get_ter_map() {
                    Ok(m) => m,
                    Err(e) => {
                        log::error!("Failed to get terminal size: {}", e);
                        break 'frame;
                    }
                };
                state.render_scroll(&mut out, &geo_tx, &term_map, node_id, scroll_y);
            }
        }
        for sel_cmd in batch.drain_selects() {
            state.handle_select(&mut out, sel_cmd);
        }

        for css_cmd in batch.drain_css_reloads() {
            if let RenderCmd::ReloadCss { css } = css_cmd {
                let term_map = match TermMap::get_ter_map() {
                    Ok(m) => m,
                    Err(e) => {
                        log::error!("Failed to get terminal size: {}", e);
                        break 'frame;
                    }
                };
                state.reload_css(&mut out, &geo_tx, &term_map, css);
            }
        }

        // Publish the frame-completion heartbeat *before* processing: the
        // input loop's watchdog uses it to tell a *stuck* render thread
        // (infinite layout loop, an enormous canvas allocation, a blocked
        // console write) apart from an idle or no-op one, which is still
        // alive and must not trip the watchdog.
        render_beat.store(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            Ordering::Relaxed,
        );
        last_frame_at = Some(Instant::now());

        // Process the deferred `TextSelection` *before* the snapshot command:
        // its repaint uses the current canvas (the union of the old and new
        // bounds), and the snapshot re-applies it on the fresh canvas — so it
        // is never lost to a snapshot arm's early `continue` (see the merge
        // comment above).
        if let Some(RenderCmd::TextSelection { anchor, end, mode }) = batch.take_text_selection() {
            state.apply_text_selection_cmd(&mut out, anchor, end, mode);
        }

        if let Some(cmd) = batch.take_snapshot() {
            let batch_t0 = Instant::now();
            // The snapshot arms below `break 'snapshot` on their no-op / error
            // paths instead of `continue`ing the frame loop: the deferred
            // hover / focus repaints at the loop tail must still run — a
            // `RenderArena` is exactly the command that defers a hover, and
            // its error branch would otherwise silently drop the highlight.
            'snapshot: {
                match cmd {
                    RenderCmd::Quit => break 'frame,

                    RenderCmd::Rerender { scroll_y } => {
                        // Resolve the new scroll position (with clamping) inside a
                        // scoped borrow so the fixed-box repin below can borrow the
                        // whole state.
                        {
                            let Some(tl) = state.term_layout.as_mut() else {
                                break 'snapshot;
                            };
                            let prev = tl.scroll_y;
                            tl.scroll_y = scroll_y;
                            tl.clamp_scroll();
                            // Defense in depth: drop rerenders whose resolved scroll
                            // position is unchanged after clamping. Most such events
                            // are already filtered in the input thread, but if a
                            // `Rerender` slips past (e.g. races between a Redraw
                            // updating the max_scroll ceiling and a queued scroll
                            // event) we still avoid the diff pass.
                            if tl.scroll_y == prev {
                                break 'snapshot;
                            }
                        }
                        // Scroll-aware `position: fixed`: re-anchor the fixed boxes
                        // to the new scroll offset and repaint the cells they occupy
                        // (old + new areas), so they stay pinned to their screen
                        // position. Documents without pinned boxes keep the cheap
                        // canvas re-slice below.
                        if state.has_pinned {
                            state.pin_fixed_and_repaint(&geo_tx);
                        }
                        // The incremental renderer diffs the shifted viewport against
                        // the previous frame, so scrolling needs no full-screen Clear.
                        let Some(tl) = state.term_layout.as_mut() else {
                            break 'snapshot;
                        };
                        tl.render(&mut out);
                        flush_frame(&mut out);
                    }

                    RenderCmd::Redraw { scroll_y, zoom } => {
                        let term_map = match TermMap::get_ter_map() {
                            Ok(m) => m,
                            Err(e) => {
                                log::error!("Failed to get terminal size: {}", e);
                                break 'snapshot;
                            }
                        };
                        state.redraw(&mut out, &geo_tx, &term_map, scroll_y, zoom);
                    }

                    RenderCmd::RenderArena {
                        arena,
                        root_id,
                        scroll_y,
                        zoom,
                        dynamic,
                        dirty,
                    } => {
                        // A hover / focus change re-styles the whole tree (the
                        // `:hover` / `:focus` highlight may sit anywhere), so the
                        // incremental repaint is only valid when the interaction
                        // state is unchanged.
                        let state_changed = state.dynamic != dynamic;
                        state.dynamic = dynamic;
                        let term_map = match TermMap::get_ter_map() {
                            Ok(m) => m,
                            Err(e) => {
                                log::error!("Failed to get terminal size: {}", e);
                                break 'snapshot;
                            }
                        };
                        state.render_arena(
                            &mut out,
                            &geo_tx,
                            &term_map,
                            &arena,
                            root_id,
                            scroll_y,
                            zoom,
                            dirty,
                            state_changed,
                        );
                    }

                    RenderCmd::RenderDocument {
                        arena,
                        root_id,
                        document_css,
                        scroll_y,
                        zoom,
                        dirty,
                    } => {
                        let term_map = match TermMap::get_ter_map() {
                            Ok(m) => m,
                            Err(e) => {
                                log::error!("Failed to get terminal size: {}", e);
                                break 'snapshot;
                            }
                        };
                        state.render_document(
                            &mut out,
                            &geo_tx,
                            &term_map,
                            arena,
                            root_id,
                            document_css,
                            scroll_y,
                            zoom,
                            dirty,
                        );
                    }

                    RenderCmd::Hover { node_id } => {
                        let prev = state.dynamic.hovered;
                        state.dynamic.hovered = node_id;
                        // Without any `:hover` rule the cascade is identical for every
                        // pointer position, so nothing can change on screen — skip the
                        // repaint entirely (browser behaviour: moving the pointer with
                        // no `:hover` styles produces no drawing).
                        if !state.pseudo_rules.contains_pseudo(PseudoClass::Hover) {
                            break 'snapshot;
                        }
                        state.repaint_dynamic(&mut out, PseudoClass::Hover, prev, node_id);
                    }

                    RenderCmd::Focus { node_id } => {
                        let prev = state.dynamic.focused;
                        state.dynamic.focused = node_id;
                        // Without any `:focus` rule the cascade is identical for every
                        // focus position, so nothing can change on screen — skip the
                        // repaint entirely (browser behaviour: focusing with no
                        // `:focus` styles produces no drawing).
                        if !state.pseudo_rules.contains_pseudo(PseudoClass::Focus) {
                            break 'snapshot;
                        }
                        state.repaint_dynamic(&mut out, PseudoClass::Focus, prev, node_id);
                    }

                    // Select dropdown commands share one handler so the pending pass
                    // can also run them (see `DrainedBatch::drain_selects`).
                    RenderCmd::SelectOpen { .. }
                    | RenderCmd::SelectClose { .. }
                    | RenderCmd::SelectMove { .. } => {
                        state.handle_select(&mut out, cmd);
                    }

                    RenderCmd::TextSelection { anchor, end, mode } => {
                        state.apply_text_selection_cmd(&mut out, anchor, end, mode);
                    }

                    RenderCmd::RenderInput {
                        node_id,
                        value,
                        cursor,
                        focused,
                        selection,
                    } => {
                        state.handle_input(&mut out, node_id, value, cursor, focused, selection);
                    }
                    RenderCmd::RenderScroll { .. } => {
                        log::debug!("Scroll command reached the snapshot queue; dropped");
                    }

                    // JS `el.focus()` / `el.blur()` requests are intercepted by the
                    // input loop (they mutate input-loop state); if one ever reaches
                    // the render thread it is stale — drop it.
                    // Session-CSS reloads are drained statefully before the
                    // snapshot; the compiler still needs this arm for
                    // exhaustiveness if a command is manually constructed.
                    RenderCmd::ReloadCss { .. } => {}
                    RenderCmd::FocusRequest { .. } | RenderCmd::BlurRequest { .. } => {
                        log::debug!("Focus request reached the render thread — dropping");
                    }

                    // Scroll markers are intercepted by the run loop (the
                    // scroll state lives on the input side); a leaked one
                    // renders nothing.
                    RenderCmd::ScrollAnimate { .. }
                    | RenderCmd::ScrollSet { .. }
                    | RenderCmd::ScrollIntoView { .. } => {
                        log::debug!("Scroll marker reached the render thread — dropping");
                    }
                }
            }

            // A single command batch taking this long usually means a bogus
            // canvas size or a blocked console write — log it so a "freeze" can
            // be traced to its exact cause instead of staying mysterious.
            let elapsed = batch_t0.elapsed();
            if elapsed > Duration::from_millis(200) {
                log::warn!("渲染线程单批命令处理耗时 {elapsed:?}");
            }
        }

        // A deferred `Hover` runs last so its repaint re-applies the newer
        // hover highlight on top of both the arena render and the selection.
        if let Some(RenderCmd::Hover { node_id }) = batch.take_hover() {
            let prev = state.dynamic.hovered;
            state.dynamic.hovered = node_id;
            if state.pseudo_rules.contains_pseudo(PseudoClass::Hover) {
                state.repaint_dynamic(&mut out, PseudoClass::Hover, prev, node_id);
            }
        }
        // A deferred `Focus` (queued behind a `Redraw` / `Rerender`) runs
        // after the snapshot render for the same reason: it re-styles the
        // fresh frame instead of displacing it.
        if let Some(RenderCmd::Focus { node_id }) = batch.take_focus() {
            let prev = state.dynamic.focused;
            state.dynamic.focused = node_id;
            if state.pseudo_rules.contains_pseudo(PseudoClass::Focus) {
                state.repaint_dynamic(&mut out, PseudoClass::Focus, prev, node_id);
            }
        }
    }
}

/// Worker of the dedicated hit-test thread.
///
/// Consumes geometry snapshots published by the render thread and replaces
/// the shared hit map with a freshly built region list. Exits when the render
/// thread drops the channel (i.e. on quit).
pub(super) fn hit_test_thread(rx: mpsc::Receiver<SharedGeometry>, hit_map: SharedHitMap) {
    while let Ok(geo) = rx.recv() {
        let map = Arc::new(HitMap::build(&geo));
        match hit_map.lock() {
            Ok(mut guard) => *guard = map,
            Err(e) => log::error!("Hit map lock poisoned: {}", e),
        }
    }
    log::debug!("Hit-test thread exiting");
}

/// Fold one `ScriptResponse::Event` into the current frame: hover updates,
/// host-pushed render commands, and — when JS mutated the DOM — a fresh arena
/// snapshot + dirty set forwarded as a (region-repaint) `RenderArena`.
/// Shared by the key-gate wait and the ordinary async reply fold. `viewport`
/// is the input loop's terminal size, carried into the arena's dynamic state
/// for the `@media` re-cascade.
pub(super) fn fold_script_event(
    state: &mut InputState,
    cmds: &mut Vec<RenderCmd>,
    hovered: &mut Option<NodeId>,
    live_dom: &mut Option<LiveDocument>,
    viewport: Viewport,
    ev: crate::script::ScriptEvent,
) {
    *hovered = ev.hovered;
    cmds.extend(ev.cmds);
    if let Some(d) = ev.dirty {
        let document_reloaded = d.document_css.is_some();
        if document_reloaded {
            state.reset_document_state(cmds);
        }
        *live_dom = Some((Arc::clone(&d.arena), d.root_id));
        if let Some(document_css) = d.document_css {
            cmds.push(RenderCmd::RenderDocument {
                arena: d.arena,
                root_id: d.root_id,
                document_css,
                scroll_y: state.scroll_y,
                zoom: state.zoom,
                dirty: d.dirty,
            });
        } else {
            cmds.push(RenderCmd::RenderArena {
                arena: d.arena,
                root_id: d.root_id,
                scroll_y: state.scroll_y,
                zoom: state.zoom,
                dynamic: DynamicState {
                    hovered: ev.hovered,
                    focused: state.focused_input,
                    viewport,
                },
                dirty: d.dirty,
            });
        }
    }
}
