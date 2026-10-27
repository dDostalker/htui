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
├── Cargo.toml              # workspace 根（members: crates/htui, crates/htui-macros）+ workspace lints + [workspace.dependencies]（依赖版本单一真源，成员用 `<dep> = { workspace = true }` 引用）
├── .editorconfig           # 全库 UTF-8（无 BOM）声明，防止编辑器按 ANSI/GBK 打开产生乱码
├── README.md               # 英文 README（GitHub 首页；与 crates/htui/README.md 内容同步）
├── README.zh-CN.md         # 中文 README
├── LICENSE-MIT             # MIT 许可证文本（cargo package 自动打入 .crate）
├── LICENSE-APACHE          # Apache-2.0 许可证文本（cargo package 自动打入 .crate）
├── .github/
│   └── workflows/
│       └── ci.yml          # GitHub Actions
├── crates/
│   ├── htui/               # 主库 crate：终端 HTML/CSS 渲染引擎（独立发布单元）
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs              # 入口：模块声明 + 公共 re-export（Session / Config / Parser / 宏 / boa_engine）
│   │   │   ├── session.rs          # Session：渲染会话（mem:// 注册表 + Config + EventBindings，render(html/css/js) 主入口）
│   │   │   ├── config.rs           # Config：渲染设置（zoom / events / frame_gap / copy_flow / copy_block / smooth_scroll）
│   │   │   ├── source.rs           # HtmlSource/CssSource/JsSource + include_html!/include_css!/include_js!（debug 携带源路径）
│   │   │   ├── resources.rs        # 资源层：mem:// 内存注册表（前缀可选；debug 注册/删除通知会话重解析），供 <script>/<link>/<img>/fetch
│   │   │   ├── graphics/           # 终端图形原语层（依赖方向：layout → graphics → tui）
│   │   │   │   ├── mod.rs          # 入口：显式 re-export（Block / TermLayout / Img / char_width / ...）
│   │   │   │   ├── canvas.rs       # TermLayout 画布（layout/last_frame/image_cache/input_states）+ InputRenderState
│   │   │   │   ├── cell.rs         # Block / FontStyle（CompactString 单元格模型）
│   │   │   │   ├── image.rs        # Img / ImageCache / render_image_at*（图像解码与半块栅格化）
│   │   │   │   ├── style.rs        # block_style（css_attr → FontStyle/颜色）
│   │   │   │   └── text.rs         # char_width / grapheme_width（Unicode 宽度）
│   │   │   ├── events/             # JS 风格事件系统（旁路，不参与渲染管线）
│   │   │   │   ├── mod.rs          # EventType / HtmlEvent / ElementRef / EventBindings（宿主函数注册表）
│   │   │   │   ├── geometry.rs     # Geometry（arena + boxes 快照，Arc 共享）
│   │   │   │   ├── hit_map.rs      # HitRegion / HitMap / build / hit_test / resolve_by_id
│   │   │   │   └── interaction.rs  # 纯逻辑鼠标交互（HoverState / on_move / on_click，脚本线程内调 ScriptEngine::fire）
│   │   │   ├── script/             # JS 脚本层（旁路：Boa 引擎 + live DOM + 宿主函数）
│   │   │   │   ├── mod.rs          # 入口：mod 声明 + 显式 re-export（ScriptEngine / ScriptState / ScriptMsg / ...）
│   │   │   │   ├── state.rs        # ScriptState（live arena + dirty + timers + fetch 通道）+ TimerEntry + ENGINE_STATE(thread_local) + EngineParts/EngineBuildError
│   │   │   │   ├── engine.rs       # ScriptEngine：new/reload/build_engine/fire/fire_key/dispatch/eval_source/invoke_action + Drop
│   │   │   │   ├── event_loop.rs   # 事件循环半：run_due_timers/poll_fetches/settle_fetch/timer_callback（第二个 impl ScriptEngine 块）
│   │   │   │   ├── bindings.rs     # stop_propagation/prevent_default/register_host_global/dispatch_host/compile_attribute_listeners
│   │   │   │   ├── host.rs         # HostFn / HostCtx（Rust 回调 → JS 可调用）+ host_fn（Boa 无关构造器）+ arg/arg_required/event/render/object 方法
│   │   │   │   ├── convert.rs      # IntoJs/FromJs 转换层 + JsObjectBuilder（Rust ↔ JS 值）+ HostError（Boa 无关错误）
│   │   │   │   ├── msg.rs          # 脚本线程桥：ScriptMsg/ScriptResponse + apply_message + run_script_thread（引擎独占线程）
│   │   │   │   └── dom/          # DOM 绑定（mod.rs = 入口 re-export + 共享助手 with_state/mark_dirty/node_wrapper/...）
│   │   │   │       ├── wrapper.rs       # make_node_wrapper（元素包装属性接线中心）
│   │   │   │       ├── builtins.rs      # install_builtins（全局对象安装）
│   │   │   │       ├── document.rs      # document 工厂/查找（getElementById/createElement/body/head/...）
│   │   │   │       ├── traverse.rs      # 只读导航（parentNode/childNodes/contains/querySelector/...）
│   │   │   │       ├── mutation.rs      # appendChild/insertBefore/replaceChild/removeChild/cloneNode
│   │   │   │       ├── content.rs       # textContent/nodeValue/innerHTML/value/data
│   │   │   │       ├── attrs.rs         # get/set/removeAttribute + id
│   │   │   │       ├── global_attrs.rs  # class/classList/hidden/dataset
│   │   │   │       ├── style.rs         # CSSStyleDeclaration（style_get/set/cssText）
│   │   │   │       ├── events.rs        # add/removeEventListener + 监听器注册表 + focus/blur
│   │   │   │       ├── timers.rs        # setTimeout/setInterval/clear
│   │   │   │       ├── fetch.rs         # native_fetch + response_object
│   │   │   │       └── console.rs       # console.log/warn/error + alert + quit
│   │   │   ├── html/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── html_parse.rs   # HTML → tdom 树（html5ever）；提取 <style> CSS / <script> JS（含 <link rel=stylesheet> 与 <script src> 经 resources 加载）
│   │   │   │   ├── box_edges.rs    # BoxEdges / Border / padding / margin / width
│   │   │   │   └── css_parse/      # CSS 解析
│   │   │   │       ├── mod.rs
│   │   │   │       ├── cascade.rs  # INHERITED_PROPS / SUPPORTED_PROPERTIES / css_into_node()
│   │   │   │       ├── matcher.rs  # selector_chain_matches(+with_hover)
│   │   │   │       ├── selector.rs # CssSelector
│   │   │   │       ├── media.rs    # @media 查询（MediaQueryList/四特征/格数语义）
│   │   │   │       ├── stylesheet.rs # StyleSheet parsing 入口
│   │   │   │       ├── values/     # CSS 值枚举 + 解析函数
│   │   │   │       │   ├── mod.rs  # 入口：mod 声明 + 显式 re-exports（Length / TextAlign / ...）
│   │   │   │       │   ├── text.rs # TextAlign/FontWeight/FontStyle/TextDecoration/TextTransform/LineHeight/WhiteSpace/WordBreak/OverflowWrap
│   │   │   │       │   ├── box_values.rs # BoxSizing/Position/Visibility/InsetSides/Positioning/CornerRadii/CssDisplay
│   │   │   │       │   ├── flex.rs # FlexDirection/FlexWrap/FlexBasis/FlexShorthand/flex-flow/justify·align-*/place 简写/gap/order
│   │   │   │       │   ├── multicol.rs # ColumnFill/columns 简写/column-rule
│   │   │   │       │   ├── color.rs# 颜色解析（hex / rgb / 命名色）
│   │   │   │       │   └── length.rs # 长度体系（Length / FontContext / 简写解析）
│   │   │   │       └── properties/ # CssAttr + AlignSelf
│   │   │   │           ├── mod.rs              # CssAttr（IndexMap + 30+ 个 getter）
│   │   │   │           └── align_self.rs       # AlignSelf
│   │   │   ├── tdom/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── node.rs         # Node / NodeArena / NodeId（+ node/ 子模块）
│   │   │   │   │   └── node/       # 与 node.rs 同模块
│   │   │   │   │       └── node_type.rs # NodeType 枚举（95 变体）+ ALL/tag_name/from_tag_name 映射
│   │   │   │   └── attribute.rs    # Attr（HTML 属性解析与查询）
│   │   │   ├── layout/
│   │   │   │   ├── mod.rs          # DEFAULT_LAYOUT 注册表 + LayoutFn + BoxConstraint
│   │   │   │   ├── layout.rs       # TermMap / TermLayout::draw()（返回 boxes）
│   │   │   │   ├── layoutbox/      # LayoutBox 数据结构（镜像浏览器 CSS Display 三轴分类）
│   │   │   │   │   ├── mod.rs      # LayoutBox struct + 三个轴分类 re-exports
│   │   │   │   │   ├── rect.rs, paint_style.rs, inline_run.rs, cursor.rs, inline.rs, wrap.rs
│   │   │   │   │   ├── position.rs  # CSS 定位后处理（resolve_positioned_rects / pin_fixed_boxes，absolute/fixed 包含块解析）
│   │   │   │   │   ├── generation/  # styled DOM → 逐节点浅分类（geometry 按 DOM 重走）
│   │   │   │   │   │   └── mod.rs  # normal/text/replaced 分类 + visible_children / contents 展平
│   │   │   │   │   ├── geometry/    # GeneratedBox → positioned LayoutBox tree
│   │   │   │   │   │   ├── mod.rs           # BoxKind × OuterDisplay × FormattingContext dispatch
│   │   │   │   │   │   ├── replaced.rs      # layout_replaced（img/input/textarea/button/select）
│   │   │   │   │   │   └── select_overlay.rs # SelectOption/SelectRow + 下拉数据（select_options/overlay_options/...）
│   │   │   │   │   ├── box_kind/   # BoxKind { Normal, Replaced, Anonymous } + ReplacedContent（单文件模块）
│   │   │   │   │   │   └── mod.rs          # 两个枚举 + 分类语义文档
│   │   │   │   │   ├── outer_display/    # OuterDisplay { BlockLevel, Inline }（单文件模块）
│   │   │   │   │   │   └── mod.rs          # enum + from_css_display 映射
│   │   │   │   │   └── inner_formatting_context/  # FormattingContext { Flow, Flex, Table, Grid }
│   │   │   │   │       ├── mod.rs          # enum + FlowExt/FlexExt/GridExt re-exports
│   │   │   │   │       ├── flow/           # Block/Inline Formatting Context 实现
│   │   │   │   │       │   ├── mod.rs              # FlowExt + impl FormattingContext::Flow
│   │   │   │   │       │   ├── tree.rs             # FontContext + layout_tree() 编排 + dispatch + layout_children
│   │   │   │       │   ├── layout_block/       # 共享 layout_block() + OuterGeometry + LayoutBlockOptions
│   │   │   │       │   │   ├── mod.rs          #   re-exports
│   │   │   │       │   │   ├── geometry.rs     #   OuterGeometry + outer-geometry helpers
│   │   │   │       │   │   ├── options.rs      #   LayoutBlockOptions
│   │   │   │       │   │   └── core.rs         #   layout_block() main pipeline
│   │   │   │       │   ├── elements.rs # 标准块元素配置表（div/p/aside/header/footer → LayoutBlockOptions）
│   │   │   │       │   ├── a.rs / generic.rs / head.rs / list.rs / img.rs
│   │   │   │       │   └── table/              # <table> 布局（拆 6 个文件）
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
│   │   │   │   │       │       ├── dispatch.rs     #   layout_flex_dispatch
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
│   │   │   │       ├── button.rs   # paint_button（`[ 文本 ]` 按钮框）
│   │   │   │       ├── image.rs    # paint_image
│   │   │   │       ├── input.rs    # paint_input（<input> 控件：值/光标/选区绘制）
│   │   │   │       ├── select.rs   # paint_select（<select> 控件绘制）
│   │   │   │       ├── textarea.rs # paint_textarea（<textarea> 控件：多行值/光标绘制）
│   │   │   │       └── text.rs     # paint_runs
│   │   │   └── tui/
│   │   │       ├── mod.rs
│   │   │       ├── run/          # 引擎驱动：run()/run_hot() 输入循环 + panic hook（mod.rs）+ 渲染/命中/脚本线程 + 增量重绘辅助
│   │   │           ├── mod.rs     # run()/run_hot() 输入主循环 + install_panic_hook
│   │   │           ├── hot_reload.rs # debug-only HTML/CSS/JS 轮询 watcher（200ms metadata 预检；signature 稳定后发送 source reload）
│   │   │           ├── state.rs     # RenderState + handle_input/handle_select/TextSel/OpenSelect + 配色常量
│   │   │           ├── pipeline.rs  # redraw/layout_and_paint/render_arena/render_html/render_document/repaint_dynamic/pin_fixed_and_repaint
│   │   │           ├── thread.rs    # render_thread / hit_test_thread / fold_script_event / PendingInput
│   │   │           ├── region.rs    # incremental_region / union_rect / collect_box_rects / pseudo_affected_region
│   │   │           ├── selection.rs # selection_lines / lines_bounds / extract_selection / highlight_lines
│   │   │           └── overlay.rs   # apply_select_overlay（select 下拉叠加绘制）
│   │   │       └── graph_api.rs    # 模块入口：re-export graphics（TermLayout / Block / Img / char_width / ...）+ 输入层与渲染器
│   │   │           └── graph_api/  # 子模块目录（与 graph_api.rs 同模块）
│   │   │               ├── render.rs   # TermLayout::render（增量） / render_diff_to_bytes / render_to_bytes
│   │   │               └── input/  # 输入层（mod.rs = InputState + 常量 + re-exports）
│   │   │                   ├── cmd.rs       # RenderCmd（含 RenderArena/RenderDocument）/ DirtyInfo / SelectionMode / DrainedBatch（渲染线程命令合并）
│   │   │                   ├── key_info.rs  # key_event_info / char_code / media·modifier_key
│   │   │                   ├── edit.rs      # 纯编辑（apply_input_key/词边界/Ctrl·Shift 导航/paste）
│   │   │                   ├── field.rs     # 字段分类（is_*_input/textarea 系列/KeyGate/collect_input_ids）
│   │   │                   ├── focus.rs     # focus_input / blur_input / apply_focus_requests
│   │   │                   ├── mouse.rs     # handle_click/mouse_down/mouse_move/mouse_up + label 解析
│   │   │                   ├── select.rs    # OverlayKind / open·close_select / refresh_datalist
│   │   │                   ├── refresh.rs   # trigger_refresh（按键分发状态机）
│   │   │                   ├── scroll_anim.rs # 平滑滚动缓动（ScrollAnim/ScrollTarget/step_target，wheel/翻页键动画）
│   │   │                   └── page.rs      # page_level_builtins（页面级滚动/缩放/复制）
│   │   ├── tests/               # 集成测试（跟随主库）
│   │   │   ├── fixtures/
│   │   │   │   ├── hot_reload.html # include_html! 编译期加载测试夹具
│   │   │   │   ├── hot_reload.css  # include_css! 编译期加载测试夹具
│   │   │   │   └── hot_reload.js   # include_js! 编译期加载测试夹具
│   │   │   ├── html_parse_tests.rs
│   │   │   ├── html_source_tests.rs # include_html/css/js! 与 Source wrapper 测试（编译期内容 + debug 路径，纯内存）
│   │   │   ├── css_parse_tests.rs
│   │   │   ├── dom_node_tests.rs    # DOM 节点 API 测试（insertBefore/replaceChild/cloneNode + Text·Comment·Fragment + document 访问器，纯内存）
│   │   │   ├── resources_tests.rs   # mem:// 资源层测试（注册 / 读取 / 前缀，纯内存）
│   │   │   ├── session_tests.rs     # Session/Config 测试（默认值 / setter / registry 访问，纯内存）
│   │   │   ├── fetch_e2e_tests.rs   # fetch 端到端测试（<script src> 加载 + Promise 微任务事件循环写 DOM，纯内存）
│   │   │   ├── layout_tests.rs
│   │   │   ├── layout_div_tests.rs
│   │   │   ├── layout_span_tests.rs
│   │   │   ├── layout_list_tests.rs
│   │   │   ├── layout_img_tests.rs
│   │   │   ├── layout_aside_tests.rs
│   │   │   ├── layout_header_footer_tests.rs
│   │   │   ├── layout_blockquote_tests.rs # 块级引用测试（UA 默认 margin 上下 1 行/左右 4 列；作者声明逐边覆盖，纯内存）
│   │   │   ├── layout_menu_dl_tests.rs    # menu/dl 测试（menu 同 ul：• 前缀；dl 上下 1 行；dd 左缩 4 列，纯内存）
│   │   │   ├── layout_q_abbr_tests.rs     # q/abbr 测试（q 嵌套引号交替；abbr[title] 下划线，纯内存）
│   │   │   ├── layout_data_time_tests.rs  # data/time 测试（行内透传；value/datetime 仅属性不渲染，纯内存）
│   │   │   ├── layout_box_model_tests.rs
│   │   │   ├── layout_font_size_tests.rs
│   │   │   ├── layout_tree_tests.rs
│   │   │   ├── layout_columns_tests.rs # CSS 多列布局测试（column-count/width/gap/rule/fill）
│   │   │   ├── layout_html_body_tests.rs
│   │   │   ├── layout_flex_tests.rs   # CSS flexbox (display: flex) 布局测试
│   │   │   ├── layout_pipeline_tests.rs # box generation + geometry + mixed end-to-end regression
│   │   │   ├── layout_a_tests.rs
│   │   │   ├── layout_display_tests.rs
│   │   │   ├── layout_em_tests.rs
│   │   │   ├── layout_emoji_tests.rs
│   │   │   ├── layout_table_tests.rs
│   │   │   ├── layout_wrap_tests.rs
│   │   │   ├── layout_wrap_runs_tests.rs # CSS Text 换行包装器直测（各 break policy 精确 run 断言，不经渲染管线，纯内存）
│   │   │   ├── layout_events_tests.rs # 事件系统测试（命中 / 冒泡 / hover / click，纯内存）
│   │   │   ├── layout_focus_tests.rs # el.focus()/el.blur() 测试（请求经输入循环生效 + focus/blur 事件派发，纯内存）
│   │   │   ├── layout_inner_html_tests.rs # innerHTML 测试（读取序列化 / 写入片段解析 / style 提取 / script 跳过，纯内存）
│   │   │   ├── layout_class_list_tests.rs # classList 测试（live DOMTokenList 读/写/迭代/校验，纯内存）
│   │   │   ├── layout_dataset_tests.rs # 全局属性测试（el.hidden 隐藏切换 + el.dataset live DOMStringMap 读/写/删/枚举，纯内存）
│   │   │   ├── layout_position_tests.rs # CSS 定位测试（relative / absolute / fixed + z-index）
│   │   │   ├── layout_input_tests.rs # <input> 输入框测试（渲染 / 编辑逻辑 / 光标状态）
│   │   │   ├── layout_textarea_tests.rs # <textarea> 多行编辑框测试（几何/绘制/编辑/滚动/value IDL，纯内存）
│   │   │   ├── layout_form_tests.rs # 表单测试（hidden/password/checkbox/radio/textarea + label/button/form submit-reset，纯内存）
│   │   │   ├── layout_details_tests.rs # <details>/<summary> 折叠控件测试（关闭只显 summary / 标记 ▸▾ / 点击切换 open + toggle 事件，纯内存）
│   │   │   ├── layout_dialog_template_tests.rs # <dialog>/<template> 测试（dialog：open 显隐 + UA 边框 + form[method=dialog] 关闭 + close 事件；template：内容惰性恒不渲染，纯内存）
│   │   │   ├── layout_form_elements_tests.rs # 表单元素补全测试（output 行内+value / optgroup 分组头 / datalist 自动补全 / fieldset·legend / progress·meter 条，纯内存）
│   │   │   ├── layout_noscript_tests.rs # <noscript> 测试（脚本启用：元素保留 DOM、原始文本解析、恒不渲染，纯内存）
│   │   │   ├── render_diff_tests.rs # 帧差增量渲染（不变零输出 / 滚动 diff / 全帧对比）
│   │   │   ├── macro_tests.rs       # 注册宏测试（commands/host/command(name) 自动注册，纯内存）
│   │   │   ├── query_selector_tests.rs # querySelector/querySelectorAll 测试（document/元素作用域、组合器、无效选择器抛错，纯内存）
│   │   │   └── script_loop_tests.rs # 脚本线程桥测试（apply_message 语义 + 阻塞宿主函数不阻塞输入，纯内存）
│   └── htui-macros/             # 过程宏 crate：#[htui::command]/#[htui::commands] + #[derive(IntoJs/FromJs)]
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs           # proc-macro 入口（rustc 要求宏定义在 crate 根）+ compile_err + 模块声明
│           ├── derive.rs        # #[derive(IntoJs/FromJs)] 展开 + struct_field_names
│           ├── command.rs       # #[htui::command] 展开（纯 fn 包装 HostFn / 有状态 HostFn 直通）+ command_attr 校验 + returns_host_fn
│           └── commands.rs      # #[htui::commands] 展开（register + js_bindings 样本 JS 接口生成，JSDoc）
├── docs/
│   ├── en-us/             # 英文文档（当前仅 todo 占位文件，内容为空）
│   ├── example.png        # 文档配图（终端示例）
│   ├── example-html.png   # 文档配图（浏览器打开 player.html）
│   └── zh-cn/
│       ├── 边界文档.md           # 边界文档（目标 / 核心承诺 / 拒绝清单 / 能力地图）
│       ├── 架构.md               # 渲染管线与线程模型（generation/geometry 分层、四线程职责、合并与安全模型）
│       ├── 能力差异清单.md       # 浏览器能力对比及差异清单（HTML / CSS / JS 全矩阵）
│       └── 教程/
│           ├── 1.HelloWorld.md              # 入门：第一个 htui 程序
│           ├── 2.实现静态终端.md             # 静态渲染（events=false + Config 全字段 + 会话级 CSS）
│           ├── 3.0资源.md                   # 资源层概念（mem:// 注册表、会话隔离、错误语义）
│           ├── 3.1资源的注册和读取.md        # mem:// 注册与读取操作手册（register/load/unregister + fetch）
│           ├── 4.0动态终端.md               # 事件与动态渲染（事件表、内置按键、门控、定时器、脏节点重绘）
│           ├── 4.1JS和Rust绑定.md           # 宿主函数绑定（command/commands/host_fn/derive + 线程模型）
│           ├── 5.日志.md                    # 日志使用说明
│           ├── 6.0示例-实现性能监控器.md      # 示例：性能监控器（占位，待编写）
│           ├── 6.1-示例-实现贪吃蛇.md        # 示例：贪吃蛇（占位，待编写）
│           ├── 6.2调试.md                   # 调试（占位，待编写）
│           └── script-layer.md              # 脚本层完整使用与架构文档（脚本 / on* / 宿主函数 / RenderArena）
├── example/                # 音乐播放器示例（cd example && cargo run；纯界面 + 交互演示，无音频）
│   ├── Cargo.toml          # 空 [workspace] 表脱离根 workspace，可独立运行
│   └── src/
│       ├── main.rs         # 入口：include_html/css/js 并调用 Session::render
│       ├── player.html     # 完整界面 + 交互（标准 HTML/CSS/JS，浏览器可直接打开）
│       ├── player.css      # 会话级终端 CSS 示例
│       └── player.init.js  # 会话级初始 JS 示例
sg/                       # ast-grep 配套目录（sgconfig.yml 在仓库根）
└── rules/                # ast-grep 规则定义（每个 .yml 一条规则）
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
                         generation::generate_node()（逐节点浅分类）
                                    │
                                    ▼
                    Vec<GeneratedBox>（无坐标、单节点分类，不递归子树）
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

