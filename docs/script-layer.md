# htui 脚本层（Script Layer）使用与架构文档

> 本文档描述 htui 的 **JS 脚本层**：如何用 `<script>` 块、`on*` 属性和宿主函数
> 编写交互式页面，以及它在渲染管线中的位置。这是 `onclick="func()"` 直接调用
> Rust 闭包（旧 `EventBindings.actions` / `eval.rs` 方案）的替代品。

---

## 1. 一句话概述

htui 内置了一个 **Boa 引擎**（完整 ECMAScript 引擎），页面可以用 `<script>`
写 JS，通过浏览器语义的 DOM API 访问/修改文档（`document.getElementById`、
`el.textContent`、`el.addEventListener`、`el.style`…），并调用 **Rust 宿主函数**
（注册在 `EventBindings.host` 里的回调）完成重活。JS 对 DOM 的任何修改都会让
渲染线程用 **引擎的 live arena** 重新布局/绘制——**不需要重新解析 HTML**。

```
HTML ──► html_parse（提取 style CSS / script JS 源码）
              │
              ▼
      ScriptEngine（Boa Context + live NodeArena + 宿主函数注册表 + JS 监听器注册表）
              │                 ▲
  会话启动执行 scripts          │ 命中区域 path
  编译 on* 属性为监听器         │ 鼠标事件
              │                 │
              ▼                 │
      JS 改 DOM（textContent/style/appendChild…）──► dom_dirty 标志
              │                                        │
              ▼                                        ▼
      RenderCmd::RenderArena { arena, root_id } ──► 渲染线程：reset_css + 级联
              │                                        + 重排 + 重绘 + 发布 Geometry
              ▼
          HitMap 重建 ──► 新一轮事件命中
```

### 与旧方案的差异

| 维度 | 旧方案（已移除） | 新方案 |
| ---- | ---------------- | ------ |
| 事件来源 | `onclick="func()"` 直接查 `EventBindings.actions`（Rust 闭包） | `<script>` 定义 JS 函数；`on*` 属性编译为 `function(event){…}`；`addEventListener` 注册 |
| 求值器 | 手写 JS 子集 `eval.rs`（仅表达式） | Boa 完整 ECMAScript 引擎 |
| DOM 访问 | 无 | `document` / element 包装对象（`textContent`、`style`、`className`…） |
| 动态重渲染 | `RenderCmd::RenderHtml`（整文档重新解析） | JS 直接改 DOM → `RenderCmd::RenderArena`（**零重解析**） |
| 宿主回调 | `EventHandler: FnMut(&HtmlEvent) -> Option<RenderCmd>` | `HostFn: FnMut(&mut HostCall) -> JsValue`（可返回值给 JS、可 push 渲染命令） |

---

## 2. 快速上手

### 2.1 页面侧（HTML + `<script>`）

```html
<!DOCTYPE html>
<html>
<head><title>Demo</title></head>
<body>
  <script>
    // JS 全局函数：既可直接绑定到 on* 属性，也可 addEventListener
    function buy() {
      // 读取/写入 DOM —— 引擎会自动标记需要重渲染
      document.getElementById('status').textContent = 'bought!';
      document.getElementById('price').style.color = 'green';
    }
    document.getElementById('btn').addEventListener('click', buy);
  </script>

  <p id="status">—</p>
  <p id="price">$10</p>
  <button id="btn" onclick="buy()">Buy</button>
</body>
</html>
```

### 2.2 宿主侧（Rust）

```rust
use htui::events::{AutoRefresh, EventBindings};
use htui::script::HostCall;
use boa_engine::JsValue;

fn main() {
    let mut bindings = EventBindings::default();
    bindings.host.insert(
        "fetchPrice".to_string(),
        Box::new(|call: &mut HostCall| {
            // 读参数：call.args
            let symbol = call.args.get(0).and_then(|v| v.as_string());
            // 返回给 JS 的值（可以是对象）
            JsValue::from(12.5)
        }),
    );
    // 可选：每 200ms 调用一次 JS 全局函数 refresh()
    bindings.auto = Some(AutoRefresh {
        every: std::time::Duration::from_millis(200),
        action: "refresh".to_string(),
    });

    let html = include_str!("page/index.html");
    let css = include_str!("page/style.css");
    htui::render_with_events(html, css, bindings);
}
```

---

## 3. 事件语义（浏览器对齐）

