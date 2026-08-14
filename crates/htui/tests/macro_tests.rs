//! Tests for the registration macros: `#[htui::commands]` (module-level
//! auto-registration), `#[htui::command]` (pure + stateful `HostFn`-returning
//! variants) and the `name = "…"` override. Pure in-memory — no terminal
//! required.

use std::sync::{Arc, Mutex};

use htui::events::EventBindings;
use htui::script::{HostFn, ScriptEngine};

// The fixture module exercises every registration feature:
//   - `greet` is registered under its function name,
//   - `format_datetime` under the explicit `name = "formatDateTime"`,
//   - `sample` is a stateful command (its counter survives across JS calls),
//     written with `host_fn` so the body never names a Boa type.
#[htui::commands]
mod app {
    use std::sync::{Arc, Mutex};

    use htui::script::HostFn;

    /// Command registered under its function name.
    #[htui::command]
    pub fn greet(name: String) -> String {
        format!("hello {name}")
    }

    /// Command registered under an explicit JS name.
    #[htui::command(name = "formatDateTime")]
    pub fn format_datetime(secs: u64) -> String {
        format!("@{secs}")
    }

    /// Stateful command: every JS call increments its internal counter and
    /// returns it as an object property.
    #[htui::command(name = "sample")]
    pub fn sample() -> HostFn {
        let calls = Arc::new(Mutex::new(0u64));
        htui::script::host_fn(move |ctx| {
            let mut n = calls.lock().unwrap();
            *n += 1;
            let mut obj = ctx.object();
            obj.set("n", *n);
            Ok(obj)
        })
    }
}

#[test]
fn commands_module_registers_every_binding() {
    let mut bindings = EventBindings::default();
    app::register(&mut bindings);

    for name in ["greet", "formatDateTime", "sample"] {
        assert!(bindings.host.contains_key(name), "missing host `{name}`");
    }
    assert_eq!(bindings.host.len(), 3);
}

#[test]
fn commands_module_makes_hosts_callable_from_js() {
    let mut bindings = EventBindings::default();
    app::register(&mut bindings);

    let mut engine = ScriptEngine::new(
        r#"
        <script>
          globalThis.__greet = greet("Ada");
          globalThis.__ts = formatDateTime(42);
          globalThis.__n = sample().n;
          globalThis.__n2 = sample().n;
        </script>
        <p>x</p>
        "#,
        bindings,
    )
    .expect("engine must build");

    assert_eq!(eval_str(&mut engine, "globalThis.__greet"), "hello Ada");
    assert_eq!(eval_str(&mut engine, "globalThis.__ts"), "@42");
    assert_eq!(
        eval_str(&mut engine, "globalThis.__n"),
        "1",
        "stateful host functions must survive across JS calls"
    );
    assert_eq!(
        eval_str(&mut engine, "globalThis.__n"),
        "1",
        "stateful host functions must survive across JS calls"
    );
    assert_eq!(eval_str(&mut engine, "globalThis.__n2"), "2");
}

/// A `#[htui::command(name = "…")]` used outside a `#[htui::commands]` module
/// still works with manual registration via `<fn>_command()`.
#[htui::command(name = "ping")]
fn ping() -> u8 {
    1
}

#[test]
fn command_name_attribute_does_not_break_manual_registration() {
    let mut bindings = EventBindings::default();
    bindings.command("ping", ping_command());

    let mut engine = ScriptEngine::new(
        r#"<script>globalThis.__p = ping();</script><p>x</p>"#,
        bindings,
    )
    .expect("engine must build");

    assert_eq!(eval_str(&mut engine, "globalThis.__p"), "1");
}

/// A stateful `#[htui::command]` returning `HostFn` also works with manual
/// registration via its generated `<fn>_command()` wrapper. The closure is
/// written with `host_fn`, so the body never names a Boa type.
#[htui::command]
fn counter() -> HostFn {
    let calls = Arc::new(Mutex::new(0u64));
    htui::script::host_fn(move |_ctx| {
        let mut n = calls.lock().unwrap();
        *n += 1;
        Ok(*n)
    })
}

#[test]
fn host_fn_returning_command_works_with_manual_registration() {
    let mut bindings = EventBindings::default();
    bindings.command("counter", counter_command());

    let mut engine = ScriptEngine::new(
        r#"<script>
          globalThis.__c = counter();
          globalThis.__c2 = counter();
        </script>
        <p>x</p>"#,
        bindings,
    )
    .expect("engine must build");

    assert_eq!(
        eval_str(&mut engine, "globalThis.__c"),
        "1",
        "state captured in the returned closure must survive across calls"
    );
    assert_eq!(eval_str(&mut engine, "globalThis.__c2"), "2");
}

/// A fixture exercising the auto-generated JS/TS bindings: primitive +
/// `Option` + `Result` args, a renamed command with `Vec`, a unit return, and
/// a stateful command that must be excluded.
#[htui::commands]
mod bindings {
    use htui::script::HostFn;

    /// Plain command: string + optional number, result return.
    #[htui::command]
    pub fn lookup(key: String, fallback: Option<f64>) -> Result<f64, String> {
        if key.is_empty() {
            return Err("empty key".into());
        }
        Ok(fallback.unwrap_or(1.0))
    }

    /// Renamed command with a Vec return.
    #[htui::command(name = "tagList")]
    pub fn tags(count: u8, names: Vec<String>) -> Vec<String> {
        names.into_iter().take(count as usize).collect()
    }

    /// Unit return.
    #[htui::command]
    pub fn reboot() {}

    /// Stateful — excluded from bindings (no static signature).
    #[htui::command]
    pub fn ticker() -> HostFn {
        htui::script::host_fn(move |_ctx| Ok(0u64))
    }
}