`generation` 对 styled DOM 做逐节点浅分类（`generate_node` 不递归子树），
产出不含几何坐标的 `GeneratedBox`（normal/text/replaced 盒语义 +
`OuterDisplay` / `FormattingContext` 分类）。`geometry` 按 DOM 逐层重走、
对每个子节点调用 `generate_node`（每节点每次布局恰好分类一次），再按
`BoxKind`、`OuterDisplay` 和 `FormattingContext` 分派 flow/flex/table
算法并生成统一的 `LayoutBox` 树，最终由 `paint::paint_layout_tree()`
绘制。现有标签级 `LayoutFn` 仅作为 normal flow 内部适配器保留；flex、
table 和 `<img>` 不再通过 `DEFAULT_LAYOUT` 抢占分派。

## 事件系统（旁路，不参与上述管线）

```
渲染线程渲染后 ──► Arc<Geometry>（arena + boxes）──► 命中线程
                                                            │
                          HitMap::build()（收集所有盒子，:hover 可作用于任意元素）
                                                            │
                                              Arc<Mutex<Arc<HitMap>>>（共享）
                                                             │
             ┌───────────────────────────────────────────────┘
             ▼
   输入循环（主线程，永不执行 JS）                脚本线程（独占 ScriptEngine）
   event::poll + 纯 UI 状态机          ScriptMsg    ├─ interaction::on_move/on_click
   （滚动/缩放/焦点/拖选） ────────────────────►     ├─ ScriptEngine::fire（沿 path 触发监听器）
             ▲                                    └─ 宿主函数（可返回值 + push RenderCmd）
             │       ScriptResponse                   │
             └────────────────────────────────────────┤
                cmds / hovered / dirty{arena,root_id} │
                                                     ▼
                       JS 改 DOM → RenderArena{arena} ──► 渲染线程
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

### 脚本线程（引擎与输入解耦）

`ScriptEngine` 独占一个**脚本线程**（`script::msg::run_script_thread`）。输入循环
永不执行 JS：DOM 只读查询走它维护的最新 arena 快照（`ScriptResponse::Event.dirty`），
DOM 写与事件派发打包成 `ScriptMsg`（Move / Click / FocusInput / BlurInput /
InputKeystroke / SelectPick）发给脚本线程，后者经 `apply_message` 统一执行后回
`ScriptResponse::Event { cmds, dirty, hovered }`。hover 状态机（`HoverState`）也在
脚本线程。**宿主函数阻塞只拖住脚本线程**——滚动、退出、鼠标拖选照常（Tauri 式 UI
解耦）；`handle_click` 等输入处理函数只读 DOM 快照、产出 `ScriptMsg`，不碰引擎
（测试用 `apply_message` 在引擎上回放这些消息）。

**已知限制**：输入循环的 DOM 读基于快照，在"JS 改 DOM → `dirty` 响应被消费"
之间有 read-behind 窗口（`focus_input` 种子、blur 提交、node_type 判断可能读到旧
文档，窗口约 0–100ms）。整文档替换（宿主函数 push `RenderCmd::RenderHtml`）
已由脚本引擎的 `ScriptEngine::reload` 在引擎内完成（单一真源），不再有渲染线程
独立解析。

### 动态内容：JS 修改 live DOM 后引擎把被改动的 `NodeId` 记入 `dirty`，脚本线程回
`ScriptResponse::Event`，输入循环据其发送
`RenderCmd::RenderArena { arena, root_id, scroll_y, zoom, hovered, dirty }`——渲染线程
重新级联（保留当前 `:hover`）→ 重排 → 重绘（**仅 dirty 区域，见下**）→ 发布新
Geometry，**无需重新解析 HTML**。整文档替换（宿主函数 push `RenderCmd::RenderHtml`）
由脚本引擎在内部执行 `ScriptEngine::reload`（重新解析 + 跑脚本 + mark root dirty），
随 `RenderArena` 同步——渲染线程不再独立解析。
定时任务：JS 侧 `setTimeout` / `setInterval`（标准 API）由脚本引擎登记到
`ScriptState.timers`，脚本线程在消息间隙经 `recv_timeout` 调度
`ScriptEngine::run_due_timers` 触发回调（监控器/时钟/动画用）。宿主函数在
脚本线程执行，重活应放后台线程（后台采样 + `Arc<Mutex<Snapshot>>` 快照发布，
宿主函数只读快照）。

资源与异步：`crate::resources` 是内存资源层（`mem://` 注册表，前缀可选，不访问
文件系统/网络），`<script src>` / `<link href>` / `<img src>` 与 JS `fetch` 都经
它解析。`fetch` 返回真 `Promise`：注册表读取即时完成，结果经
`ScriptState.fetch_rx` 回传，`ScriptEngine::poll_fetches` 从 GC-rooted 的
`__htuiFetchResolvers` registry 取出 resolver settle promise，再 `run_jobs()`
驱动微任务（`.then` / `await` 链）——这是事件循环的微任务半；
`run_script_thread` 每轮循环都调用它。