- **`on*` 属性值是代码**：`onclick="buy()"` 编译为 `function (event) { buy() }`，
  与浏览器一致。`onclick="buy"`（无括号）只求值不调用。
- **`addEventListener(type, fn)`**：注册到元素；监听器收到一个 `Event` 对象
  （`type` / `target` / `currentTarget` / `x` / `y` / `bubbles` /
  `stopPropagation()` / `preventDefault()`）。
- **冒泡**：事件沿 DOM 祖先链从最深元素向上冒泡（target-first）。`stopPropagation()`
  可终止。
- **hover**：`onmouseover` / `onmouseout` 在指针进入/离开时触发；CSS `:hover`
  规则通过 `RenderCmd::Hover` 重级联生效（无 `:hover` 规则则无视觉变化）。
- 解析失败 / 未定义函数仅 `log::warn`，不崩溃（对齐浏览器 ReferenceError 语义）。

### 支持的事件类型

| 事件 | 属性 | JS `addEventListener` 类型 |
| ---- | ---- | -------------------------- |
| 点击 | `onclick` | `"click"` |
| 指针进入 | `onmouseover` | `"mouseover"` |
| 指针离开 | `onmouseout` | `"mouseout"` |
| 聚焦（`<input>` 获得键盘焦点） | `onfocus` | `"focus"` |
| 失焦（`<input>` 失去焦点） | `onblur` | `"blur"` |
| 输入（编辑中每次值变化） | `oninput` | `"input"` |
| 提交（失焦/Enter 且值变化） | `onchange` | `"change"` |

`focus` / `blur` / `input` / `change` 目前仅由 `<input>` 字段产生：点击聚焦触发
`focus`（mousedown 时聚焦，对齐浏览器时序），编辑每键触发 `input`，Esc /
Enter / 点击别处 / 焦点切换提交时触发 `blur`（值有变化时额外触发 `change`）。
这些事件沿 DOM 祖先链冒泡，与鼠标事件同一套分发机制。

### 鼠标拖选与复制

- **输入框内拖选**：左键 mousedown 落在 `<input>` 上时聚焦并把光标定位到按下点；
  按住左键拖动时按字符选择（复用 `Shift+←/→` 的选中高亮与 `selection_range` 渲染），
  列→字符映射与绘制共用同一套 caret 居中窗口计算（`value_window`），宽字符
  （CJK/emoji）按显示列处理。松键保留选择；`Ctrl+C` 复制选中字符。
- **左键：页面文本流式选择 + 复制**（对齐浏览器）：mousedown 落在普通内容上时，
  按住左键拖动按**阅读顺序**选中从起点到当前点的连续文本（起始行从 anchor 列到
  行尾、中间行整行、末行到 end 列，反方向拖动对称）。**复制时按 DOM 块结构重建
  文本**（`layout::extract_flow_text`）：同一块内的视觉折行直接拼接成连续字符串
  （换行符只出现在块边界），与浏览器一致——选一段折行段落复制出来是一整段原文。
  宽字符填充格跳过。提取文本写入共享缓冲；`Ctrl+C` / `Ctrl+X` 把缓冲写入会话
  剪贴板，随后可粘贴进 `<input>`（`Ctrl+V`）。
- **右键：矩形框选**（vim `Ctrl+V` / tmux 风格）：右键按下并拖动框选一个列/行
  对齐的矩形，高亮与复制均为矩形内文本（行内去尾空格）。右键不改变输入框焦点。
- 点击别处或按任意非 Ctrl 键清除选择。选择高亮在每次重绘（含 `RenderArena` /
  hover / 滚动）后自动叠加，动态页面（如 sysmon 自动刷新）下选择保持可见。

---

## 4. DOM API 一览

脚本层提供浏览器子集的 DOM 绑定（`src/script/dom.rs`）：

**`document`**
- `document.getElementById(id)` → element 或 `null`
- `document.createElement(tag)` → 新元素（尚未挂到文档）

**Element 包装对象**（`getElementById` / `event.target` 返回）
- 属性访问器：`textContent`、`className`、`id`、`parentNode`、`value`
  （`<input>` 字段的已提交值，可读写；写值会标记重渲染）
