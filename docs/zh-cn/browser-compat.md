# htui 与主流浏览器能力对比及差异清单

本文件主要回答两个问题：主流 Web 平台的能力，htui 支持到什么程度,htui 渲染
结果与浏览器的差异部分。

> 状态图例：✅ 支持 · ◐ 部分/近似 · ✗ 不支持 · — 无对应

# HTML 特性清单

## 1. 文档结构与元数据

| 标签    | htui | 说明                                         |
| ------- | :--: | -------------------------------------------- |
| `html`  |      | 文档根元素                                   |
| `head`  |      | 文档元数据容器                               |
| `body`  |      | 文档主体内容                                 |
| `title` |      | 文档标题                                     |
| `base`  |      | 文档基准 URL 与默认跳转目标                  |
| `link`  |      | 外部资源链接(样式表、图标、预加载、预连接等) |
| `meta`  |      | 元数据(字符编码、视口、描述、CSP 等)         |
| `style` |      | 内嵌 CSS,参与级联                            |

## 2. 章节(Sections)

| 标签                          | htui | 说明                                   |
| ----------------------------- | :--: | -------------------------------------- |
| `article`                     |      | 独立的可分发内容(文章、帖子、卡片)     |
| `section`                     |      | 主题相关的内容分组,通常带标题          |
| `nav`                         |      | 导航链接区域                           |
| `aside`                       |      | 与主内容松散关联的侧边内容(旁注、广告) |
| `h1` `h2` `h3` `h4` `h5` `h6` |      | 六级标题                               |
| `hgroup`                      |      | 标题组(主标题 + 副标题)                |
| `header`                      |      | 区段或页面头部(导航/标题/搜索框)       |
| `footer`                      |      | 区段或页面尾部(版权/链接/作者)         |
| `address`                     |      | 文章/文档作者的联系信息                |

## 3. 分组内容(Grouping content)

| 标签         | htui | 说明                                       |
| ------------ | :--: | ------------------------------------------ |
| `p`          |      | 段落                                       |
| `hr`         |      | 主题级分隔线                               |
| `pre`        |      | 预格式化文本,保留空白与换行                |
| `blockquote` |      | 块级引用                                   |
| `ol`         |      | 有序列表                                   |
| `ul`         |      | 无序列表                                   |
| `menu`       |      | 命令/菜单列表                              |
| `li`         |      | 列表项(属于 `ol`/`ul`/`menu`)              |
| `dl`         |      | 描述列表容器                               |
| `dt`         |      | 描述术语                                   |
| `dd`         |      | 描述内容                                   |
| `figure`     |      | 带可选标题的独立内容单元(图表、代码、媒体) |
| `figcaption` |      | `figure` 的标题/说明                       |
| `main`       |      | 文档的主内容(每页最多一个)                 |
| `search`     |      | 搜索/过滤区域容器                          |
| `div`        |      | 通用块级容器,无语义                        |

## 4. 文本级语义(Text-level semantics)

| 标签        | htui | 说明                                |
| ----------- | :--: | ----------------------------------- |
| `a`         |      | 超链接或占位锚点                    |
| `em`        |      | 语义强调(通常斜体)                  |
| `strong`    |      | 重要性强调(通常粗体)                |
| `small`     |      | 旁注/细则(版权、免责声明)           |
| `s`         |      | 不再准确或不再相关(删除线)          |
| `cite`      |      | 作品标题引用(书名、文章名)          |
| `q`         |      | 行内引用(自动加引号)                |
| `dfn`       |      | 定义术语的首次出现                  |
| `abbr`      |      | 缩写或首字母缩略词(配 `title` 全称) |
| `ruby`      |      | 注音容器(主要用于中日韩文字)        |
| `rt`        |      | 注音文本                            |
| `rp`        |      | 不支持 `ruby` 时的回退括号          |
| `data`      |      | 机器可读数据(`value` 属性)          |
| `time`      |      | 日期/时间(`datetime` 属性)          |
| `code`      |      | 计算机代码片段                      |
| `var`       |      | 数学/程序中的变量                   |
| `samp`      |      | 程序或系统的示例输出                |
| `kbd`       |      | 用户键盘输入                        |
| `sub` `sup` |      | 下标 / 上标                         |
| `i`         |      | 不同语态/习语(技术术语、外语,斜体)  |
| `b`         |      | 引人注意(关键词、产品名,粗体)       |
| `u`         |      | 未表达清楚的标注(下划线)            |
| `mark`      |      | 高亮/标记(搜索结果命中)             |
| `bdi`       |      | 双向文本隔离                        |
| `bdo`       |      | 双向文本方向覆盖                    |
| `span`      |      | 通用行内容器,无语义                 |
| `br`        |      | 强制换行                            |
| `wbr`       |      | 可选换行点(长单词断行提示)          |