## 渲染合并优化（避免无意义的重排）

终端刷新卡顿常见于两处，本项目已内置缓解：

1. **值未变不触发渲染**：`el.textContent = v` 在 `v` 与当前文本相同时直接短路，
   不置 `dirty`——监控器每 200ms 刷新但值基本不变时，实际 `RenderArena`
   次数远少于刷新次数（监控器页面实测 200 次刷新只触发约 14 次渲染）。
2. **命令合并**：合并语义由渲染线程的 `DrainedBatch` 单点实现（`graph_api/input/cmd.rs`）。
   快照命令（`Rerender`/`Redraw`/`RenderArena`/`RenderHtml`/`Quit`）只保留最后一个；
   `RenderInput`/`Select*` 保序全执行；`TextSelection` 延后到快照之后应用；落后于
   pending `RenderArena` 的 `Hover` 延后叠加、`Focus` 合并进 arena 的 `dynamic`——
   DOM 变更与高亮增量都不会丢失。发送前输入循环还会把 batch 最终焦点状态盖写进
   arena 快照（`el.focus()` 在 fold 之后展开，fold 时捕获的 `focused` 可能过时）。
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
    实测（监控器页面）：hover 全量重绘 ~1.6ms → 区域重绘 ~0.6ms。
5. **RenderArena 每批 2 次全树拷贝是架构下限（已论证，勿再尝试 `Arc::try_unwrap`）**：
   拷贝 ① 脚本线程快照（`script/msg.rs` `Arc::new(engine.arena())`）——引擎的 live
   arena 必须保留给后续 JS 执行，无法移交；拷贝 ② 渲染线程级联副本（`tui/run/pipeline.rs`
   `render_arena` 的 `arena.clone()`）——输入循环 `live_dom` 读快照（`run/thread.rs`
   `fold_script_event`）始终同时持有一份 `Arc` 引用（refcount ≥ 2），`try_unwrap` 必然
   失败，且同迭代内 `handle_input` 依赖该快照的新鲜度，不能不持有。已评估的替代方案：
   (a) live_dom 改读渲染线程发布的 `Arc<Geometry>`（`geo.arena`）——可行，稳态可降至
   1 次拷贝，代价是 DOM 读滞后 ≤ 1 帧（~16ms），当前判定不值得牺牲读新鲜度；
   (b) 渲染线程持有常驻工作树 + 仅同步 dirty 节点——**不可行**：slotmap 的 NodeId
   跨 arena 无法对齐（appendChild 新节点在两个 slotmap 中得到不同 key，会破坏
   hover/焦点/监听器等按 NodeId 索引的会话状态）。若未来放宽读新鲜度容忍度，可重开 (a)。

