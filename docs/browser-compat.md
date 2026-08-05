# htui 与主流浏览器能力对比及差异清单

> 本文件回答两个问题：
> 1. **主流 Web 平台的能力，htui 支持到什么程度**（对比表）；
> 2. **htui 渲染结果与浏览器有哪些不一致**（差异清单，含示例与代码定位）。
>
> 状态图例：✅ 支持 · ◐ 部分/近似 · ✗ 不支持 · — 无对应
> 差异等级：【刻意】设计决定（见 `docs/boundaries.md` §4）·【简化】近似实现 ·
> 【已知】记录在案的 bug 级差异。
>
> 配套阅读：`docs/boundaries.md`（范围）、`docs/web-support.md`（支持清单）、
> `docs/css-js-compat.md`（CSS/JS 逐项明细）。

---

## 1. HTML 元素对比

### 1.1 文档结构

| 标签 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `html` `head` `body` | ✅ | ✅ | 结构骨架 |
| `title` | ✅ | ✅ | 写入终端窗口标题 |
| `meta` | ✅ | ✅ | 解析但忽略属性 |
| `link` | ✅ | ✗ | 不加载外部 CSS/图标 |
| `style` | ✅ | ✅ | `<style>` 内 CSS 参与级联 |
| `script` | ✅ | ✅ | 执行内嵌脚本（Boa） |
| `script src=...` | ✅ | ✗ | 不加载外部脚本 |
| `noscript` | ✅ | ✗ | 无 |

### 1.2 块级与文本

| 标签 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `div` `p` | ✅ | ✅ | |
| `h1`–`h6` | ✅ | ◐ | 仅 h1–h5；默认字号 2em/1.5em/1.17em/1em/0.83em |
| `pre` | ✅ | ✅ | `white-space: pre` |
| `blockquote` | ✅ | ✗ | 无 |
| `hr` | ✅ | ✗ | 无（可用 `border` 模拟） |
| `section` `article` `main` `nav` | ✅ | ✅ | 通用块 |
| `header` `footer` | ✅ | ✅ | 背景色带 |
| `aside` | ✅ | ✅ | 背景色带 |
| `figure` `figcaption` | ✅ | ◐ | 注册，按通用块，无图注对齐语义 |
| `details` `summary` | ✅ | ◐ | 注册，**无折叠展开**（`open` 属性忽略） |

### 1.3 内联文本

| 标签 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `span` | ✅ | ✅ | |
| `em` | ✅ | ✅ | 默认斜体 |
| `a` | ✅ | ✅ | 默认下划线 |
| `strong` `b` | ✅ | ✗ | 用 `font-weight` 代替 |
| `i` `u` `s` `small` | ✅ | ✗ | 用 `font-style/decoration` 代替 |
| `code` `kbd` `samp` `var` `mark` `time` | ✅ | ✗ | 无 |
| `sub` `sup` | ✅ | ✗ | 无 |
| `br` | ✅ | ◐ | 布局换行 ✅；**复制不产生换行符**【已知】 |
| `abbr` `q` `cite` | ✅ | ✗ | 无 |

### 1.4 列表

| 标签 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `ul` `ol` `li` | ✅ | ✅ | marker（`•` / 数字） |
| 嵌套列表 | ✅ | ✅ | 按层级缩进 |
| `dl` `dt` `dd` | ✅ | ✗ | 无 |
| `list-style-type` | ✅ | ◐ | 固定 marker，不解析该属性 |
| `start` `reversed` 属性 | ✅ | ✗ | 序号从 1 开始 |

### 1.5 表格

| 标签 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `table` `caption` `thead` `tbody` `tfoot` `tr` `th` `td` `colgroup` `col` | ✅ | ✅ | 简化表格布局 |
| `colspan` `rowspan` | ✅ | ◐ | 按实现现状 |
| `border` 属性 | ✅ | ◐ | 可通过 CSS 设置 |
| `border-collapse` | ✅ | ✗ | 不解析 |
| `table-layout` | ✅ | ✗ | 固定按内容测量 |

### 1.6 表单控件

| 标签 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `input` | ✅ | ✅ | 唯一完整控件（见 §8 专项） |
| `input type=password/number/...` | ✅ | ✗ | 一律按文本框渲染 |
| `textarea` | ✅ | ◐ | 注册，按通用块，**不可编辑** |
| `select` `option` | ✅ | ◐ | 原生下拉（选项浮层/键盘导航/change）✅；`option` 仅为数据 |
| `button` | ✅ | ◐ | 注册，可挂 `onclick`，无默认提交 |
| `form` | ✅ | ◐ | 注册，无 `submit` 事件 |
| `label` | ✅ | ◐ | 注册，点击不关联聚焦 |
| `fieldset` `legend` | ✅ | ✗ | 无 |

### 1.7 媒体 / 其它

