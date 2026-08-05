# htui CSS / JS 逐项兼容明细

> 以 `SUPPORTED_PROPERTIES`（`html/css_parse/cascade.rs`）和
> `script/dom.rs` 绑定为权威依据，逐属性、逐取值、逐 API 标注支持状态。
>
> 图例：✅ 完整 · ◐ 部分/近似 · ✗ 不支持（解析后忽略或报错）
> 本文件是 `docs/web-support.md` 与 `docs/browser-compat.md` 的明细补充。

---

## A. CSS 逐属性明细

### A.0 总览

- **属性白名单**：`SUPPORTED_PROPERTIES` 共 **49 个属性名**（含简写与长手）。
  白名单之外的属性（`opacity`、`cursor`、`overflow`…）**不解析、不生效**。
- **继承属性**：`INHERITED_PROPS` 共 8 个 —— `color`、`text-align`、
  `font-weight`、`font-style`、`text-decoration`、`font-size`、
  `text-transform`、`line-height`。
- **级联**：内联 `style` 属性 > 选择器规则；同优先级按出现顺序；无
  `!important`、无 CSS 变量。

### A.1 颜色与背景

| 属性 | 状态 | 支持值 | 说明 |
| ---- | :--: | ------ | ---- |
| `color` | ✅ | `#RRGGBB` / `#RGB` / `rgb(r,g,b)` / 148 个命名色 | 继承 |
| `background-color` | ✅ | 同上 | 不继承 |
| `background` | ✅ | 仅解析其中的颜色部分（`background: #333`） | `background-image` 等忽略 |

**不支持**：`hsl()` / `hwb()` / `rgba()`（alpha）/ `transparent` /
`currentColor` / `color-mix()` / 渐变（`linear-gradient` 等）。

### A.2 盒模型

| 属性 | 状态 | 支持值 / 说明 |
| ---- | :--: | ---- |
| `width` `height` | ✅ | 长度单位（见 A.3）；`auto` 由布局决定 |
| `min-width` `max-width` | ✅ | 长度；`none`/`auto` → 不限制 |
| `min-height` `max-height` | ✅ | 同上 |
| `padding` | ✅ | 1–4 值简写 |
| `padding-top/right/bottom/left` | ✅ | 长手覆盖简写 |
| `margin` | ✅ | 1–4 值简写；`auto` 仅水平居中 |
| `margin-top/right/bottom/left` | ✅ | |
| `border` | ✅ | `border: <width> <style> <color>` 简写 |
| `border-top/right/bottom/left` | ✅ | 单边简写 |
| `border-width` | ✅ | 通用四边宽度 |
| `border-style` | ◐ | 仅 `solid` / `none`；dashed/dotted 等不画 |
| `border-color` | ✅ | |
| `box-sizing` | ◐ | `border-box` / `content-box`；**默认 `border-box`**（浏览器默认 `content-box`，且可继承性不同——这是最大盒模型差异） |
| `border-radius` | ✅ | 1–4 值长度；>1 的半径与 1 渲染相同 |

### A.3 长度单位（`Length`）

| 单位 | 状态 | 说明 |
| ---- | :--: | ---- |
| `px` | ✅ | 1px = 1 格（`PX_PER_CELL = 1.0`） |
| 裸数字（`40`） | ✅ | 按单元格计 |
| `%` | ✅ | 相对包含块**宽度** |
| `em` | ✅ | 相对当前元素字号（格数） |
| `rem` | ✅ | 相对根字号；终端无根字号差异，语义同 `em` |
| `vw` / `vh` | ✅ | 相对终端列/行，任何属性中都锚定视口 |
| `auto` | ✅ | 布局侧解释 |
| `vmin` `vmax` `ch` `ex` `pt` `cm` `mm` `in` `pc` | ✗ | 解析返回 `None` |
| `calc()` | ✗ | 无 |

### A.4 文本与排版

| 属性 | 状态 | 支持值 | 说明 |
| ---- | :--: | ------ | ---- |
| `font-size` | ◐ | 长度单位 | 按"每字符 N 格"扩展，非像素字号 |
| `font-weight` | ◐ | `normal` `bold` `bolder` `700–900`（→加粗）；`lighter` `100–600`（→常规） | 仅两态 |
| `font-style` | ◐ | `normal` `italic` `oblique`（斜体同渲染） | |
| `text-decoration` | ✅ | `none` `underline` `line-through` 及组合 | |
| `text-align` | ✅ | `left` `center` `right` | 逐视觉行 |
| `text-transform` | ✅ | `none` `uppercase` `lowercase` `capitalize` | 影响宽度测量 |
| `line-height` | ◐ | `normal` / 数字倍数 / 长度（`%` 相对自身字号） | 行格模型，取整行 |
| `white-space` | ✅ | `normal` `pre` `nowrap` `pre-wrap` `pre-line` | `break-spaces` ✗ |
| `word-break` | ✅ | `normal` `break-all` `keep-all` | |
| `overflow-wrap`（`word-wrap` 别名） | ✅ | `normal` `break-word` `anywhere` | |

