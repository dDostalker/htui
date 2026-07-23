# htui 架构总览

> 本文档描述 htui 终端 HTML/CSS 渲染引擎的当前实现架构。文档以**重构后的两阶段布局管线**为中心，覆盖 `html → tdom → layout → tui` 四层、模块划分、数据流以及扩展新元素的标准流程。

---

## 1. 项目定位

`htui` 是一个用 Rust 编写的终端 HTML 渲染器，对外暴露 `htui::render(html, css)`，在终端中以 ANSI 转义序列的形式展示结构化的 HTML+CSS 文档。它不追求完整的浏览器引擎，而是覆盖"块级布局 + 文字渲染 + 终端适配"这一核心子集。

主要能力：
- 解析 HTML5 文档（`html5ever`）。
- 解析 CSS 选择器、声明块（`cssparser`）。
- 将 CSS 应用到 DOM（含 `color` / `text-align` / `font-weight` 等可继承属性的级联）。
- 按"两阶段布局管线"渲染为终端字符单元 `Block`。
- 响应式刷新（滚动、缩放、重绘）。

不在范围内：
- JavaScript、SVG、Canvas、媒体查询、动画。
- 行内布局（`inline`）的精细处理（仅 `Span` / `Text`）。
- 表单控件的交互。

---

## 2. 顶层架构

```
HTML 字符串 ──► html_parse::Parser ──► (NodeId, NodeArena)
                                              │
                                          arena 引用
                                              ▼
CSS 字符串 ──► css_parse::Parser ──► CssRule
                                              │
                                       css_into_node()
                                              ▼
                                   NodeArena（含 CSS 属性）
                                              │
                                              ▼
                            ┌─────────────────────────────┐
                            │  两阶段布局管线              │
                            │                             │
                            │   ① layout_tree()           │
                            │      DOM → Vec<LayoutBox>   │
                            │                             │
                            │   ② paint_layout_tree()     │
                            │      LayoutBox → Block 网格 │
                            └─────────────────────────────┘
                                              │
                                              ▼
                                    Vec<Block>（虚拟画布）
                                              │
                                              ▼
                              TermLayout::render()（ANSI 转义）
```

四层职责严格分离：

| 层 | 模块 | 输入 | 输出 |
| --- | --- | --- | --- |
| HTML 解析 | `src/html/html_parse.rs` | HTML 字符串 | `(NodeId, NodeArena, embedded_css)` |
| CSS 解析 | `src/html/css_parse.rs` | CSS 字符串 | `CssRule` |
| DOM + CSS 应用 | `src/tdom/`, `src/html/css_parse.rs::css_into_node` | `(NodeId, NodeArena)`, `CssRule` | 含 `css_attr` 的 `NodeArena` |
| 布局 | `src/layout/` | `NodeArena` + `TermMap` | `Vec<LayoutBox>` |
| 绘制 | `src/tui/graph_api.rs` | `Vec<LayoutBox>` + `NodeArena` | `TermLayout.layout` |
| 终端输出 | `src/tui/graph_api.rs::render` | `TermLayout` | ANSI 序列到 `stdout` |

---

## 3. 目录结构

```
htui/
├── src/
│   ├── lib.rs                  # render() 主入口 + 渲染线程
│   ├── html/
│   │   ├── mod.rs
│   │   ├── html_parse.rs       # HTML → tdom（html5ever）
│   │   ├── css_parse.rs        # CSS 选择器/声明解析 + css_into_node
│   │   └── html.md             # HTML 解析器文档
│   ├── tdom/
│   │   ├── mod.rs
│   │   ├── node.rs             # NodeType / Node / NodeArena / NodeId
│   │   ├── attributes.rs       # Attr（HTML 属性解析与查询）
│   │   └── element_fn/         # 元素行为函数（占位，未来扩展）
│   ├── layout/
│   │   ├── mod.rs              # DEFAULT_LAYOUT 注册表 + LayoutFn 签名
│   │   ├── box_tree.rs         # Rect / BoxKind / LayoutBox / Cursor
│   │   ├── tree.rs             # layout_tree() 编排器
│   │   ├── paint.rs            # paint_layout_tree() 编排器
│   │   ├── layout.rs           # TermMap + TermLayout::draw 入口
│   │   └── layout_fn/          # 各元素布局函数
│   │       ├── p.rs, div.rs, head.rs
│   │       ├── header_footer.rs
│   │       ├── aside.rs
│   │       └── img.rs
│   └── tui/
│       ├── mod.rs
│       └── graph_api.rs        # Block / TermLayout / paint_image_at / render
├── Cargo.toml
├── visual_test/                # 可视化测试（cargo run -p visual_test）
└── tests/                      # 单元 / 集成测试
```

---

## 4. 数据流（端到端）

### 4.1 入口

```rust
// src/lib.rs
pub fn render(html: &str, css: &str) { ... }
```

`render` 启动一个**渲染线程**接收 `RenderCmd`：

