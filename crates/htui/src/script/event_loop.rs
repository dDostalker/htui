use std::time::{Duration, Instant};

use boa_engine::object::builtins::JsFunction;
use boa_engine::{Context, JsError, JsValue, js_string};

use crate::{resources::AsyncResult, tui::graph_api::RenderCmd};

use super::dom::response_object;
use super::engine::ScriptEngine;

impl ScriptEngine {
    /// How long the message loop may block before the earliest pending timer
    /// fires, or `None` when no timer is active (block indefinitely).
    pub fn next_timer_timeout(&self) -> Option<Duration> {
        let st = self.state.borrow();
        st.timers
            .iter()
            .map(|t| t.deadline.saturating_duration_since(Instant::now()))
            .min()
    }

    /// Fire every timer whose deadline has passed, in registration order.
    ///
    /// Callbacks run in the calling thread — the script thread in production —
    /// exactly like a browser event-loop task, so a callback that rewrites the
    /// DOM marks nodes dirty as usual. Returns render commands pushed by host
    /// functions during the dispatch. `setInterval` callbacks are re-scheduled
    /// relative to the fire time; a badly-lagged interval jumps to `now +
    /// period` instead of bursting. `setTimeout` callbacks are one-shot.
    pub fn run_due_timers(&mut self) -> Vec<RenderCmd> {
        let now = Instant::now();
        // Snapshot the due ids first: callbacks may register or clear timers,
        // so the table mutates while we iterate.
        let due: Vec<u32> = {
            let st = self.state.borrow();
            st.timers
                .iter()
                .filter(|t| t.deadline <= now)
                .map(|t| t.id)
                .collect()
        };
        if due.is_empty() {
            return Vec::new();
        }
        // Re-schedule / remove before firing so a slow callback cannot make a
        // missed repeat deadline fire again immediately after.
        {
            let mut st = self.state.borrow_mut();
            st.timers.retain_mut(|t| {
                if t.deadline > now {
                    return true;
                }
                if t.repeat {
                    t.deadline = now + t.period;
                    true
                } else {
                    false
                }
            });
        }
        for id in due {
            let callback = timer_callback(&mut self.context, id);
            if !callback.is_callable() {
                log::debug!("Timer {id} has no callable callback — skipping");
                continue;
            }
            if let Some(f) = callback.to_object(&mut self.context).ok()
                && let Err(e) = f.call(&JsValue::undefined(), &[], &mut self.context)
            {
                log::warn!("Timer callback threw: {e}");
            }
        }
        let mut st = self.state.borrow_mut();
        let cmds = std::mem::take(&mut st.render_cmds);
        drop(st);
        self.intercept_render_html(cmds)
    }

    /// Drain completed loads and settle their `fetch` promises.
    ///
    /// Called on every script-thread loop iteration after message / timer
    /// processing. Each completed [`AsyncResult`] carries a `{id}:{url}` key
    /// (see [`crate::resources::load_async`]); the id looks up the promise's
    /// `resolve` / `reject` functions stashed by `fetch`, which are invoked
    /// with a `Response`-like object (or an error). Promise reaction callbacks
    /// (`.then` / `await`) run through [`Context::run_jobs`], so this is the
    /// microtask half of the engine's event loop.
    pub fn poll_fetches(&mut self) {
        loop {
            let result = self.state.borrow().fetch_rx.try_recv();
            match result {
                Ok(AsyncResult { id, url, outcome }) => {
                    self.settle_fetch(id, &url, outcome);
                }
                Err(e) => {
                    if !matches!(e, std::sync::mpsc::TryRecvError::Empty) {
                        log::debug!("fetch channel error: {e}");
                    }
                    break;
                }
            }
        }
        if let Err(e) = self.context.run_jobs() {
            log::warn!("Microtask queue run failed: {e}");
        }
    }

    /// Resolve or reject the promise for a single completed fetch.
    ///
    /// The resolvers live in the GC-rooted `__htuiFetchResolvers` JS registry
    /// (keyed by fetch id); this method reads them out, settles the promise,
    /// and removes the entry.
    fn settle_fetch(
        &mut self,
        id: u32,
        url: &str,
        outcome: Result<std::sync::Arc<Vec<u8>>, String>,
    ) {
        if !self.state.borrow_mut().active_fetches.remove(&id) {
            log::warn!("fetch result for unknown id {id} (url {url}) — dropped");
            return;
        }
        let (resolve, reject) = match self.take_fetch_resolvers(id) {
            Some(v) => v,
            None => {
                log::warn!("fetch id {id} has no JS resolvers — dropped");
                return;
            }
        };
        let value = match outcome {
            Ok(resource) => response_object(&mut self.context, url.to_string(), resource),
            Err(message) => Err(JsError::from_opaque(
                js_string!(format!("fetch failed for {url}: {message}")).into(),
            )),
        };
        let result = match value {
            Ok(v) => resolve.call(&JsValue::undefined(), &[v], &mut self.context),
            Err(e) => reject.call(
                &JsValue::undefined(),
                &[e.to_opaque(&mut self.context)],
                &mut self.context,
            ),
        };
        if let Err(e) = result {
            log::warn!("fetch promise settlement failed: {e}");
        }
    }

    /// Pull the `[resolve, reject]` array for a fetch id out of the JS
    /// registry and delete the entry (each fetch settles at most once).
    fn take_fetch_resolvers(&mut self, id: u32) -> Option<(JsFunction, JsFunction)> {
        let global = self.context.global_object();
        let registry = global
            .get(js_string!("__htuiFetchResolvers"), &mut self.context)
            .ok()?
            .to_object(&mut self.context)
            .ok()?;
        let key = js_string!(id.to_string());
        let entry = registry.get(key.clone(), &mut self.context).ok()?;
        if entry.is_undefined() {
            return None;
        }
        let obj = entry.to_object(&mut self.context).ok()?;
        let resolve = obj
            .get(js_string!("resolve"), &mut self.context)
            .ok()
            .and_then(|v| v.as_callable())
            .and_then(JsFunction::from_object);
        let reject = obj
            .get(js_string!("reject"), &mut self.context)
            .ok()
            .and_then(|v| v.as_callable())
            .and_then(JsFunction::from_object);
        if let Err(e) = registry.delete_property_or_throw(key, &mut self.context) {
            log::warn!("Failed to remove fetch resolver for id {id}: {e}");
        }
        match (resolve, reject) {
            (Some(r), Some(j)) => Some((r, j)),
            _ => None,
        }
    }
}

/// Fetch a GC-rooted timer callback from `globalThis.__htuiTimers` by id.
fn timer_callback(context: &mut Context, id: u32) -> JsValue {
    let global = context.global_object();
    let Ok(registry) = global.get(js_string!("__htuiTimers"), context) else {
        return JsValue::undefined();
    };
    let Ok(registry) = registry.to_object(context) else {
        return JsValue::undefined();
    };
    registry
        .get(js_string!(id.to_string()), context)
        .unwrap_or(JsValue::undefined())
}