## 5. 编辑(Edits)

| 标签  | htui | 说明       |
| ----- | :--: | ---------- |
| `ins` |      | 新增的内容 |
| `del` |      | 删除的内容 |

## 6. 嵌入内容(Embedded content)

| 标签      | htui | 说明                                        |
| --------- | :--: | ------------------------------------------- |
| `picture` |      | 多源图片容器,按条件选择 `source`            |
| `source`  |      | 为 `picture`/`video`/`audio` 指定替代媒体源 |
| `img`     |      | 位图图片                                    |
| `iframe`  |      | 内嵌独立浏览上下文                          |
| `embed`   |      | 外部内容嵌入点(插件)                        |
| `object`  |      | 外部资源/插件(可含回退内容)                 |
| `video`   |      | 视频播放器                                  |
| `audio`   |      | 音频播放器                                  |
| `track`   |      | 媒体的定时文本轨道(字幕、章节)              |
| `map`     |      | 客户端图像映射容器                          |
| `area`    |      | 图像映射中的可点击热点                      |

## 7. 表格(Tabular data)

| 标签       | htui | 说明                  |
| ---------- | :--: | --------------------- |
| `table`    |      | 二维表格容器          |
| `caption`  |      | 表格标题              |
| `colgroup` |      | 列分组                |
| `col`      |      | 列属性定义(跨列样式)  |
| `thead`    |      | 表头行分组            |
| `tbody`    |      | 表格主体行分组        |
| `tfoot`    |      | 表尾行分组            |
| `tr`       |      | 表格行                |
| `td`       |      | 数据单元格            |
| `th`       |      | 头部单元格(行/列表头) |

## 8. 表单(Forms)

| 标签              | htui | 说明                             |
| ----------------- | :--: | -------------------------------- |
| `form`            |      | 表单容器,管理提交                |
| `label`           |      | 控件的文字标签                   |
| `input`           |      | 通用输入控件(见下表各 `type`)    |
| `button`          |      | 按钮(submit/reset/button)        |
| `select`          |      | 下拉选择框                       |
| `datalist`        |      | `input` 的自动补全候选值列表     |
| `optgroup`        |      | `select` 内的选项分组            |
| `option`          |      | `select`/`datalist` 中的一个选项 |
| `textarea`        |      | 多行文本输入                     |
| `output`          |      | 计算结果输出                     |
| `progress`        |      | 进度条(`value`/`max`)            |
| `meter`           |      | 标量度量(磁盘用量、评分)         |
| `fieldset`        |      | 表单控件分组                     |
| `legend`          |      | `fieldset` 的标题                |
| `selectedcontent` |      | `select` 选中项的显示内容投射    |

## 8.1 `input` 的 `type` 状态

| type             | htui | 说明                     |
| ---------------- | :--: | ------------------------ |
| `hidden`         |      | 隐藏值,不渲染            |
| `text`           |      | 单行文本输入             |
| `search`         |      | 搜索词输入               |
| `tel`            |      | 电话号码输入             |
| `url`            |      | URL 输入                 |
| `email`          |      | 邮箱输入                 |
| `password`       |      | 密码输入(掩码)           |
| `date`           |      | 日期选择(无时区)         |
| `month`          |      | 年月选择                 |
| `week`           |      | 年周选择                 |
| `time`           |      | 时间选择(无时区)         |
| `datetime-local` |      | 本地日期时间选择(无时区) |
| `number`         |      | 数字输入                 |
| `range`          |      | 滑块选择数值区间         |
| `color`          |      | 颜色选择器               |
| `checkbox`       |      | 复选框                   |
| `radio`          |      | 单选按钮                 |
| `file`           |      | 文件上传                 |
| `submit`         |      | 提交按钮                 |
| `image`          |      | 图片提交按钮             |
| `reset`          |      | 重置按钮                 |
| `button`         |      | 自定义按钮(无默认行为)   |

