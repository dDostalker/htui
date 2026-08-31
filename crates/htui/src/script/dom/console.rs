//! `console` / `alert` / `quit` natives (log-backed output).

use boa_engine::{Context, JsResult, JsValue};

use crate::tui::graph_api::RenderCmd;

use super::with_state;

pub(super) fn console_log(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let msg = format_args(args, context)?;
    // `target: "console"` separates JS console output from engine log
    // records (hosts can filter on `record.target()`); the `[console.log]`
    // prefix keeps it readable in plain-text streams.
    log::info!(target: "console", "[console.log] {msg}");
    Ok(JsValue::undefined())
}

pub(super) fn console_warn(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let msg = format_args(args, context)?;
    log::warn!(target: "console", "[console.warn] {msg}");
    Ok(JsValue::undefined())
}

pub(super) fn console_error(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let msg = format_args(args, context)?;
    log::error!(target: "console", "[console.error] {msg}");
    Ok(JsValue::undefined())
}

fn format_args(args: &[JsValue], context: &mut Context) -> JsResult<String> {
    let mut parts = Vec::new();
    for a in args {
        let s = match a {
            v if v.is_string() => v.to_string(context)?.to_std_string_lossy(),
            v if v.is_number() => v.to_number(context)?.to_string(),
            v if v.is_null() => "null".to_string(),
            v if v.is_undefined() => "undefined".to_string(),
            v if v.is_boolean() => v.to_boolean().to_string(),
            v => {
                let s = v.to_string(context)?.to_std_string_lossy();
                if s.is_empty() {
                    "[object Object]".to_string()
                } else {
                    s
                }
            }
        };
        parts.push(s);
    }
    Ok(parts.join(" "))
}

pub(super) fn native_alert(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let msg = format_args(args, context)?;
    log::warn!("[alert] {msg}");
    Ok(JsValue::undefined())
}

pub(super) fn native_quit(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    if let Some(rc) = with_state() {
        let mut st = rc.borrow_mut();
        st.render_cmds.push(RenderCmd::Quit);
    }
    Ok(JsValue::undefined())
}