- 方法：`getAttribute(name)` / `setAttribute(name, value)` / `removeAttribute(name)`
- `addEventListener(type, fn)` / `removeEventListener(type, fn)`
- `appendChild(child)` / `removeChild(child)`
- `style`：对象，包含 `cssText` 和常用属性访问器（`color`、`backgroundColor`、
  `fontSize`、`fontWeight`、`textAlign`、`display`、`padding`、`margin`、
  `flexDirection` 等，camelCase ↔ kebab-case）

**内置全局函数**
- `console.log` / `console.warn` / `console.error` → 写入 htui 日志
- `alert(msg)` → `log::warn`；`quit()` → 退出渲染循环
- 宿主函数（`EventBindings.host` 的每个条目）→ 对应名字的 JS 全局函数

---

## 5. 命令（Commands，即宿主函数）

命令是"JS ↔ Rust 之间的桥"：页面脚本调用它们完成 Rust 侧的工作（系统采样、
IO、复杂计算），并把结果返回给 JS。写法只有一种：`#[htui::command]`。
（文档有时沿用"宿主函数"一词指代同一个概念；底层类型为 `HostFn` / `HostCall`。）

### 5.1 推荐写法：`#[htui::commands]` + `#[htui::command]`（tauri 式，不碰 Boa）

写一个普通 Rust 函数 + derive，注册后 JS 里直接调用。把命令集中放在一个模块里，
用 `#[htui::commands]` 一次注册全部（连 `bindings.command(...)` 都省掉）：

```rust
use htui::{commands, command, IntoJs, events::EventBindings};

/// 返回给 JS 的结构体：字段自动映射成 JS 对象属性
#[derive(IntoJs)]
struct SampleData {
    host: String,
    #[js_name = "cpuBar"]   // Rust 字段名 ↔ JS 属性名映射
    cpu_bar: String,
    proc: u64,
}

/// 所有宿主函数集中在一个模块，`register()` 由宏生成
#[htui::commands(auto = "refresh", every_ms = 200)]  // 顺带配置定时刷新
mod app {
    use super::*;

    /// 注册为 JS 全局函数 `sample()`；参数 FromJs，返回值 IntoJs
    #[htui::command]
    pub fn sample() -> SampleData { /* … */ }

    /// 带参数 + 错误：`Err` 变成 JS throw，页面 `try/catch` 可捕获
    #[htui::command]
    pub fn set_threshold(value: f64) -> Result<(), String> { /* … */ }

    /// 有状态命令：返回 `HostFn` 闭包（闭包内自行捕获锁、Arc 等状态）
    #[htui::command]
    pub fn snapshot() -> HostFn {
        let shared = Arc::new(Mutex::new(State::default()));
        Box::new(move |call: &mut HostCall| { /* 读 shared，构造返回对象 */ })
    }

    /// 需要重命名的命令：JS 侧叫 `formatDateTime`
    #[htui::command(name = "formatDateTime")]
    pub fn format_datetime(secs: u64) -> String { /* … */ }
}

let mut bindings = EventBindings::default();
app::register(&mut bindings);   // 一条语句注册模块内全部命令 + 定时刷新
```

页面侧：`var d = sample(); d.cpuBar`；出错时 `try { set_threshold(120) } catch (e) { … }`。

- **只有 `#[htui::command]` 一种写法**，注册统一走 `<fn>_command()`：
  - 普通函数（参数 `FromJs`、返回 `IntoJs`）自动包成 `HostFn`；
  - 返回 `HostFn` 的函数（零参数）用于有状态场景，闭包捕获什么由你决定。
- `#[htui::commands]` 扫描模块内的 `#[htui::command]` 函数，生成 `register(&mut
  EventBindings)`：JS 名默认取函数名，`#[htui::command(name = "…")]` 可改名。
  属性上的 `auto = "…"` + `every_ms = N` 会顺带设置 `AutoRefresh`。
- 返回值 `IntoJs` 支持：`String`/`&str`/数值/`bool`/`()`/`Option<T>`/`Vec<T>`/`Result<T, E>`
  （`E: String/&str/JsNativeError`，`Err` 抛异常）以及 `#[derive(IntoJs)]` 结构体。
- 参数 `FromJs` 支持：`String`/`f64`/整数/`bool`/`Option<T>`/`Vec<T>`，以及
  `#[derive(FromJs)]` 结构体（同样支持 `#[js_name]` 属性名映射，如
  `{ maxCpu: 80 }` → `max_cpu` 字段）。