**不支持**：`font-family`、`letter-spacing`、`word-spacing`、
`text-shadow`、`text-indent`、`text-overflow`、`text-align-last`。

### A.5 布局与定位

| 属性 | 状态 | 说明 |
| ---- | :--: | ---- |
| `display` | ◐ | 24 个值，见 A.6 |
| `position` | ✅ | `static` `relative` `absolute` `fixed`；`sticky` ✗ |
| `top` `right` `bottom` `left` | ✅ | 长度；`auto` → 保留静态位置 |
| `z-index` | ✅ | 整数，可负；仅对定位盒生效 |

**不支持**：`float` `clear`、多列（`column-*`）、`overflow`、
`visibility`、`object-fit`。

### A.6 `display` 取值明细（`CssDisplay`）

| 值 | 状态 | 行为 |
| --- | :--: | ---- |
| `block` | ✅ | 块级，BFC |
| `flow` | ✅ | 同 `block` |
| `flow-root` | ✅ | 同 `block`（BFC 以 `container` 标志暴露） |
| `inline` | ✅ | 参与父内联流 |
| `inline-block` | ✅ | 行内级 + 子 BFC |
| `list-item` | ✅ | 块级，v1 不渲染 marker（`li` 的 marker 由标签布局补） |
| `flex` / `inline-flex` | ✅ | flex 上下文 |
| `table` 族（`table` `inline-table` `table-row-group` `table-header-group` `table-footer-group` `table-row` `table-cell` `table-column-group` `table-column` `table-caption`） | ✅ | 简化表格 |
| `grid` / `inline-grid` | ◐ | **可解析，布局空壳**（geometry 返回 None） |
| `none` | ✅ | 元素与后代不生成盒 |
| `contents` | ✅ | 自身不生成盒，子元素展平到父级 |
| 其它（`ruby`、`math`、`run-in`…） | ✗ | 解析返回 None → 回退 UA 默认 |

### A.7 Flex 属性取值明细

| 属性 | 支持值 | 拒绝值 | 说明 |
| ---- | ------ | ------ | ---- |
| `flex-direction` | `row` `column` | `row-reverse` `column-reverse` | 仅正向 |
| `flex-wrap` | `nowrap` `wrap` | `wrap-reverse` | |
| `justify-content` | `flex-start` `flex-end` `center` `space-between` `space-around` `space-evenly`（`start`/`end` 别名） | `left`/`right`（旧值）、`stretch` | |
| `align-items` `align-self` | `stretch` `flex-start` `flex-end` `center` | `baseline` | **`stretch` 仅容器有显式交叉轴尺寸时生效**（v1 通常 no-op） |
| `align-content` | 同 `justify-content` | | |
| `flex-grow` `flex-shrink` | 非负浮点（`0` `1` `2` `0.5` `1.25`） | 负数、非数字 | |
| `flex-basis` | `auto`、长度 | `content`、`fit-content` | |
| `order` | 任意整数（可负） | | |
| `gap` `row-gap` `column-gap` | 1–2 个长度 | **百分比 gap 不支持**、`normal` | 长手覆盖简写 |
| `flex` 简写 | ✗ | — | 未列入白名单，须写 `flex-grow`/`flex-shrink`/`flex-basis` |

### A.8 选择器明细

| 语法 | 状态 | 说明 |
| ---- | :--: | ---- |
| 类型 `div` | ✅ | 映射自 47 个已注册标签 |
| `.class` | ✅ | |
| `#id` | ✅ | |
| 复合 `.a.b` `div.foo` | ✅ | |
| 后代 ` ` | ✅ | |
| 子代 `>` | ✅ | |
| 相邻兄弟 `+` | ✅ | |
| 后续兄弟 `~` | ✅ | |
| `:hover` | ✅ | 唯一动态伪类，级联时重算 |
| `:nth-child(...)` | ◐ | `even` / `odd` / 整数 / `<a>n[+b|-b]`；**拒绝** `-n+3`、`n`、`3n-2` 隐式系数、`0n` |
| 分组 `a, b` | ◐ | 按实现现状 |
| `:not()` `:first-child` `:last-child` `:focus` `:active` `:first-of-type` 等 | ✗ | 解析报错 |
| `::before` `::after` | ✗ | 无 |
| `[attr]` 属性选择器 | ✗ | 无 |

