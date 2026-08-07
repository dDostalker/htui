# htui 支持的 Web 能力边界（Web Support Boundary）

> 本清单是“htui 支持什么 Web 语法”的**权威依据**，以代码实现为准。
> 清单外的语法一律视为**不支持**：能解析则忽略，不能解析则报错——
> 绝不静默渲染错误结果（见 `docs/boundaries.md` §6）。
>
> 状态图例：✅ 完整语义 · ◐ 部分/按通用规则 · ✗ 不支持
>
> 想了解“主流浏览器支持什么、htui 差在哪”的全景对比，见
> [`docs/browser-compat.md`](browser-compat.md)；想逐属性、逐 API 核对
> 支持明细，见 [`docs/css-js-compat.md`](css-js-compat.md)。

---

## 1. HTML 元素

### 1.1 文档结构（不渲染为可视内容）

`html` `head` `title` `meta` `link` `style`（`<style>` 内的 CSS 会级联）；
`<link rel="stylesheet">` / `<script src="...">` 经资源层从 `mem://` 注册表加载
（`mem://` 前缀可选，不访问文件系统/网络，失败仅 warn 静默跳过）

### 1.2 块级与文本（✅ 专用布局/渲染）

| 标签 | 语义 |
| ---- | ---- |
| `div` `p` `section` `article` `main` `nav` | 块级流式布局 |
| `pre` | `white-space: pre`（保留空白/换行，不折行） |
| `span` `em` | 内联文本（`em` 默认斜体） |
| `a` | 内联链接（默认下划线） |
| `h1`–`h5` | 标题（默认 `font-size`: 2em/1.5em/1.17em/1em/0.83em） |
| `aside` `header` `footer` | 块级 + 背景色带（chrome 语义） |
| `figure` `figcaption` `details` `summary` | 按通用块渲染，无折叠/图注专用语义（◐） |
| `ul` `ol` `li` | 列表（`li` 自带项目符号/序号 marker） |
| `img` | 替换元素：加载图片光栅化到字符格（`src` 经资源层从 `mem://` 注册表解析） |
| `input` | 替换元素：可聚焦输入框（值/光标/选择/剪贴板/事件） |

### 1.3 表格（✅ 简化表格布局）

`table` `thead` `tbody` `tfoot` `tr` `th` `td` `caption` `colgroup` `col`

- 支持列宽按内容测量、跨行/跨列（`rowspan`/`colspan`）按实现现状
- 与浏览器表格语义的差异以 `layoutbox/inner_formatting_context/flow/table/` 为准

### 1.4 表单控件（◐ 部分控件化）

`form` `button` `option` `label` `textarea`

- ✅ **`select`**：原生下拉选择框——点击展开浮层（覆盖下层内容）、**指针悬停
  或按住拖动时高亮跟随选项行**、点击/松开选中、`↑/↓/Enter/Esc` 键盘导航、
  选中写回 `value` 属性并触发 `change`、`el.value` 读写选中值。点击外部/
  滚动/按非导航键自动关闭。宽度按最长选项自适应，可用 `width` 属性限定。
- ◐ `form` `button` `option` `label` `textarea`：可解析、可设样式、可挂事件，
  但无浏览器控件语义（`textarea` 不可编辑、`button` 无默认提交、
  `form` 无 `submit`）。唯一完整实现的编辑控件是 `input`。

## 2. CSS

### 2.1 颜色与背景

- `color`、`background-color`（`background` 简写回退）
- 值：`#rrggbb` / `#rgb` / `rgb(r,g,b)` / 常用命名色

### 2.2 盒模型

- `width` `height` `min-width` `max-width` `min-height` `max-height`
- `padding` 简写 + `padding-top/right/bottom/left`
- `margin` 简写 + `margin-*` 四边（`auto` 水平居中）
- `border` 简写 + `border-width/style/color` + 四边；样式支持 `solid`（`none`）
- `box-sizing`（`border-box` / `content-box`）
- `border-radius`（四角，圆角字符绘制）
- 长度单位：`px`、`em`、`rem`（相对根字号）、`%`（相对包含块宽度）、`vw` / `vh`（视口）、裸数字（按单元格）

### 2.3 文本

| 属性 | 支持值 |
| ---- | ------ |
| `font-size` | 长度单位（`px`/`em`/`%`），按单元格扩展渲染 |
| `font-weight` | `normal` `bold` `bolder` `100–900`（仅加粗开关） |
| `font-style` | `normal` `italic` `oblique`（仅斜体开关） |
| `text-decoration` | `underline` `line-through` 及组合 |
| `text-align` | `left` `center` `right` |
| `text-transform` | 以 `TextTransform` 枚举为准（大小写变换） |
| `line-height` | 数值/单位，以行高单元格解析为准 |
| `white-space` | `normal` `nowrap` `pre` `pre-wrap` `pre-line` |
| `word-break` | `normal` `break-all` `keep-all` |
| `overflow-wrap` | `normal` `anywhere` `break-word` |

### 2.4 布局与定位

- `display`：`block` `inline` `flex` `table`（族）`inline-block` `list-item`
  `grid`（◐ 可解析，布局为空壳预留）
- `position`：`static` `relative` `absolute` `fixed`；`top/right/bottom/left`
  偏移；`z-index` 层叠

### 2.5 Flexbox

✅ `flex-direction`（row/column…）· `flex-wrap` · `justify-content` ·
`align-items` · `align-self` · `align-content` · `flex-grow` · `flex-shrink` ·
`flex-basis` · `order` · `gap`

（具体值集以 `css_parse/values` 枚举为准；与浏览器的已知差异见各 flex 模块注释）

