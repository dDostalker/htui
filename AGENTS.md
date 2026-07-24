# htui — 终端 HTML/CSS 渲染引擎

> 本文档是 对LLM Agent的**提示与约束文档**。*本文档是 LLM 操作本项目的行为契约。任何时候应当以本文件为最高优先级参考。*
> 在LLM与本项目交互时，必须严格遵循以下规范。

---

## 项目概述

**htui** 是一个 Rust 编写的终端 HTML 渲染器。作为终端界的tuari使用。

## 项目结构
以下是项目的结构，如果不是特殊要求不得修改项目结构，仅当提示词要求需要时修改，修改前进行询问，并**必须**对项目结构进行同步
```
htui/
├── src/
│   ├── lib.rs              # 入口：render() 主循环 + 渲染线程
│   ├── html/
│   │   ├── mod.rs
│   │   ├── html_parse.rs   # HTML → tdom 树（html5ever）
│   │   ├── css_parse.rs    # CSS 解析 + 选择器匹配 + 属性赋值
│   │   └── html.md         # 模块文档
│   ├── tdom/
│   │   ├── mod.rs
│   │   ├── node.rs         # NodeType / Node / NodeArena / NodeId
│   │   ├── attributes.rs   # Attr（HTML 属性解析与查询）
│   │   └── element_fn/     # 元素行为函数
│   ├── layout/
│   │   ├── mod.rs          # DEFAULT_LAYOUT 注册表 + LayoutFn 类型
│   │   ├── layout.rs       # TermMap / TermLayout::draw()
│   │   └── layout_fn/      # 各元素布局函数
│   │       ├── div.rs, p.rs, span.rs
│   │       ├── head.rs (h1-h5)
│   │       ├── header_footer.rs
│   │       ├── aside.rs
│   │       └── img.rs
│   └── tui/
│       ├── mod.rs
│       └── graph_api.rs    # Block / TermLayout / RenderCmd / 图像加载
├── Cargo.toml
├── visual_test/          # 可视化测试工具（cargo run -p visual_test）
│   └── src/
│       ├── main.rs
│       ├── all_in_one/   # index.html: 可直接浏览器打开，内部 <style> 内联 CSS
│       ├── headings/
│       ├── image/        # includes pic.png
│       ├── colors/
│       ├── nested/
│       ├── text/
│       ├── css/
│       ├── header_footer/
│       ├── aside/
│       ├── helloworld/
│       └── font_styles/  # bold / italic / underline / line-through
├── tests/
│   ├── fixtures/          # HTML/CSS test fixtures + images
│   ├── html_parse_tests.rs
│   ├── css_parse_tests.rs
│   ├── layout_tests.rs
│   ├── layout_aside_tests.rs
│   ├── layout_header_footer_tests.rs
│   ├── attributes_tests.rs
│   └── graph_api_tests.rs
```

## 架构

```
HTML 字符串 ──► html_parse::Parser ──► (NodeId, NodeArena)
                                           │
                                        arena 引用
                                           │
                                           ▼
CSS 字符串 ───► css_parse::Parser ──► CssRule
                                           │
                                      css_into_node()
                                           │
                                           ▼
                                    NodeArena（含 CSS 属性）
                                           │
                                           ▼
                              layout::TermLayout::draw()
                                    │
                          ┌─────────┼──────────┐
                          ▼         ▼          ▼
                     layout_p  layout_div  layout_img  ...
                          │         │          │
                          └─────────┼──────────┘
                                    ▼
                            Vec<Block>（画布）
                                    │
                                    ▼
                          TermLayout::render()
                                    │
                                    ▼
                              终端输出（ANSI 转义序列）
```

## 必要事项
`html → tdom → layout → tui` 四层不可破坏。不要引入新的抽象层，除非有充分理由。在修改之前进行询问。

新增 HTML 元素支持时，必须在 `layout::mod.rs` 的 `DEFAULT_LAYOUT` 数组中注册对应的 `LayoutFn`，同时在 `layout_fn/` 下新增模块。
不要用字符串匹配代替 `NodeType` 枚举。新增标签必须：
- 在 `tdom::node::NodeType` 枚举中添加变体
- 在 `html_parse::tag_name_to_node_type()` 中添加映射
- 在 `css_parse::tag_str_to_node_type()` 中添加 CSS 选择器映射

通过 `Node.css_attr`（类型 `CssAttr`）访问已解析的 CSS 属性。不要直接解析 style 属性字符串。
使用 `log::debug! / info! / warn! / error!`，不要使用 `println!` 进行调试输出。
修改代码时不得随意修改已经存在的API的注释，只有实现修改导致内容和实现不符时修改，并询问用户

## 禁止使用git
本项目禁止LLM调用git

## 避免unsafe和添加足够的调试信息支持
本项目不需要 `unsafe` 代码。如需性能优化，优先考虑算法层面。
在必要要的情况下添加调试信息，

## 错误处理
- 返回报错时不得直接unwrap而是处理错误或是返回完整报错信息
- 不得使用let _ = 等任何形式忽略报错