| 标签 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `img` | ✅ | ◐ | 静态光栅到字符格；`src` 读本地文件 |
| `img srcset` `gif` `svg` | ✅ | ✗ | 无 |
| `audio` `video` `canvas` `picture` `iframe` | ✅ | ✗ | 无 |
| `dialog` `menu` `progress` `meter` | ✅ | ✗ | 无 |

### 1.8 全局属性

| 属性 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `id` `class` `style` `title` | ✅ | ✅ | |
| `onclick` `onmouseover` `onfocus` 等 `on*` | ✅ | ✅ | 编译为 `function(event){...}` |
| `data-*` | ✅ | ◐ | `getAttribute` 可读写 |
| `hidden` | ✅ | ✗ | 无 |
| `tabindex` | ✅ | ✗ | 无焦点遍历 |
| `contenteditable` | ✅ | ✗ | 无 |
| `placeholder` `maxlength` `disabled` `readonly` | ✅ | ✗ | 仅 `value`/`width` 生效 |

---

## 2. CSS 对比

### 2.1 选择器

| 能力 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| 类型 `div` | ✅ | ✅ | |
| 类 `.foo` / ID `#foo` | ✅ | ✅ | |
| 后代 ` ` / 子代 `>` / 相邻 `+` / 后续 `~` | ✅ | ✅ | |
| 复合 `.a.b` `div.foo` | ✅ | ✅ | |
| `:hover` | ✅ | ✅ | 唯一动态伪类 |
| `:nth-child(an+b)` | ✅ | ◐ | 仅 `even`/`odd`/整数/`an+b`（无 `-n+3`、无 `n`、无隐式系数） |
| `:first-child` `:last-child` `:not()` `:focus` `:active` | ✅ | ✗ | 解析即报错 |
| `::before` `::after` 伪元素 | ✅ | ✗ | 无 |
| 属性选择器 `[attr]` | ✅ | ✗ | 无 |
| 分组 `a, b` | ✅ | ◐ | 按实现现状 |

### 2.2 盒模型

| 属性 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `width` `height` | ✅ | ✅ | |
| `min-width/max-width/min-height/max-height` | ✅ | ✅ | |
| `padding` + 四边 | ✅ | ✅ | |
| `margin` + 四边、`auto` | ✅ | ◐ | `auto` 仅水平居中 |
| `border` 简写 + `border-width/style/color` + 四边 | ✅ | ◐ | 样式仅 `solid`/`none` |
| `box-sizing` | ✅ | ◐ | 两者可设；**引擎默认 `border-box`**，浏览器默认 `content-box` |
| `border-radius` | ✅ | ✅ | 圆角字符 |
| `outline` | ✅ | ✗ | 无 |
| `box-shadow` | ✅ | ✗ | 无 |

### 2.3 文本

| 属性 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `color` `background-color` | ✅ | ✅ | hex/rgb/命名色 |
| `font-size` | ✅ | ◐ | 按"每字符 N 格"扩展 |
| `font-weight` | ✅ | ◐ | 仅粗/细两态 |
| `font-style` | ✅ | ◐ | 仅斜体开关 |
| `text-decoration` | ✅ | ◐ | underline/line-through 及组合 |
| `text-align` | ✅ | ✅ | left/center/right |
| `text-transform` | ✅ | ✅ | |
| `line-height` | ✅ | ◐ | 行格模型 |
| `white-space` | ✅ | ✅ | normal/nowrap/pre/pre-wrap/pre-line |
| `word-break` | ✅ | ✅ | normal/break-all/keep-all |
| `overflow-wrap` | ✅ | ✅ | normal/anywhere/break-word |
| `letter-spacing` `word-spacing` `text-shadow` `font-family` | ✅ | ✗ | 无 |

### 2.4 布局

| 能力 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `display: block` `inline` | ✅ | ✅ | |
| `display: flex` | ✅ | ✅ | 见 flex 表 |
| `display: table` 族 | ✅ | ✅ | 简化 |
| `display: inline-block` | ✅ | ✅ | |
| `display: list-item` | ✅ | ✅ | |
| `display: grid` `inline-grid` | ✅ | ✗ | 可解析，布局空壳 |
| `display: contents` `none` | ✅ | ◐ | `contents` 支持；`none` 按实现 |
| `position: static/relative/absolute/fixed` | ✅ | ✅ | |
| `position: sticky` | ✅ | ✗ | 无 |
| `top/right/bottom/left`（inset） | ✅ | ✅ | |
| `z-index` | ✅ | ✅ | 层叠排序 |
| `float` `clear` | ✅ | ✗ | 无 |
| 多列 `column-*` | ✅ | ✗ | 无 |
| `overflow: hidden/scroll/auto` | ✅ | ✗ | 块级溢出固定裁剪 |
| `object-fit`（img） | ✅ | ✗ | 无 |

### 2.5 Flexbox（逐属性）