### 2.6 选择器

- ✅ 类型选择器（`div`）、`.class`、`#id`
- ✅ 组合器：后代（空格）、`>` 子代、`+` 相邻兄弟、`~` 后续兄弟
- ✅ 伪类：`:hover`、`:nth-child(an+b)`（子集：`even`/`odd`/整数/`an+b`）
- ✗ 其他伪类（`:first-child`、`:not` 等）解析即报错

### 2.7 CSS 级联

- ✅ 继承属性表（`INHERITED_PROPS`）、内联 `style` 属性 > 选择器规则、
  相同优先级按出现顺序
- ✅ 支持 `<style>` 标签内联 CSS；也支持 `<link rel="stylesheet">`（`href` 经资源层从 `mem://` 注册表解析，失败静默跳过）

## 3. JavaScript / DOM

### 3.1 语言层（由 Boa 引擎提供）

✅ 标准 ES 语法：变量/函数/闭包/对象/数组/字符串（含模板字符串）/数值运算/
`try/catch` 等。语言能力以 Boa 实现为准。

### 3.2 全局对象

| API | 说明 |
| --- | ---- |
| `document.getElementById(id)` | ✅ 返回元素或 `null` |
| `document.createElement(tag)` | ✅ 新元素（未挂入文档） |
| `console.log/warn/error(...)` | ✅ 写入 htui 日志 |
| `alert(msg)` | ✅ `log::warn` |
| `quit()` | ✅ 退出渲染循环 |
| `fetch(url)` | ✅ 异步资源加载，返回 `Promise`（`mem://` 虚拟资源，前缀可选）；Response 含 `status`/`ok`/`url`/`text()`。注册表读取即时完成，Promise 经微任务队列 settle（`.then`/`await` 可用）。不访问文件系统/网络 |
| 宿主函数 | ✅ `EventBindings.host` 注册的名字（`#[htui::command]` 生成） |

### 3.3 Element 包装对象

- ✅ 属性：`textContent` `className` `id` `parentNode` `value`（`<input>` 已提交值）
- ✅ 方法：`getAttribute` `setAttribute` `removeAttribute`
  `addEventListener` `removeEventListener` `appendChild` `removeChild`
- ✅ `el.style`：见下方属性清单 + `cssText`
- ✗ 未实现：`querySelector(All)`、`classList`、`innerHTML`、`children`、
  `firstChild`、`dataset`、`focus()` 等

### 3.4 `el.style` 可读写属性

`color` `backgroundColor` `fontSize` `fontWeight` `fontStyle` `textAlign`
`textDecoration` `textTransform` `lineHeight` `display` `width` `height`
`padding`(+`Top/Right/Bottom/Left`) `margin`(+四边) `border` `borderWidth`
`borderStyle` `borderColor` `whiteSpace` `flexDirection` `flexWrap`
`justifyContent` `alignItems` `alignContent` `gap` `cssText`

（camelCase ↔ kebab-case 自动映射；写空值移除属性）

### 3.5 事件

| 事件 | 触发条件 | 绑定 |
| ---- | -------- | ---- |
| `click` | 左键按下并同格释放 | `onclick` + `addEventListener` |
| `mouseover` / `mouseout` | 指针进入/离开 | `onmouseover/out` + 监听器 |
| `focus` / `blur` | `<input>` 聚焦/失焦 | `onfocus/onblur` + 监听器 |
| `input` | 编辑中每次值变化 | `oninput` + 监听器 |
| `change` | 失焦/Enter 且值变化 | `onchange` + 监听器 |

✅ 沿 DOM 祖先链冒泡、`stopPropagation()`、`Event` 对象（`type`/`target`/
`currentTarget`/`x`/`y`/`bubbles`/`preventDefault()`）、`on*` 属性按
`function(event){ body }` 编译（裸标识符只求值不调用）。

✅ 定时器：`setTimeout` / `setInterval` / `clearTimeout` / `clearInterval`——
回调在脚本线程的消息间隙执行（`ScriptEngine::run_due_timers`），改 DOM 照常触发渲染。

✅ 微任务事件循环：`fetch` 的 `.then` / `await` 回调经 Boa job 队列执行
（`ScriptEngine::poll_fetches` + `Context::run_jobs`），与定时器同属脚本线程的消息循环。

## 4. 输入与交互（终端特有）

- ✅ 鼠标：点击、hover、滚轮滚动、`Ctrl+滚轮/±/0` 缩放
- ✅ 左键拖动：`<input>` 内字符选择；普通文本**流式选择**（跨块复制换行）
- ✅ 右键拖动：矩形框选（vim 块选）
- ✅ `Ctrl+A/C/X/V`（全选/复制/剪切/粘贴）、`Shift+←/→` 扩展选择
- ✅ 剪贴板为**会话内**（无系统剪贴板/OSC 52）

## 5. 明确不支持的 Web 能力

| 能力 | 说明 |
| ---- | ---- |
| CSS 动画 / 过渡 / `transform` / `opacity` | ✗ 拒绝项 |
| CSS 完整 `grid` | ✗ 空壳预留 |
| `calc()`、CSS 变量、媒体查询、`float`、多列 | ✗ |
| `querySelector(All)`、`classList`、`innerHTML` | ✗ |
| 系统剪贴板（OSC 52） | ✗ 拒绝项 |
| 像素级渲染（除 `<img>`） | ✗ 拒绝项 |
| 触摸 / 无障碍 / 多文档窗口 | ✗ 拒绝项 |

---

*维护提示：修改 `tdom/node.rs` 标签集、`css_parse/properties` 属性、或
`script/dom.rs` 绑定后，同步更新本清单对应条目。*
