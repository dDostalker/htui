# 示例（codex / yazi / snake）与遇到的问题记录

> 本文档记录用 htui 库搭建三个交互示例时**真实遇到的限制与处理方式**。
> 原则：遇到问题先记录，能用现有能力就做（不为此改库）；确需改库的列为待办。

## 示例清单

| 目录 | 模仿对象 | 交互方式 | 入口 |
| ---- | ---- | ---- | ---- |
| `visual_test/src/codex/` | Codex CLI 聊天界面 | `<input>` + Enter 提交（`change` 事件） | `cargo run -p visual_test -- codex` |
| `visual_test/src/yazi/` + `yazi.rs` | Yazi 文件管理器 | 点击目录进入 / 点击文件选中 / `[ .. ]` 返回 | `cargo run -p visual_test -- yazi` |
| `visual_test/src/snake/` + `snake.rs` | 贪吃蛇 | 方向键 / WASD 转向 + `r` 重开（`<body onkeydown>`）+ 点击方向按钮 + 150ms auto 驱动 | `cargo run -p visual_test -- snake` |

## 记录的问题（按发现顺序）

### 1.【已支持】`<input>` 之外的键盘事件（`keydown`/`keyup`）
- **现象**：Yazi 想用 `j/k` / `↑↓` 移动选中、Snake 想用方向键转向——事件系统只支持
  `click` / `mouseover` / `mouseout` / `focus` / `blur` / `input` / `change`，键盘事件
  只会被聚焦的 `<input>` 消费（作为编辑输入），JS 侧拿不到"按键"。
- **处理**：Yazi 用点击导航；Snake 用点击按钮转向。**当时未改库**。
- **现状（已支持）**：新增 `keydown` / `keyup` 事件类型 + 输入循环把每次按键（含
  auto-repeat）派发为 `keydown`（目标 = 聚焦 `<input>`，否则 `<body>`），事件对象
  携带 `key`/`code`/`keyCode`/修饰键/`repeat`/`defaultPrevented`。`<body
  onkeydown="...">` 或 `el.addEventListener('keydown', ...)` 即可实现 Yazi 的
  `j/k` 导航、Snake 的方向键。**滚动/缩放/退出键带门控**：处理器调
  `event.preventDefault()` 可丢弃对应内置动作（脚本线程忙时 8ms 超时回退内置，
  解耦保证不破，见 `docs/browser-compat.md` §6.5.5）。**Snake 已升级为方向键/WASD
  转向并对方向键 `preventDefault()`**——即使终端超高也不再滚动页面
  （`visual_test/src/snake/index.html`）；Yazi 保持点击导航（其 `j/k` 由页面自行
  绑定并对 `j`/`k` `preventDefault()` 即可同时取消内置滚动）。

### 2.【`<input>` 语义】编辑中 `el.value` 读取滞后
- **现象**：`input` 事件每击键都触发，但 DOM `value` 属性只在**提交**（blur / Enter）时
  写入——JS 在 `input` 事件里读 `el.value` 拿到的是**上次提交值**，不是当前编辑缓冲。
- **处理**：Codex 改用 `change`（提交时触发）读取整条消息 ✓；Snake 放弃"用输入框实时
  转向"的方案，改用按钮。**未改库**。
- **若要支持**：`Input` 事件对象携带当前值，或编辑时同步写 DOM value。

### 3.【DOM API】动态元素的 `on*` 属性不会编译
- **现象**：`onclick` 等属性只在引擎初始化时编译一次；JS 用 `createElement` +
  `appendChild` 动态加入的元素即使写了 `onclick="..."` 也不会有监听器。
- **处理**：三个示例的动态元素统一用 `el.addEventListener(...)`（运行时注册，JS 侧
  生效）✓。**未改库**（与浏览器行为一致：动态属性应走 `addEventListener`）。

### 4.【`<input>` 交互】Enter 提交后输入框失焦
- **现象**：Codex 发送消息按 Enter → 字段 blur（浏览器语义），**没有编程式
  `el.focus()`**，下一条消息必须重新点击输入框。
- **处理**：页面加提示文案。**未改库**。
- **若要支持**：`focus()`/`blur()` 方法或提交后自动保持焦点。

