//! Tests for the registration macros: `#[htui::commands]` (module-level
//! auto-registration), `#[htui::command]` (pure + stateful `HostFn`-returning
//! variants) and the `name = "…"` override. Pure in-memory — no terminal
//! required.

use std::sync::{Arc, Mutex};

use htui::events::EventBindings;
use htui::script::{HostCall, HostFn, ScriptEngine};

// The fixture module exercises every registration feature:
//   - `greet` is registered under its function name,
//   - `format_datetime` under the explicit `name = "formatDateTime"`,
//   - `sample` is a stateful command returning a `HostFn` (its counter
//     survives across JS calls) — the only way stateful hosts are written.
#[htui::commands]
mod app {
    use std::sync::{Arc, Mutex};

    use htui::script::{HostCall, HostFn};

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

    /// Stateful command: every JS call increments its internal counter.
    #[htui::command(name = "sample")]
    pub fn sample() -> HostFn {
        let calls = Arc::new(Mutex::new(0u64));
        Box::new(move |call: &mut HostCall| {
            let mut n = calls.lock().unwrap();
            *n += 1;
            Ok(htui::js_obj!(call.context, "n" => *n))
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
    assert_eq!(eval_str(&mut engine, "globalThis.__n"), "1", "stateful host functions must survive across JS calls");
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
/// registration via its generated `<fn>_command()` wrapper.
#[htui::command]
fn counter() -> HostFn {
    let calls = Arc::new(Mutex::new(0u64));
    Box::new(move |call: &mut HostCall| {
        let mut n = calls.lock().unwrap();
        *n += 1;
        Ok(htui::js_obj!(call.context, "n" => *n))
    })
}

#[test]
fn host_fn_returning_command_works_with_manual_registration() {
    let mut bindings = EventBindings::default();
    bindings.command("counter", counter_command());

    let mut engine = ScriptEngine::new(
        r#"<script>
          globalThis.__c = counter().n;
          globalThis.__c2 = counter().n;
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

fn eval_str(engine: &mut ScriptEngine, expr: &str) -> String {
    let v = engine
        .eval_source(&format!("String({expr})"))
        .expect("eval must succeed");
    v.as_string()
        .map(|s| s.to_std_string_lossy())
        .unwrap_or_default()
}
