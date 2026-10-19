<p align="center">
  <img src="https://raw.githubusercontent.com/dDostalker/htui/main/logo.svg" alt="htui logo" width="140">
</p>

# htui

**An HTML/CSS/JS engine for the terminal — build interactive TUIs the way you build web pages.**

[![crates.io](https://img.shields.io/crates/v/htui.svg)](https://crates.io/crates/htui)
[![docs.rs](https://img.shields.io/docsrs/htui)](https://docs.rs/htui)
[![license](https://img.shields.io/crates/l/htui.svg)](https://github.com/dDostalker/htui)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)

[中文说明](https://github.com/dDostalker/htui/blob/main/README.zh-CN.md)

`htui` is a Rust library that renders HTML documents with CSS styling inside
the terminal and runs each document's `<script>` blocks in an embedded
JavaScript engine ([Boa](https://github.com/boa-dev/boa)) with browser-style
DOM bindings. Rust application code talks to the page through typed **host
functions** that become callable JS globals. Everything the document sees is
served from an in-memory `mem://` registry — no filesystem, no network.

## Features

- **HTML + CSS rendering** — flow, flexbox, tables, multi-column, positioning,
  box model, text styling, `:hover` / `:focus`
- **JavaScript scripting** — browser-aligned DOM bindings, timers,
  `fetch` (Promise-based), `console`
- **Rust ↔ JS interop** — `#[htui::command]` turns plain Rust functions into
  typed JS globals
- **DOM-style events** — click / hover / focus / input / key / form events
  with bubbling and `preventDefault()`
- **Form controls** — input, textarea, select + datalist, checkbox / radio,
  form submit / reset, details, dialog
- **Images** — PNG / JPEG / WebP and SVG as truecolor half-block cells
- **Smooth rendering** — incremental diff output, region repaints, 60 fps cap
- **No `unsafe`** — page JS runs under runtime limits; decoding and resources
  are sandboxed away from the filesystem and network

## Quick start

```toml
[dependencies]
htui = "0.1"
```

```rust
use htui::Session;

fn main() {
    let mut session = Session::new();
    session.render(
        r#"
        <h1 style="color: steelblue">Hello, htui</h1>
        <p>Press <b>Esc</b> to quit.</p>
        "#,
        "",
    );
}
```

Interactivity is one attribute away — `<button onclick="greet()">` with a
`#[htui::command]` Rust function behind it. See the tutorials below.

## Documentation

| Document | Content |
| --- | --- |
| [docs.rs/htui](https://docs.rs/htui) | API reference |
| [中文文档](https://github.com/dDostalker/htui/blob/main/README.zh-CN.md) | Chinese introduction |
| [Tutorial: HelloWorld](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/教程/1.HelloWorld.md) | First rendering |
| [Tutorial: Dynamic terminal](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/教程/4.0动态终端.md) | Events, timers, built-in keys |
| [Tutorial: JS/Rust bindings](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/教程/4.1JS和Rust绑定.md) | Host functions, interop |
| [Tutorial: Resources](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/教程/3.0资源.md) | The `mem://` registry |
| [Architecture](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/架构.md) | Pipeline & threading model |
| [Capability matrix](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/能力差异清单.md) | Full HTML/CSS/JS matrix vs. browsers |

## Platform

Rust **1.88+** (edition 2024). Windows console and POSIX terminals via
[crossterm](https://github.com/crossterm-rs/crossterm); truecolor recommended.

## License

Licensed under either of
[Apache-2.0](https://github.com/dDostalker/htui/blob/main/LICENSE-APACHE) or
[MIT](https://github.com/dDostalker/htui/blob/main/LICENSE-MIT) at your option.
