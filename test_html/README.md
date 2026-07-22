# test_html - htui 视觉测试套件

## 概述

此 crate 包含 htui 终端渲染器的肉眼观察测试（Visual Regression Tests）。
每个测试渲染特定场景的 HTML + CSS，用户通过终端界面观察渲染效果。

## 运行方式

```bash
# 运行默认综合测试
cargo run -p test_html

# 运行指定测试
cargo run -p test_html -- <test_name>

# 可用测试名称
cargo run -p test_html -- unknown  # 会列出所有可用测试
```

## 可用测试列表

| 测试名 | 文件 | 覆盖场景 |
|--------|------|----------|
| `all_in_one` | `all_in_one.html` | **默认** 综合演示：标题、段落、嵌套、图片、颜色、特殊字符 |
| `headings` | `headings.html` | H1-H5 标题层级渲染 + CSS 颜色 |
| `image` | `image.html` | 图片渲染：多种尺寸、纵横比、多图序列 |
| `colors` | `colors.html` | CSS 颜色渲染：red/green/blue/yellow/cyan/magenta/white/grey |
| `nested` | `nested.html` | 深度嵌套 div 结构 (1-4 层)，混合嵌套 |
| `text` | `text.html` | 文本内容：长短段落、空段落、Unicode/CJK/Emoji、特殊字符 |
| `css` | `css.html` | 类 CSS 布局 Demo (原项目自带) |
| `helloworld` | `helloworld.html` | 最简 HTML 测试 (原项目自带) |

## 交互操作

- **退出**: 按 `q` 或 `Esc`
- **刷新**: 调整终端窗口大小自动重新渲染
- **鼠标**: 消费鼠标事件但不触发重绘

## 测试文件说明

### all_in_one.html — 综合演示
覆盖场景最全，一次性展示 htui 的主要渲染能力：
- 标题层级 (H1-H5)
- 段落文本（短/长）
- 深度嵌套 div (4 层)
- 图片渲染 ( braille 盲文字符)
- CSS 颜色应用 (class 选择器)
- HTML 实体和 Unicode 字符
- 自闭合标签

### image.html — 图片渲染
测试 `Img` 的 braille 盲文渲染：
- 默认尺寸（充满终端）
- 小图 80×40
- 中图 160×80
- 宽图 320×80
- 高图 80×160
- 多图序列

**注意**: 需要 `src/pic.png` 文件存在。

### colors.html — 颜色测试
测试 CSS `color` 属性通过类选择器的应用：
- `.red`, `.green`, `.blue`, `.yellow`, `.cyan`, `.magenta`, `.white`, `.grey`
- 标题元素上的颜色

### nested.html — 嵌套测试
测试深度嵌套的 HTML 结构：
- 1-4 层 div 嵌套
- 混合标题+段落嵌套
- 深层元素的 CSS 类应用

### text.html — 文本测试
测试各种文本内容的渲染：
- 短文本 / 长文本
- 空段落
- ASCII 特殊字符
- CJK 字符 (中文/日文/韩文)
- Emoji 和 Unicode 符号
- 数学符号

## 测试覆盖矩阵

| 能力 | all_in_one | headings | image | colors | nested | text | css | helloworld |
|------|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| H1-H5 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| P 段落 | ✓ | ✓ | - | ✓ | ✓ | ✓ | ✓ | ✓ |
| Img 图片 | ✓ | - | ✓ | - | - | - | - | - |
| Div 嵌套 | ✓ | - | - | - | ✓ | - | ✓ | - |
| CSS 颜色 | ✓ | ✓ | - | ✓ | ✓ | - | ✓ | - |
| Unicode | ✓ | - | - | - | - | ✓ | - | - |
| Emoji | ✓ | - | - | - | - | ✓ | - | - |
| 空元素 | - | - | - | - | - | ✓ | - | - |
| 多图序列 | - | - | ✓ | - | - | - | - | - |

---

*最后更新: 2026-07-22*
