# 布局管线（两阶段）

> 本文聚焦于重构后的 `src/layout/` 模块。读完本文应能完整理解：
> - `LayoutBox` / `Rect` / `BoxKind` / `Cursor` / `BoxConstraint` 的语义与生命周期。
> - `layout_tree()` 与 `paint_layout_tree()` 的输入、输出、错误约定。
> - 每个元素 `LayoutFn` 的"声明 + 委托"模式。
> - 如何排查"位置不对 / 背景错 / 文字错位"等问题。

---

## 1. 总体定位

布局管线的职责：**把 DOM 树（带 CSS）翻译成"虚拟画布上若干矩形 + 矩形里该写什么文字/颜色"**。

输入：
- `&NodeArena` —— DOM 树。
- `NodeId` —— 根节点（约定为 `NodeType::Global`）。
- `&TermMap` —— 终端尺寸 `(columns, rows)`。
- 隐式：每个节点上已挂好的 `css_attr`（由 `css_into_node` 完成）。

输出：
- `TermLayout.layout: Vec<Block>` —— 已按 `Rect` 写入虚拟画布。
- `TermLayout.rendered_rows` —— 实际用到的行数（用于滚动计算）。

中间状态：
- `Vec<LayoutBox>` —— 由 `layout_tree()` 产生，被 `paint_layout_tree()` 消费。

---

## 2. 两阶段管线

### 2.1 为什么是两阶段

旧实现的问题（详见 `AGENTS.md`）：

| 问题 | 表现 | 根因 |
| --- | --- | --- |
| 子元素读不到自己的 `x`/`width` | `text-align` 用终端宽度而非块宽算 padding | 所有 `LayoutFn` 共享 `TermLayout.point`，没有"我所在的盒"概念 |
| `<aside>` 无法渲染子块 | `aside` 用 `collect_all_text()` 把子树压成一段文本 | 没有子盒的"位置 → 绘制"通路 |
| 嵌套 div 背景被覆盖 | 没有 bg 的 div 也可能"擦掉"外层 bg | 旧的 `fill_to_col` 行为耦合 |

两阶段管线的核心思路：**先在纯内存中算出所有矩形（布局阶段），再按矩形写画布（绘制阶段）**。两阶段之间传递的是 `LayoutBox`，不涉及任何共享光标。

### 2.2 调用链

```
TermLayout::draw(arena, root_id, term_map)
   │
   │   构造 BoxConstraint::root(term_map) = (0, columns)
   │   构造 Cursor { y: 0 }
   │
   ├──► layout_tree(arena, root_id, containing, &mut cursor) ──► Vec<LayoutBox>
   │
   └──► paint_layout_tree(&boxes, arena, &mut self) ──► self.layout 已写入
                                                  ──► self.rendered_rows 更新
```

---

## 3. 数据类型

### 3.1 `Rect`

```rust
// src/layout/box_tree.rs
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}
```

- 坐标单位：**列 / 行**（不是像素）。
- `x ∈ [0, columns)`，`y ∈ [0, ∞)`，但实际画布按 `cols * rows * 20` 上限截断。
- `width == 0` 表示空盒（绘制阶段跳过）。
- `height == 0` 在块级盒中不会出现（块级 `height ≥ 1`）。

### 3.2 `BoxKind`

```rust
pub enum BoxKind {
    Block { fill_bg: bool, spacing_after: usize },
    Image  { cols: usize, rows: usize },
}
```

- `Block.fill_bg`：
  - `true` → 绘制时把整个 `Rect` 涂上 `background-color`（或 `aside` 用的无条件填白）。
  - `false` → 不涂背景，只写文字（嵌套场景下保留外层 bg）。
- `Block.spacing_after`：
  - 在 `Rect.height` 之后额外保留的行数（`<header>` / `<footer>` 视觉留白 = 2）。
- `Image.cols` / `Image.rows`：栅格化后的字符单元尺寸。

### 3.3 `LayoutBox`

