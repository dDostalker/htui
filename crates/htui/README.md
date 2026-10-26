<p align="center">
  <img src="https://raw.githubusercontent.com/dDostalker/htui/main/logo.svg" alt="htui logo" width="140">
</p>

# htui

**Write interactive terminal interfaces with HTML, CSS and JavaScript**

[![crates.io](https://img.shields.io/crates/v/htui.svg)](https://crates.io/crates/htui)
[![docs.rs](https://img.shields.io/docsrs/htui)](https://docs.rs/htui)
[![license](https://img.shields.io/crates/l/htui.svg)](https://github.com/dDostalker/htui)
[![Rust](https://img.shields.io/badge/rust-1.89%2B-orange.svg)](https://www.rust-lang.org)

[中文说明](https://github.com/dDostalker/htui/blob/main/README.zh-CN.md)

`htui` is a Rust library that renders HTML documents in the terminal to build
TUIs, with partial CSS and script support. It aligns as closely as possible
with the pure-HTML development experience, functionality and ease of use —
delivering as many features as possible while keeping the library as easy as
possible to pick up.

> This project is in an early stage of rapid development. No stability
> guarantees are made for the API, the rendered interface, or the internal
> implementation — use with caution.

## Quick start

```toml
[dependencies]
htui = "0.1"
```

```rust
use htui::{CssSource, HtmlSource, JsSource, Session};

fn main() {
    let html = HtmlSource::inline(
        r#"
        <h1 style="color: steelblue">Hello, htui</h1>
        <p>Press <b>Esc</b> to quit.</p>
        "#,
    );
    let css = CssSource::inline("");
    let js = JsSource::inline("console.log('htui ready')");
    let mut session = Session::new();
    session.render(&html, &css, &js);
}
```

Interactivity is one attribute away — `<button onclick="greet()">`, backed by
a `#[htui::command]` Rust function. See the tutorials below.

## Debug source hot reload

Load HTML, CSS, and initial JS with the matching macros:

```rust
use htui::{Session, include_css, include_html, include_js};

fn main() {
    let page = include_html!("src/page.html");
    let style = include_css!("src/page.css");
    let script = include_js!("src/page.js");
    let mut session = Session::new();
    session.render(&page, &style, &script);
}
```

In debug builds, htui watches each source. Saving HTML reloads the whole
document, saving CSS replaces session CSS, and saving JS replaces/re-runs
session JS. Registering or removing a `mem://` resource also invalidates the
running document in debug: the input loop re-parses the current HTML so
`<script src>`, `<link>`, images, and `fetch` resolve against the updated
registry. Release builds embed the sources only: no watcher and no source paths
are included. Paths are relative to the current Cargo package root.

## Example: terminal music player

The [repository](https://github.com/dDostalker/htui) ships a complete music
player UI under `example/` — album art, progress bar, transport controls, and
a playlist with hover and selection highlighting. It is an interface +
interaction demo only; no audio is played. Clone the repository and run:

```bash
cd example
cargo run
```

| Terminal — `cargo run` | Browser — same `player.html` |
| --- | --- |
| ![htui music player in the terminal](https://raw.githubusercontent.com/dDostalker/htui/main/docs/example.png) | ![htui music player in the browser](https://raw.githubusercontent.com/dDostalker/htui/main/docs/example-html.png) |

## Documentation

| Document | Content |
| --- | --- |
| [docs.rs/htui](https://docs.rs/htui) | API reference |
| [Tutorial: HelloWorld](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/教程/1.HelloWorld.md) | Your first page |
| [Tutorial: Static terminal](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/教程/2.实现静态终端.md) | Disabling the event system, render configuration |
| [Tutorial: Resources](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/教程/3.0资源.md) | The `mem://` concept |
| [Tutorial: Registering and reading resources](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/教程/3.1资源的注册和读取.md) | The `mem://` operations manual |
| [Tutorial: Dynamic terminal](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/教程/4.0动态终端.md) | Events, timers, built-in keys |
| [Tutorial: JS and Rust bindings](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/教程/4.1JS和Rust绑定.md) | Host functions, interop |
| [Architecture](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/架构.md) | Render pipeline and threading model |
| [Boundary document](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/边界文档.md) | Goals, promises and rejection list |
| [Capability matrix](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/能力差异清单.md) | Full comparison against browsers |
| [Script layer reference](https://github.com/dDostalker/htui/blob/main/docs/zh-cn/教程/script-layer.md) | In-depth script layer documentation |

## Platform

Rust **1.89+** (edition 2024).

## License

Dual-licensed under
[Apache-2.0](https://github.com/dDostalker/htui/blob/main/LICENSE-APACHE) or
[MIT](https://github.com/dDostalker/htui/blob/main/LICENSE-MIT), at your option.
