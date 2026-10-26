<p align="center">
  <img src="logo.svg" alt="htui logo" width="140">
</p>

# htui

**用 HTML、CSS 和 JavaScript 编写终端交互界面**

[![crates.io](https://img.shields.io/crates/v/htui.svg)](https://crates.io/crates/htui)
[![docs.rs](https://img.shields.io/docsrs/htui)](https://docs.rs/htui)
[![license](https://img.shields.io/crates/l/htui.svg)](https://github.com/dDostalker/htui)
[![Rust](https://img.shields.io/badge/rust-1.89%2B-orange.svg)](https://www.rust-lang.org)

[English](README.md)

`htui` 是一个终端里渲染 HTML 文档来实现TUI界面的 rust 库，并支持部分css和script脚本。
该库尽可能的对齐了纯 HTML 的开发体验、功能性和使用易用性。在确保最简单上手的情况下实现最多的功能。

> 当前项目处于早期快速开发阶段，不保证接口、渲染界面、内部实现的稳定，谨慎使用

## 快速上手

```sh
cargo add htui
```

```rust
use htui::{CssSource, HtmlSource, JsSource, Session};

fn main() {
    let html = HtmlSource::inline(
        r#"
        <h1 style="color: steelblue">Hello, htui</h1>
        <p>按 <b>Esc</b> 退出。</p>
        "#,
    );
    let css = CssSource::inline("");
    let js = JsSource::inline("console.log('htui ready')");
    let mut session = Session::new();
    session.render(&html, &css, &js);
}
```

## Debug 源码热加载

用配套宏加载 HTML、CSS 和初始 JS：

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

debug 构建会监听这些源文件：保存 HTML 触发整文档重载，保存 CSS 替换会话级
CSS，保存 JS 替换并重新执行会话级 JS。release 构建只嵌入源码，不包含 watcher
和源码路径。路径相对当前 Cargo package 根目录。

## 示例：终端音乐播放器

`example/` 自带一个完整的音乐播放器界面——封面、进度条、控制按钮，
以及支持悬停与选中高亮的播放列表。仅为界面 + 交互演示，不播放音频。
运行：

```bash
cd example
cargo run
```

| 终端 —— `cargo run` | 浏览器 —— 同一份 `player.html` |
| --- | --- |
| ![htui 音乐播放器（终端）](docs/example.png) | ![htui 音乐播放器（浏览器）](docs/example-html.png) |

## 文档

| 文档 | 内容 |
| --- | --- |
| [docs.rs/htui](https://docs.rs/htui) | API 参考 |
| [教程：HelloWorld](docs/zh-cn/教程/1.HelloWorld.md) | 第一个页面 |
| [教程：静态终端](docs/zh-cn/教程/2.实现静态终端.md) | 关闭事件系统、渲染配置 |
| [教程：资源层](docs/zh-cn/教程/3.0资源.md) | `mem://` 概念 |
| [教程：资源的注册和读取](docs/zh-cn/教程/3.1资源的注册和读取.md) | `mem://` 操作手册 |
| [教程：动态终端](docs/zh-cn/教程/4.0动态终端.md) | 事件、定时器、内置按键 |
| [教程：JS 和 Rust 绑定](docs/zh-cn/教程/4.1JS和Rust绑定.md) | 宿主函数、互操作 |
| [架构](docs/zh-cn/架构.md) | 渲染管线与线程模型 |
| [边界文档](docs/zh-cn/边界文档.md) | 目标、承诺与拒绝清单 |
| [能力差异清单](docs/zh-cn/能力差异清单.md) | 与浏览器的全矩阵对比 |
| [脚本层完整文档](docs/zh-cn/教程/script-layer.md) | 脚本层深度参考 |

## 平台

Rust **1.89+**（edition 2024）。

## 许可证

双许可证：[Apache-2.0](LICENSE-APACHE) 或 [MIT](LICENSE-MIT)，任选其一。