```rust
pub struct LayoutBox {
    pub node_id: NodeId,
    pub rect: Rect,
    pub kind: BoxKind,
    pub text: Option<String>,        // 折叠后的直接文本（不含嵌套块）
    pub image_path: Option<String>,  // <img> 用
    pub children: Vec<LayoutBox>,    // 嵌套块（按从上到下顺序）
}
```

- `text` 为 `None`：纯结构容器（如 `<aside><div>...</div></aside>` 的 aside）。
- `text` 为 `Some("")`：构造时显式置空，绘制时跳过。
- `children` 里的盒 **从父盒第一行下方** 开始堆叠；它们使用父盒的 `rect.x / rect.width` 作为新 `containing`。

### 3.4 `Cursor`

```rust
pub struct Cursor { pub y: usize }

impl Cursor {
    pub fn allocate(&mut self, height: usize) -> usize;  // 预占 height 行，返回起始 y
    pub fn peek(&self) -> usize;                          // 只看不改
}
```

- **兄弟块共享同一个 `Cursor`**，由 `layout_tree` 通过 `&mut` 传入。
- 子盒递归时需要**不消耗父盒的行号**——所以子盒用 `Cursor { y: cursor.peek() }` 创建快照，写完后再 `cursor.y = child_cursor.y` 同步。
- 所有 `LayoutFn` **必须**通过 `cursor.allocate(...)` 取得 `y`，禁止直接读 `cursor.y` 来"猜"。

### 3.5 `BoxConstraint`

```rust
pub struct BoxConstraint { pub x: usize, pub width: usize }

impl BoxConstraint {
    pub fn root(term_map: &TermMap) -> Self;
    pub fn centered(self, width: usize) -> Self;
}
```

- 是子盒的"水平视域"：决定了 `text-align` 的有效宽度、`background-color` 的填充范围。
- 父盒在递归子节点时构造：
  ```rust
  let child_containing = BoxConstraint { x: rect.x, width: rect.width };
  ```
- CSS `width: Npx / N% / auto` → `resolve_block_width(css_width, containing.width)`：
  - `auto` 或缺失 → `containing.width`。
  - `Npx` → `N`。
  - `N%` → `(containing.width * N / 100)`。
  - 一律 `min(value, containing.width)`。
- 居中左偏移 → `centered_left(containing, block_width)`。

### 3.6 `FormattingContext`

`FormattingContext { containing: BoxConstraint }` 是 `BoxConstraint` 的兼容封装，保留 `LayoutFn` 老签名语义。新的 `LayoutFn` 直接收 `BoxConstraint`，不再使用 `FormattingContext`。`FormattingContext` 仅在 `graph_api.rs::render_text` 旧函数中存在。

---

## 4. 阶段一：`layout_tree()`

### 4.1 函数签名

```rust
// src/layout/tree.rs
pub fn layout_tree(
    arena: &NodeArena,
    root_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Vec<LayoutBox>>;
```

返回值：根节点的**块级子节点们**布局好的 `Vec<LayoutBox>`。注意：

- **`root_id` 本身不会被布局**——只迭代 `arena[root_id].children`。
- 块级子节点由 `DEFAULT_LAYOUT` 注册表派发（第一个返回 `Some(b)` 的胜出）。
- 未识别的节点（`None` / `Style` / `Link` 等）被静默跳过。

### 4.2 派发逻辑

```rust
fn dispatch(arena, node_id, containing, cursor) -> Result<Option<LayoutBox>> {
    for layout_fn in DEFAULT_LAYOUT {
        if let Some(b) = layout_fn(arena, node_id, containing, cursor)? {
            return Ok(Some(b));
        }
    }
    Ok(None)  // 无 handler
}
```

注册顺序就是优先级顺序。当前顺序（`src/layout/mod.rs`）：

```rust
pub static DEFAULT_LAYOUT: &[LayoutFn] = &[
    layout_fn::p::layout_p,
    layout_fn::head::layout_hn,
    layout_fn::img::layout_img,
    layout_fn::div::layout_div,
    layout_fn::header_footer::layout_header,
    layout_fn::header_footer::layout_footer,
    layout_fn::aside::layout_aside,
];
```