| 属性 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `flex-direction` | ✅ | ✅ | row/row-reverse/column/column-reverse |
| `flex-wrap` | ✅ | ✅ | nowrap/wrap/wrap-reverse |
| `justify-content` | ✅ | ✅ | flex-start/flex-end/center/space-between/space-around/space-evenly |
| `align-items` | ✅ | ◐ | 默认 stretch **为 no-op**【简化】 |
| `align-self` | ✅ | ◐ | 同上 |
| `align-content` | ✅ | ✅ | 多行对齐 |
| `flex-grow` `flex-shrink` | ✅ | ✅ | |
| `flex-basis` | ✅ | ✅ | |
| `flex` 简写 | ✅ | ◐ | 按实现现状 |
| `order` | ✅ | ✅ | |
| `gap`（row-gap/column-gap） | ✅ | ✅ | |

### 2.6 单位 / 值 / 其它

| 能力 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `px` | ✅ | ✅ | 1 格 |
| `em` | ✅ | ◐ | 相对父节点字号（格数） |
| `%` | ✅ | ◐ | 相对包含块宽度 |
| `vw` `vh` | ✅ | ✅ | 相对终端视口 |
| `rem` `ch` `ex` `vmin` `vmax` `calc()` | ✅ | ◐ | `rem` 支持（=em 语义）；`ch`/`ex`/`vmin`/`vmax`/`calc()` 无 |
| `hsl()` `rgba()` `currentColor` 透明色 | ✅ | ✗ | 无 alpha |
| CSS 变量 `var(--x)` | ✅ | ✗ | 无 |
| `!important` | ✅ | ✗ | 无 |
| `@media` 查询 | ✅ | ✗ | 视口 = 终端 |
| `@keyframes` 动画 | ✅ | ✗ | 【刻意】 |
| `transition` `transform` `opacity` | ✅ | ✗ | 【刻意】 |
| `cursor` | ✅ | ✗ | 无 |
| 继承 | ✅ | ◐ | 自定义继承表 |

---

## 3. JavaScript / DOM 对比

### 3.1 语言层

| 能力 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| 变量/函数/闭包/作用域 | ✅ | ✅ | Boa |
| 字符串/模板字符串/正则 | ✅ | ✅ | |
| 数组/对象/Map/Set | ✅ | ✅ | |
| `Math` `Date` `JSON` | ✅ | ✅ | Boa 内建 |
| `try/catch` 异常 | ✅ | ✅ | |
| `async`/`await`/`Promise` | ✅ | ◐ | 语法可解析，**无事件循环驱动** |
| `Symbol` `Proxy` `Reflect` 等进阶 | ✅ | ◐ | 以 Boa 0.21 实现为准 |
| `globalThis` | ✅ | ✅ | |
| `window` | ✅ | ✗ | 未注册（脚本无 `window`） |

### 3.2 全局对象

| API | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `document.getElementById` | ✅ | ✅ | |
| `document.createElement` | ✅ | ✅ | |
| `document.body` / `document.title` | ✅ | ✗ | 无 |
| `document.querySelector(All)` | ✅ | ✗ | 无 |
| `console.log/warn/error` | ✅ | ◐ | 写 htui 日志 |
| `alert` `confirm` `prompt` | ✅ | ◐ | 仅 `alert`（写日志），无交互对话框 |
| `setTimeout` `setInterval` `requestAnimationFrame` | ✅ | ✗ | 用 `EventBindings::auto` |
| `fetch`/`XMLHttpRequest`/`WebSocket` | ✅ | ✗ | 无网络 |
| `localStorage`/`sessionStorage`/`cookie` | ✅ | ✗ | 无 |
| 宿主函数（`#[htui::command]`） | — | ✅ | htui 独有 |

### 3.3 Element 包装对象

| 能力 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `textContent` | ✅ | ✅ | 读写 |
| `className` `id` | ✅ | ✅ | 读写 |
| `parentNode` | ✅ | ✅ | 只读 |
| `value`（input） | ✅ | ✅ | 读写 DOM 提交值 |
| `style.*` `cssText` | ✅ | ✅ | 见 web-support.md |
| `getAttribute/setAttribute/removeAttribute` | ✅ | ✅ | |
| `addEventListener/removeEventListener` | ✅ | ✅ | |
| `appendChild/removeChild` | ✅ | ✅ | |
| `tagName` `nodeType` | ✅ | ✗ | 无 |
| `children` `firstChild` `nextSibling` `parentElement` | ✅ | ✗ | 无 |
| `innerHTML` `outerHTML` | ✅ | ✗ | 无 |
| `classList` `dataset` | ✅ | ✗ | 无 |
| `insertBefore` `replaceChild` `cloneNode` `remove()` | ✅ | ✗ | 无 |
| `focus()` `blur()` `click()` | ✅ | ✗ | 无编程式触发 |
| `getBoundingClientRect` | ✅ | ✗ | 无 |

### 3.4 事件（JS 可监听）

