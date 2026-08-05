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
├── Cargo.toml              # workspace 根（members: crates/htui, crates/htui-macros, visual_test）
├── crates/
│   ├── htui/               # 主库 crate：终端 HTML/CSS 渲染引擎（独立发布单元）
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs              # 入口：render() / render_with_events() 主循环 + 渲染线程 + 命中线程
│   │   │   ├── logging.rs          # 文件 logger 初始化
│   │   │   ├── events/             # JS 风格事件系统（旁路，不参与渲染管线）
│   │   │   │   ├── mod.rs          # EventType / HtmlEvent / ElementRef / EventBindings（宿主函数注册表）
│   │   │   │   ├── geometry.rs     # Geometry（arena + boxes 快照，Arc 共享）
│   │   │   │   ├── hit_map.rs      # HitRegion / HitMap / build / hit_test / resolve_by_id
│   │   │   │   └── interaction.rs  # 纯逻辑鼠标交互（HoverState / on_move / on_click，调 ScriptEngine::fire）
│   │   │   ├── script/             # JS 脚本层（旁路：Boa 引擎 + live DOM + 宿主函数）
│   │   │   │   ├── mod.rs          # ScriptEngine：Context + ScriptState(thread_local) + 引导脚本 + fire/invoke_action + on* 属性编译
│   │   │   │   ├── host.rs         # HostFn / HostCall（Rust 回调 → JS 可调用）+ arg/arg_required/ret 便捷方法
│   │   │   │   ├── convert.rs      # IntoJs/FromJs 转换层 + js_obj! 宏 + JsObjectBuilder（Rust ↔ JS 值）
│   │   │   │   └── dom.rs          # DOM 绑定：document / element 包装 / style / addEventListener / console / alert / quit
│   │   │   ├── html/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── html_parse.rs   # HTML → tdom 树（html5ever）
│   │   │   │   ├── box_edges.rs    # BoxEdges / Border / padding / margin / width
│   │   │   │   └── css_parse/      # CSS 解析
│   │   │   │       ├── mod.rs
│   │   │   │       ├── cascade.rs  # INHERITED_PROPS / SUPPORTED_PROPERTIES / css_into_node()
│   │   │   │       ├── matcher.rs  # selector_chain_matches(+with_hover)
│   │   │   │       ├── selector.rs # CssSelector
│   │   │   │       ├── stylesheet.rs # StyleSheet parsing 入口
│   │   │   │       ├── values.rs   # CSS 值枚举 + 解析函数（Length / TextAlign / ...）
│   │   │   │       └── properties/ # CssAttr + AlignSelf
│   │   │   │           ├── mod.rs              # CssAttr（IndexMap + 30+ 个 getter）
│   │   │   │           └── align_self.rs       # AlignSelf
│   │   │   ├── tdom/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── node.rs         # NodeType / Node / NodeArena / NodeId
│   │   │   │   └── attribute.rs    # Attr（HTML 属性解析与查询）
│   │   │   ├── layout/
│   │   │   │   ├── mod.rs          # DEFAULT_LAYOUT 注册表 + LayoutFn + BoxConstraint
│   │   │   │   ├── layout.rs       # TermMap / TermLayout::draw()（返回 boxes）
│   │   │   │   ├── layoutbox/      # LayoutBox 数据结构（镜像浏览器 CSS Display 三轴分类）
│   │   │   │   │   ├── mod.rs      # LayoutBox struct + 三个轴分类 re-exports
│   │   │   │   │   ├── rect.rs, paint_style.rs, inline_run.rs, cursor.rs, inline.rs, wrap.rs
│   │   │   │   │   ├── generation/  # styled DOM → geometry-free GeneratedBox tree
│   │   │   │   │   │   └── mod.rs  # normal/text/anonymous/replaced + display classification
│   │   │   │   │   ├── geometry/    # GeneratedBox → positioned LayoutBox tree
│   │   │   │   │   │   └── mod.rs  # BoxKind × OuterDisplay × FormattingContext dispatch
│   │   │   │   │   ├── box_kind/   # BoxKind { Normal, Replaced, Anonymous } + ReplacedContent
│   │   │   │   │   │   ├── mod.rs          # enum + ReplacedContent
│   │   │   │   │   │   ├── normal.rs       # NormalExt trait
│   │   │   │   │   │   ├── replaced.rs     # ReplacedExt trait
│   │   │   │   │   │   └── anonymous.rs    # AnonymousExt trait
│   │   │   │   │   ├── outer_display/    # OuterDisplay { Block, Inline }
│   │   │   │   │   │   ├── mod.rs          # enum
│   │   │   │   │   │   ├── block.rs        # BlockExt trait
│   │   │   │   │   │   └── inline.rs       # InlineExt trait
│   │   │   │   │   └── inner_formatting_context/  # FormattingContext { Flow, Flex, Table, Grid }
│   │   │   │   │       ├── mod.rs          # enum + FlowExt/FlexExt/GridExt re-exports
│   │   │   │   │       ├── flow/           # Block/Inline Formatting Context 实现
│   │   │   │   │       │   ├── mod.rs              # FlowExt + impl FormattingContext::Flow
│   │   │   │   │       │   ├── tree.rs             # FontContext + layout_tree() 编排 + dispatch + layout_children
│   │   │   │   │       │   ├── layout_block/       # 共享 layout_block() + OuterGeometry + LayoutBlockOptions
│   │   │   │   │       │   │   ├── mod.rs          #   re-exports
│   │   │   │   │       │   │   ├── geometry.rs     #   OuterGeometry + outer-geometry helpers
│   │   │   │   │       │   │   ├── options.rs      #   LayoutBlockOptions
│   │   │   │   │       │   │   └── core.rs         #   layout_block() main pipeline
│   │   │   │   │       │   ├── a.rs / aside.rs / body.rs / div.rs / generic.rs / head.rs
│   │   │   │   │       │   ├── header_footer.rs / html.rs / list.rs / p.rs
│   │   │   │   │       │   ├── img.rs
│   │   │   │   │       │   └── table/              # <table> 布局（拆 6 个文件）
│   │   │   │   │       │       ├── mod.rs          #   re-exports
│   │   │   │   │       │       ├── table.rs        #   layout_table orchestrator
│   │   │   │   │       │       ├── row_group.rs    #   layout_row_group pass-through
│   │   │   │   │       │       ├── tr.rs           #   layout_tr pass-through
│   │   │   │   │       │       ├── td.rs           #   layout_td + layout_th pass-through
│   │   │   │   │       │       ├── cell.rs         #   CellInfo + layout_cell
│   │   │   │   │       │       └── measure.rs      #   collect_rows + collect_cells + cell_text_width + compute_column_widths
│   │   │   │   │       ├── flex/           # Flex Formatting Context 实现
│   │   │   │   │       │   ├── mod.rs              # FlexExt + impl FormattingContext::Flex
│   │   │   │   │       │   └── tree/               # 拆 6 个文件
│   │   │   │   │       │       ├── mod.rs          #   re-exports + tree 入口
│   │   │   │   │       │       ├── dispatch.rs     #   layout_flex_dispatch + is_flex_capable_node + is_block_child_node
│   │   │   │   │       │       ├── container.rs    #   layout_flex_container + FlexItemPrep + FLEX_ITEM_MIN_CROSS
│   │   │   │   │       │       ├── items.rs        #   layout_flex_items
│   │   │   │   │       │       ├── prepare.rs      #   prepare_flex_item + measure_text_content_width + natural_cross_axis_size + resolve_item_cross_size
│   │   │   │   │       │       └── algorithm.rs    #   effective_align + wrap_into_lines + distribute_main_axis + justify_spacing + align_content_spacing + align_on_cross_axis
│   │   │   │   │       └── grid/           # Grid Formatting Context（预留）
│   │   │   │   │           └── mod.rs              # GridExt + impl FormattingContext::Grid
│   │   │   │   └── paint/                # 绘制编排（与布局解耦，独立阶段）
│   │   │   │       ├── mod.rs      # paint_layout_tree() 编排器 + PaintCanvas + paint_box()
│   │   │   │       ├── block.rs    # paint_block
│   │   │   │       ├── border.rs   # paint_borders
│   │   │   │       ├── image.rs    # paint_image
│   │   │   │       └── text.rs     # paint_runs
│   │   │   └── tui/
│   │   │       ├── mod.rs
│   │   │       └── graph_api.rs    # 模块入口：re-exports（TermLayout / Block / RenderCmd / Img / ...）
│   │   │           └── graph_api/  # 子模块目录（与 graph_api.rs 同模块）
│   │   │               ├── canvas.rs   # TermLayout 画布
│   │   │               ├── cell.rs     # Block / FontStyle
│   │   │               ├── image.rs    # Img / 图像加载
│   │   │               ├── input.rs    # RenderCmd（含 RenderArena） / InputState / trigger_refresh / 鼠标点击与 hover 跟踪
│   │   │               ├── render.rs   # TermLayout::render（增量） / render_diff_to_bytes / render_to_bytes
│   │   │               ├── style.rs    # block_style
│   │   │               └── text.rs     # char_width / grapheme_width
│   │   ├── tests/               # 集成测试（跟随主库）
│   │   │   ├── html_parse_tests.rs
│   │   │   ├── css_parse_tests.rs
│   │   │   ├── layout_tests.rs
│   │   │   ├── layout_div_tests.rs
│   │   │   ├── layout_span_tests.rs
│   │   │   ├── layout_list_tests.rs
│   │   │   ├── layout_img_tests.rs
│   │   │   ├── layout_aside_tests.rs
│   │   │   ├── layout_header_footer_tests.rs
│   │   │   ├── layout_box_model_tests.rs
│   │   │   ├── layout_font_size_tests.rs
│   │   │   ├── layout_tree_tests.rs
│   │   │   ├── layout_html_body_tests.rs
│   │   │   ├── layout_flex_tests.rs   # CSS flexbox (display: flex) 布局测试
│   │   │   ├── layout_pipeline_tests.rs # box generation + geometry + mixed end-to-end regression
│   │   │   ├── layout_a_tests.rs
│   │   │   ├── layout_display_tests.rs
│   │   │   ├── layout_em_tests.rs
│   │   │   ├── layout_emoji_tests.rs
│   │   │   ├── layout_table_tests.rs
│   │   │   ├── layout_wrap_tests.rs
│   │   │   ├── layout_events_tests.rs # 事件系统测试（命中 / 冒泡 / hover / click，纯内存）
│   │   │   ├── render_diff_tests.rs # 帧差增量渲染（不变零输出 / 滚动 diff / 全帧对比）
│   │   │   └── logging_tests.rs
│   │   └── benches/               # criterion 基准（parse / cascade / paint / render_to_bytes / full_redraw）
│   └── htui-macros/             # 过程宏 crate：#[htui::command] + #[derive(IntoJs/FromJs)]
│       ├── Cargo.toml
│       └── src/lib.rs           # proc-macro：命令包装 / 结构体 ↔ JS 对象 derive
├── docs/
│   └── script-layer.md    # 脚本层完整使用与架构文档（脚本 / on* / 宿主函数 / RenderArena）
├── visual_test/          # 可视化测试工具（cargo run -p visual_test）
│   └── src/
│       ├── main.rs
│       ├── sysmon.rs     # 系统监控器示例逻辑（sysinfo 采样 + 页面重建）
│       ├── all_in_one/   # index.html: 可直接浏览器打开，内部 <style> 内联 CSS
│       ├── headings/
│       ├── image/        # includes pic.png
│       ├── colors/
│       ├── nested/
│       ├── text/
│       ├── css/
│       ├── events/       # 事件系统交互演示（onclick / hover / 动态重渲染，render_with_events）
│       ├── sysmon/       # 系统状态监控器（auto 每 200ms 触发，分层采样 CPU/内存/磁盘并重建页面）
│       ├── header_footer/
│       ├── aside/
│       ├── helloworld/
│       ├── font_styles/  # bold / italic / underline / line-through
│       ├── list/
│       ├── box_model/
│       ├── font_size/
│       ├── flex/
│       ├── flex_align_content/
│       ├── flex_align_self/
│       ├── flex_nested/
│       ├── pre/
│       ├── showcase/
│       ├── table/
│       └── viewport_units/
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
                                    ▼
                         generation::generate_box_tree()
                                    │
                                    ▼
                    Vec<GeneratedBox>（无坐标、统一盒分类树）
                                    │
                                    ▼
                         geometry::layout_generated_tree()
                                    │
                    ┌───────────────┼────────────────┐
                    ▼               ▼                ▼
              flow geometry    flex geometry    table geometry
                    │               │                │
                    └───────────────┼────────────────┘
                                    ▼
                         Vec<LayoutBox>（已定位）
                                    │
                                    ▼
                                    paint::paint_layout_tree()
                                              │
                                              ▼
                            Vec<Block>（画布）
                                    │
                                    ▼
                          TermLayout::render()
                                    │
                                              ▼
                              终端输出（ANSI 转义序列）
