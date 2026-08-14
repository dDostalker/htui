//! End-to-end test: `<script src="mem://…">` loads a script through the
//! resource layer, and `fetch` on a second `mem://` resource settles through
//! the microtask event loop to write the DOM.

use htui::events::{EventBindings, HitMap, HoverState};
use htui::script::{ScriptEngine, ScriptMsg, apply_message};

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

fn eval_str(engine: &mut ScriptEngine, expr: &str) -> String {
    let v = engine
        .eval_source(&format!("String({expr})"))
        .expect("eval must succeed");
    v.as_string()
        .map(|s| s.to_std_string_lossy())
        .unwrap_or_default()
}

#[test]
fn mem_script_and_fetch_drive_dom_together() {
    init();
    htui::resources::register("mem://e2e/app.js", b"loadApp()".to_vec())
        .expect("register app script");
    htui::resources::register("mem://e2e/data.txt", b"fetched via event loop".to_vec())
        .expect("register data resource");

    let html = r#"
      <script>
        function loadApp() {
          fetch('mem://e2e/data.txt')
            .then(function (r) { return r.text(); })
            .then(function (t) {
              document.getElementById('out').textContent = t;
            });
        }
      </script>
      <script src="mem://e2e/app.js"></script>
      <div id="out">pending</div>
    "#;
    let mut engine = ScriptEngine::new(html, EventBindings::default()).expect("engine initializes");

    // Drain the event loop the way the script thread does: `mem://e2e/app.js`
    // ran during construction and called loadApp(), which started the fetch.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        engine.poll_fetches();
        let v = eval_str(&mut engine, "document.getElementById('out').textContent");
        if v == "fetched via event loop" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    assert_eq!(
        eval_str(&mut engine, "document.getElementById('out').textContent"),
        "fetched via event loop",
        "the mem:// script must run and its fetch must settle through the event loop"
    );

    // Sanity: `apply_message` still works after a fetch settled (engine state
    // consistent — no panics on the interaction path).
    let mut hover = HoverState::new();
    let hit_map = HitMap::default();
    let _ = apply_message(
        &mut engine,
        &mut hover,
        &hit_map,
        ScriptMsg::Move { x: 0, y: 0 },
    );

    htui::resources::unregister("mem://e2e/app.js");
    htui::resources::unregister("mem://e2e/data.txt");
}
