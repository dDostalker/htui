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

---

## 4. DOM API 一览

脚本层提供浏览器子集的 DOM 绑定（`src/script/dom.rs`）：

**`document`**
- `document.getElementById(id)` → element 或 `null`
- `document.createElement(tag)` → 新元素（尚未挂到文档）

**Element 包装对象**（`getElementById` / `event.target` 返回）
- 属性访问器：`textContent`、`className`、`id`、`parentNode`
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

## 5. 宿主函数（Host Functions）

宿主函数是"JS ↔ Rust 之间的桥"：页面脚本调用它们完成 Rust 侧的工作（系统采样、
IO、复杂计算），并把结果返回给 JS。

```rust
pub type HostFn = Box<dyn FnMut(&mut HostCall) -> JsValue + Send>;

pub struct HostCall<'a> {
    pub args: &'a [JsValue],      // JS 传入的参数
    pub event: Option<&'a HtmlEvent>, // 当前派发中的事件（如有）
    pub render: &'a mut Vec<RenderCmd>, // push 渲染命令（如 Quit）
    pub context: &'a mut Context,  // Boa Context，可构造返回对象
}
```

- 注册：`bindings.host.insert("name".into(), Box::new(|call: &mut HostCall| …))`
- 返回值直接成为 JS 调用结果（可用 `ObjectInitializer` 构造对象、数组）。
- 需要触发重渲染时 push `RenderCmd`（例如 `RenderCmd::Quit`）。
- **不要在宿主函数里做长时间阻塞操作**：宿主函数在输入循环的线程上执行。
  建议重活放后台线程 + `Arc<Mutex<Snapshot>>`，宿主函数只读快照
  （参考 `visual_test/src/sysmon.rs` 的后台采样线程模式）。

### 定时任务（AutoRefresh）

`bindings.auto = Some(AutoRefresh { every, action })`：每 `every` 调用一次
JS 全局函数 `action`（可以是在 `<script>` 里定义的函数，也可以是宿主函数名）。

---

## 6. 渲染集成（RenderArena）

JS 修改 DOM 后，引擎把被改动的节点 id 记入 `dirty`（一组 `NodeId`）；输入循环把引擎的
live arena 通过 `RenderCmd::RenderArena { arena, root_id, scroll_y, zoom, hovered, dirty }`
发给渲染线程。渲染线程：

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

> 注意：`RenderCmd::RenderHtml` 仍保留用于"整文档替换"场景；输入循环遇到它时
> 会交给引擎重新解析并走 `RenderArena` 路径。

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
3. **增量重绘的前提是 `:hover` 未变**：hover 目标一变，整个树都可能换高亮，
   渲染线程检测到 `hovered` 变化即回退全量重绘。

如果仍觉得卡顿，优先检查：**宿主函数是否在输入线程做了慢操作**（重活必须放
后台线程 + 快照发布，参考 `visual_test/src/sysmon.rs`）。

---

## 7. 架构位置（与既有管线的边界）

```
html → tdom → layout → tui    （不可破坏的既有四层）
             ▲
             │ 只读：arena 快照（Geometry）经 RenderArena 从引擎流入
             │
      script 层（旁路）：Boa Context + live NodeArena + 宿主函数
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
  变更标记 dirty、CSS `:hover` 级联（纯内存，不依赖终端）。
- `visual_test/src/events/`：事件演示页（`<script>` + on* + addEventListener +
  DOM 变更计数器）。
- `visual_test/src/sysmon/`：系统监控器——后台采样线程 + 宿主 `sample()`
  返回快照对象 + JS `refresh()` 每 200ms 更新 DOM。

运行：`cargo test --test layout_events_tests`；
可视化：`cargo run -p visual_test -- events` / `cargo run -p visual_test -- sysmon`。