```

`generation` 统一从 styled DOM 生成不含几何坐标的 `GeneratedBox` 树，负责
normal/text/anonymous/replaced 盒语义和 `OuterDisplay` / `FormattingContext`
分类。`geometry` 再按 `BoxKind`、`OuterDisplay` 和 `FormattingContext` 分派
flow/flex/table 算法并生成统一的 `LayoutBox` 树，最终由
`paint::paint_layout_tree()` 绘制。现有标签级 `LayoutFn` 仅作为 normal flow
内部适配器保留；flex、table 和 `<img>` 不再通过 `DEFAULT_LAYOUT` 抢占分派。

## 事件系统（旁路，不参与上述管线）

```
渲染线程渲染后 ──► Arc<Geometry>（arena + boxes）──► 命中线程
                                                            │
                          HitMap::build()（收集所有盒子，:hover 可作用于任意元素）
                                                            │
                                              Arc<Mutex<Arc<HitMap>>>（共享）
                                                            ▼
                   主线程 input ── 鼠标 Down/Up/Moved ──► interaction::on_click / on_move
                                                            │
                                    ScriptEngine::fire(region, type, x, y)
                                                            │
                           沿 path 触发 JS 监听器（addEventListener / 编译后的 on* 属性体）
                                                            │
                                     宿主函数（HostFn，可返回值 + push RenderCmd）
                                                            │
                        JS 改 DOM → dom_dirty → RenderArena{arena} ──► 渲染线程