## 必要事项
`html → tdom → layout → tui` 四层不可破坏。不要引入新的抽象层，除非有充分理由。在修改之前进行询问。

新增 HTML 元素支持时，必须先接入 `layoutbox::generation` 的盒分类。仅当元素
需要特殊 normal-flow 几何行为时，才在
`layoutbox/inner_formatting_context/flow/` 增加 `LayoutFn` 并注册到
`DEFAULT_LAYOUT`；flex、table、replaced element 由 geometry 分类分派。
不要用字符串匹配代替 `NodeType` 枚举。新增标签必须：
- 在 `tdom::node::NodeType` 枚举中添加变体，并同步加入 `NodeType::ALL`
- 在 `NodeType::tag_name()` 中登记标签名（`from_tag_name()`、`html_parse::tag_name_to_node_type()`、CSS 选择器类型映射均派生自它，无需重复维护）

通过 `Node.css_attr`（类型 `CssAttr`）访问已解析的 CSS 属性。不要直接解析 style 属性字符串。
使用 `log::debug! / info! / warn! / error!`，不要使用 `println!` 进行调试输出。（ast-grep 规则：`no-println`）
修改代码时不得随意修改已经存在的API的注释，只有实现修改导致内容和实现不符时修改，并询问用户

## 禁止使用git
本项目禁止LLM调用git