顺序原则：更具体的标签放前面。`p` / `h1-h5` 都是独立标签，不重叠；`div` 必须排在 `aside` / `header` / `footer` 之前以便未来若需 fallback 时生效（目前不重叠）。

### 4.3 `LayoutFn` 的契约

```rust
pub type LayoutFn = fn(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>>;
```

每个 `LayoutFn` 必须遵守：

1. **类型守门**：若 `arena[node_id].node_type` 不匹配，返回 `Ok(None)`（让下一个注册项接手）。
2. **预占行号**：通过 `let y = cursor.allocate(height);` 取得 `y`，**绝不能** 直接修改 `cursor.y`。
3. **递归子级**：调用 `layout_children(arena, &node.children.clone(), child_containing, &mut child_cursor)`，写完把 `cursor.y` 同步到 `child_cursor.y`。
4. **不要访问 DOM 的 `<style>` / `<link>` / `<script>` 节点**：这些是 `Style` / `Link` 类型，应当被跳过。
5. **不要直接修改 `TermLayout`**：这是绘制阶段的事。

### 4.4 元素实例：`<p>`

```rust
// src/layout/layout_fn/p.rs
pub fn layout_p(arena, node_id, containing, cursor) -> Result<Option<LayoutBox>> {
    let node = &arena[node_id];
    if node.node_type != NodeType::P {
        return Ok(None);
    }

    let block_width = resolve_block_width(node.css_attr.get_width(containing.width), containing.width);
    let x = centered_left(containing, block_width);
    let y = cursor.allocate(1);

    let text = {
        let raw = collect_direct_text(arena, node);
        if raw.is_empty() { None } else { Some(raw) }
    };

    // 子块在父盒的"矩形"内堆叠，复用父盒的 containing。
    let child_containing = BoxConstraint { x, width: block_width };
    let mut child_cursor = Cursor { y: cursor.peek() };
    let children = layout_children(arena, &node.children.clone(), child_containing, &mut child_cursor)?;
    cursor.y = child_cursor.y;

    Ok(Some(LayoutBox::new_block(node_id, Rect::new(x, y, block_width, 1), false, 0, text, children)))
}
```

要点：

- `fill_bg = false`：`<p>` 默认不画背景。
- `text` 仅收集直接文本（`<span>` 会被展平），嵌套块不进 `text`。
- 子盒的 `containing` 使用 `p` 自己的 `x / width`，不是终端宽度——这是 containing block 传递的核心。

### 4.5 元素实例：`<aside>`

```rust
pub fn layout_aside(...) {
    if node.node_type != NodeType::Aside { return Ok(None); }
    let block_width = resolve_block_width(...);
    let x = centered_left(containing, block_width);
    let y = cursor.allocate(1);
    let text = collect_direct_text(...);
    let child_containing = BoxConstraint { x, width: block_width };
    let mut child_cursor = Cursor { y: cursor.peek() };
    let children = layout_children(...)?;
    cursor.y = child_cursor.y;

    // 裹住嵌套块：rect.height 随子盒深度增长，最低为 1。
    let height = cursor.y.saturating_sub(y).max(1);

    Ok(Some(LayoutBox::new_block(
        node_id,
        Rect::new(x, y, block_width, height),
        true,        // fill_bg：即使 bg=None 也涂"实心条带"
        0,           // spacing_after：aside 不用额外间距
        text,
        children,
    )))
}
```

要点：

- **`fill_bg = true`** 写死——这是历史视觉约定：`<aside>` 即使没有 `background-color` 也呈现"实心条带"。
- **`rect.height` 动态计算**：先 `cursor.allocate(1)` 预占本行，然后递归 `layout_children`，最后 `height = (cursor.y - y).max(1)`。这样 `<aside><p>X</p></aside>` 会得到 `height = 2`，背景同时涂满 row 0 / row 1；空 aside 仍是 `height = 1`。
- **`spacing_after = 0`**：aside 不像 header/footer 那样在下面留空白行。

### 4.6 元素实例：`<img>`

