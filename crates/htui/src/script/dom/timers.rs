//! Timer natives: `setTimeout` / `setInterval` / `clearTimeout` /
//! `clearInterval` (callbacks GC-rooted in a JS registry, deadlines in the
//! Rust timer table the script thread drives).

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

/// Register `args[0]` (a callable) to fire after `args[1]` ms, returning the
/// timer id. `repeat` selects `setInterval` (re-scheduled after each fire)
/// vs `setTimeout` (one-shot). The callback is GC-rooted in the
/// `__htuiTimers` JS registry; the deadline lives in the Rust timer table the
/// script thread drives.
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
        });
        id
    };
    // GC-root the callback in the JS registry (state borrow already released).
    let global = context.global_object();
    let registry = global
        .get(js_string!("__htuiTimers"), context)?
        .to_object(context)?;
    registry.set(js_string!(id.to_string()), callback, false, context)?;
    Ok(id.into())
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