## 避免unsafe和添加足够的调试信息支持
本项目不需要 `unsafe` 代码。如需性能优化，优先考虑算法层面。（ast-grep 规则：`no-unsafe`，severity: error）
在必要要的情况下添加调试信息，

## 错误处理
- 返回报错时不得直接unwrap而是处理错误或是返回完整报错信息（ast-grep 规则：`no-unwrap`）
- 不得使用let _ = 等任何形式忽略报错（ast-grep 规则：`no-ignore-result`）
- 禁止使用无意义的Result<>进行包裹信息

## 测试不依赖终端
- 单元测试不应依赖实际的终端环境。如果必须测试终端相关功能，使用条件编译或 mock。
- 完成全新的功能时检查是否存在测试用例，如果不存在或者测试不完全时，添加需要的测试用例在当前目录下test文件中。
- **非必要不新增测试**：修小错误、等价重构、格式调整、注释整理等改动不写新测试；仅当行为变更且既有测试未覆盖时，先询问用户再补最小测试。
- 如果是实现元素支持时，添加到html-test。
- 代码实现文件禁止添加测试。
- 如果时测试专用的api**禁止**混入代码文件中，需要在测试文件中定义

## 代码风格约束

- **注释语言**：公开 API 后续必须使用英文注释。
- **注释克制**：非必要不写注释。仅公开 API 的功能注释（见「API创建」）与确实难以理解的部分需要注释；修小错误、等价重构、格式调整等小改动不加解释性注释（不得写"修复说明"类注释，实现意图由代码与既有注释自说明）。
- **命名规范**：遵循 Rust 标准命名（snake_case 函数/变量，CamelCase 类型，SCREAMING_SNAKE_CASE 常量）。
- **import 风格**：使用 crate 内相对路径引用，不在模块顶部做通配导入。（ast-grep 规则：`no-wildcard-import`）