```rust
pub fn layout_img(...) {
    if node.node_type != NodeType::Img { return Ok(None); }
    let Some(src) = node.attributes.get_src().cloned() else { return Ok(None); };
    let max_w = node.attributes.get_width().unwrap_or(containing.width as u32 * 2);
    let max_h = node.attributes.get_height().unwrap_or(containing.width as u32 * 4);
    let img_cols = max_w.div_ceil(2).max(1) as usize;  // 半格字符
    let img_rows = max_h.div_ceil(4).max(1) as usize;  // 四分之一格字符
    let block_width = img_cols.min(containing.width);
    let x = centered_left(containing, block_width);
    let y = cursor.allocate(img_rows);
    Ok(Some(LayoutBox::new_image(node_id, Rect::new(x, y, block_width, img_rows), img_cols, img_rows, src)))
}
```

- 半格字符：每个字符代表 2 列像素。`max_w / 2` = 字符列数。
- 四分之一格字符：每个字符代表 4 行像素。`max_h / 4` = 字符行数。
- 图像永远在自己的 `Rect` 内，不被子盒"撞穿"。

### 4.7 公共辅助

```rust
// src/layout/tree.rs
pub fn collect_direct_text(arena, node) -> String;  // 仅折叠 <span>，不进块
pub fn resolve_block_width(css_width, containing_width) -> usize;
pub fn centered_left(containing, block_width) -> usize;
```

`collect_direct_text` 是文本折叠的安全版本：只跨过 `<span>`，**绝不**跨过 `<div>` / `<p>` 等块级元素——否则会与子盒绘制冲突。

---

## 5. 阶段二：`paint_layout_tree()`

### 5.1 函数签名

```rust
// src/layout/paint.rs
pub fn paint_layout_tree(
    boxes: &[LayoutBox],
    arena: &NodeArena,
    term_layout: &mut TermLayout,
) -> Result<()>;
```

遍历每个 `LayoutBox`，按 `Rect` 直接写入 `term_layout.layout[]`，**不依赖**任何共享 `point` 光标。

`parent_bg: Option<Color>` 从顶层递归传入 `paint_box → paint_block → paint_text_row` ，是“文字继承父级 bg”的通路（详见 §7.5）。

### 5.2 块盒绘制

```rust
fn paint_block(lb, arena, term_layout, fill_bg, spacing_after) -> Result<()> {
    let node = &arena[lb.node_id];
    let fg = node.css_attr.get_color();
    let bg = node.css_attr.get_bg_color();
    let bold = matches!(node.css_attr.get_font_weight(), FontWeight::Bold);
    let align = node.css_attr.get_text_align();

    let r = lb.rect;
    let cols = term_layout.columns as usize;
    let cell_start = r.y * cols + r.x;

    if fill_bg {
        // 涂多行；rows_to_fill = height - spacing_after，所以 <header>/<footer>
        // 的 "spacing 行为" 行保持空白。aside 即便 bg=None 也涂（让"实心条带"生效）。
        let bg_to_use = bg;
        let rows_to_fill = r.height.saturating_sub(spacing_after);
        for row_offset in 0..rows_to_fill {
            let row_cell_start = cell_start + row_offset * cols;
            for i in 0..r.width {
                let idx = row_cell_start + i;
                if idx < term_layout.layout.len() {
                    term_layout.layout[idx] = Block::new(" ".into(), fg, bg_to_use, bold);
                }
            }
        }
    }

    if let Some(text) = &lb.text {
        if !text.is_empty() {
            let text_w = text.chars().map(char_width).sum::<usize>().min(r.width);
            let offset = match align {
                TextAlign::Left   => 0,
                TextAlign::Center => (r.width - text_w) / 2,
                TextAlign::Right  => r.width - text_w,
            };
            paint_text_row(term_layout, cell_start + offset, text, fg, bg, bold);
        }
    }
    Ok(())
}
```

关键点：

