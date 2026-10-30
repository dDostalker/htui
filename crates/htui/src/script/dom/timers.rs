//! Timer natives: `setTimeout` / `setInterval` / `clearTimeout` /
//! `clearInterval` / `requestAnimationFrame` / `cancelAnimationFrame`
//! (callbacks GC-rooted in a JS registry, deadlines in the Rust timer table
//! the script thread drives).

use std::time::{Duration, Instant};

use boa_engine::{Context, JsNativeError, JsResult, JsValue, js_string};

use crate::script::TimerEntry;

use super::with_state;

pub(super) fn native_set_timeout(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    schedule_timer(context, args, false)
}

pub(super) fn native_set_interval(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    schedule_timer(context, args, true)
}

const MIN_INTERVAL_PERIOD_MS: u64 = 4;

/// Register `args[0]` (a callable) to fire after `args[1]` ms, returning the
/// timer id. `repeat` selects `setInterval` (re-scheduled after each fire)
/// vs `setTimeout` (one-shot). The callback is GC-rooted in the
/// `__htuiTimers` JS registry; the deadline lives in the Rust timer table
/// the script thread drives.
fn schedule_timer(context: &mut Context, args: &[JsValue], repeat: bool) -> JsResult<JsValue> {
    let callback = args.first().cloned().unwrap_or(JsValue::undefined());
    if !callback.is_callable() {
        return Err(JsNativeError::typ()
            .with_message("setTimeout/setInterval expects a callable first argument")
            .into());
    }
    let ms = args
        .get(1)
        .map(|v| v.to_number(context))
        .transpose()?
        .unwrap_or(0.0);
    let ms = if ms.is_finite() && ms > 0.0 { ms } else { 0.0 };
    let ms = if repeat {
        ms.max(MIN_INTERVAL_PERIOD_MS as f64)
    } else {
        ms
    };
    let period = Duration::from_millis(ms as u64);
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let id = {
        let mut st = rc.borrow_mut();
        let id = st.next_timer_id;
        st.next_timer_id += 1;
        st.timers.push(TimerEntry {
            id,
            deadline: Instant::now() + period,
            period,
            repeat,
            frame_time: None,
        });
        id
    };
    root_timer_callback(context, id, callback)?;
    Ok(id.into())
}

/// GC-root a timer callback in the `__htuiTimers` JS registry under `id`.
fn root_timer_callback(context: &mut Context, id: u32, callback: JsValue) -> JsResult<()> {
    let global = context.global_object();
    let registry = global
        .get(js_string!("__htuiTimers"), context)?
        .to_object(context)?;
    registry.set(js_string!(id.to_string()), callback, false, context)?;
    Ok(())
}

pub(super) fn native_clear_timeout(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    clear_timer(context, args)
}

pub(super) fn native_clear_interval(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    clear_timer(context, args)
}

/// The frame period `requestAnimationFrame` schedules on, in ms (~60fps),
/// anchored to a session-global grid: there is no vsync in a terminal, so
/// the frame clock is the next multiple of this period since the engine
/// started. CSS animation / transition stay out of scope by design; a frame
/// callback rewriting the DOM drives repaint through the normal render
/// pipeline.
const RAF_PERIOD_MS: u64 = 16;

/// `requestAnimationFrame(callback)` — run `callback` once, on the next
/// frame. Browser-aligned one-shot semantics: the callback fires exactly
/// once, at the next grid point of the session's ~60fps frame clock, and
/// receives that frame's grid timestamp in ms since session start
/// (`DOMHighResTimeStamp` semantics) as its first argument — a late wake
/// still reports the grid time, so animation positions stay a pure function
/// of time. Chained requests (`requestAnimationFrame` inside the callback)
/// land on the following grid point, so per-callback jitter is absorbed
/// instead of accumulating. `cancelAnimationFrame` cancels a pending one.
pub(super) fn native_request_animation_frame(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(callback) = args.first().cloned() else {
        return Err(JsNativeError::typ()
            .with_message("requestAnimationFrame expects a function")
            .into());
    };
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let id = {
        let mut st = rc.borrow_mut();
        let now = Instant::now();
        // The next grid point strictly after `now`: a chained request made
        // inside a frame callback (which runs at/after its own grid point)
        // lands on the following frame.
        let elapsed_ms = now.saturating_duration_since(st.frame_epoch).as_millis() as u64;
        let frame_index = elapsed_ms / RAF_PERIOD_MS + 1;
        let frame_time = (frame_index * RAF_PERIOD_MS) as f64;
        let deadline = st.frame_epoch + Duration::from_millis(frame_index * RAF_PERIOD_MS);
        let id = st.next_timer_id;
        st.next_timer_id += 1;
        st.timers.push(TimerEntry {
            id,
            deadline,
            period: Duration::ZERO,
            repeat: false,
            frame_time: Some(frame_time),
        });
        id
    };
    root_timer_callback(context, id, callback)?;
    Ok(id.into())
}

/// `cancelAnimationFrame(id)` — stop a frame callback (same table as
/// `clearInterval`).
pub(super) fn native_cancel_animation_frame(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    clear_timer(context, args)
}

/// Remove the timer with `args[0]`'s id from both the Rust table and the JS
/// registry, if present.
fn clear_timer(context: &mut Context, args: &[JsValue]) -> JsResult<JsValue> {
    let id = args
        .first()
        .map(|v| v.to_number(context))
        .transpose()?
        .unwrap_or(-1.0);
    let id = if id.is_finite() && id >= 0.0 {
        id as u32
    } else {
        u32::MAX
    };
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    rc.borrow_mut().timers.retain(|t| t.id != id);
    let global = context.global_object();
    if let Ok(registry) = global
        .get(js_string!("__htuiTimers"), context)
        .and_then(|v| v.to_object(context))
        && let Err(e) = registry.delete_property_or_throw(js_string!(id.to_string()), context)
    {
        log::warn!("Failed to clear timer callback: {e}");
    }
    Ok(JsValue::undefined())
}