## 测试不依赖终端
- 单元测试不应依赖实际的终端环境。如果必须测试终端相关功能，使用条件编译或 mock。
- 完成全新的功能时检查是否存在测试用例，如果不存在或者测试不完全时，添加需要的测试用例在当前目录下test文件中。
- 如果是实现元素支持时，添加到html-test。
- 代码实现文件禁止添加测试。
- 如果时测试专用的api**禁止**混入代码文件中，需要在测试文件中定义

## 代码风格约束

- **注释语言**：公开 API 后续必须使用英文注释。
- **命名规范**：遵循 Rust 标准命名（snake_case 函数/变量，CamelCase 类型，SCREAMING_SNAKE_CASE 常量）。
- **import 风格**：使用 crate 内相对路径引用，不在模块顶部做通配导入。

## API创建
- API创建时必须对功能进行///注释，并且其他位置除难以理解的部分不要注释，必要时可以注释。
- 对于API的共享保持最小共享原则。

## 添加新 HTML 元素支持 — 操作清单

当你需要支持一个新的 HTML 标签时，
1. **`tdom/node.rs`** — 在 `NodeType` 枚举中加入变体
2. **`html/html_parse.rs`** — 在 `tag_name_to_node_type()` 中添加映射
3. **`html/css_parse.rs`** — 在 `tag_str_to_node_type()` 中添加 CSS 选择器映射
4. **`layout/layout_fn/`** — 新建 `<element>.rs`，实现 `LayoutFn` 签名
5. **`layout/layout_fn/mod.rs`** — 声明模块并导出
6. **`layout/mod.rs`** — 在 `DEFAULT_LAYOUT` 中注册


## 关键类型速查
类型在创建前优先查看是否存在需要的类型，在修改创建前进行询问，并**必须**对项目已有的类型进行同步

| 类型               | 位置                    | 说明                      |
| ------------------ | ----------------------- | ------------------------- |
| `NodeId`           | `tdom::node`            | slotmap 生成的类型安全 ID |
| `NodeArena`        | `tdom::node`            | `SlotMap<NodeId, Node>`   |
| `Node`             | `tdom::node`            | DOM 节点（含子节点/属性） |
| `NodeType`         | `tdom::node`            | 节点类型枚举（28 种标签） |
| `Attr`             | `tdom::attributes`      | HTML 属性存储与查询       |
| `CssAttr`          | `html::css_parse`       | 解析后的 CSS 属性         |
| `CssRule`          | `html::css_parse`       | 完整 CSS 规则表           |
| `CssSelector`      | `html::css_parse`       | CSS 选择器（含组合器）    |
| `FontWeight`       | `html::css_parse`       | `font-weight` 枚举（`Normal` / `Bold`） |
| `FontStyleKind`    | `html::css_parse`       | `font-style` 枚举（`Normal` / `Italic` / `Oblique`） |
| `TextDecoration`   | `html::css_parse`       | `text-decoration` 枚举（`None` / `Underline` / `LineThrough` / `UnderlineLineThrough`） |
| `LayoutFn`         | `layout::mod`           | 布局函数签名              |
| `TermMap`          | `layout::layout`        | 终端尺寸信息              |
| `TermLayout`       | `tui::graph_api`        | 终端画布 + 渲染方法       |
| `Block`            | `tui::graph_api`        | 单个屏幕字符单元格（含 `style: FontStyle`） |
| `FontStyle`        | `tui::graph_api`        | `Block` 的聚合样式位掩码（`BOLD` / `ITALIC` / `UNDERLINED` / `CROSSED_OUT`） |
| `RenderCmd`        | `tui::graph_api`        | 渲染线程命令枚举          |
| `EdgeSizes`        | `layout::box_edges`     | 四边长度（top/right/bottom/left），用于 padding / margin |
| `BorderStyle`      | `layout::box_edges`     | `border-style` 枚举（`None` / `Solid`） |
| `Border`           | `layout::box_edges`     | 单边 border 的 width/style/color |
| `BorderSides`      | `layout::box_edges`     | 四边 `Border` 组合 |
| `BoxEdges`         | `layout::box_edges`     | padding + border + margin 三件套，供布局函数使用 |

### 盒模型在 `CssAttr` 上的访问器

`CssAttr` 实例方法（`self` 是节点的 CSS 属性集合）：

| 方法                              | 返回         | 说明 |
| --------------------------------- | ------------ | ---- |
| `get_padding(containing_width)`   | `EdgeSizes`  | 解析 `padding` / `padding-{top,right,bottom,left}` |
| `get_margin(containing_width)`    | `EdgeSizes`  | 解析 `margin` / `margin-{top,right,bottom,left}`，`auto` → 0 |
| `get_border(containing_width)`    | `BorderSides`| 解析 `border` / `border-{top,right,bottom,left}` / `border-width` / `border-style` / `border-color` |
| `get_box_edges(containing_width)` | `BoxEdges`   | 一次性取 padding + border + margin |

`containing_width` 是父级 content-box 的宽度（用于将来 `auto` / 百分比的支持）。



## 依赖更新约束
- 不要随意修改 `html5ever` / `cssparser` 主版本，它们与 Servo 生态紧密耦合
- 如果需要添加依赖，必须使用cargo add添加最新版本
---
