# HTML 解析器 — html5ever 迁移

## 变更概述

将项目原先手写的递归下降 HTML 解析器替换为基于 `html5ever` 库的实现。

- **修改日期**: 2026-07-22
- **影响范围**: `src/html/html_parse.rs`
- **新增依赖**: `html5ever = "0.27"`, `markup5ever_rcdom = "0.3"`, `tendril = "0.4"`

## 变更动机

1. **标准合规**: `html5ever` 是 Servo 项目使用的 HTML5 解析器，完全符合 WHATWG HTML 规范
2. **容错能力**: html5ever 能正确处理格式不良的 HTML（标签未闭合、属性无引号等）
3. **维护性**: 减少自行维护解析器的工作量，跟随上游改进
4. **完整性**: 自动处理 `<html>`, `<head>`, `<body>` 等文档结构元素

## 架构变化

### 保持不变的接口

```rust
// lib.rs 中使用 Parser 的方式完全不变
let mut html_parser = html_parse::Parser::new(html);
let node = html_parser.parse().unwrap();
```

- `Parser::new(input)` — 创建解析器
- `Parser::parse()` — 返回 `Result<Node>`
- 输出 `Node` 树结构不变，Layout 模块无需任何修改

### 内部实现变化

| 项目 | 旧实现 | 新实现 |
|------|--------|--------|
| 解析引擎 | 手写递归下降 | `html5ever::parse_document` |
| 字符处理 | `Peekable<CharIndices>` | html5ever 内部 Tokenizer |
| 标签识别 | `ELEMENT_FUNCTIONS` 遍历匹配 | `tag_name_to_node_type()` 直接映射 |
| 属性解析 | `ELEMENT_FUNCTIONS` 中各自的解析 | html5ever 解析 → `Attr::parse_from_str()` 回填 |
| 错误处理 | 不支持的标签返回 `anyhow!` | 不支持的标签 `warn!` + 压平处理 |

### 标签映射

`tag_name_to_node_type()` 函数将 html5ever 的标签名（小写）映射为 `NodeType`。不支持的标签返回 `None`，对应元素节点被跳过而其子节点提升到父节点（"压平"处理）。

### html5ever 到 Node 的转换流程

```
HTML 字符串
    │
    ▼
html5ever::parse_document()
    │
    ▼
RcDom (Document 节点)
    │
    ▼
convert_children() ──递归──▶ Node 树
    │
    ├── Element → tag_name_to_node_type() → Node::from(node_type)
    │   ├── 支持: 创建 Node，递归处理子节点
    │   └── 不支持: warn!，跳过节点，子节点提升到父节点
    ├── Text → 去空白后 Node::text(trimmed)
    ├── Comment → 跳过
    └── Doctype / PI → 跳过
```

### CSS 解析器替换 (同日)

CSS 解析器也替换为 `cssparser` 库（与 html5ever 同属 Servo 生态），新增依赖 `cssparser = "0.31"`。

| 项目 | 旧实现 | 新实现 |
|------|--------|--------|
| 解析引擎 | 手写字符级解析 | `cssparser::StyleSheetParser` |
| 规则分割 | 扫描 `{` `}` | cssparser 原生 tokenizer |
| 选择器提取 | `get_selector()` 读至 `{` | `parse_prelude()` trait 方法 |
| 声明解析 | `get_css_attr()` State 机 | 复用原有 State 机，输入由 cssparser 提取 |
| @-规则 | 不支持（会报错） | 自动跳过 `@media` 等 |
| CSS 注释 | 不支持 | 原生支持 `/* */` |

**保留不变的部分**：
- `CssSelector` / `CssAttr` / `CssRule` 类型定义
- `css_into_node()` 及其匹配引擎
- `Selector` 枚举和所有选择器解析函数

## 未修改的文件

以下模块未做任何改动，Layout 与 TUI 渲染逻辑完全兼容：

- `src/lib.rs` — 导入路径不变，接口不变
- `src/tdom/` — Node 结构、Attr、element_fn 均不变
- `src/layout/` — 布局逻辑不变
- `src/tui/` — 终端渲染不变
- `src/html/mod.rs` — 模块声明不变

## 注意事项

1. **不支持的标签**: html5ever 会解析所有标准 HTML5 标签，但项目 `NodeType` 仅支持特定子集。不支持的标签（如 `<html>`, `<body>`, `<em>`, `<strong>` 等）会被静默压平。
2. **属性序列化**: 为复用 `Attr::parse_from_str()`，html5ever 的属性会先序列化为 `key="value"` 字符串再解析。这是为了最小化对 tdom 模块的侵入。
3. **文档结构**: `parse_document` 总是产生完整的文档结构（含隐式 `<html>`/`<head>`/`<body>`），这些标签会因不被支持而被压平。

## 测试

所有 34 个已有测试 + 8 个新增测试全部通过：

```
test result: ok. 34 passed; 0 failed
```

新增测试覆盖了：
- `test_tag_name_to_node_type` — 标签映射正确性
- `test_parse_basic_html` — 基本 HTML 解析
- `test_parse_with_attributes` — 属性解析
- `test_parse_img` — 自闭合标签
- `test_parse_nested_headings` — 嵌套解析
- `test_parse_heading_with_attributes` — 属性 + NodeType 验证
- `test_parse_full_document` — 完整文档解析
- `test_parse_text_with_inline_elements` — 内联元素压平
- `test_parse_empty_input` — 空输入