| 事件 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `click` | ✅ | ✅ | |
| `mouseover` `mouseout` | ✅ | ✅ | |
| `mousemove` `mousedown` `mouseup` | ✅ | ✗ | 无（JS 不可见） |
| `focus` `blur` `input` `change` | ✅ | ✅ | 仅 `<input>` 产生 |
| `keydown` `keyup` `keypress` | ✅ | ✗ | 键盘不可编程监听 |
| `wheel` `scroll` `resize` `dblclick` `contextmenu` `submit` | ✅ | ✗ | 无 |

### 3.5 Event 对象

| 能力 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| `type` `target` `currentTarget` `bubbles` | ✅ | ✅ | |
| `x` `y` | ✅ | ◐ | canvas 坐标，非像素坐标 |
| `stopPropagation()` | ✅ | ✅ | |
| `preventDefault()` | ✅ | ◐ | 存在但多数事件无默认行为可阻止 |
| `keyCode` `button` `clientX` 等 | ✅ | ✗ | 无 |

---

## 4. 交互与输入对比

| 能力 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| 鼠标点击 / hover | ✅ | ✅ | |
| 滚轮滚动 | ✅ | ✅ | 每滚 3 行 |
| `Ctrl+滚轮/±/0` 缩放 | ✅ | ◐ | 字号缩放（zoom 系数），非视口缩放 |
| 文本选择 | ✅ | ✅ | 左键流式 + 右键框选 |
| 双击选词 / 三击选行 | ✅ | ✗ | 无 |
| 右键上下文菜单 | ✅ | ✗ | 右键改为框选【刻意】 |
| 系统剪贴板互通 | ✅ | ✗ | 会话内剪贴板【刻意】 |
| 文本编辑（IME/方向键/Home/End/删除） | ✅ | ✅ | `<input>` 内 |
| `Tab` 焦点遍历 | ✅ | ✗ | 焦点仅鼠标点击 |
| 快捷键体系 | ✅ | ◐ | 引擎固定（j/k/方向滚动、q 退出、Ctrl 缩放），脚本不可接管 |
| HTML5 拖放 | ✅ | ✗ | 无 |
| 触摸/触控板 | ✅ | ✗ | 【刻意】 |

---

## 5. 渲染与平台对比

| 能力 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| 渲染模型 | 像素合成 | 字符网格 + ANSI | 本质差异 |
| 字体 | 系统字体/可变/连字/字距 | 单字体宽度表 | |
| 图片 | 静态/动画/矢量 | 静态光栅 | |
| 颜色 | 全色域 + 色彩管理 | 终端色，真彩取决于终端 | |
| 滚动 | 平滑/惯性/嵌套 | 整行滚动 | |
| 视口 | `@media`/视口单位 | 终端行列 + `vw/vh` | |
| 增量渲染 | 合成器逐层 | 单元格 diff + 区域重绘 | 方向一致，粒度不同 |
| 无障碍 | ARIA/朗读 | ✗ | 【刻意】 |
| 打印/分页 | ✅ | ✗ | 【刻意】 |

---

## 6. 不一致清单（含示例与代码定位）

> 每条给出：**差异** → **示例**（浏览器 vs htui）→ **代码定位**。

### 6.1 布局与盒模型

#### 6.1.1 【简化】`align-items: stretch` 不生效
- **浏览器**：flex 容器默认 `align-items: stretch`，把 `height: auto` 的子项拉满交叉轴。
- **htui**：stretch 是 no-op，子项按内容高度。
- **示例**：`<div style="display:flex; height:10"><div>a</div><div>b</div></div>`
  浏览器两个子项都高 10；htui 只高 1（文本行高）。
- **代码**：`layout/layoutbox/inner_formatting_context/flex/tree/`（注释注明 v1 no-op）。

#### 6.1.2 【简化】`margin: auto` 仅水平居中
- **浏览器**：`margin: auto` 在弹性/块布局中可做水平或垂直居中（配合约束）。
- **htui**：仅水平 `auto` 生效（`margin-left:auto` 推到右侧）。
- **示例**：`<div style="width:20; margin:auto">x</div>` 水平居中 ✅；垂直 `auto` 无效。
- **代码**：`html/css_parse/properties/mod.rs` `get_margin`（注释 "centering primitive for margins"）。

#### 6.1.3 【简化】无外边距折叠
- **浏览器**：相邻块级垂直 `margin` 取两者最大值（折叠），父子也可能折叠。
- **htui**：直接累加声明值。
- **示例**：`<div style="margin:5">a</div><div style="margin:5">b</div>`
  浏览器两行间距 5，htui 为 10。
- **代码**：`layout/layoutbox/inner_formatting_context/flow/`（布局推进逻辑）。

#### 6.1.4 【简化】行内格式化上下文近似
- **浏览器**：baseline 对齐、`vertical-align`、行盒内多基线体系。
- **htui**：同一行内 `span`/`a`/`em` 按统一行格放置，无基线/上下标概念。
- **代码**：`layout/layoutbox/inline.rs`（`collect_runs`）+ `layout/paint/text.rs`。

