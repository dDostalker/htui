# HTML解析器

## 概述

HTML解析器是htui项目中的一个核心模块，负责将HTML字符串解析为树形DOM结构（称为tdom）。解析器基于 **html5ever** 库实现，该库是 Servo 浏览器引擎使用的符合 WHATWG HTML5 规范的解析器。

## 核心数据结构

### Parser结构体

```rust
#[derive(Debug)]
pub struct Parser {
    input: String,
}
```

Parser结构体封装了html5ever的解析流程，对外提供简洁的接口。

### NodeType枚举

HTML解析器支持的节点类型包括：
- `Global`: 根节点类型
- `Span`, `Div`, `P`: 基础标签
- `Img`, `Input`: 自闭合标签
- `H1`-`H5`: 标题标签
- `A`, `Button`, `Form`, `Label`: 交互元素
- `Header`, `Footer`, `Nav`, `Main`, `Section`, `Article`, `Aside`: 语义元素
- `Figure`, `Figcaption`, `Details`, `Summary`: 语义元素
- `Ul`, `Ol`, `Li`: 列表元素
- `Select`, `HtmlOption`, `Textarea`: 表单元素
- `Text(String)`: 文本节点

## 解析流程

### 主解析入口

```rust
impl Parser {
    pub fn parse(&mut self) -> Result<Node> {
        // 1. 使用 html5ever::parse_document 解析 HTML
        // 2. 将 RcDom 树转换为内部 Node 树
        // 3. 返回以 NodeType::Global 为根的树结构
    }
}
```

### html5ever 转换流程

1. **HTML解析**: `html5ever::parse_document(RcDom::default(), opts).one(input)` 产生 `RcDom` 树
2. **节点转换**: `convert_children()` 递归遍历 RcDom 树：
   - `Element` → 通过 `tag_name_to_node_type()` 映射为 `NodeType`，创建 `Node`
   - `Text` → 创建 `Text(String)` 文本节点
   - `Document` → 递归处理子节点
   - `Comment` / `Doctype` → 跳过
3. **属性转换**: html5ever 的 `Attribute` → 序列化为 `key="value"` 字符串 → `Attr::parse_from_str()` 解析
4. **不支持的标签**: `tag_name_to_node_type()` 返回 `None` 时，元素节点被跳过，子节点提升到父节点（"压平"处理）

### 标签映射函数

```rust
fn tag_name_to_node_type(tag: &str) -> Option<NodeType> {
    match tag.to_ascii_lowercase().as_str() {
        "div" => Some(NodeType::Div),
        "p" => Some(NodeType::P),
        "span" => Some(NodeType::Span),
        "h1" => Some(NodeType::H1),
        // ... 共 28 种支持的标签
        _ => None,  // 不支持的标签：压平处理
    }
}
```

## 使用示例

### 基本用法

```rust
use htui::html::html_parse::Parser;

let html = "<div><p>Hello World</p></div>";
let mut parser = Parser::new(html);
let dom = parser.parse().unwrap();
```

### 带属性的HTML解析

```rust
let html = r#"<div class="container" id="main">
    <h1>标题</h1>
    <p>段落文本</p>
</div>"#;

let mut parser = Parser::new(html);
let dom = parser.parse().unwrap();
```

## 与旧实现的对比

| 特性 | 旧实现（手写） | 新实现（html5ever） |
|------|---------------|---------------------|
| 解析引擎 | 递归下降 | HTML5 规范 Tokenizer+TreeBuilder |
| 容错能力 | 有限 | 完整的 HTML5 错误恢复 |
| 文档结构 | 需手动处理 | 自动构建 html/head/body |
| 自闭合标签 | 仅支持 img/input | 支持所有合法自闭合标签 |
| 属性解析 | 自定义 | html5ever + Attr::parse_from_str 回填 |
| 维护成本 | 高 | 低（跟随上游） |

## 错误处理

- 格式不良的HTML由html5ever自动容错处理
- 严重解析错误通过`anyhow::Result`返回
- 不支持的标签通过`warn!`日志记录，不中断解析

## 测试

解析器包含单元测试，验证基本功能：

```rust
#[test]
fn test_tag_name_to_node_type() {
    assert_eq!(tag_name_to_node_type("span"), Some(NodeType::Span));
    assert_eq!(tag_name_to_node_type("div"), Some(NodeType::Div));
    assert_eq!(tag_name_to_node_type("DIV"), Some(NodeType::Div)); // 大小写不敏感
    assert_eq!(tag_name_to_node_type("unsupported"), None);
}

#[test]
fn test_parse_basic_html() {
    let html = "<div><p>Hello</p><p>World</p></div>";
    let mut p = Parser::new(html);
    let node = p.parse().unwrap();
    assert!(!node.children.is_empty());
}
```

## 与TUI集成

解析后的tdom结构传递给布局引擎（`layout` 模块）进行屏幕布局计算，最终通过TUI框架渲染到终端。html5ever替换后，该流程完全不变。

---

*文档最后更新：2026年7月22日*

*相关文件：`src/html/html_parse.rs`, `src/tdom/node.rs`, `src/tdom/attributes.rs`*