## 9. 交互元素(Interactive elements)

| 标签      | htui | 说明                      |
| --------- | :--: | ------------------------- |
| `details` |      | 可展开/折叠的详细信息区块 |
| `summary` |      | `details` 的可见摘要      |
| `dialog`  |      | 对话框/模态框             |

## 10. 脚本(Scripting)

| 标签             | htui | 说明                    |
| ---------------- | :--: | ----------------------- |
| `script`         |      | 内嵌或外部脚本          |
| `script src=...` |      | 外部脚本(经资源层加载)  |
| `noscript`       |      | 脚本禁用时的替代内容    |
| `template`       |      | 模板内容(不渲染,可克隆) |
| `slot`           |      | Shadow DOM 中的内容插槽 |
| `canvas`         |      | 位图绘图画布(2D/WebGL)  |

## 11. 全局属性(所有元素可用)

| 属性              | htui | 说明                   |
| ----------------- | :--: | ---------------------- |
| `class`           |      | 类名(用于 CSS/JS 选择) |
| `id`              |      | 文档内唯一标识         |
| `style`           |      | 行内 CSS               |
| `title`           |      | 提示信息/语义化标题    |
| `lang` `xml:lang` |      | 内容语言代码           |
| `dir`             |      | 文本方向(ltr/rtl/auto) |
| `translate`       |      | 是否参与翻译           |
| `data-*`          |      | 自定义私有数据         |
| `tabindex`        |      | 焦点顺序控制           |
| `accesskey`       |      | 快捷键分配             |
| `hidden`          |      | 隐藏元素               |
| `contenteditable` |      | 内容是否可编辑         |
| `draggable`       |      | 是否可拖拽             |
| `spellcheck`      |      | 是否启用拼写检查       |
| `enterkeyhint`    |      | 软键盘回车键提示       |
| `inert`           |      | 使区域不可交互         |
| `popover`         |      | 声明式弹出层           |
| `autofocus`       |      | 页面加载时自动聚焦     |
| `role` `aria-*`   |      | ARIA 无障碍语义        |

# CSS 特性清单 (CSS Snapshot 2024)

## 1. 显示与定位

| 特性                          | htui | 说明                                                   |
| ----------------------------- | :--: | ------------------------------------------------------ |
| `display`                     |      | 显示类型(block/inline/flex/grid/flow/none/contents 等) |
| `position`                    |      | 定位方式(static/relative/absolute/fixed/sticky)        |
| `top` `right` `bottom` `left` |      | 定位偏移量                                             |
| `float`                       |      | 浮动(left/right/none)                                  |
| `clear`                       |      | 清除浮动                                               |
| `z-index`                     |      | 层叠顺序                                               |
| `contain`                     |      | 隔离优化(layout/style/paint/size/strict)               |
| `isolation`                   |      | 强制创建层叠上下文                                     |
| `will-change`                 |      | 性能提示,预告将变化的属性                              |
| `visibility`                  |      | 可见性(visible/hidden/collapse)                        |
| `overflow`                    |      | 溢出处理(visible/hidden/scroll/auto)                   |
| `box-sizing`                  |      | 盒模型计算(content-box/border-box)                     |

## 2. Flex 弹性布局

| 特性                          | htui | 说明                                                         |
| ----------------------------- | :--: | ------------------------------------------------------------ |
| `display: flex`/`inline-flex` |      | 启用 flex 容器(见 §1 display)                                |
| `flex`                        |      | 简写(grow shrink basis)                                      |
| `flex-direction`              |      | 主轴方向(row/column/row-reverse/column-reverse)              |
| `flex-wrap`                   |      | 换行(nowrap/wrap/wrap-reverse)                               |
| `flex-flow`                   |      | direction + wrap 简写                                        |
| `justify-content`             |      | 主轴对齐(flex-start/center/space-between/space-around/space-evenly) |
| `align-items`                 |      | 交叉轴对齐                                                   |
| `align-self`                  |      | 单个项目交叉轴对齐                                           |
| `align-content`               |      | 多行交叉轴对齐                                               |
| `order`                       |      | 项目排列顺序                                                 |
| `flex-grow`                   |      | 放大比例                                                     |
| `flex-shrink`                 |      | 缩小比例                                                     |
| `flex-basis`                  |      | 初始主轴尺寸                                                 |