#### 6.1.5 【简化】`width/height: auto` 语义简化
- **浏览器**：块级 `auto` 占满包含块；行内按 shrink-to-fit；`min-content/max-content` 复杂链。
- **htui**：块级固定占满；flex item 走自己的测量链；无 `min/max-content`。
- **代码**：`layout/layoutbox/inner_formatting_context/flow/layout_block/`。

#### 6.1.6 【简化】表格为简化实现
- **浏览器**：`border-collapse`、`table-layout: fixed/auto`、复杂列宽算法。
- **htui**：列宽按内容测量，行高按内容；无折叠边框模式。
- **代码**：`layout/layoutbox/inner_formatting_context/flow/table/`。

#### 6.1.7 【简化】`colspan`/`rowspan` 部分支持
- **浏览器**：任意跨行跨列合并。
- **htui**：按实现现状，跨越多行/多列的单元格行为可能与浏览器不同。
- **代码**：`layout/layoutbox/inner_formatting_context/flow/table/`。

#### 6.1.8 【简化】无 `float`/`clear`、无多列
- **浏览器**：浮动脱离文档流、文字环绕。
- **htui**：文档流外布局只有 `absolute`/`fixed`。
- **代码**：无对应模块（拒绝项）。

#### 6.1.9 【简化】`position: sticky` 不支持
- **浏览器**：相对滚动容器吸顶。
- **htui**：仅 static/relative/absolute/fixed。
- **代码**：`html/css_parse/values` 的 `Position` 枚举（无 Sticky 变体）。

#### 6.1.10 【简化】百分比/高度语义有限
- **浏览器**：`%` 高度相对父级、`min/max-height` 完整百分比链。
- **htui**：`%` 主要相对包含块宽度；高度百分比按 `get_height` 实现。
- **代码**：`html/css_parse/properties/mod.rs` `get_height`。

#### 6.1.11 【简化】`line-height` 行格模型
- **浏览器**：行盒高度由字体度量（ascender/descender）决定。
- **htui**：以"行格"为单位，额外行高按 `glyph_row_within_line` 上 (`line_h-font`)/2 下分配。
- **代码**：`layout/paint/block.rs`（`glyph_row_within_line` 计算）。

#### 6.1.12 【简化】`display: contents` 按实现、`display: none` 语义
- **浏览器**：`contents` 让子元素直接参与父布局；`none` 完全移除。
- **htui**：`contents` 支持（generation 展平）；`none` 按实现现状。
- **代码**：`layout/layoutbox/generation/mod.rs`。

### 6.2 文本与排版

#### 6.2.1 【刻意】字符网格渲染
- **浏览器**：像素级定位、抗锯齿、亚像素。
- **htui**：每字符 1 或 2 格，无亚像素。
- **代码**：`tui/graph_api/text.rs`（`char_width`/`grapheme_width`）。
- **影响**：文字排布整齐但不具备浏览器精细排版。

#### 6.2.2 【简化】字体度量查表
- **浏览器**：使用系统字体实际度量。
- **htui**：内置宽度表（ASCII=1、CJK/emoji=2），`font-family` 无效。
- **代码**：`tui/graph_api/text.rs`。

#### 6.2.3 【简化】`font-size` 是"每字符 N 格"
- **浏览器**：像素字号，字形缩放。
- **htui**：`font-size: 2em` 令每字符占 2 格槽位（glyph 左对齐 + 背景填充）。
- **示例**：`<h1>Hi</h1>` 浏览器是放大的连续字形；htui 是 "H i" 各占 2 格，格间有背景。
- **代码**：`layout/paint/text.rs`（`font_size` 扩展）。

#### 6.2.4 【简化】折行算法近似
- **浏览器**：Unicode UAX #14 + 连字符 + 字距压缩。
- **htui**：贪心按词/grapheme 断行 + `word-break`/`overflow-wrap` 子集；`<br>` 强制断行。
- **代码**：`layout/layoutbox/wrap.rs`。

#### 6.2.5 【已知】`<br>` 复制不换行
- **浏览器**：复制 `a<br>b` 得到 `a\nb`。
- **htui**：流式复制按块边界换行，`<br>` 处**无换行符**（布局正确）。
- **示例**：`<div>a<br>b</div>` 全选复制 → htui 得 `ab`。
- **代码**：`layout/mod.rs` `extract_flow_text`。

#### 6.2.6 【简化】文本选择是单元格级
- **浏览器**：像素级选区 + 自动扩展词/半字符。
- **htui**：流式选择按"行带 + 文本列"，选中整字符（宽字符整体）。
- **代码**：`layout/mod.rs` `selection_text_lines` + `tui/graph_api/input.rs`。

#### 6.2.7 【简化】`white-space` 折叠子集
- **浏览器**：对 NBSP、换行、制表符等多类空白做精确折叠。
- **htui**：`apply_white_space` 覆盖主要规则，非全部空白字符类。
- **代码**：`layout/layoutbox/wrap.rs`。