- 两种形态可以混用：`visual_test/src/sysmon.rs` 把纯格式化函数
  （`bar` / `gb` / `format_datetime`）做成 `#[htui::command]`，`sample()`
  命令（返回 `HostFn`）内部直接复用这些纯函数（Rust 侧调用，不走 JS 层），
  页面脚本也可直接 `bar(0.5, 14)` / `formatDateTime(...)` 调用。

### 5.2 手写闭包（有状态 / 高级场景）

```rust
pub type HostFn = Box<dyn FnMut(&mut HostCall) -> JsResult<JsValue> + Send>;

pub struct HostCall<'a> {
    pub args: &'a [JsValue],        // JS 传入的参数
    pub event: Option<&'a HtmlEvent>, // 当前派发中的事件（如有）
    pub render: &'a mut Vec<RenderCmd>, // push 渲染命令（如 Quit）
    pub context: &'a mut Context,   // Boa Context，可构造返回对象
}
```

- 注册：`bindings.host.insert("name".into(), Box::new(|call: &mut HostCall| …))`。
  更推荐把闭包包进零参数函数、用 `#[htui::command]`（返回 `HostFn`）放进
  `#[htui::commands]` 模块统一注册（闭包内的状态自行创建），例如
  `visual_test/src/sysmon.rs` 的 `sample()`。
- `HostFn` 返回 `JsResult<JsValue>`：`Err` 抛 JS 异常，`Ok` 是调用结果。
- 便捷方法：`call.arg::<T>(i)` / `call.arg_required::<T>(i)`（`FromJs` 转换）、
  `call.ret(value)`（`IntoJs` 转换）；构造对象用 `htui::js_obj!(call.context, "k" => v, …)`。
- 需要触发重渲染时 push `RenderCmd`（例如 `RenderCmd::Quit`）。
- **宿主函数在脚本线程上执行**：引擎（Boa Context + live DOM + hover 状态机）独占一个
  脚本线程，输入循环只发 `ScriptMsg`、收 `ScriptResponse`，永不执行 JS。因此宿主函数
  阻塞只会拖住脚本线程——滚动、退出、鼠标拖选照常（Tauri 式 UI 解耦）。但**慢宿主
  函数会延迟后续 JS 派发**（hover/click/auto-refresh 排队等待），所以重活仍建议放
  后台线程 + `Arc<Mutex<Snapshot>>`，宿主函数只读快照
  （参考 `visual_test/src/sysmon.rs` 的后台采样线程模式）。

### 定时任务（AutoRefresh）

`bindings.auto = Some(AutoRefresh { every, action })`：每 `every` 调用一次
JS 全局函数 `action`（可以是在 `<script>` 里定义的函数，也可以是宿主函数名）。
`#[htui::commands(auto = "…", every_ms = N)]` 会自动设置这一项。定时器在**脚本线程**
上跑（消息间隙经 `recv_timeout` 触发），所以即便 auto 的宿主函数较慢，也不冻结
输入与渲染。

---

## 6. 渲染集成（RenderArena）

JS 修改 DOM 后，引擎把被改动的节点 id 记入 `dirty`（一组 `NodeId`）；脚本线程经
`ScriptResponse::Event { cmds, dirty{arena,root_id}, hovered }` 回传，输入循环据此发送
`RenderCmd::RenderArena { arena, root_id, scroll_y, zoom, hovered, dirty }`
给渲染线程。渲染线程：