## API创建
- API创建时必须对功能进行///注释，并且其他位置除难以理解的部分不要注释，必要时可以注释。
- 对于API的共享保持最小共享原则。

## ast-grep 自动检查

以下规则定义在 `sg/rules/` 中，配置入口为仓库根 `sgconfig.yml`。LLM 在修改代码后**必须**运行 `ast-grep scan` 自检，确保不引入违规。

| 规则 ID | 对应条款 | severity | 适用范围 | 说明 |
| ------- | -------- | -------- | -------- | ---- |
| `no-unsafe` | 避免unsafe | error | 全部 | 禁止 `unsafe` 块 / `unsafe fn` / `unsafe impl`；如需 FFI 等正当理由需 PR 说明 |
| `no-unwrap` | 错误处理 | warning | 仅 src | 禁止 `.unwrap()`；改用 `?` / `.expect("说明")` / `match`。不会误报 `.unwrap_or*()` |
| `no-ignore-result` | 错误处理 | warning | 仅 src | 禁止 `let _ = expr` 忽略返回值；改用 `.ok()` / `if let Ok(_) = …` 或加注释 |
| `no-println` | 日志 | warning | 仅 src | 禁止 `println!` / `eprintln!` / `dbg!`；改用 `log::debug!` 等 |
| `no-wildcard-import` | import 风格 | warning | 全部 | 禁止 `use ...::*;` / `pub use ...::*;` 通配导入 |

**测试与基准代码的宽松策略**：`no-unwrap` / `no-ignore-result` / `no-println` 三条规则通过 `ignores` 字段排除了 `**/tests/**`。在这些目录中，`.unwrap()`、`let _ = env_logger::try_init()` 和 `eprintln!` 是标准测试模式，不视为违规。`no-unsafe` 和 `no-wildcard-import` 在所有目录生效。

### 使用方式
```powershell
ast-grep scan                           # 全项目扫描（用 sg/rules/ 全部规则）
ast-grep scan crates/htui/src           # 只扫指定路径
ast-grep scan -r sg/rules/no-unwrap.yml # 只跑某条规则
ast-grep run -p 'PATTERN' --lang rust crates  # 一次性查询（不需规则文件）
```