## 3. Grid 网格布局

| 特性                                  | htui | 说明                                              |
| ------------------------------------- | :--: | ------------------------------------------------- |
| `grid`                                |      | 简写(template + auto-flow)                        |
| `grid-template`                       |      | rows + columns + areas 简写                       |
| `grid-template-columns`               |      | 列轨道定义                                        |
| `grid-template-rows`                  |      | 行轨道定义                                        |
| `grid-template-areas`                 |      | 命名网格区域                                      |
| `grid-area`                           |      | 项目区域定位(row-start/col-start/row-end/col-end) |
| `grid-column` `grid-row`              |      | 项目起止线简写                                    |
| `grid-column-start` `grid-column-end` |      | 列起止线                                          |
| `grid-row-start` `grid-row-end`       |      | 行起止线                                          |
| `grid-auto-columns`                   |      | 隐式列轨道宽度                                    |
| `grid-auto-rows`                      |      | 隐式行轨道高度                                    |
| `grid-auto-flow`                      |      | 自动放置算法(row/column/dense)                    |

## 4. 间距 / 多列布局

| 特性                                                        | htui | 说明                    |
| ----------------------------------------------------------- | :--: | ----------------------- |
| `gap`                                                       |      | flex/grid 行列间距简写  |
| `row-gap` `column-gap`                                      |      | 行间距 / 列间距         |
| `place-content` `place-items` `place-self`                  |      | align + justify 简写    |
| `columns`                                                   |      | 多列简写(width + count) |
| `column-width` `column-count`                               |      | 列宽 / 列数             |
| `column-gap`                                                |      | 列间距                  |
| `column-rule`                                               |      | 列分隔线简写            |
| `column-rule-color` `column-rule-style` `column-rule-width` |      | 分隔线颜色/样式/宽度    |
| `column-fill`                                               |      | 填充方式(auto/balance)  |
| `column-span`                                               |      | 跨列(none/all)          |

## 5. 盒模型 / 尺寸

| 特性                                                         | htui | 说明                            |
| ------------------------------------------------------------ | :--: | ------------------------------- |
| `width` `height`                                             |      | 宽 / 高                         |
| `min-width` `min-height` `max-width` `max-height`            |      | 最小/最大尺寸                   |
| `margin` `margin-top` `margin-right` `margin-bottom` `margin-left` |      | 外边距                          |
| `padding` `padding-top` `padding-right` `padding-bottom` `padding-left` |      | 内边距                          |
| `border`                                                     |      | 边框简写                        |
| `border-width` `border-style` `border-color`                 |      | 边框宽/样式/颜色                |
| `border-top/right/bottom/left`                               |      | 各方向边框                      |
| `border-top/right/bottom/left-width/style/color`             |      | 各方向边框分量                  |
| `border-radius`                                              |      | 圆角简写                        |
| `border-top-left/right-radius` `border-bottom-left/right-radius` |      | 各角圆角                        |
| `border-image` 及 `-source/-slice/-width/-outset/-repeat`    |      | 图片边框                        |
| `border-collapse`                                            |      | 表格边框合并(collapse/separate) |
| `border-spacing`                                             |      | 表格边框间距                    |
| `box-shadow`                                                 |      | 盒阴影                          |
| `box-decoration-break`                                       |      | 分片时装饰处理(slice/clone)     |
| `outline` `outline-width` `outline-style` `outline-color` `outline-offset` |      | 轮廓(不占空间)                  |

## 6. 颜色与背景

