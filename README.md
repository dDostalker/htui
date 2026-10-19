# htui

**An HTML/CSS/JS engine for the terminal — build interactive TUIs the way you build web pages.**

[![crates.io](https://img.shields.io/crates/v/htui.svg)](https://crates.io/crates/htui)
[![docs.rs](https://img.shields.io/docsrs/htui)](https://docs.rs/htui)
[![license](https://img.shields.io/crates/l/htui.svg)](https://github.com/dDostalker/htui)
[![Rust](https://img.shields.io/badge/rust-1.89%2B-orange.svg)](https://www.rust-lang.org)

[中文说明](README.zh-CN.md)

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

## Visual test gallery

The repository ships an interactive gallery (`visual_test`) covering every
supported HTML/CSS/JS feature — layout, flexbox, tables, forms, images, events
and scripting. From the repository root:

```bash
# Interactive gallery (default page: showcase; Esc to quit)
cargo run -p visual_test

# A specific demo
cargo run -p visual_test -- flex
cargo run -p visual_test -- events

# Headless preview: plain-text rows, no raw mode / alternate screen
cargo run -p visual_test -- --dump showcase --cols 68
cargo run -p visual_test -- --dump bg showcase   # background-fill map (█)
```

Each demo is a folder under `visual_test/src/` (`index.html` + optional
`style.css`). Available demos: `all_in_one`, `aside`, `blockquote`, `box_model`,
`colors`, `columns`, `configurator`, `css`, `css_props`, `details`,
`dialog_template`, `events`, `fetch_demo`, `flex`, `flex_advanced`,
`flex_align_content`, `flex_align_self`, `flex_nested`, `font_size`,
`font_styles`, `form`, `form_elements`, `header_footer`, `headings`,
`helloworld`, `image`, `input`, `list`, `menu_dl`, `nested`, `place`,
`position`, `pre`, `q_abbr`, `showcase`, `table`, `text`, `viewport_units`.

## Documentation

| Document | Content |
| --- | --- |
| [docs.rs/htui](https://docs.rs/htui) | API reference |
| [中文文档](README.zh-CN.md) | Chinese introduction |
| [教程：HelloWorld](docs/zh-cn/教程/1.HelloWorld.md) | First rendering |
| [教程：动态终端](docs/zh-cn/教程/4.0动态终端.md) | Events, timers, built-in keys |
| [教程：JS 和 Rust 绑定](docs/zh-cn/教程/4.1JS和Rust绑定.md) | Host functions, interop |
| [教程：资源层](docs/zh-cn/教程/3.0资源.md) | The `mem://` registry |
| [架构](docs/zh-cn/架构.md) | Pipeline & threading model |
| [能力差异清单](docs/zh-cn/能力差异清单.md) | Full HTML/CSS/JS matrix vs. browsers |

## Platform

Rust **1.89+** (edition 2024). Windows console and POSIX terminals via
[crossterm](https://github.com/crossterm-rs/crossterm); truecolor recommended.

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or
[MIT](LICENSE-MIT) at your option.