CI（`.github/workflows/ci.yml`）按**变更路径分区触发**（`dorny/paths-filter`）：
`crates/htui/src/html|tdom` 等改动只跑 HTML/CSS/DOM 组测试，`layout|graphics` 只跑
布局组，`script|events` 与 `htui-macros` 跑脚本/宏组，`tui` 跑渲染/输入组，
`example/**` 只构建示例；fmt / clippy / cargo doc / MSRV / ast-grep 仅在 Rust
代码变更时执行；纯文档改动只跑 UTF-8 编码检查。跨界测试文件（input/textarea/
form 系列、fetch_e2e、font_size）在相关两组中**重复执行**，宁可重复不可漏测。
跨领域根改动（`lib.rs` / `Cargo.toml` / 工作流自身）、手动触发与每周定时任务
（周一 03:00 UTC）触发 3 平台（Linux / Windows / macOS）全量 `cargo test
--workspace` 矩阵作为安全网。新增测试文件时必须把它登记进 ci.yml 对应分区
（跨领域的登记到多组）。本地提交前应至少跑一遍 `cargo fmt`、clippy 与
ast-grep。

### 跳过规则（仅限正当理由）
在违规行**上一行**加注释：
```rust
// ast-grep-ignore-next-line no-unwrap
let x = find(b).unwrap();
```
跳过需在 PR 中说明理由。`no-unsafe` 不允许跳过。

### 添加新规则
1. 在 `sg/rules/` 新建 `<rule-id>.yml`（参考现有规则文件结构）
2. 在本表登记规则 ID 与对应条款

## 添加新 HTML 元素支持 — 操作清单

当你需要支持一个新的 HTML 标签时，
1. **`tdom/node/node_type.rs`** — 在 `NodeType` 枚举中加入变体，并同步加入 `NodeType::ALL`
2. **`tdom/node/node_type.rs`** — 在 `NodeType::tag_name()` 中登记标签名（`from_tag_name` / `html_parse::tag_name_to_node_type` / CSS 选择器类型映射均派生自它，测试 `css_parse_tests::test_e_type_selector_covers_every_element_tag` 锁完备性）
3. **`layout/layoutbox/generation/`** — 添加盒类型与 display/formatting-context 分类
4. **`layout/layoutbox/geometry/`** — 如需专用几何算法，添加分类分派
5. **`layout/layoutbox/inner_formatting_context/flow/`** — 仅特殊 normal-flow 元素新增 `LayoutFn`
6. **`layout/mod.rs`** — 仅 normal-flow 标签适配器加入 `DEFAULT_LAYOUT`


## 关键类型速查
类型在创建前优先查看是否存在需要的类型，在修改创建前进行询问，并**必须**对项目已有的类型进行同步