| 特性                    | htui | 说明                                 |
| ----------------------- | :--: | ------------------------------------ |
| `color`                 |      | 前景文字色                           |
| `opacity`               |      | 不透明度(0–1)                        |
| `background`            |      | 背景简写                             |
| `background-color`      |      | 背景色                               |
| `background-image`      |      | 背景图                               |
| `background-repeat`     |      | 平铺方式                             |
| `background-position`   |      | 位置                                 |
| `background-size`       |      | 尺寸(cover/contain/长度)             |
| `background-attachment` |      | 滚动方式(scroll/fixed/local)         |
| `background-clip`       |      | 绘制区域(border/padding/content-box) |
| `background-origin`     |      | 定位参考区                           |
| `background-blend-mode` |      | 背景混合模式                         |
| `mix-blend-mode`        |      | 元素与下层混合模式                   |
| `caret-color`           |      | 光标颜色                             |

## 7. 字体

| 特性                                                         | htui | 说明                                |
| ------------------------------------------------------------ | :--: | ----------------------------------- |
| `font`                                                       |      | 字体简写                            |
| `font-family`                                                |      | 字体族                              |
| `font-size`                                                  |      | 字号                                |
| `font-size-adjust`                                           |      | 字号调节(按 x-height)               |
| `font-weight`                                                |      | 字重(100–900)                       |
| `font-style`                                                 |      | 字形(normal/italic/oblique)         |
| `font-stretch` `font-width`                                  |      | 字宽(condensed/expanded 等)         |
| `font-variant`                                               |      | 字体变体简写                        |
| `font-variant-alternates`                                    |      | 备选字形                            |
| `font-variant-caps`                                          |      | 大写形式(small-caps 等)             |
| `font-variant-east-asian`                                    |      | 东亚字形变体                        |
| `font-variant-emoji`                                         |      | emoji 表现(text/emoji)              |
| `font-variant-ligatures`                                     |      | 连字                                |
| `font-variant-numeric`                                       |      | 数字形式(tnum, frac 等)             |
| `font-variant-position`                                      |      | 上下标字形                          |
| `font-feature-settings`                                      |      | OpenType 特性开关                   |
| `font-variation-settings`                                    |      | 可变字体轴                          |
| `font-kerning`                                               |      | 字偶距调整                          |
| `font-language-override`                                     |      | 语言专用字形                        |
| `font-optical-sizing`                                        |      | 光学尺寸                            |
| `font-palette`                                               |      | colored font 调色板                 |
| `font-synthesis` `font-synthesis-weight/style/small-caps/position` |      | 合成字体(加粗/斜体/小型大写/上下标) |

## 8. 文本

| 特性                                                         | htui | 说明                                   |
| ------------------------------------------------------------ | :--: | -------------------------------------- |
| `text-align` `text-align-all` `text-align-last`              |      | 对齐(左/右/中/两端)                    |
| `text-decoration` `text-decoration-line` `text-decoration-style` `text-decoration-color` |      | 文本装饰线                             |
| `text-underline-position`                                    |      | 下划线位置                             |
| `text-emphasis` `text-emphasis-style` `text-emphasis-color` `text-emphasis-position` |      | 着重号                                 |
| `text-indent`                                                |      | 首行缩进                               |
| `text-justify`                                               |      | 两端对齐方式                           |
| `text-orientation`                                           |      | 文字朝向(mixed/upright/sideways)       |
| `text-overflow`                                              |      | 溢出省略(ellipsis/clip)                |
| `text-shadow`                                                |      | 文字阴影                               |
| `text-transform`                                             |      | 大小写(uppercase/lowercase/capitalize) |
| `text-combine-upright`                                       |      | 竖排中横排组合                         |
| `letter-spacing`                                             |      | 字母间距                               |
| `word-spacing`                                               |      | 词间距                                 |
| `word-break` `line-break`                                    |      | 换行规则                               |
| `word-wrap` `overflow-wrap`                                  |      | 溢出换行                               |
| `line-height`                                                |      | 行高                                   |
| `white-space`                                                |      | 空白处理(normal/pre/nowrap/pre-wrap)   |
| `hyphens`                                                    |      | 连字符断行                             |
| `tab-size`                                                   |      | 制表符宽度                             |
| `hanging-punctuation`                                        |      | 标点悬挂                               |
| `quotes`                                                     |      | 引号字符                               |
| `direction`                                                  |      | 文字方向(ltr/rtl)                      |
| `writing-mode`                                               |      | 书写模式(horizontal-tb/vertical-rl/lr) |
| `unicode-bidi`                                               |      | 双向算法覆盖                           |
| `vertical-align`                                             |      | 垂直对齐(baseline/top/middle/bottom)   |

