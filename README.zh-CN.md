# htui

**用 HTML、CSS 和 JavaScript 编写终端交互界面——终端界的 Tauri。**

[![crates.io](https://img.shields.io/crates/v/htui.svg)](https://crates.io/crates/htui)
[![docs.rs](https://img.shields.io/docsrs/htui)](https://docs.rs/htui)
[![license](https://img.shields.io/crates/l/htui.svg)](https://github.com/dDostalker/htui)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)

[English](README.md)

`htui` 是一个 Rust 库：它在终端里渲染 HTML 文档（带完整 CSS 子集），并在内嵌
的 JavaScript 引擎（[Boa](https://github.com/boa-dev/boa)）中运行页面的
`<script>` 脚本，DOM 绑定与浏览器语义一致。Rust 侧通过**宿主函数**与页面
交互——注册后即为 JS 可直接调用的全局函数。文档能看到的一切资源都来自
内存 `mem://` 注册表：不访问文件系统、不访问网络。

## 特性

- **HTML + CSS 渲染**——流式排版、flexbox、表格、多列、定位、盒模型、文字
  样式、`:hover` / `:focus`
- **JavaScript 脚本**——对齐浏览器的 DOM 绑定、定时器、`fetch`（真
  Promise）、`console`
- **Rust ↔ JS 互操作**——`#[htui::command]` 把普通 Rust 函数变成带类型的
  JS 全局函数
- **DOM 风格事件**——click / hover / focus / input / 键盘 / 表单事件，冒泡 +
  `preventDefault()`
- **表单控件**——input、textarea、select + datalist、checkbox / radio、表单
  提交 / 重置、details、dialog
- **图片**——PNG / JPEG / WebP 与 SVG，真彩色半块字符渲染
- **流畅渲染**——增量 diff 输出、区域重绘、60fps 上限
- **全程无 `unsafe`**——页面 JS 受运行时限制；图片解码与资源层与文件系统 /
  网络完全隔离

## 快速上手

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
        <p>按 <b>Esc</b> 退出。</p>
        "#,
        "",
    );
}
```

交互只需一个属性——`<button onclick="greet()">`，背后是一个
`#[htui::command]` 的 Rust 函数。见下方教程。

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
| [脚本层完整文档](docs/zh-cn/script-layer.md) | 脚本层深度参考 |

## 平台

Rust **1.88+**（edition 2024）。经
[crossterm](https://github.com/crossterm-rs/crossterm) 支持 Windows 控制台与
POSIX 终端；建议开启真彩色。

## 许可证

双许可证：[Apache-2.0](LICENSE-APACHE) 或 [MIT](LICENSE-MIT)，任选其一。