1. 记录 `hovered` 节点（`RenderArena` 直接携带，无需单独的 `Hover` 命令）；
2. `reset_css_attr` + `css_into_node_with_hover`（保留当前 `:hover`）重新级联；
3. 重新布局（`layout_only`，不做画布重置）；
4. **增量重绘**：若 `dirty` 非空、`:hover` 目标未变、且重排后每个受影响盒子的
   rect 与缓存的布局完全一致，则只清空并重绘这些节点所在的区域（见
   [增量重绘](#增量重绘region-repaint)）；否则回退到全量 `reset_canvas` + 重绘；
5. 发布新 `Geometry` 快照给命中线程 → HitMap 重建。

**好处**：不再像旧 `RenderCmd::RenderHtml` 那样整文档重新解析——CSS 级联/布局
在渲染线程内完成，只有引擎持有的 arena 被克隆发送；且单个 `textContent` 变化
不再触发整张画布的清空与重绘。

> 注意：`RenderCmd::RenderHtml` 仍保留用于"整文档替换"场景——宿主函数 push 后由
> 渲染线程重新解析并渲染（脚本引擎自身的 live arena 不变）。

### 脚本线程（引擎与输入解耦）

`ScriptEngine` 独占一个**脚本线程**（`script::msg::run_script_thread`），输入循环永不
执行 JS：

1. **只读查询走快照**：输入循环维护引擎最新 arena 快照（来自 `Ready` / 每次 `dirty`
   回传），`handle_*` 内部读快照判断 node 类型、value、select 选项——不碰引擎。
2. **写与派发走消息**：聚焦/失焦/输入/select 选择等打包成 `ScriptMsg`（Move / Click /
   FocusInput / BlurInput / InputKeystroke / SelectPick）发给脚本线程，`apply_message`
   统一执行（含 blur→change、mouseout→mouseover 的顺序保证），回 `ScriptResponse`。
3. **hover 状态机在脚本线程**：`interaction::on_move/on_click` 由脚本线程调用，
   `RenderCmd::Hover` 与 `hovered` 经响应回传，`RenderArena` 据此重级联 `:hover`。
4. **宿主函数阻塞不冻结 UI**：滚动、缩放、退出、拖选全在输入循环，宿主函数再慢
   也只拖脚本线程（后续 JS 派发排队）。

**已知限制（read-behind）**：输入循环的 DOM 读取基于快照，在"JS 改动 → `dirty`
响应被输入循环消费"之间会看到旧文档。影响面：`focus_input` 用旧 `value` 种子编辑
缓冲、blur 提交可能覆盖 JS 刚改的值、node_type / select 选项判断同理。窗口约
0–100ms，通常只在"JS 改 `input.value` 后紧跟点击聚焦编辑提交"这类组合触发；需要
严格一致性时可改走消息往返（代价是输入循环等待脚本线程）。`RenderCmd::RenderHtml`
后引擎 live arena 不同步属同一族问题（见 §6 上文注意）。

### 增量重绘（Region Repaint）

JS 每 200ms 刷一次监控器，大部分帧只改几个 `<p>` 的文本。旧实现每帧都
`reset_canvas`（清空 `cols×rows×20` 个 cell）+ 全树重绘，单帧 ~2.1–2.7ms。
增量重绘把这一路径拆开：

1. **引擎侧记账**：每个 DOM 原生写操作（`textContent` / `style` / `setAttribute` /
   `appendChild`…）把对应 `NodeId` 追加进 `ScriptState.dirty`；`take_dirty()`
   一次性取出，随 `RenderArena` 送到渲染线程。
2. **渲染线程判定**：对每个 dirty 节点沿 DOM 向上找到第一个"有盒子的祖先"
   （inline 元素折叠进外层 block 的 runs，因此按祖先盒子的 rect 计算区域）。
   该盒子的 rect 必须在缓存布局与新布局中完全一致——否则说明改动影响了几何
   （换行 / 结构变化），回退全量重绘。
3. **区域重绘**：`TermLayout::repaint_region(arena, boxes, region)` 先清空 region
   内的 cell，再走一次 `paint_layout_tree_region`——每个 cell 写操作通过
   `PaintCanvas.region` 门控，只落 region 内的格子；祖先盒子的背景填充照常经过
   `parent_bg` 链传递，文字不会丢背景。`render_diff` 自然只输出变化的格子。

实测（80×24，60 行文档，debug 构建）：全量 draw ~2.1ms → 增量路径 ~1.1ms，其中
布局 ~0.7ms、区域重绘 ~0.23ms、diff 输出 ~0.2ms。sysmon 页面（~15 个元素）更小。

### 渲染合并优化（避免无意义的重排）

1. **值未变不触发渲染**：`el.textContent = v` 在 `v` 与当前文本相同时直接短路，
   不置 `dirty`。监控器每 200ms 刷新但值基本不变时，实际 `RenderArena`
   次数远少于刷新次数（sysmon 实测 200 次刷新只触发约 14 次渲染）。
2. **命令合并**：输入循环发送前合并命令流。`RenderArena` 携带 `hovered` 字段，
   重排时自动重新应用当前 `:hover` 高亮，因此同轮的 `Hover` 命令会被丢弃；
   渲染线程的 `try_recv` 合并本就只保留最后一个命令，避免重复重绘。
3. **增量重绘的前提是 `:hover` 未变**：`RenderArena` 携带的 hover 目标一旦变化，
   整个树都可能换高亮，渲染线程检测到 `hovered` 变化即回退全量重绘（内容与高亮
   同帧变动时最安全）。**单独的 `Hover` 命令则有自己的增量**：只清空并重绘新旧
   hover 目标（含 `:hover` 可匹配的祖先）所在区域的并集，样式表无 `:hover` 规则
   时直接跳过渲染——实测 hover 从全量 ~1.6ms 降到区域 ~0.6ms，鼠标高亮跟随更跟手。

如果仍觉得卡顿，优先检查：**宿主函数是否做了慢操作**——虽然它只在脚本线程上跑、
不再冻结输入，但会排队延迟后续 JS 派发与 auto-refresh（重活放后台线程 + 快照发布，
参考 `visual_test/src/sysmon.rs`）。

---

## 7. 架构位置（与既有管线的边界）

```
html → tdom → layout → tui    （不可破坏的既有四层）
             ▲
             │ 只读：arena 快照（Geometry）经 RenderArena 从渲染线程发布
             │
      script 层（旁路）：独占脚本线程的 Boa Context + live NodeArena + 宿主函数
```

- **布局/绘制仍是渲染线程的职责**；脚本层不参与布局，只维护 DOM 与监听器。
- 命中检测（`HitMap` / `HitRegion`）不变：所有盒子都成为命中区域（支持任意元素
  的 `:hover`）。
- `events` 模块现在只保留 `EventType` / `HtmlEvent` / `ElementRef` /
  `HitMap` / `HitRegion` / `interaction`（hover 状态机）与 `EventBindings`；
  分发逻辑移到 `script::ScriptEngine::fire`。

---

## 8. 关键实现要点

- **零 `unsafe`**：Boa 的 `NativeFunction::from_fn_ptr` / Copy 闭包捕获
  `&'static str` 属性名（`from_copy_closure_with_captures`）；共享状态放
  `thread_local!`（`Rc<RefCell<ScriptState>>`），native 经 `with_state()` 取用。
- **GC 安全**：JS 监听器与元素包装对象缓存在 JS 侧全局对象
  （`__htuiListeners` / `__htuiElements`），由 Boa GC 统一追踪；Rust 侧不存
  `JsValue`。
- **NodeId 往返**：元素包装对象用隐藏属性 `_nodeId`（f64）保存 node id，
  native 通过 `slotmap::KeyData::from_ffi` 恢复。
- **属性编译**：`ScriptEngine::new` 遍历 arena，把 `on*` 属性体
  `(function (event) { body })` 编译并注册为监听器（与浏览器一致）。

---

## 9. 测试

- `tests/layout_events_tests.rs`：命中区域、`on*`/`addEventListener` 分发、
  冒泡与 `stopPropagation`、hover、宿主函数、`invoke_action`、`textContent`
  变更标记 dirty、CSS `:hover` 级联，以及 `#[htui::command]` /
  `#[derive(IntoJs / FromJs)]` 端到端（含 `#[js_name]` 重命名与
  `Result::Err` → JS throw，纯内存，不依赖终端）。
- `tests/macro_tests.rs`：`#[htui::commands]` 模块自动注册（纯命令 + 有状态
  `HostFn` 返回命令 + auto-refresh）、JS 可调用性，以及
  `#[htui::command(name = "…")]` 与手工注册的兼容（纯内存）。
- `tests/script_loop_tests.rs`：脚本线程桥——`apply_message` 语义（hover 进出顺序 /
  blur→change / select 提交+dirty）、`run_script_thread` 端到端，以及**阻塞宿主
  函数不阻塞输入侧通道**的解耦保证（纯内存）。
- `visual_test/src/events/`：事件演示页（`<script>` + on* + addEventListener +
  DOM 变更计数器）。
- `visual_test/src/sysmon/`：系统监控器——后台采样线程 + 宿主 `sample()`
  返回快照对象 + JS `refresh()` 每 200ms 更新 DOM（宿主函数全部由
  `#[htui::commands]` 模块注册）。

运行：`cargo test --test layout_events_tests` / `cargo test --test macro_tests` /
`cargo test --test script_loop_tests`；
可视化：`cargo run -p visual_test -- events` / `cargo run -p visual_test -- sysmon`。