- **`text-align` 在 `rect.width` 内计算**，不是终端宽度。这就是为什么 `<aside width:40px><p style="text-align:center">X</p></aside>` 的 `X` 会落在 `aside_left + (40-1)/2` 而不是 `(80-1)/2`。
- 宽字符（CJK）：占 2 列。`paint_text_row` 处理了右半格的占位（`Block::new(String::new(), ...)`），渲染时会被跳过。
- 涂背景不会"擦掉"外层 bg——因为子盒写入的是它**自己的** `Rect` 范围。
- **多行涂背景**：`fill_bg = true` 时涂满 `rect.height - spacing_after` 行。`<header>/<footer>` 的 `spacing_after = 2` 会保留最后两行空白；`<aside>` `spacing_after = 0` + `rect.height >= 2`（嵌套块）会让背景包裹所有子块行。

### 5.3 图像绘制

```rust
fn paint_image(lb, term_layout, cols, rows) -> Result<()> {
    let Some(path) = lb.image_path.as_ref() else { return Ok(()) };
    paint_image_at(term_layout, path, (lb.rect.x, lb.rect.y), (cols as u32, rows as u32))
}
```

`paint_image_at` 在 `src/tui/graph_api.rs` 中：复用 `Img::_create_img` 的半格/四分之一格字符采样，但**不再依赖 `TermLayout.point`**——按 `(x, y)` 起点直接画。

### 5.4 `rendered_rows`

```rust
let mut max_y = 0usize;
for b in boxes {
    paint_box(b, ...)?;
    let bottom = b.rect.y + b.rect.height.max(1);
    if bottom > max_y { max_y = bottom; }
}
if max_y as u32 > term_layout.rendered_rows {
    term_layout.rendered_rows = max_y as u32;
}
```

`rendered_rows` 是滚动/缩放辅助需要用到的字段。这里用**实际用到的最低底行**更新它，避免老的"光标走到哪里就以为有多少行"的副作用。

---

## 6. 元素 → `LayoutFn` 速查

| 元素 | `LayoutFn` | 关键差异 |
| --- | --- | --- |
| `<p>` | `layout_p` | `fill_bg = false`，固定 1 行 |
| `<h1>` - `<h5>` | `layout_hn` | 文本前缀 `"# "` / `"## "` …，含在 `text-align` 宽度里 |
| `<div>` | `layout_div` | `fill_bg = css.bg.is_some()`，固定 1 行 |
| `<header>` | `layout_header` | `fill_bg = true`，`spacing_after = 2`，`height = 1 + spacing_after` |
| `<footer>` | `layout_footer` | 同 header |
| `<aside>` | `layout_aside` | `fill_bg = true`（无条件），`height = max(1, cursor.y - y)` 以裹住嵌套块 |
| `<img>` | `layout_img` | `BoxKind::Image`，按 2×4 像素单元换算字符行列 |
| `<span>` | （无注册） | 在 `collect_direct_text` 中被展平 |

`<aside>` 的高度随嵌套块动态增长，这是为了在绘制阶段让背景（`fill_bg = true`）覆盖所有子盒行——反之 `<div>` / `<header>` / `<footer>` 默认只占一行，背景也仅充充一行；`<header>`/`<footer>` 额外靠 `spacing_after` 保留空白行。

---

## 7. 常见陷阱

### 7.1 子盒消耗父盒的行号

错误写法：
```rust
let y = cursor.allocate(1);
let mut child_cursor = *cursor;       // ← 拷贝了 y，但父块也要用
let children = layout_children(arena, &node.children.clone(), child_containing, &mut child_cursor)?;
// 忘记把 child_cursor.y 同步回 cursor，导致父块的"我占用了多少行"信息丢失
```

正确写法（参考 `div.rs`）：
```rust
let y = cursor.allocate(1);
let mut child_cursor = Cursor { y: cursor.peek() };  // 快照，不消耗
let children = layout_children(arena, &node.children.clone(), child_containing, &mut child_cursor)?;
cursor.y = child_cursor.y;                            // 同步最大底行
```

### 7.2 文字折叠跨越块级

错误写法：
```rust
let text = collect_text(arena, node);  // ← 会把 <div> 子块里的文本也吸进来
```

`collect_text` 是 `tdom::node` 提供的，只跨文本节点和 `<span>`——但它对块级子节点的"直接性"判断弱，在嵌套场景下会和子盒绘制冲突。