#### 6.2.8 【简化】`text-align` 是块级行内对齐
- **浏览器**：含 `text-align-last`、`text-justify` 等。
- **htui**：left/center/right 作用于每视觉行。
- **代码**：`layout/paint/text.rs`（`paint_one_line` offset 计算）。

### 6.3 CSS 语义

#### 6.3.1 【简化】继承表自定义
- **浏览器**：完整继承属性集（含 `visibility`、`cursor` 等）。
- **htui**：`INHERITED_PROPS` 子集。
- **代码**：`html/css_parse/cascade.rs`。

#### 6.3.2 【简化】级联优先级
- **浏览器**：来源（user agent/author/important）→ 特异性 → 顺序。
- **htui**：内联 `style` > 选择器规则、同优先级按出现顺序；无 `!important`。
- **代码**：`html/css_parse/cascade.rs`。

#### 6.3.3 【简化】选择器子集
- **浏览器**：完整选择器 Level 4。
- **htui**：`:hover` 唯一动态伪类；`:nth-child(-n+3)`、`:not`、属性选择器解析即报错。
- **示例**：`.item:nth-child(-n+3)` → htui 解析报错（不会静默错选）。
- **代码**：`html/css_parse/selector.rs`。

#### 6.3.4 【简化】颜色子集
- **浏览器**：hex/rgb/hsl/hwb/named/`currentColor`/alpha/渐变。
- **htui**：hex/rgb/命名色，无 alpha、无渐变。
- **代码**：`html/css_parse/values/color.rs`。

#### 6.3.5 【简化】无 `overflow` 属性
- **浏览器**：`overflow: visible/hidden/scroll/auto` 控制块级裁剪与滚动。
- **htui**：内容溢出固定裁剪 + 页面级滚动，无块级滚动容器。
- **代码**：`layout/paint/`（绘制裁剪逻辑）。

#### 6.3.6 【简化】`border-style` 仅 solid/none
- **浏览器**：dashed/dotted/double/groove 等。
- **htui**：非 `solid` 一律不画或按 none。
- **代码**：`html/box_edges.rs`（`BorderStyle`）。

#### 6.3.7 【简化】`em` 基于格数体系
- **浏览器**：`em` 相对父级像素字号。
- **htui**：相对父节点"单元格字号"，数值语义同构但量纲不同。
- **代码**：`html/css_parse/values/length.rs`。

#### 6.3.8 【简化】`display: grid` 空壳
- **浏览器**：完整 grid 布局。
- **htui**：可解析、`FormattingContext::Grid` 存在但无布局算法。
- **代码**：`layout/layoutbox/inner_formatting_context/grid/mod.rs`（预留）。

#### 6.3.9 【简化】无 CSS 变量 / `calc` / `@media`
- **浏览器**：现代 CSS 基础能力。
- **htui**：均不解析。
- **代码**：`html/css_parse/`（无对应解析）。

#### 6.3.10 【刻意】无动画 / 变换 / 透明度
- **浏览器**：`transition`/`animation`/`transform`/`opacity`。
- **htui**：拒绝项，解析无效果。
- **代码**：`html/css_parse/properties/mod.rs`（无 getter）。

### 6.4 JS / DOM 语义

#### 6.4.1 【简化】DOM API 子集
- **浏览器**：完整 DOM 树遍历与操作。
- **htui**：仅 `getElementById`/`createElement`/`textContent`/`style`/`value`/`appendChild` 等。
- **代码**：`script/dom.rs`。

#### 6.4.2 【简化】事件子集
- **浏览器**：键盘/滚轮/滚动/双击/提交等事件。
- **htui**：仅 click/mouseover/mouseout/focus/blur/input/change。
- **示例**：`el.addEventListener('keydown', f)` 静默不触发（未注册类型）。
- **代码**：`events/mod.rs`（`EventType`）。

#### 6.4.3 【简化】无定时器 / 无异步驱动
- **浏览器**：`setInterval` 等驱动动态页面。
- **htui**：脚本无法自驱动；用 `EventBindings::auto`（Rust 侧）周期性 `invoke_action`。
- **示例**：`setInterval(() => {...}, 1000)` → `ReferenceError: setInterval is not defined`。
- **代码**：`script/mod.rs` + `tui/run.rs`（auto 触发）。

#### 6.4.4 【简化】`Promise`/`async` 语法存在但无事件循环
- **浏览器**：完整异步运行时。
- **htui**：Boa 可解析 `async` 语法，但没有任何 microtask 驱动点，`await` 实际无法完成。
- **代码**：`script/mod.rs`（同步 `fire`/`invoke_action`）。

#### 6.4.5 【简化】事件时序近似
- **浏览器**：完整 `mousedown→mouseup→click` + `focus` 精确交错。
- **htui**：mousedown 聚焦、mouseup 同格触发 click；主序列对齐，细节有差（见 §7）。
- **代码**：`tui/graph_api/input.rs`（`handle_mouse_down`/`handle_click`）。