```rust
pub enum RenderCmd {
    Rerender { scroll_y: u32 },
    Redraw   { scroll_y: u32, zoom: f32 },
    Quit,
}
```

线程主循环每次收到 `Redraw` 时：

1. `parse_and_apply(html, css)`：HTML→`NodeArena`，再 `css_into_node` 应用 CSS（含可继承属性级联）。
2. 构造 `TermLayout::new(&term_map, zoom)`：分配 `cols * rows * 20` 个 `Block` 的虚拟画布。
3. `tl.draw(&arena, root_id, &term_map)`：**两阶段布局管线**（详见 §5）。
4. 设置 `scroll_y`，`render()` 写 ANSI 序列。

### 4.2 DOM 阶段

- `NodeArena` 是 `SlotMap<NodeId, Node>`，类型安全、Copy + Send + Sync。
- `Node` 持有 `node_type: NodeType`、`children: Vec<NodeId>`、`parent: Option<NodeId>`、`attributes: Attr`、`css_attr: CssAttr`。
- `NodeType` 枚举 28 种标签（参见 `src/tdom/node.rs`）。

### 4.3 CSS 应用阶段

`css_into_node(arena, root_id, css)` 做两件事：

1. 对每条 CSS 规则，递归匹配 DOM，应用 `css_attr`。
2. `inherit_css` 把 `color` / `text-align` / `font-weight` 三个可继承属性沿父链向下传播。

这一步结束，每个节点都携带了完整的 CSS 视图（自己的 + 继承的），布局和绘制阶段只读 `node.css_attr`。

### 4.4 布局与绘制（核心）

详见 `doc/layout.md`。

---

## 5. 两阶段布局管线（核心）

旧实现是"边布局边绘制"：每个 `LayoutFn` 直接调用 `TermLayout::paint_string` / `new_line`，所有元素共享一个全局 `point` 光标。问题在于：

- 子元素**不知道**自己的 `x` 和 `width`，只能读到 `term_layout.point`，无法判断它在父盒中的相对位置。
- `text-align` 计算基于终端宽度而不是块宽，嵌套时位置错误。
- `aside` 想渲染嵌套元素只能 `collect_all_text()`，把子块"压平"成纯文本。

重构后：

```rust
// src/layout/layout.rs
impl TermLayout {
    pub fn draw(&mut self, arena, root_id, term_map) -> Result<()> {
        let containing = BoxConstraint::root(term_map);
        let mut cursor  = Cursor::new();
        let boxes = layout_tree(arena, root_id, containing, &mut cursor)?;
        paint_layout_tree(&boxes, arena, self)?;
        Ok(())
    }
}
```

- **阶段 1 — 布局树**：`layout_tree()` 遍历 DOM 注册表，构建 `Vec<LayoutBox>`，每个盒都计算好自己的 `Rect { x, y, width, height }`。
- **阶段 2 — 绘制**：`paint_layout_tree()` 按 `Rect` 直接写 `Block` 到虚拟画布，**不使用共享光标**。

子盒的 `containing` 是父盒的 `(rect.x, rect.width)`，CSS `width` / `text-align` / `background-color` 都按盒计算，不依赖终端全局坐标。

---

## 6. 关键类型速查

| 类型 | 位置 | 说明 |
| --- | --- | --- |
| `NodeId` | `tdom::node` | slotmap 类型安全 ID |
| `NodeArena` | `tdom::node` | `SlotMap<NodeId, Node>` |
| `Node` | `tdom::node` | DOM 节点（含子节点、属性、`css_attr`） |
| `NodeType` | `tdom::node` | 节点类型枚举（28 种标签 + `Text(String)`） |
| `Attr` | `tdom::attributes` | HTML 属性字符串解析与查询 |
| `CssAttr` | `html::css_parse` | 解析后的 CSS 属性（`color`、`background-color`、`text-align`、`font-weight`、`width`） |
| `CssRule` | `html::css_parse` | 完整 CSS 规则表 |
| `CssSelector` | `html::css_parse` | CSS 选择器（含组合器） |
| `BoxConstraint` | `layout::mod` | 子盒的 `(x, width)` 约束 |
| `FormattingContext` | `layout::mod` | `BoxConstraint` 的兼容封装 |
| `Rect` | `layout::box_tree` | `(x, y, width, height)` 矩形 |
| `BoxKind` | `layout::box_tree` | `Block { fill_bg, spacing_after }` / `Image { cols, rows }` |
| `LayoutBox` | `layout::box_tree` | 布局树节点（`node_id` + `rect` + `kind` + `text` + `children`） |
| `Cursor` | `layout::box_tree` | 当前可用行号（块级兄弟共享） |
| `LayoutFn` | `layout::mod` | `(arena, node_id, containing, cursor) -> Result<Option<LayoutBox>>` |
| `DEFAULT_LAYOUT` | `layout::mod` | `LayoutFn` 注册表（按顺序，第一个返回 `Some` 的胜出） |
| `TermMap` | `layout::layout` | 终端尺寸 `(columns, rows)` |
| `TermLayout` | `tui::graph_api` | 虚拟画布 + 渲染入口 |
| `Block` | `tui::graph_api` | 单个屏幕字符单元（`word`, `fcolor`, `bcolor`, `bold`） |
| `RenderCmd` | `tui::graph_api` | 渲染线程命令枚举 |