```

事件目标用元素 `node_id`（会话内稳定）传递，命中路径沿 DOM 祖先链冒泡。
`on*` 属性值按浏览器语义编译为 `function (event) { body }`：裸标识符只求值不调用
（与浏览器 `function (event) { buy }` 一致），`buy()` 才调用——脚本层由 Boa 引擎求值
（`src/script/`），未定义/解析失败仅告警不崩溃，对齐浏览器的 ReferenceError
失败模式。脚本通过浏览器语义的 DOM API（`document.getElementById` /
`el.textContent` / `el.style` / `addEventListener` / `appendChild`…）读写文档；
Rust 应用回调以宿主函数（`EventBindings.host` → `HostFn`）形式注册为 JS 全局函数。
hover 驱动 `onmouseover`/`onmouseout` 事件，并在悬停目标变化时发出
`RenderCmd::Hover`，渲染线程克隆缓存 arena 后经 `reset_css_attr` +
`css_into_node_with_hover` 重新级联再重绘——只有样式表写了 `:hover` 规则才产生视觉
高亮，与浏览器一致。

### 动态内容：JS 修改 live DOM 后引擎把被改动的 `NodeId` 记入 `dirty`，输入循环发送
`RenderCmd::RenderArena { arena, root_id, scroll_y, zoom, hovered, dirty }`——渲染线程
重新级联（保留当前 `:hover`）→ 重排 → 重绘（**仅 dirty 区域，见下**）→ 发布新
Geometry，**无需重新解析 HTML**。`RenderCmd::RenderHtml` 仍保留用于整文档替换。
定时任务：`EventBindings::auto`（`AutoRefresh`）让输入循环按间隔经
`ScriptEngine::invoke_action` 自动触发 JS 全局函数（监控器/时钟/动画用）。宿主函数在
输入循环线程执行，重活应放后台线程（参考 `visual_test/src/sysmon.rs` 后台采样 +
快照模式）。

## 渲染合并优化（避免无意义的重排）

终端刷新卡顿常见于两处，本项目已内置缓解：

1. **值未变不触发渲染**：`el.textContent = v` 在 `v` 与当前文本相同时直接短路，
   不置 `dirty`——监控器每 200ms 刷新但值基本不变时，实际 `RenderArena`
   次数远少于刷新次数（sysmon 实测 200 次刷新只触发约 14 次渲染）。
2. **命令合并**：输入循环发送前合并命令流。`RenderArena` 携带 `hovered` 字段，
   重排时会重新应用当前 `:hover` 高亮，因此同轮的 `Hover` 命令会被丢弃
   （渲染线程的 `try_recv` 合并本就只保留最后一个命令）。
3. **增量重绘（Region Repaint）**：引擎侧每个 DOM 写操作（`textContent` /
   `style` / `setAttribute` / `appendChild`…）把 `NodeId` 记入 `ScriptState.dirty`，
   随 `RenderArena` 携带。渲染线程对每个 dirty 节点沿 DOM 找到最近的"有盒子的
   祖先"，若其 rect 在缓存布局与新布局中完全一致（说明改动不涉及几何），就只
   清空并重绘该区域（`TermLayout::repaint_region` + 写门控 `PaintCanvas.region`），
   否则回退全量 `reset_canvas` + 重绘。
4. **Hover 增量**：单独的 `RenderCmd::Hover` 不再全量重绘——只清空并重绘
   **新旧 hover 目标**（含 `:hover` 规则可匹配的祖先）所在区域的并集；样式表
   没有 `:hover` 规则时直接跳过（与浏览器一致，移动指针不产生绘制）。
   `RenderArena` 携带的 hover 变化仍回退全量（内容与高亮同帧变动时最安全）。
   实测 sysmon：hover 全量重绘 ~1.6ms → 区域重绘 ~0.6ms。

## 必要事项
`html → tdom → layout → tui` 四层不可破坏。不要引入新的抽象层，除非有充分理由。在修改之前进行询问。

新增 HTML 元素支持时，必须先接入 `layoutbox::generation` 的盒分类。仅当元素
需要特殊 normal-flow 几何行为时，才在
`layoutbox/inner_formatting_context/flow/` 增加 `LayoutFn` 并注册到
`DEFAULT_LAYOUT`；flex、table、replaced element 由 geometry 分类分派。
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
- 禁止使用无意义的Result<>进行包裹信息

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
3. **`html/css_parse/stylesheet.rs`** — 在 `tag_str_to_node_type()` 中添加 CSS 选择器映射
4. **`layout/layoutbox/generation/`** — 添加盒类型与 display/formatting-context 分类
5. **`layout/layoutbox/geometry/`** — 如需专用几何算法，添加分类分派
6. **`layout/layoutbox/inner_formatting_context/flow/`** — 仅特殊 normal-flow 元素新增 `LayoutFn`
7. **`layout/mod.rs`** — 仅 normal-flow 标签适配器加入 `DEFAULT_LAYOUT`


## 关键类型速查
类型在创建前优先查看是否存在需要的类型，在修改创建前进行询问，并**必须**对项目已有的类型进行同步

| 类型               | 位置                    | 说明                      |
| ------------------ | ----------------------- | ------------------------- |
| `NodeId`           | `tdom::node`            | slotmap 生成的类型安全 ID |
| `NodeArena`        | `tdom::node`            | `SlotMap<NodeId, Node>`   |
| `Node`             | `tdom::node`            | DOM 节点（含子节点/属性） |
| `NodeType`         | `tdom::node`            | 节点类型枚举（30 种标签） |
| `Attr`             | `tdom::attribute`      | HTML 属性存储与查询       |
| `CssAttr`          | `html::css_parse::properties` | 解析后的 CSS 属性（`IndexMap<String, String>` + 30+ 个 getter） |
| `CssRule`          | `html::css_parse`       | 完整 CSS 规则表           |
| `CssSelector`      | `html::css_parse`       | CSS 选择器（含组合器）    |
| `FontWeight`       | `html::css_parse::values` | `font-weight` 枚举（`Normal` / `Bold`） |
| `FontStyleKind`    | `html::css_parse::values` | `font-style` 枚举（`Normal` / `Italic` / `Oblique`） |
| `TextDecoration`   | `html::css_parse::values` | `text-decoration` 枚举（`None` / `Underline` / `LineThrough` / `UnderlineLineThrough`） |
| `LayoutBox`        | `layout::layoutbox`     | 布局树中的单个矩形节点（`BoxKind` × `OuterDisplay` × `FormattingContext` 三轴分类） |
| `BoxKind`          | `layout::layoutbox::box_kind` | 盒子内容形状（`Normal` / `Replaced` / `Anonymous`） |
| `ReplacedContent`  | `layout::layoutbox::box_kind` | `BoxKind::Replaced` 的内容负载（当前只有 `<img>`） |
| `Rect`             | `layout::layoutbox::rect` | 列/行空间内的矩形 |
| `PaintStyle`       | `layout::layoutbox::paint_style` | 视觉 chrome（padding/border/margin + inline runs + 背景/间距标志） |
| `InlineRun`        | `layout::layoutbox::inline_run` | 单个 inline 文本段 |
| `Cursor`           | `layout::layoutbox::cursor` | block 流行游标 |
| `OuterDisplay`     | `layout::layoutbox::outer_display` | 盒子在父级流中的角色（`Block` / `Inline`） |
| `FormattingContext`| `layout::layoutbox::inner_formatting_context` | 盒子安排子项的格式化上下文（`Flow` / `Flex` / `Grid`） |
| `AlignSelf`        | `html::css_parse::properties::align_self` | `align-self` 解析结果（`Auto` / `Value(AlignItems)`） |
| `LayoutFn`         | `layout::mod`           | 布局函数签名              |
| `TermMap`          | `layout::layout`        | 终端尺寸信息              |
| `TermLayout`       | `tui::graph_api`        | 终端画布 + 渲染方法       |
| `Block`            | `tui::graph_api`        | 单个屏幕字符单元格（含 `style: FontStyle`） |
| `FontStyle`        | `tui::graph_api`        | `Block` 的聚合样式位掩码（`BOLD` / `ITALIC` / `UNDERLINED` / `CROSSED_OUT`） |
| `RenderCmd`        | `tui::graph_api`        | 渲染线程命令枚举（Rerender / Redraw / RenderArena / RenderHtml / Hover / Quit） |
| `DirtyInfo`        | `tui::graph_api`        | `RenderArena` 携带的脏节点集合（`nodes: Vec<NodeId>`，JS 本轮改动的节点） |
| `InputState`       | `tui::graph_api::input` | 输入循环状态（scroll_y / zoom / pressed / hover）          |
| `EventType`        | `events`                | DOM 事件类型（Click / MouseOver / MouseOut）               |
| `HtmlEvent`        | `events`                | 事件对象（event_type / target / current_target / x / y / bubbles）|
| `ElementRef`       | `events`                | 冒泡路径上的单个元素（id + node_id）                |
| `HitRegion`        | `events::hit_map`       | 元素命中区域（node_id + rect + path 冒泡链，所有盒子）|
| `HitMap`           | `events::hit_map`       | 全部命中区域 + hit_test() / resolve_by_id() / build()     |
| `EventBindings`    | `events`                | 宿主函数注册表（host: 名字 → HostFn）+ auto 定时刷新      |
| `AutoRefresh`      | `events`                | 周期自动触发的 action（every + JS 全局函数名）            |
| `HoverState`       | `events::interaction`   | 指针悬停跟踪（enter / leave 判定，不产生视觉）        |
| `ScriptEngine`     | `script`                | Boa 引擎封装：live NodeArena + DOM 绑定 + fire/invoke_action + eval_source |
| `HostCall`         | `script::host`          | 宿主函数调用上下文（args / event / render / context）+ arg/arg_required/ret 便捷方法 |
| `HostFn`           | `script::host`          | 宿主函数类型：`Box<dyn FnMut(&mut HostCall) -> JsResult<JsValue> + Send>`（Err → JS throw） |
| `IntoJs` / `FromJs` | `script::convert`       | Rust ↔ JS 值转换 trait（基础类型 + Option/Vec/Result 实现） |
| `js_obj!`          | `script::convert`       | JS 对象字面量宏（`js_obj!(ctx, "k" => v, …)`） |
| `#[htui::command]` | `htui_macros`           | 把 `fn(… FromJs 参数) -> IntoJs 返回值` 包装成 `HostFn`（生成 `<fn>_command()`） |
| `#[derive(IntoJs/FromJs)]` | `htui_macros`    | 结构体 ↔ JS 对象（`#[js_name = "…"]` 重命名属性） |
| `EdgeSizes`        | `html::box_edges`       | 四边长度（top/right/bottom/left），用于 padding / margin |
| `BorderStyle`      | `html::box_edges`       | `border-style` 枚举（`None` / `Solid`） |
| `Border`           | `html::box_edges`       | 单边 border 的 width/style/color |
| `BorderSides`      | `html::box_edges`       | 四边 `Border` 组合 |
| `BoxEdges`         | `html::box_edges`       | padding + border + margin 三件套，供布局函数使用 |

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