#### 6.4.6 【已知】`document` 全局极小
- **浏览器**：`document.body`、`document.title` 等。
- **htui**：仅 `getElementById`/`createElement`。
- **代码**：`script/dom.rs`（`install_builtins`）。

#### 6.4.7 【简化】`preventDefault()` 无实质效果
- **浏览器**：可阻止链接跳转、表单提交等默认行为。
- **htui**：存在该方法，但事件没有可阻止的默认行为（链接不导航、无提交）。
- **代码**：`script/mod.rs`（事件对象构建）。

#### 6.4.8 【简化】`on*` 属性事件名子集
- **浏览器**：`onkeydown`、`onwheel` 等全部事件属性。
- **htui**：仅支持 7 个事件对应属性，其他 `on*` 不编译。
- **代码**：`script/mod.rs`（`on*` 编译）。

#### 6.4.9 【简化】脚本与宿主函数边界
- **浏览器**：无（浏览器脚本只能调 Web API）。
- **htui**：宿主函数在输入循环线程同步执行，重活需自行放后台线程（sysmon 模式）。
- **代码**：`script/host.rs` + `visual_test/src/sysmon.rs`。

#### 6.4.10 【简化】`el.value` 语义仅限 `<input>`
- **浏览器**：`value` 还适用于 `<textarea>`/`<select>`/`<option>`。
- **htui**：非 input 元素读写 `value` 无效果。
- **代码**：`script/dom.rs`（`native_get_value`）。

### 6.5 交互与输入

#### 6.5.1 【刻意】剪贴板会话内
- **浏览器**：`Ctrl+C/V` 走系统剪贴板。
- **htui**：仅会话内（`InputState.input_clipboard`）；退出即失。
- **示例**：htui 中复制 → 切到浏览器 `Ctrl+V` 无内容。
- **代码**：`tui/graph_api/input.rs`（`apply_ctrl_shortcut`）。

#### 6.5.2 【刻意】右键 = 框选
- **浏览器**：右键上下文菜单。
- **htui**：右键按下拖动 = 矩形框选（vim 块选）。
- **代码**：`tui/graph_api/input.rs`（`handle_mouse_down` Right 分支）。

#### 6.5.3 【简化】无双击选词 / 三击选行
- **浏览器**：双击按词扩展选区。
- **htui**：双击等同两次点击（无扩展）。
- **代码**：`tui/graph_api/input.rs`（无 dblclick 处理）。

#### 6.5.4 【简化】无 `Tab` 焦点遍历
- **浏览器**：`Tab` 在可聚焦元素间移动 + `:focus-visible`。
- **htui**：焦点仅鼠标点击 input；`Tab` 无效果。
- **代码**：`tui/graph_api/input.rs`（Key 分支）。

#### 6.5.5 【简化】键盘不可编程监听
- **浏览器**：页面可监听并拦截键盘。
- **htui**：方向键/`j`/`k` 滚动、`q` 退出、`Ctrl` 缩放为引擎固定行为；脚本不可接管。
- **代码**：`tui/graph_api/input.rs`（`trigger_refresh` Key 分支）。

#### 6.5.6 【简化】滚动整行步进
- **浏览器**：平滑/惯性/像素级。
- **htui**：滚轮 3 行/次、PageUp/Down 按页、无动画。
- **代码**：`tui/graph_api/input.rs`（ScrollUp/Down 分支）。

#### 6.5.7 【简化】缩放是字号缩放
- **浏览器**：视口缩放（像素放大）。
- **htui**：`zoom` 系数参与布局字号与列宽计算。
- **代码**：`tui/graph_api/canvas.rs`（`effective_columns`）。

#### 6.5.8 【简化】`Ctrl+A` 全选仅限聚焦 input
- **浏览器**：`Ctrl+A` 选中整个文档。
- **htui**：聚焦 input 时全选字段内容；页面级无全选。
- **代码**：`tui/graph_api/input.rs`（`apply_ctrl_shortcut`）。

#### 6.5.9 【简化】`Esc`/`Enter` 语义
- **浏览器**：`Esc` 取消、`Enter` 提交表单（trigger `submit`）。
- **htui**：`Esc`/`Enter` 均触发 input 失焦（blur+change），无 `submit`。
- **代码**：`tui/graph_api/input.rs`（Key 分支）。

#### 6.5.10 【简化】无编程式焦点/事件触发
- **浏览器**：`el.focus()`、`el.click()`、`dispatchEvent`。
- **htui**：均无。
- **代码**：`script/dom.rs`（Element 方法集）。

### 6.6 渲染与平台

#### 6.6.1 【刻意】像素能力缺失
- **浏览器**：canvas/svg/合成/动画。
- **htui**：全部字符 + ANSI。
- **代码**：`layout/paint/`。

#### 6.6.2 【简化】图片静态光栅
- **浏览器**：gif 动画、svg 矢量、缩放滤镜。
- **htui**：`<img>` 按目标格数解码静态位图。
- **代码**：`tui/graph_api/image.rs` + `layout/paint/image.rs`。