---

## 7. 设计原则（与项目 `AGENTS.md` 对齐）

1. **四层不可破坏**：`html → tdom → layout → tui` 严格分离，不引入跨层抽象。
2. **不引入新的抽象层**，除非有充分理由且全员可见。
3. **不破坏 `DEFAULT_LAYOUT` 注册表约定**：新增元素必须在注册表中注册对应的 `LayoutFn`。
4. **不引入 `unsafe`**：性能优化走算法而非 unsafe。
5. **不引入 `println!`**：调试一律 `log::debug!/info!/warn!/error!`。
6. **不忽略错误**：禁止 `let _ = ...`，统一走 `anyhow::Result`。
7. **API 最小共享**：内部 helper 不要泄漏到 `lib.rs` 之外的公开面。
8. **测试不依赖终端**：所有布局测试用 `mock_term_map()`，不调用 `terminal::size()`。
9. **代码实现文件不加测试**：单测统一放在 `tests/`。
10. **公开 API 注释用英文**：内部 helper 注释可用中文。

---

## 8. 添加新 HTML 元素的标准流程

> 假设我们要支持 `<section>`。

1. **`src/tdom/node.rs`**：在 `NodeType` 枚举中加入 `Section` 变体。
2. **`src/html/html_parse.rs`**：在 `tag_name_to_node_type()` 中加入 `"section" => Some(NodeType::Section)`。
3. **`src/html/css_parse.rs`**：在 `tag_str_to_node_type()` 中加入 `NodeType::Section` 的 CSS 选择器映射。
4. **`src/layout/layout_fn/section.rs`**（新文件）：实现 `pub fn layout_section(arena, node_id, containing, cursor) -> Result<Option<LayoutBox>>`，参照 `div.rs` 的结构。
5. **`src/layout/layout_fn/mod.rs`**：声明 `pub mod section;`。
6. **`src/layout/mod.rs`**：在 `DEFAULT_LAYOUT` 中追加 `layout_fn::section::layout_section`。
7. **`tests/layout_tests.rs`** 或 `tests/layout_tree_tests.rs`：新增覆盖基础渲染、宽度、背景、文本对齐的测试。

无需改动 `paint.rs` 或 `tree.rs`——它们是元素无关的通用绘制与编排。

---

## 9. 错误处理约定

- 所有公开函数返回 `anyhow::Result<T>`，避免 `unwrap` / `expect`（单元测试可用 `expect`，但要在 panic 信息里写清楚假设）。
- 解析错误通过 `log::warn!` 记录并尽量继续（HTML 不支持标签压平、CSS 选择器失败跳过该规则）。
- 渲染阶段 I/O 错误用 `log::error!`，不 panic（终端已损坏时继续运行可能更友好）。

---

## 10. 性能与扩展性

### 10.1 性能

- 画布大小：`cols * rows * 20` 的 `Vec<Block>`，按 cell 索引 O(1) 写入。
- 布局阶段：`O(n)` DOM 节点，单次深度优先遍历。
- 绘制阶段：每个 `LayoutBox` 独立写入，理论上可并行（未来）。
- 当前**没有** layout cache——每次 `Redraw` 重新跑两阶段管线。可优化空间：增量布局（仅重算 CSS 改变的子树）。

### 10.2 扩展点

- **新元素**：见 §8。
- **新 CSS 属性**：在 `CssAttr` 加 getter，在 `InheritableProperty` 中加入需要继承的属性。
- **新文本度量**：替换 `tui::graph_api::char_width` 即可。
- **新图像格式**：替换 `tui::graph_api::paint_image_at` 内部的 `Img::_create_img`。

---

## 11. 已知遗留

- `tui::graph_api::render_text` / `paint_string` / `paint_img` / `new_line` 仍是旧的"共享光标"API；已标 `#[deprecated]` 但暂未删除。新的两阶段管线**不会**调用它们。
- 虚拟画布按 `cols * rows * 20` 预分配，超长文档会提前 `is_full()` 截断——目前没有"流式渲染"模式。
- `<header>` / `<footer>` 的 `spacing_after = 2` 是硬编码常量，未来可读 CSS `margin-bottom`。
- 背景与渲染：`<aside>` / `<div>` 等块级背景已经支持多行填充（见 `doc/layout.md` §5.2、§7.5）；文字在递归绘制时**会继承**父级 `effective_bg = own_bg.or(parent_bg)`，所以嵌套 `<p>` 在 `<aside bg=cyan>` 内呈现“坐在” cyan 上，与浏览器一致。

---

## 12. 参考

- 项目总体约束：`AGENTS.md`
- HTML/CSS 解析细节：`src/html/html.md`
- 两阶段布局细节：`doc/layout.md`
- 框模型 / 坐标：`doc/layout.md` §3