正确做法：用 `tree::collect_direct_text`（仅跨 `<span>`，不进块）。

### 7.3 修改共享 `TermLayout.point`

旧 `LayoutFn` 的常见做法（**禁止**）：
```rust
term_layout.new_line();
term_layout.paint_string(...);
```

新管线**完全没有 `point` 概念**。`TermLayout.point` 仍存在仅为兼容旧字段，绘制结束后由 `draw()` 同步一次。

### 7.4 `width: auto` 时的居中

`resolve_block_width(None, containing_width)` 返回 `containing_width`——也就是父盒的完整宽度。这意味着 `<div width:auto>` 在父盒内是满宽的，不会缩成一个角落里的窄条。如果想"默认缩到内容宽"，需要另起一个 `fit-content` 策略，目前**未实现**。

### 7.5 嵌套 `<p>` 在 `<aside>` 中的视觉

`<aside width:40px>` 的 `fill_bg=true` 会让 row 0 涂成实心条带；嵌套 `<p>` 在 `text-align: center` 下文字落在 `(aside.x + 20, row 1)`——也就是**独立行**，与 aside 的 bg 重叠（因为 `layout_aside` 会让 `rect.height` 增长到包含所有嵌套块，背景同时涂在 row 0 / row 1）。这是有意设计：每个盒只对自己的 `Rect` 负责，aside 的 bg 覆盖它整个 rect，嵌套 `<p>` 是独立盒。

为了让文字“坐在”父盒 bg 上（不镂空），`paint_box` 把 `effective_bg = own_bg.or(parent_bg)` 下推到 `paint_text_row` 和子盒递归。`parent_bg` = 父盒的 `effective_bg`，递归传递。所以：

- `<aside bg=cyan><p>30col</p></aside>`：`<p>` 文字 bg = `None.or(Some(cyan))` = cyan。✓
- `<aside bg=cyan><p bg=yellow>kid</p></aside>`：`<p>` 文字 bg = `Some(yellow).or(Some(cyan))` = yellow（自己的优先）。
- `<aside bg=blue><div bg=red><p>X</p></div></aside>`：`<p>` 文字 bg = red（离它最近的盒的 bg），不是 blue。
- `<aside><p>foo</p></aside>`（都没 bg）：文字 bg = None（终端默认 bg）。

这与浏览器渲染一致。锁于 `tests/layout_aside_tests.rs::test_nested_p_text_inherits_aside_bg` / `test_nested_p_text_bg_wins_over_parent` / `test_nested_p_inherits_through_intermediate_div` / `test_nested_p_inherits_when_parent_has_no_bg`。

### 7.6 顶层循环不会自动递归——`rendered_rows` 只看一层会偏小

**历史踩坑**：重构后曾因为滚动失效回到这个问题。

`paint_layout_tree` 是递归的，但**如果顶层循环里只对 `boxes` 迭代算 `max_y`，而不下推到 `paint_box` 里累加**，那么只有顶层盒的 `rect.y + rect.height` 被计入。后果：

```rust
// ❌ 错误写法
let mut max_y = 0;
for b in boxes {
    paint_box(b, arena, term_layout)?;     // 递归绘制了 ✓
    let bottom = b.rect.y + b.rect.height; // 只算顶层 ✗
    if bottom > max_y { max_y = bottom; }
}
```

`rendered_rows` 钉在第一个顶层盒的底部 → `max_scroll = rendered_rows - rows = 0 (saturating_sub)` → `clamp_scroll` 把 `scroll_y` 永远钉在 0 → **滚动完全失效**。

正确写法是把累加器下推到递归：

```rust
// ✓ 正确写法
fn paint_box(lb, arena, tl, max_y: &mut usize) -> Result<()> {
    let bottom = lb.rect.y + lb.rect.height.max(1);
    if bottom > *max_y { *max_y = bottom; }
    // … paint …
    for child in &lb.children {
        paint_box(child, arena, tl, max_y)?;
    }
    Ok(())
}
```

