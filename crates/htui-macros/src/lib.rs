//! Procedural macros for tauri-style host bindings.
//!
//! - `#[derive(IntoJs)]` — turn a named-field struct into a JS object
//!   (`field` → property, optionally renamed with `#[js_name = "..."]`).
//! - `#[derive(FromJs)]` — read a struct back from a JS object.
//! - `#[htui::command]` — the only way to expose a Rust function to JS:
//!   - a plain function (args `FromJs`, return `IntoJs`) is wrapped into a
//!     `HostFn` (`name_command()`), or
//!   - a zero-argument function returning a `HostFn` is registered as-is,
//!     letting the returned closure capture arbitrary Rust state.
//!     Accepts an optional `name = "..."` to control the JS-side name used by
//!     `#[htui::commands]`.
//! - `#[htui::commands]` — collect every `#[htui::command]` function in a
//!   module and generate one `register(&mut EventBindings)` entry point, plus
//!   `js_bindings()` / `write_js_bindings(path)` that emit an auto-generated
//!   sample JS interface for the module's plain commands — removing the
//!   per-command `bindings.command(...)` boilerplate and keeping the JS side
//!   in sync with the Rust signatures.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

mod command;
mod commands;
mod derive;

/// `#[derive(IntoJs)]` — struct → JS object.
#[proc_macro_derive(IntoJs, attributes(js_name))]
pub fn derive_into_js(input: TokenStream) -> TokenStream {
    derive::expand_into_js(input)
}

/// `#[derive(FromJs)]` — JS object → struct.
#[proc_macro_derive(FromJs, attributes(js_name))]
pub fn derive_from_js(input: TokenStream) -> TokenStream {
    derive::expand_from_js(input)
}

/// `#[htui::command]` — the single way to expose a Rust function to JS: a
/// plain function (args `FromJs`, return `IntoJs`) is wrapped into a `HostFn`
/// (`name_command()`); a zero-argument function returning a `HostFn` is
/// registered as-is (stateful, closure captures state). Accepts an optional
/// `name = "..."` used by `#[htui::commands]` as the JS registration name.
#[proc_macro_attribute]
pub fn command(attr: TokenStream, item: TokenStream) -> TokenStream {
    command::expand_command(attr, item)
}

/// `#[htui::commands]` — collect every `#[htui::command]` function in a
/// module and generate one `register(&mut EventBindings)` entry point, plus
/// `js_bindings()` / `write_js_bindings(path)` that emit an auto-generated
/// sample JS interface for the module's plain commands — removing the
/// per-command `bindings.command(...)` boilerplate and keeping the JS side
/// in sync with the Rust signatures.
#[proc_macro_attribute]
pub fn commands(attr: TokenStream, item: TokenStream) -> TokenStream {
    commands::expand_commands(attr, item)
}

fn compile_err(msg: &str) -> TokenStream2 {
    quote! { compile_error!(#msg); }
}