### A.9 @规则与其它

| 语法 | 状态 | 说明 |
| ---- | :--: | ---- |
| `@media` | ✗ | 无媒体查询 |
| `@keyframes` / `@font-face` / `@import` / `@supports` | ✗ | 无 |
| `!important` | ✗ | 无 |
| CSS 变量 `var(--x)` | ✗ | 无 |
| 注释 `/* */` | ✅ | cssparser 跳过 |

### A.10 明确不支持的 CSS 属性全清单

按类列出（不在 `SUPPORTED_PROPERTIES`，写了无效果）：

- **外观**：`opacity` `cursor` `filter` `backdrop-filter` `box-shadow` `outline` `appearance`
- **排版**：`font-family` `letter-spacing` `word-spacing` `text-shadow` `text-indent` `text-overflow` `direction` `unicode-bidi` `writing-mode` `font-variant`
- **布局**：`float` `clear` `overflow` `overflow-x/y` `visibility` `object-fit` `position:sticky` `columns` `column-count` `column-width` `flex`（简写）
- **动画**：`transition` `animation` `transform` `transform-origin`
- **背景**：`background-image` `background-size` `background-position` `background-repeat`
- **列表**：`list-style` `list-style-type` `list-style-position`
- **表格**：`border-collapse` `border-spacing` `table-layout` `empty-cells`
- **用户交互**：`user-select` `pointer-events` `resize` `touch-action`
- **其它**：`content`（伪元素用）`z-index` 之外的一切 `counter-*`

---

## B. JavaScript / DOM 逐项明细

### B.1 全局对象

| 全局 | 状态 | 成员 / 说明 |
| ---- | :--: | ---- |
| `document` | ◐ | 仅 `getElementById(id)`、`createElement(tag)` |
| `console` | ◐ | `log` `warn` `error`（写入 htui 日志） |
| `alert` | ◐ | 写入日志（无对话框） |
| `quit` | ✅ | 退出渲染循环 |
| 宿主函数 | ✅ | `EventBindings.host` 注册名；`#[htui::command]` 生成 |
| `globalThis` | ✅ | Boa 提供 |
| `window` / `self` | ✗ | 未注册 |
| `localStorage` `sessionStorage` `indexedDB` `cookie` | ✗ | 无 |
| `fetch` `XMLHttpRequest` `WebSocket` | ✗ | 无网络 |
| `setTimeout` `setInterval` `requestAnimationFrame` `queueMicrotask` | ✗ | 无计时器；用 `EventBindings::auto` |

### B.2 Element 对象

**属性（可读写 ✅ 只读 ◐）**

| 属性 | 状态 | 说明 |
| ---- | :--: | ---- |
| `textContent` | ✅ | 读写；等值写入短路不触发重渲染 |
| `className` | ✅ | 读写 |
| `id` | ✅ | 读写 |
| `parentNode` | ✅ | 只读 |
| `value` | ✅ | 读写 `<input>` 提交值 |
| `style` | ✅ | 对象，见 B.3 |
| `tagName` `nodeType` | ✗ | 无 |
| `children` `firstChild` `lastChild` `nextSibling` `parentElement` | ✗ | 无 |
| `innerHTML` `outerHTML` `text` | ✗ | 无 |
| `classList` `dataset` | ✗ | 无 |
| `offsetWidth` `getBoundingClientRect` | ✗ | 无 |

**方法**

| 方法 | 状态 | 说明 |
| ---- | :--: | ---- |
| `getAttribute(name)` | ✅ | 无 `href` 等特殊值处理 |
| `setAttribute(name, value)` | ✅ | 标记重渲染 |
| `removeAttribute(name)` | ✅ | |
| `addEventListener(type, fn)` | ✅ | 支持 7 个事件类型 |
| `removeEventListener(type, fn)` | ✅ | |
| `appendChild(child)` | ✅ | 仅接受 `createElement` 产物 |
| `removeChild(child)` | ✅ | |
| `insertBefore` `replaceChild` `cloneNode` `remove()` | ✗ | 无 |
| `focus()` `blur()` `click()` | ✗ | 无编程式触发 |
| `setAttributeNode` 等 | ✗ | 无 |

### B.3 `el.style` 可访问属性（30 个）