## 9. 表格

| 特性           | htui | 说明                    |
| -------------- | :--: | ----------------------- |
| `table-layout` |      | 布局算法(auto/fixed)    |
| `caption-side` |      | 标题位置(top/bottom)    |
| `empty-cells`  |      | 空单元格边框(show/hide) |

(border-collapse / border-spacing 见 §5)

## 10. 列表与生成内容

| 特性                                | htui | 说明                                  |
| ----------------------------------- | :--: | ------------------------------------- |
| `list-style`                        |      | 列表样式简写                          |
| `list-style-type`                   |      | 标记类型(disc/circle/decimal/none 等) |
| `list-style-position`               |      | 标记位置(inside/outside)              |
| `list-style-image`                  |      | 标记图片                              |
| `counter-reset` `counter-increment` |      | 计数器重置/递增                       |
| `content`                           |      | 生成内容(::before/::after 用)         |

## 11. 动画与过渡

| 特性                         | htui | 说明                                     |
| ---------------------------- | :--: | ---------------------------------------- |
| `animation`                  |      | 动画简写                                 |
| `animation-name`             |      | @keyframes 名称                          |
| `animation-duration`         |      | 持续时间                                 |
| `animation-timing-function`  |      | 时间函数(ease/linear/cubic-bezier/steps) |
| `animation-delay`            |      | 延迟                                     |
| `animation-iteration-count`  |      | 循环次数(infinite)                       |
| `animation-direction`        |      | 方向(normal/reverse/alternate)           |
| `animation-fill-mode`        |      | 填充模式(forwards/backwards/both)        |
| `animation-play-state`       |      | 播放状态(running/paused)                 |
| `transition`                 |      | 过渡简写                                 |
| `transition-property`        |      | 参与过渡的属性                           |
| `transition-duration`        |      | 持续时间                                 |
| `transition-timing-function` |      | 时间函数                                 |
| `transition-delay`           |      | 延迟                                     |

## 12. 变换

| 特性               | htui | 说明                                           |
| ------------------ | :--: | ---------------------------------------------- |
| `transform`        |      | 2D/3D 变换(translate/scale/rotate/skew/matrix) |
| `transform-origin` |      | 变换原点                                       |
| `transform-box`    |      | 变换参考盒                                     |

## 13. 滤镜 / 遮罩 / 形状

| 特性                                                         | htui | 说明                                                |
| ------------------------------------------------------------ | :--: | --------------------------------------------------- |
| `filter`                                                     |      | 滤镜(blur/brightness/contrast/drop-shadow/sepia 等) |
| `flood-color` `flood-opacity`                                |      | 滤镜泛光色/不透明度                                 |
| `lighting-color`                                             |      | 滤镜光照色                                          |
| `color-interpolation-filters`                                |      | 滤镜颜色插值空间                                    |
| `clip`                                                       |      | 旧裁剪(已被 clip-path 取代)                         |
| `clip-path`                                                  |      | 裁剪路径                                            |
| `clip-rule`                                                  |      | 裁剪填充规则                                        |
| `mask` `mask-image` `mask-mode` `mask-repeat` `mask-position` `mask-size` `mask-clip` `mask-composite` `mask-origin` `mask-type` |      | 遮罩                                                |
| `mask-border` 及 `-source/-slice/-width/-outset/-repeat/-mode` |      | 边框遮罩                                            |
| `shape-outside`                                              |      | 文字环绕形状                                        |
| `shape-margin`                                               |      | 环绕边距                                            |
| `shape-image-threshold`                                      |      | 图像形状阈值                                        |
| `image-rendering`                                            |      | 图片缩放算法                                        |
| `image-orientation`                                          |      | 图片方向校正                                        |

## 14. 滚动与吸附