#### 6.6.3 【简化】颜色取决于终端
- **浏览器**：色彩管理。
- **htui**：输出 ANSI 颜色，真彩支持取决于终端。
- **代码**：`tui/graph_api/style.rs`。

#### 6.6.4 【简化】视口 = 终端行列
- **浏览器**：独立视口 + 滚动条。
- **htui**：视口即终端尺寸，`resize` 事件驱动重排。
- **代码**：`tui/run.rs`（Resize 分支）。

#### 6.6.5 【简化】增量渲染粒度
- **浏览器**：合成层局部重绘。
- **htui**：单元格 diff（按 run 批量）+ 区域重绘 + hover 增量。
- **代码**：`tui/graph_api/render.rs` + `layout/layout.rs`。

#### 6.6.6 【简化】终端标题/光标管理
- **浏览器**：标签页标题、页面光标。
- **htui**：`<title>` 写终端窗口标题；光标由 crossterm 管理（可隐藏/定位）。
- **代码**：`tui/run.rs`（`extract_title`）。

---

## 7. 事件时序对比（场景表）

| 场景 | 浏览器序列 | htui 序列 | 差异 |
| ---- | ---------- | --------- | ---- |
| 点击空白处（input 已聚焦） | `mousedown → blur(旧) → mouseup → click(空白)` | `mousedown(blur 旧) → mouseup → click(空白)` | 主序列一致 |
| 点击另一个 input | `mousedown → blur(旧) → focus(新) → mouseup → click(新)` | `mousedown(blur 旧 + focus 新) → mouseup → click(新)` | 一致 |
| 点击 input 内 | `mousedown → focus → mouseup → click` | `mousedown(focus) → mouseup → click` | 一致 |
| 输入字符 | `keydown → beforeinput → input → keyup` | `key → input`（每键） | 无 keydown/keyup/beforeinput |
| `Enter` | `keydown → submit/change → keyup` | `blur + change` | 无 submit |
| `Esc` | `keydown → 取消` | `blur + change` | 语义不同（htui 也提交） |
| hover 进入 | `mouseover → (CSS :hover 生效)` | `mouseover → Hover 重级联` | 一致 |
| 拖选 | `mousedown → mousemove… → mouseup`（选区实时） | `mousedown → Drag… → mouseup` | 一致（内部用 Drag 事件） |

## 8. `<input>` 控件与浏览器 `<input>` 的差异

| 能力 | 浏览器 | htui | 说明 |
| ---- | :----: | :---: | ---- |
| 渲染 | 系统控件/原生框 | `[value]` 字符框 | |
| 宽度 | 由 `size`（默认 ~20 字符）决定、固定 | 由 `width` 属性（默认 8 格）决定、固定 | 均不随内容变宽 |
| `type` 属性 | password/number/checkbox… | 一律文本 | |
| `placeholder` | 空值时灰字提示 | ✗ | 无 |
| `maxlength` | 限制输入长度 | ✗ | 不限制 |
| `readonly`/`disabled` | 不可编辑/不可交互 | ✗ | 可编辑 |
| 聚焦 | 点击/`Tab`/`.focus()` | 仅鼠标点击 | |
| 聚焦样式 | `:focus` 边框 | 青色边框（自定义） | |
| 光标 | 文本插入光标 | 反色单元格 | |
| 选择 | 鼠标/键盘/双击 | 鼠标拖选 + Shift+←/→ + Ctrl+A | 无双击选词 |
| 剪贴板 | 系统 | 会话内 | |
| 右键菜单（复制/粘贴） | ✅ | ✗ | 右键是框选 |
| IME 中文输入 | ✅ | ✅ | 多字节安全（字符索引↔字节换算） |
| 事件 | focus/blur/input/change + keydown… | focus/blur/input/change | 无 keydown |
| 失焦提交 | change（值变化时） | blur + change（值变化时） | 一致 |
| `el.value` | 读写当前值 | 读/写 DOM 提交值 | 一致（编辑中值在引擎侧） |

---

## 9. 总体定位

- **同构对齐（用户不应感知差异）**：CSS 级联顺序、盒模型（padding/border/margin）、
  flex 主轴/交叉轴、`position` + `z-index`、事件冒泡与 `stopPropagation`、
  `on*` 编译语义、流式复制块边界换行、`:hover` 级联重算。
- **近似但可用（功能在、边界不同）**：行内布局、表格、折行、`font-size` 语义、
  文本选择、事件时序、`line-height`。
- **明确缺失（见 boundaries §4）**：动画/像素/完整 CSS/网络/系统剪贴板/无障碍。

> 判定原则：遇到"清单外"或"近似"的语法，应**明确报错或文档化**，而不是让
> 用户承受静默的错误布局（见 `docs/boundaries.md` §6）。

---

*维护提示：本表随 `docs/web-support.md` 同步维护；新增能力或改变差异级别时
同时更新两处。代码定位指向的模块若移动，同步修正。*