fn eval_str(engine: &mut ScriptEngine, expr: &str) -> String {
    let v = engine
        .eval_source(&format!("String({expr})"))
        .expect("eval must succeed");
    v.as_string()
        .map(|s| s.to_std_string_lossy())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Auto-generated JS/TS bindings via `#[htui::commands]`.
// ---------------------------------------------------------------------------

#[test]
fn commands_module_generates_js_bindings() {
    let text = bindings::js_bindings();
    // JSDoc-annotated sample declarations derived from the Rust signatures.
    assert!(text.contains("function lookup(key, fallback) {}"));
    assert!(text.contains("* @param {string} key"));
    assert!(text.contains("* @param {number | undefined} fallback"));
    assert!(text.contains("* @returns {number}"));
    assert!(text.contains("* Throws on error."));
    assert!(text.contains("function tagList(count, names) {}"));
    assert!(text.contains("* @param {number} count"));
    assert!(text.contains("* @param {string[]} names"));
    assert!(text.contains("* @returns {string[]}"));
    assert!(text.contains("function reboot() {}"));
    // Stateful commands have no static signature → excluded.
    assert!(!text.contains("function ticker"));
    assert!(!app::js_bindings().contains("function sample"));
    // `name = "..."` is honoured in the declarations too.
    assert!(app::js_bindings().contains("function formatDateTime(secs) {}"));
}

#[test]
fn commands_module_writes_js_bindings_to_a_file() {
    let dir = std::env::temp_dir().join(format!("htui-js-bindings-{}", std::process::id()));
    let path = dir.join("bindings.js");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    bindings::write_js_bindings(&path).expect("write bindings");
    let text = std::fs::read_to_string(&path).expect("read bindings back");
    assert!(text.contains("function lookup(key, fallback) {}"));
    assert!(text.contains("* @returns {number}"));
    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn bindings_fixture_commands_register() {
    let mut bindings = EventBindings::default();
    bindings::register(&mut bindings);
    for name in ["lookup", "tagList", "reboot", "ticker"] {
        assert!(bindings.host.contains_key(name), "missing host `{name}`");
    }
}

// ---------------------------------------------------------------------------
// Boa-free host functions via `htui::script::host_fn`.
// ---------------------------------------------------------------------------

#[test]
fn host_fn_closures_are_fully_boa_free() {
    use htui::script::HostError;

    let mut bindings = EventBindings::default();
    // Object return, optional argument, error path — none of it names a Boa
    // type (no `JsValue` / `Context` / `JsResult` / `JsError` in this test).
    bindings.host.insert(
        "describe".to_string(),
        htui::script::host_fn(|ctx| {
            let name: String = ctx.arg_required(0)?;
            let count: i64 = ctx.arg(1)?.unwrap_or(0);
            let mut obj = ctx.object();
            obj.set("greeting", format!("hi {name}"));
            obj.set("count", count);
            Ok(obj)
        }),
    );
    bindings.host.insert(
        "bounded".to_string(),
        htui::script::host_fn(|ctx| -> Result<i64, HostError> {
            let n: i64 = ctx.arg_required(0)?;
            if n < 0 {
                return Err(HostError::type_error("must be non-negative"));
            }
            Ok(n * 2)
        }),
    );

    let mut engine = ScriptEngine::new(
        r#"
        <script>
          globalThis.__g = describe("Ada", 3).greeting;
          globalThis.__c = describe("Bob").count;
          globalThis.__d = bounded(21);
          try { bounded(-1); globalThis.__threw = false; }
          catch (e) { globalThis.__threw = e instanceof TypeError; }
        </script>
        <p>x</p>
        "#,
        bindings,
    )
    .expect("engine must build");

    assert_eq!(eval_str(&mut engine, "globalThis.__g"), "hi Ada");
    assert_eq!(eval_str(&mut engine, "globalThis.__c"), "0");
    assert_eq!(eval_str(&mut engine, "globalThis.__d"), "42");
    assert_eq!(
        eval_str(&mut engine, "globalThis.__threw"),
        "true",
        "HostError::type_error must surface as a JS TypeError"
    );
}

#[test]
fn host_fn_captures_state_across_calls() {
    let calls = std::sync::Arc::new(std::sync::Mutex::new(0u64));
    let mut bindings = EventBindings::default();
    bindings.host.insert(
        "counter".to_string(),
        htui::script::host_fn({
            let calls = std::sync::Arc::clone(&calls);
            move |_ctx| {
                let mut n = calls.lock().unwrap();
                *n += 1;
                Ok(*n)
            }
        }),
    );

    let mut engine = ScriptEngine::new(
        r#"<script>
          globalThis.__a = counter();
          globalThis.__b = counter();
        </script>
        <p>x</p>"#,
        bindings,
    )
    .expect("engine must build");

    assert_eq!(eval_str(&mut engine, "globalThis.__a"), "1");
    assert_eq!(eval_str(&mut engine, "globalThis.__b"), "2");
}

#[test]
fn host_fn_can_push_render_commands() {
    use htui::tui::graph_api::RenderCmd;

    let mut bindings = EventBindings::default();
    bindings.host.insert(
        "refresh".to_string(),
        htui::script::host_fn(|ctx| {
            ctx.render().push(RenderCmd::Rerender { scroll_y: 0 });
            Ok(())
        }),
    );

    let mut engine = ScriptEngine::new(
        r#"<script>globalThis.__ok = (refresh(), true);</script><p>x</p>"#,
        bindings,
    )
    .expect("engine must build");

    assert_eq!(eval_str(&mut engine, "globalThis.__ok"), "true");
}