| 特性                                                         | htui | 说明                           |
| ------------------------------------------------------------ | :--: | ------------------------------ |
| `scroll-snap-type`                                           |      | 吸附类型与轴向                 |
| `scroll-snap-align`                                          |      | 项目吸附对齐(start/end/center) |
| `scroll-snap-stop`                                           |      | 强制停留(always/normal)        |
| `scroll-padding` 及 `-top/-right/-bottom/-left/-block/-inline` 等 |      | 滚动容器内边距(吸附偏移)       |
| `scroll-margin` 及 `-top/-right/-bottom/-left/-block/-inline` 等 |      | 滚动子项外边距                 |

## 15. UI 与光标

| 特性                | htui | 说明               |
| ------------------- | :--: | ------------------ |
| `cursor`            |      | 鼠标光标           |
| `resize`            |      | 是否可拖拽调整大小 |
| `user-select`(原型) |      | 文字可选           |

## 16. 分页 / 分片

| 特性                                                       | htui | 说明                        |
| ---------------------------------------------------------- | :--: | --------------------------- |
| `break-before` `break-after` `break-inside`                |      | 分片控制(page/column/avoid) |
| `page-break-before` `page-break-after` `page-break-inside` |      | 旧版分页(已被 break-* 取代) |
| `orphans` `widows`                                         |      | 页底/页顶最少行数           |

## 17. 语音 / 听觉(CSS2 遗留,大多未实现)

| 特性                                                         | htui | 说明                    |
| ------------------------------------------------------------ | :--: | ----------------------- |
| `azimuth` `elevation`                                        |      | 声源水平/垂直角度       |
| `cue` `cue-before` `cue-after`                               |      | 提示音                  |
| `pause` `pause-before` `pause-after`                         |      | 停顿                    |
| `rest` `rest-before` `rest-after`                            |      | 间歇                    |
| `speak` `speak-as`                                           |      | 是否/如何朗读           |
| `speak-header` `speak-numeral` `speak-punctuation`           |      | 表头/数字/标点朗读      |
| `speech-rate` `voice-rate`                                   |      | 语速                    |
| `pitch` `pitch-range` `voice-pitch` `voice-range`            |      | 音调                    |
| `voice-family`                                               |      | 朗读语音族              |
| `voice-balance` `voice-duration` `voice-stress` `voice-volume` |      | 声道平衡/时长/重音/音量 |
| `volume`                                                     |      | 音量                    |
| `richness` `stress`                                          |      | 声音丰富度 / 重音强度   |
| `play-during`                                                |      | 背景音                  |
| `glyph-orientation-vertical`                                 |      | 竖排字形朝向(遗留)      |

## 18. At-Rule(规则)

| 规则                                                         | htui | 说明                                   |
| ------------------------------------------------------------ | :--: | -------------------------------------- |
| `@import`                                                    |      | 导入外部样式表                         |
| `@charset`                                                   |      | 样式表字符编码                         |
| `@namespace`                                                 |      | XML 命名空间前缀                       |
| `@media`                                                     |      | 媒体查询条件块                         |
| `@supports`                                                  |      | 特性支持查询条件块                     |
| `@keyframes`                                                 |      | 动画关键帧定义                         |
| `@font-face`                                                 |      | 自定义字体资源                         |
| `@font-feature-values`                                       |      | 字体特性命名预设                       |
| `@font-palette-values`                                       |      | colored font 调色板预设                |
| `@counter-style`                                             |      | 自定义列表计数器样式                   |
| `@stylistic` `@styleset` `@character-variant` `@swash` `@ornaments` `@annotation` `@historical-forms` |      | font-feature-values 下的字形变体子规则 |

## 19. 选择器(Selector Level 3 官方定义)

### 19.1 简单与组合选择器

| 选择器     | htui | 说明     |
| ---------- | :--: | -------- |
| `*`        |      | 通用     |
| `E` (类型) |      | 元素类型 |
| `.class`   |      | 类       |
| `#id`      |      | id       |
| `E F`      |      | 后代     |
| `E > F`    |      | 子代     |
| `E + F`    |      | 相邻兄弟 |
| `E ~ F`    |      | 后续兄弟 |
| `E,A`      |      | 分组     |

