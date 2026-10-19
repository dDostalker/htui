# htui-macros

Procedural macros for the [htui](https://crates.io/crates/htui) terminal
HTML/CSS/JS renderer. You normally depend on `htui` and use these through its
re-exports (`htui::command`, `htui::commands`, `htui::{IntoJs, FromJs}`).

- `#[htui::command]` — wrap a plain Rust function as a typed host binding
  (callable from page JS as a global), or return an `htui::script::HostFn`
  for stateful bindings.
- `#[htui::commands]` — module-level auto-registration: generates
  `register(&mut EventBindings)` plus `js_bindings()` sample JS stubs.
- `#[derive(IntoJs)]` / `#[derive(FromJs)]` — struct ↔ JS object conversion.

Licensed under MIT OR Apache-2.0.