| 类型               | 位置                    | 说明                      |
| ------------------ | ----------------------- | ------------------------- |
| `Session`          | `session`               | 渲染会话：mem:// 注册表 + `Config` + `EventBindings`，`render()` 主入口 |
| `Config`           | `config`                | 渲染设置（zoom / events / frame_gap / copy_flow / copy_block / smooth_scroll） |
| `NodeId`           | `tdom::node`            | slotmap 生成的类型安全 ID |
| `NodeArena`        | `tdom::node`            | `SlotMap<NodeId, Node>`   |
| `Node`             | `tdom::node`            | DOM 节点（含子节点/属性） |
| `NodeType`         | `tdom::node`            | 节点类型枚举（95 个变体：90 种 HTML 标签 + Document/DocumentType/DocumentFragment/Text/Comment） |
| `Attr`             | `tdom::attribute`      | HTML 属性存储与查询       |
| `CssAttr`          | `html::css_parse::properties` | 解析后的 CSS 属性（`IndexMap<Arc<str>, Arc<str>>` + 30+ 个 getter；键/值共享引用，级联逐属性 O(1) 拷贝） |
| `CssRule`          | `html::css_parse`       | 完整 CSS 规则表           |
| `CssSelector`      | `html::css_parse`       | CSS 选择器（含组合器）    |
| `MediaQueryList`   | `html::css_parse`       | `@media` 查询列表（逗号 OR；值按终端格计，匹配物理视口） |
| `FontWeight`       | `html::css_parse::values` | `font-weight` 枚举（`Normal` / `Bold`） |
| `FontStyleKind`    | `html::css_parse::values` | `font-style` 枚举（`Normal` / `Italic` / `Oblique`） |
| `TextDecoration`   | `html::css_parse::values` | `text-decoration` 枚举（`None` / `Underline` / `LineThrough` / `UnderlineLineThrough`） |
| `LayoutBox`        | `layout::layoutbox`     | 布局树中的单个矩形节点（`BoxKind` × `OuterDisplay` × `FormattingContext` 三轴分类） |
| `BoxKind`          | `layout::layoutbox::box_kind` | 盒子内容形状（`Normal` / `Replaced` / `Anonymous`） |
| `ReplacedContent`  | `layout::layoutbox::box_kind` | `BoxKind::Replaced` 的内容负载（`<img>` 栅格 / `<input>` 标记） |
| `Rect`             | `layout::layoutbox::rect` | 列/行空间内的矩形 |
| `PaintStyle`       | `layout::layoutbox::paint_style` | 视觉 chrome（padding/border/margin + inline runs + 背景/间距标志） |
| `InlineRun`        | `layout::layoutbox::inline_run` | 单个 inline 文本段 |
| `Cursor`           | `layout::layoutbox::cursor` | block 流行游标 |
| `OuterDisplay`     | `layout::layoutbox::outer_display` | 盒子在父级流中的角色（`BlockLevel { container }` / `Inline { block }`） |
| `FormattingContext`| `layout::layoutbox::inner_formatting_context` | 盒子安排子项的格式化上下文（`Flow` / `Flex` / `Grid`） |
| `AlignSelf`        | `html::css_parse::properties::align_self` | `align-self` 解析结果（`Auto` / `Value(AlignItems)`） |
| `LayoutFn`         | `layout::mod`           | 布局函数签名              |
| `TermMap`          | `layout::layout`        | 终端尺寸信息              |
| `TermLayout`       | `graphics`（`tui::graph_api` re-export） | 终端画布 + 渲染方法        |
| `Block`            | `graphics`（`tui::graph_api` re-export） | 单个屏幕字符单元格（含 `style: FontStyle`） |
| `FontStyle`        | `graphics`（`tui::graph_api` re-export） | `Block` 的聚合样式位掩码（`BOLD` / `ITALIC` / `UNDERLINED` / `CROSSED_OUT`） |
| `RenderCmd`        | `tui::graph_api`        | 渲染线程命令枚举（Rerender / Redraw / RenderArena / RenderHtml / Hover / Quit） |
| `DirtyInfo`        | `tui::graph_api`        | `RenderArena` 携带的脏节点集合（`nodes: Vec<NodeId>` + `injected_css: IndexMap<NodeId, String>`，后者按宿主键控、重写即替换，渲染端内容不变则跳过重解析） |
| `InputState`       | `tui::graph_api::input` | 输入循环状态（scroll_y / scroll_offsets / scroll_anim / zoom / pressed / 焦点输入框 / 编辑缓冲 / 选择 / open_select；hover 归脚本线程） |
| `ScrollAnim`       | `tui::graph_api` | 单次缓动滚动 tween（ease-out-cubic，位置按墙钟计算；输入循环持有，输入重定向/手动滚动取消） |
| `ScrollTarget`     | `tui::graph_api` | 滚动动画目标轴（`Page` / `Container(NodeId)`） |
| `InputRenderState` | `graphics::canvas`      | `<input>` 渲染状态（value / cursor / focused），`TermLayout.input_states` |
| `EventType`        | `events`                | DOM 事件类型（Click / MouseOver / MouseOut）               |
| `HtmlEvent`        | `events`                | 事件对象（event_type / target / current_target / x / y / bubbles）|
| `ElementRef`       | `events`                | 冒泡路径上的单个元素（id + node_id）                |
| `HitRegion`        | `events::hit_map`       | 元素命中区域（node_id + rect + path 冒泡链，所有盒子）|
| `HitMap`           | `events::hit_map`       | 全部命中区域 + hit_test() / resolve_by_id() / build()     |
| `EventBindings`    | `events`                | 宿主函数注册表（host: 名字 → HostFn）                     |
| `HoverState`       | `events::interaction`   | 指针悬停跟踪（enter / leave 判定，不产生视觉）；生产环境归脚本线程所有 |
| `ScriptEngine`     | `script`                | Boa 引擎封装：live NodeArena + DOM 绑定 + fire/invoke_action/run_due_timers + eval_source（脚本线程独占） |
| `ScriptMsg`        | `script::msg`           | 脚本线程消息协议：Move / Click / FocusInput / BlurInput / InputKeystroke / SelectPick / Resize / Quit |
| `ScriptResponse`   | `script::msg`           | 脚本线程回复：Ready / Failed / Event{cmds, dirty{arena,root_id}, hovered} |
| `SharedUiState`    | `script`                | 输入循环与脚本线程共享的 UI 状态（cols/rows/scroll_y 原子）；`window.innerWidth` & co 读它 |
| `matchMedia(q)`    | `script::dom::view`     | JS 媒体查询（复用 CSS `@media` 解析）；`change` 事件在 resize 时经 `evaluate_media_changes` 重评估派发 |
| `HostFn`           | `script::host`          | 宿主函数类型：`Box<dyn FnMut(&mut HostCtx) -> JsResult<JsValue> + Send>`（Err → JS throw；由 host_fn/宏生成，勿手写） |
| `HostCtx`          | `script::host`          | Boa 无关调用句柄（`host_fn` 闭包参数）：arg/arg_required/event/render/object，不暴露 Context |
| `host_fn`          | `script::host`          | Boa 无关宿主函数构造器：`host_fn(|ctx| ...) -> HostFn`，用户侧不出现任何 Boa 类型（参数/返回值/对象/错误均纯 Rust） |
| `HostError`        | `script::convert`       | Boa 无关错误类型：`new`/`type_error`/`range_error` + `From<String|&str|JsError>`；`Result<T, HostError>` 映射为 JS 异常 |
| `IntoJs` / `FromJs` | `script::convert`       | Rust ↔ JS 值转换 trait（基础类型 + Option/Vec/Result 实现） |
| `#[htui::command]` | `htui_macros`           | 命令写法之一：普通 `fn(… FromJs 参数) -> IntoJs 返回值` 自动包成 `HostFn`，或零参数 `fn() -> HostFn`（有状态，闭包捕获状态，函数体用 `host_fn` 写），均生成 `<fn>_command()`；`name = "…"` 指定 JS 注册名 |
| `#[htui::commands]` | `htui_macros`          | 模块级自动注册：扫描模块内 `#[htui::command]`，生成 `register(&mut EventBindings)` + `js_bindings()`/`write_js_bindings(path)`（自动生成样本 JS 接口：JSDoc 标注的 function 声明，供代码分析器识别，有状态命令除外）；周期任务由页面 JS `setInterval` 调度 |
| `#[derive(IntoJs/FromJs)]` | `htui_macros`    | 结构体 ↔ JS 对象（`#[js_name = "…"]` 重命名属性） |
| `EdgeSizes`        | `html::box_edges`       | 四边长度（top/right/bottom/left），用于 padding / margin |
| `BorderStyle`      | `html::box_edges`       | `border-style` 枚举（`None` / `Solid`） |
| `Border`           | `html::box_edges`       | 单边 border 的 width/style/color |
| `BorderSides`      | `html::box_edges`       | 四边 `Border` 组合 |
| `BoxEdges`         | `html::box_edges`       | padding + border + margin 三件套，供布局函数使用 |
| `Position`         | `html::css_parse::values` | CSS `position` 枚举（`Static` / `Relative` / `Absolute` / `Fixed`） |
| `InsetSides`       | `html::css_parse::values` | `top`/`right`/`bottom`/`left` 解析结果（`Option<usize>`） |
| `Positioning`      | `html::css_parse::values` | 定位组合（position + inset + z-index），布局期解析后存于 `LayoutBox.positioning` |

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