### 19.2 伪类

| 伪类                                             | htui | 说明                   |
| ------------------------------------------------ | :--: | ---------------------- |
| `:root`                                          |      | 文档根                 |
| `:empty`                                         |      | 无子节点               |
| `:first-child` `:last-child` `:only-child`       |      | 第一个/最后/唯一子元素 |
| `:first-of-type` `:last-of-type` `:only-of-type` |      | 同类型首/末/唯一       |
| `:nth-child()` `:nth-last-child()`               |      | 第 n 个子元素(正/倒序) |
| `:nth-of-type()` `:nth-last-of-type()`           |      | 同类型第 n 个          |
| `:not()`                                         |      | 反选                   |
| `:lang()`                                        |      | 语言匹配               |
| `:link` `:visited`                               |      | 未访问/已访问链接      |
| `:active` `:hover` `:focus`                      |      | 用户交互状态           |
| `:target`                                        |      | URL 锚点目标           |
| `:enabled` `:disabled`                           |      | 表单启用/禁用          |
| `:checked`                                       |      | 勾选状态               |

### 19.3 伪元素

| 伪元素               | htui | 说明     |
| -------------------- | :--: | -------- |
| `::before` `::after` |      | 生成内容 |
| `::first-line`       |      | 首行     |
| `::first-letter`     |      | 首字母   |

## 20. 值类型与函数

| 类型/函数                                                    | htui | 说明                                           |
| ------------------------------------------------------------ | :--: | ---------------------------------------------- |
| `<length>`                                                   |      | 长度(px/em/rem/vw/vh/cm…)                      |
| `<percentage>`                                               |      | 百分比                                         |
| `<color>`                                                    |      | 颜色(named/hex/rgb()/hsl()/oklch()/color()…)   |
| `<angle>`                                                    |      | 角度(deg/turn/rad/grad)                        |
| `<time>` `<frequency>`                                       |      | 时间 / 频率                                    |
| `<resolution>`                                               |      | 分辨率(dpi/dpcm/dppx)                          |
| `<url>`                                                      |      | 资源引用                                       |
| `<integer>` `<number>`                                       |      | 整数 / 数值                                    |
| `calc()`                                                     |      | 数学运算                                       |
| `min()` `max()` `clamp()`                                    |      | 比较取值                                       |
| `var()`                                                      |      | 自定义属性变量                                 |
| `--*`                                                        |      | 自定义属性                                     |
| `attr()`                                                     |      | 取 HTML 属性值                                 |
| `linear-gradient()` `radial-gradient()` `conic-gradient()`   |      | 渐变                                           |
| `image()` `image-set()`                                      |      | 图像选择                                       |
| `cross-fade()`                                               |      | 图像混合                                       |
| `transform` 函数                                             |      | translate/scale/rotate/skew/matrix/perspective |
| `blur()` `brightness()` `contrast()` `drop-shadow()` `grayscale()` `hue-rotate()` `invert()` `opacity()` `saturate()` `sepia()` |      | filter 滤镜函数                                |
| `color-mix()` `relative color`                               |      | 颜色混合 / 相对颜色                            |
| `easing` `linear()` `cubic-bezier()` `steps()`               |      | 缓动函数                                       |
| `env()`                                                      |      | 环境变量(safe-area-inset 等)                   |
| `counter()` `counters()`                                     |      | 计数器值                                       |
| `symbols()`                                                  |      | 符号计数                                       |
| `ray()` `offset-path`                                        |      | motion path 路径                               |
| `toggle()`                                                   |      | 循环值                                         |
| `local()` `format()`                                         |      | @font-face 字体引用                            |

## 21. 全局关键字(可用于任意属性)

| 关键字         | htui | 说明                              |
| -------------- | :--: | --------------------------------- |
| `initial`      |      | 重置为规范初始值                  |
| `inherit`      |      | 强制继承父值                      |
| `unset`        |      | 继承属性取 inherit,否则取 initial |
| `revert`       |      | 回退到浏览器默认                  |
| `revert-layer` |      | 回退到上一层 cascade layer        |
| `all`          |      | 一次性重置所有属性                |