为什么旧实现没踩？因为旧 `LayoutFn` 都共享 `TermLayout.point`，`point` 通过 `new_line` / `paint_string` 自然推到最深一行，`rendered_rows = point / cols` 自动反映真实深度。新管线每个盒独立绘制，必须显式递归统计。

单测锁定：`tests/layout_tree_tests.rs::rendered_rows_walks_full_layout_tree` 与 `scroll_limits_match_layout_depth`。

---

## 8. 调试技巧

- 想看某个盒的 `Rect`：在 `paint_box` 入口加 `log::debug!("box {lb:?}")`。
- 想看布局树全貌：在 `layout_tree` 出口 `log::debug!("{:#?}", boxes)`。
- 想看 DOM 的 `css_attr` 是否正确继承：在 `css_into_node` 后遍历 `Node::descendants` 检查 `css_attr.0`。
- 想看宽字符（CJK）是否被正确占位：在 `paint_text_row` 加日志，注意 `idx + 1` 那块 `Block::new(String::new(), ...)`。

---

## 9. 测试

- **基础渲染**：`tests/layout_tests.rs`、`tests/layout_aside_tests.rs`、`tests/layout_div_tests.rs`、`tests/layout_header_footer_tests.rs`。
- **两阶段管线不变量**：`tests/layout_tree_tests.rs`：
  - `two_paragraphs_land_on_different_rows`：块级兄弟独占行。
  - `nested_paragraph_in_aside_is_separate_row`：`<aside>` 子 `<p>` 不被压平。
  - `nested_paragraph_uses_parent_containing_block_for_text_align`：containing block 真正传递。
  - `nested_div_in_div_stacks_below_outer_div`：嵌套块堆叠。
  - `layout_box_rect_is_deterministic`：布局确定性（多次运行 `layout_tree` 矩形一致）。
  - `rendered_rows_walks_full_layout_tree`：递归 `max_y` 不能只看顶层。
  - `scroll_limits_match_layout_depth`：高度溢出终端时滚动必须可用。
- **多行背景锁定**（`tests/layout_aside_tests.rs`）：
  - `test_aside_rect_height_includes_nested_children`：aside 的 `rect.height` 含子盒行。
  - `test_aside_bg_covers_nested_p_row`：row 1 携带 aside 的 bg。
  - `test_aside_bg_covers_two_nested_p_rows`：两个嵌套 `<p>` 都在 aside bg 内。
  - `test_aside_with_nested_div_extends_bg_to_div_row`：嵌套 `<div>` 同样被背景包裹。
  - `test_aside_without_children_still_has_height_one`：空 aside 仍 1 行，避免过度修复。
  - `test_header_spacing_rows_remain_bg_free` / `test_footer_spacing_rows_remain_bg_free`：`<header>/<footer>` 的 `spacing_after` 行必须空白。
- **背景继承锁定**（`tests/layout_aside_tests.rs`）：
  - `test_nested_p_text_inherits_aside_bg`：`<p>` 文字携带 aside 的 cyan bg。
  - `test_nested_p_text_bg_wins_over_parent`：子盒有 bg 时，自己的优先于父级。
  - `test_nested_p_inherits_through_intermediate_div`：穿过中间 `<div bg=red>` 时取 red 而非外层。
  - `test_nested_p_inherits_when_parent_has_no_bg`：根路径上无 bg 时不误继承 None。
  - `test_nested_p_in_aside_with_fixed_width_centered`：复现 `visual_test/aside` 的 30col 场景，文字居中且全 bg。

新增元素时**必须**在这两个文件之一补单测，且**不要**依赖真实终端（用 `TermMap { columns: 80, rows: 24 }` mock）。

---

## 10. 未来方向

- **inline 布局**：当前 `<span>` 仅在 `collect_direct_text` 里被展平。未来需要支持行内 `<span>` 的颜色/粗细差异 → 需要把 `LayoutBox` 拆成"块盒 + 内联盒列表"。
- **layout cache**：增量布局（仅重算 CSS 改变的子树）以减少重绘成本。
- **并行绘制**：各 `LayoutBox` 独立，可并行（`rayon`）。
- **image 缓存**：当前每次 `paint_image_at` 都重新采样，可加 LRU。