### 5.【交互细节】无双击事件
- **现象**：Yazi 理想是"单击选中、双击进入目录"，但无 `dblclick` 事件。
- **处理**：单击目录即进入、单击文件选中（折中）。**未改库**。

### 6.【工具限制】`--dump` 不执行脚本
- **现象**：headless `--dump` 只做静态布局，不跑 `<script>`——codex 显示初始提示、
  yazi 停留在 "loading…"、snake 显示空白棋盘。
- **处理**：交互行为需真实终端运行；`--dump` 仅验证静态骨架。

## 验证有效的部分

- `<input>` + `change` 提交是可行的"命令行输入"模式（Codex 已用）。
- 宿主函数返回 `#[derive(IntoJs)]` 的 `Vec<结构体>`（`listDir`）→ JS 侧直接是对象
  数组（`e.name` / `e.is_dir` / `e.size`），无需手动序列化 ✓。
- 页面内 `setInterval(tick, 150)` 直接驱动 JS 游戏循环 ✓（`#[htui::commands]`
  仅做命令注册，不再承载定时配置）。
- 动态元素 `addEventListener` + 点击命中正常 ✓；`el.style.backgroundColor` 高亮 ✓。

## 若要"改库"才能更接近原版（待办）

1. ~~非输入键盘事件（`keydown`）→ 解锁 Yazi 键盘导航、Snake 方向键~~ ✅ 已支持
   （`events/mod.rs` + `script/msg.rs` + `tui/graph_api/input.rs`）。
2. `Input` 事件携带当前编辑值 → 解锁"输入框实时命令"。
3. `el.focus()` / 提交后保持焦点 → Codex 连续对话体验。
4. `dblclick` → Yazi 经典"单击选中、双击进入"。

---

## 实测发现的问题（codex 交互验证）

### A.【已修复】Codex 提交后输入框文本留在原地，重新点击才更新
- **现象**：发送消息（Enter → `change`）后，输入框仍显示已发送的文本；重新点击
  （重新聚焦）才清空。
- **根因**：`<input>` 显示值有两个真源——`TermLayout.input_states`（渲染线程缓存，
  `RenderInput` 写）与 DOM `value`（脚本线程 arena）。blur 时输入循环把编辑缓冲
  （"hello"）写进缓存；JS 的 `composer.value = ''` 只改 DOM / arena，没刷缓存——下次
  RenderArena 重绘仍用缓存的旧值。
- **修复（架构，非补丁）**：
  1. `ScriptEngine::set_value` 提交即 **mark dirty**（等值短路）——每次 blur 提交必产出
     `RenderArena`，把脚本线程 arena（含 JS blur 处理器对 `value` 的改写）同步到渲染线程；
  2. `input_states` 收敛为**瞬态**：`TermLayout::layout_only` 每次全新布局丢弃未聚焦条目，
     未聚焦字段一律从 DOM `value` 绘制（显示真源唯一）；未聚焦缓存条目只承担 blur 后
     那一瞬的重绘（防闪旧值）；
  3. 删除原"重布局后刷新缓存"补丁。
  验证：新增回归测试（`layout_drops_unfocused_input_state_but_keeps_focused` /
  `blur_handler_rewrite_reports_the_final_value_in_the_dirty_arena`），全部测试通过。

### B.【已修复】内容溢出视口时无法下滑到底部
- **现象**：消息增多超出视口后，无法用 j / ↓ / PageDown 滚到最底部。
- **已排除**：布局/滚动机制本身正常——headless 诊断验证 `rendered_rows` /
  `max_scroll` 正确、max_scroll 处最后一行（hint）可见。
- **根因（交互层）**：聚焦的 `<input>` 在 `trigger_refresh` 里对**所有按键**直接
  `return cmds`（编辑分支无条件返回），页面导航键被吞。
- **修复（架构，非补丁）**：输入框只消费自有键（编辑 + 光标移动，
  `is_input_field_key`）；未消费的键**回落页面级分支**（浏览器语义：input 不拦截它
  不编辑的键）——聚焦时 ↓ / ↑ / PageDown / PageUp 仍可滚动，Esc 退出编辑后
  j / k 恢复。验证：新增 `input_field_key_classifies_editing_keys` 测试。
- **剩余缺口**：聊天流新增内容不会自动滚到底部（无自动滚底 API，非本次范围）。