`color` `backgroundColor` `fontSize` `fontWeight` `fontStyle` `textAlign`
`textDecoration` `textTransform` `lineHeight` `display` `width` `height`
`padding` `paddingTop` `paddingRight` `paddingBottom` `paddingLeft`
`margin` `marginTop` `marginRight` `marginBottom` `marginLeft`
`border` `borderWidth` `borderStyle` `borderColor`
`whiteSpace` `flexDirection` `flexWrap` `justifyContent` `alignItems`
`alignContent` `gap` `cssText`

- camelCase ↔ kebab-case 自动映射；
- 写空字符串 = 移除该属性；其余通过 `style_set` 写入 style 属性并重渲染；
- **只读**：`style` 之外无计算样式 API（无 `getComputedStyle`）。

### B.4 事件

**可监听事件（7 个）**

| 事件 | 属性绑定 | 说明 |
| ---- | ---- | ---- |
| `click` | `onclick` | 左键同格按下/释放 |
| `mouseover` | `onmouseover` | 指针进入 |
| `mouseout` | `onmouseout` | 指针离开 |
| `focus` | `onfocus` | `<input>` 聚焦（mousedown 时） |
| `blur` | `onblur` | `<input>` 失焦（点击他处/Esc/Enter/切换） |
| `input` | `oninput` | 编辑中每次值变化 |
| `change` | `onchange` | 失焦/Enter 且值变化 |

**Event 对象成员**

| 成员 | 状态 | 说明 |
| ---- | :--: | ---- |
| `type` | ✅ | 事件名 |
| `target` `currentTarget` | ✅ | 元素包装（同一性缓存） |
| `bubbles` | ✅ | 恒为 `true` |
| `x` `y` | ✅ | canvas 坐标（含滚动偏移） |
| `stopPropagation()` | ✅ | 终止冒泡 |
| `preventDefault()` | ◐ | 存在，但事件无可阻止的默认行为 |
| `keyCode` `button` `clientX` `shiftKey` 等 | ✗ | 无修饰键字段 |

**不支持监听**：`mousedown` `mouseup` `mousemove` `keydown` `keyup`
`wheel` `scroll` `resize` `dblclick` `contextmenu` `submit` `change` 之外的
表单事件。

### B.5 JS 语言内建（Boa 引擎）

以 Boa 0.21 实现为准，主要内建均可用：

- ✅ 标准语法：函数/闭包/解构/`...` 展开/可选链 `?.`/空值合并 `??`/`class`/模板字符串/`for...of`/`async` 语法
- ✅ 内建对象：`Object` `Array` `String` `Number` `Boolean` `Math` `Date` `JSON` `RegExp` `Map` `Set` `WeakMap` `WeakSet` `Error` 族 `Symbol` `BigInt` `Proxy` `Reflect` `Intl`（以 Boa 为准）
- ◐ `Promise`：对象可用，但**无事件循环驱动**——`then` 回调不会在后续同步代码后自动执行（宿主 `fire`/`invoke_action` 是同步的），不要依赖 `await` 时序
- ✗ `fetch`、`Worker`、`WebAssembly` 集成等浏览器/宿主 API

### B.6 常见 JS 写法在 htui 的可用性速查

| 写法 | 可用？ | 说明 |
| ---- | :--: | ---- |
| `document.getElementById('x').textContent = v` | ✅ | 动态文本标准写法 |
| `el.style.color = 'red'` | ✅ | 动态样式标准写法 |
| `el.addEventListener('click', fn)` | ✅ | |
| `document.createElement('div')` + `appendChild` | ✅ | 动态构建 DOM |
| `el.innerHTML = '<div>…</div>'` | ✗ | 无 |
| `document.querySelectorAll('.x')` | ✗ | 无 |
| `setInterval(fn, 1000)` | ✗ | 用 Rust `EventBindings::auto` |
| `el.classList.add('on')` | ✗ | 用 `setAttribute('class', …)` |
| `el.dataset.id` | ✗ | 用 `getAttribute('data-id')` |
| `async function refresh() { await … }` | ◐ | 语法可解析，await 无驱动 |
| `console.log(x)` | ✅ | 写日志 |
| `JSON.parse(str)` / `JSON.stringify(o)` | ✅ | Boa 内建 |
| `String(x).padStart(2,'0')` | ✅ | Boa 内建 |

---

*维护提示：白名单（`cascade.rs`）、取值枚举（`values/`）、DOM 绑定
（`dom.rs`）、事件（`events/`）变更后同步本文件与 `web-support.md`。*
