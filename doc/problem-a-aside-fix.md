# 问题 A：`<aside>` 位置错位 与 重构后滚动失效

> 本文档是 `architecture.md` / `layout.md` 的姊妹篇，专门讲两个被一起修掉的坑：
> 1. **问题 A**：`<aside>` 在内容上方预留了一个空白行，看起来不像网页。
> 2. **重构后滚动失效**：`rendered_rows` 只算了顶层盒，没递归到最深叶子，`max_scroll()` 塌成 `0`，鼠标滚轮按了不响应。

本文档面向“想理解这次重构动机 / 复现 / 防回归”的读者，按 **症状 → 根因 → 修复 → 防回归测试** 四步展开。

---

## 1. 症状

### 1.1 `<aside>` 多出一行

输入：

```html
<aside style="background-color: darkblue;">
  <p>The rough-skinned newt defends itself with a deadly neurotoxin.</p>
</aside>
```

重构前的渲染：

```
row 0: [darkblue darkblue darkblue ... darkblue]   ← aside 的“自有行”，纯背景
row 1: [The rough-skinned newt ...      darkblue]   ← 嵌套 <p> 的文本行
row 2: [                                  ...    ]   ← row 2 是普通背景
```

而浏览器（用户预期）应该是：

```
row 0: [The rough-skinned newt ...      darkblue]   ← 嵌套 <p> 直接坐在 aside 上
row 1: [                                  ...    ]   ← row 1 之外是普通背景
```

区别：旧的实现多了一个“aside 自有空白行”，把 `bg` 与 `text` 在垂直方向错开了 1 行。

### 1.2 重构后无法滚动

旧布局是“边布局边绘制”，每个 `LayoutFn` 直接写 `term_layout.layout[]` 并维护一个全局 `point` 光标。`point / columns` 就是“已写到的最大行”，`rendered_rows` 沿用同一个值，所以 `max_scroll()` 自然能算对。

新管线分成两阶段：

```
layout_tree()        → Vec<LayoutBox>          （不写画布）
paint_layout_tree()  → 把 LayoutBox 写到画布   （不维护 point）
```

第一次迁移时 `paint_layout_tree` 只对 `boxes`（顶层盒）累加 `max_y`，没有递归到子盒：

```rust
// 错误的初版
for b in boxes {
    paint_box(b, arena, term_layout);    // 忘了把子盒的 bottom 也参与 max_y
}
if max_y as u32 > term_layout.rendered_rows {
    term_layout.rendered_rows = max_y as u32;
}
```

后果：DOM 实际有 30 行内容时，`rendered_rows` 只反映顶层 root box（高度 1），`max_scroll() = (1).saturating_sub(rows) = 0`，滚动条形同虚设。

---

## 2. 根因

### 2.1 `<aside>` 多出一行的根因

旧版 `layout_aside`（来自某次“快速修复”）：

```rust
let y = cursor.allocate(1);                // ① 先给 aside 自己占一行
let mut child_cursor = Cursor { y };
let children = layout_children(...)?;      // ② 子盒从这一行的下一行开始
cursor.y = child_cursor.y;
let height = cursor.y.saturating_sub(y).max(1);  // ③ height 至少是 1
```

它模仿 `<header>`/`<footer>` 的“块级盒子自己一行”的视觉约定，但 `<header>`/`<footer>` 那个约定本来就是为了 header banner / footer 与内容拉开间距而硬编码的；`<aside>` 是个普通内容容器，**不应该**被套上这条规则。

结果就是 ① 步把 y 抬高了一格，② 步把子盒（`<p>`）放到下一格。`rect.height` 因为 `.max(1)` 至少也是 1，所以即使 aside 没子盒，它也会“画一行空 bg”，观感比浏览器“多了一条边”。

### 2.2 滚动失效的根因

`rendered_rows` 在旧管线里由 `paint_string` / `paint_img` 每次写画布时顺带维护：

```rust
// graph_api.rs: paint_string
let current_row = self.point / self.columns as u32;
if current_row > self.rendered_rows { self.rendered_rows = current_row; }
```

新管线把这两条 API 标了 `#[deprecated]`，`paint_layout_tree` 是新的唯一入口，但初版没有把 `max_y` 的累加改成递归。`paint_box` 自己没动 `max_y`，子盒永远不进 `rendered_rows`。

`max_scroll()` 完全依赖 `rendered_rows`：

```rust
pub fn max_scroll(&self) -> u32 {
    let total_lines = self.rendered_rows.max(self.point / self.columns as u32);
    total_lines.saturating_sub(self.rows as u32)
}
```

`rendered_rows = 1`，终端 `rows = 24`，结果 `max_scroll = 0`，`clamp_scroll()` 把 `scroll_y` 钳到 `0`，滚轮事件被忽略。

---

## 3. 修复

### 3.1 修复 `<aside>`：peek 而非 allocate

`src/layout/layout_fn/aside.rs`：

```rust
pub fn layout_aside(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Aside { return Ok(None); }

    let block_width = resolve_block_width(node.css_attr.get_width(containing.width), containing.width);
    let x = centered_left(containing, block_width);
    let text = {
        let raw = collect_direct_text(arena, node);
        if raw.is_empty() { None } else { Some(raw) }
    };

    // Peek the next row without reserving — the aside's children will start
    // here, so the aside's rect should encompass the children's rows rather
    // than sitting on top of an empty "own row".
    let y = cursor.peek();                                 // ← 不再 allocate(1)

    let child_containing = BoxConstraint { x, width: block_width };
    let mut child_cursor = Cursor { y };
    let block_children = node.children.clone();
    let children = layout_children(arena, &block_children, child_containing, &mut child_cursor)?;

    // Aside height equals the number of rows occupied by its children (or 1
    // when empty). This keeps the bg flush with the content — no empty
    // stripe above — and leaves an empty aside at height 1 so siblings don't
    // collide.
    let height = child_cursor.y.saturating_sub(y).max(1);  // ← 仍是 max(1)，保留“空 aside 占 1 行”
    cursor.y = y + height;

    Ok(Some(LayoutBox::new_block(
        node_id,
        Rect::new(x, y, block_width, height),
        true,   // fill_bg
        0,      // spacing_after
        text,
        children,
    )))
}
```

关键变化只有一行：`cursor.allocate(1)` → `cursor.peek()`。`Cursor::peek` 与 `allocate` 的区别是“看一眼不消耗”——`peek()` 只返回当前 `y` 而不推进它，于是子盒从 `y` 开始排，aside 的 `rect` 与第一个子盒共享行首。

注意三件不动的事：

1. **空 aside 仍然 `height = 1`**：`.max(1)` 仍在——空 aside 不该塌成 `height = 0`，否则兄弟块会撞上来。
2. **`fill_bg = true` 仍然写死**：`<aside>` 的“实心条带”视觉约定保留。
3. **`spacing_after = 0`**：aside 下面不留额外空白行。

### 3.2 修复滚动：`paint_box` 递归累加 `max_y`

`src/layout/paint.rs`：

```rust
fn paint_box(
    lb: &LayoutBox,
    arena: &NodeArena,
    term_layout: &mut TermLayout,
    max_y: &mut usize,
    parent_bg: Option<Color>,
) -> Result<()> {
    // 关键：累加当前盒子的底部，不仅仅是顶层
    let bottom = lb.rect.y.saturating_add(lb.rect.height.max(1));
    if bottom > *max_y {
        *max_y = bottom;
    }

    let my_bg = match lb.kind {
        BoxKind::Block { .. } => arena[lb.node_id].css_attr.get_bg_color(),
        BoxKind::Image { .. } => None,
    };
    match lb.kind {
        BoxKind::Block { fill_bg, spacing_after } => {
            paint_block(lb, arena, term_layout, fill_bg, spacing_after, my_bg, parent_bg)?;
        }
        BoxKind::Image { cols, rows } => {
            paint_image(lb, term_layout, cols, rows)?;
        }
    }

    // 背景继承：当前盒若有自己 bg，子盒看到的是它；否则看到父级。
    let effective_bg = my_bg.or(parent_bg);
    for child in &lb.children {
        paint_box(child, arena, term_layout, max_y, effective_bg)?;
    }
    Ok(())
}
```

三个关键点：

1. **`paint_box` 自己累加 `max_y`**：进入即 `bottom = y + max(height, 1)`，保证空盒至少计 1。
2. **`effective_bg = my_bg.or(parent_bg)` 沿递归透传**：让嵌套 `<p>` 在 `<aside bg=cyan>` 内“坐在” cyan 上，而不是默认背景。
3. **`paint_layout_tree` 末尾再把 `max_y` 同步到 `rendered_rows`**：保证旧的 scroll/zoom 辅助字段（`point` / `rendered_rows`）保持一致。

`TermLayout::draw` 在两阶段结束后再补一句：

```rust
self.point = (self.rendered_rows * self.columns as u32).min(self.max_cells() as u32);
```

让已经标 `#[deprecated]` 的 `point` 也跟新管线一致，免得将来 debug 时被骗。

### 3.3 几何对照表

修复后 `<aside><p>...</p></aside>` 的盒子坐标（终端宽 80 列）：

| 盒子 | x | y | width | height |
| --- | --- | --- | --- | --- |
| `<aside>` | 0 | 0 | 80 | 1 |
| `<p>` (child) | 0 | 0 | 80 | 1 |

注意子 `<p>` 与 aside **共享 y=0**，背景层叠关系在绘制阶段由 `paint_block` 的 `bg` 填充 + `paint_text_row` 的文本覆盖实现。

多子元素场景 `<aside><p>first</p><p>second</p></aside>`：

| 盒子 | x | y | width | height |
| --- | --- | --- | --- | --- |
| `<aside>` | 0 | 0 | 80 | 2 |
| `<p>` #1 | 0 | 0 | 80 | 1 |
| `<p>` #2 | 0 | 1 | 80 | 1 |

aside 高度 = 子盒总行数（`child_cursor.y - y`），无空 stripe。

---

## 4. 防回归测试

### 4.1 问题 A（aside 位置）

`tests/layout_aside_tests.rs`：

| 测试名 | 旧断言（错） | 新断言（对） |
| --- | --- | --- |
| `test_aside_bg_covers_nested_p_row` | row 0 全 bg、row 1 有 bg | row 0 有 bg、row 1 **无** bg |
| `test_aside_rect_height_includes_nested_children` | `height == 2`、child.y == 1 | `height == 1`、child.y == aside.y |
| `test_aside_bg_covers_two_nested_p_rows` | loop `0..3` 含 3 行 bg | loop `0..2` 只 2 行 bg |
| `test_aside_with_nested_div_extends_bg_to_div_row` | row 1 有 bg（嵌套 div 的行） | row 0 有 bg（嵌套 div 与 aside 共享） |
| `test_aside_without_children_still_has_height_one` | `height == 1` | `height == 1`（不变，是守卫） |

`tests/layout_tree_tests.rs::nested_paragraph_shares_aside_row_with_bg`（取代旧的 `nested_paragraph_in_aside_is_separate_row`）：

```rust
assert_eq!(
    paragraph_row, aside_row,
    "nested <p> must share the aside's row (no empty stripe above)"
);
```

这条断言直接锁住“aside 不能有自己的行”这个不变量。

### 4.2 滚动恢复

`tests/layout_tree_tests.rs`：

| 测试名 | 锁住的不变量 |
| --- | --- |
| `rendered_rows_walks_full_layout_tree` | `rendered_rows` 必须等于布局树中最深叶子的底行；否则 `max_scroll()` 塌成 `0` |
| `scroll_limits_match_layout_depth` | DOM 高度大于视口时 `rendered_rows > rows`；滚轮才有可滚动的余地 |
| `nested_paragraph_shares_aside_row_with_bg` | 嵌套 `<p>` 与 `<aside>` 同行；aside 行后必须 bg-free |

如果以后有人在 `paint_box` 里又忘了递归累加 `max_y`，`rendered_rows_walks_full_layout_tree` 立刻会红；如果有人改回去给 `<aside>` `allocate(1)`，`nested_paragraph_shares_aside_row_with_bg` 立刻会红。

### 4.3 文本 bg 继承

`tests/layout_aside_tests.rs` 里另外 5 条用例（`test_nested_p_text_inherits_aside_bg`、`test_nested_p_text_bg_wins_over_parent` 等）锁住“文本继承父级 bg”的通路——这条通路是 `paint_box` 的 `effective_bg = my_bg.or(parent_bg)` 递归形成的。如果有人未来把 `parent_bg` 改回 `None`，这些测试会立刻报错。

---

## 5. 复现 / 调试步骤

### 5.1 复现 aside 多行

```bash
cargo test --test layout_aside_tests test_aside_bg_covers_nested_p_row
```

跑前先 `git stash`，把 `src/layout/layout_fn/aside.rs` 里的 `cursor.peek()` 改回 `cursor.allocate(1)`，再跑这条测试，应该会红：

```
row 0 should be fully darkblue  → 但 row 0 现在已经有 "Nested content" 文本了
```

这就是用户说的 “看起来和网页一样” 的反例。

### 5.2 复现滚动失效

`examples/debug_aside.rs` 之外可以临时构造：

```rust
let html = "<div><p>r1</p><p>r2</p><p>r3</p>...<p>r30</p></div>";
// 故意没让 aside 参与
```

```bash
cargo test --test layout_tree_tests rendered_rows_walks_full_layout_tree
```

如果 `paint_box` 忘了递归 `max_y`，`rendered_rows` 只会是 `1`，断言 `tl.rendered_rows as usize == max_leaf` 失败。

### 5.3 现场看

`cargo run -p visual_test -- aside`：

```
Default aside (full width)
Salamanders are a type of amphibian.
[cyan 80cols] The rough-skinned newt defends itself with a deadly neurotoxin. [cyan]
End of default aside section.
```

`<aside bg=darkblue>` 的“实心条带”与嵌套 `<p>` 文本在同一行，没有上方空白。

---

## 6. 一句话总结

- **问题 A**：`layout_aside` 多写了 `cursor.allocate(1)`，删掉改成 `cursor.peek()` 即可；保留 `.max(1)` 以确保空 aside 仍占 1 行。
- **滚动失效**：`paint_box` 漏掉了对子盒递归累加 `max_y`，导致 `rendered_rows` 永远 = 顶层盒高度；补上递归 + `effective_bg` 透传即可。
- **防回归**：上面 7 条测试是“以后不要再出现” 的护栏——任何回归必须先红再修。

---

## 7. 浏览器对齐：块级元素的水平位置

> 本节是上一个重构的姐妹问题。重构后 `<aside width: 50%>` / `<aside width: 30px>` 仍被“居中”，与浏览器不符。

### 7.1 症状

```html
<aside style="width: 50%; background-color: darkblue;">Half</aside>
```

旧实现：

```
[20 列默认背景] [40 列 darkblue，包含 "Half"] [20 列默认背景]
```

浏览器预期（用户反馈）：

```
[40 列 darkblue，包含 "Half"] [40 列默认背景]
```

即块默认 “靠左”，只有 `text-align` 控制**文本**在块内的位置。

### 7.2 根因

`src/layout/tree.rs::centered_left` 总是返回 `containing.x + (containing.width - block_width) / 2`，所有块级布局函数（`aside` / `div` / `p` / `head` / `header` / `footer`）都调用它——它默认居中。

这不符合 CSS 块布局默认：`margin-left: 0`（不是 `auto`），所以块默认靠左。`text-align` 只影响文本，**不**影响块位置。

### 7.3 修复

`src/layout/tree.rs`：将 `centered_left` 重命名为 `block_left`，实现为 `containing.x`（贴左）。`<img>` 有自己的私有 `centered_left`（仍居中），不受影响。

```rust
// 旧：强制居中
pub fn centered_left(containing, block_width) -> usize {
    containing.x + containing.width.saturating_sub(block_width) / 2
}

// 新：CSS 默认靠左
pub fn block_left(containing, _block_width) -> usize {
    containing.x
}
```

五个块级布局函数都换成 `block_left`：`aside.rs`、`div.rs`、`p.rs`、`head.rs`、`header_footer.rs`。`<img>` 仍使用其模块内的私有 `centered_left`。

### 7.4 几何对照表

`<aside width: 50%>`（终端 80 列）修复前 vs 修复后：

| 盒子 | 修复前 (x, width) | 修复后 (x, width) |
| --- | --- | --- |
| aside | (20, 40) | (0, 40) |
| 内部 text | 不变（由 `text-align` 决定） | 不变 |

`<aside width: 40px; text-align: center>Centered</aside>`：

| 元素 | 修复前 col | 修复后 col |
| --- | --- | --- |
| aside 起点 | 20 | 0 |
| "C" 位置 | 20 + (40-8)/2 = 36 | (40-8)/2 = 16 |
| "C" 距离块左 | 16 | 16 |

**文本相对于块左的位置不变；变的只是块的位置**。

### 7.5 防回归测试

`tests/layout_aside_tests.rs` / `tests/layout_div_tests.rs` / `tests/layout_header_footer_tests.rs` / `tests/layout_tree_tests.rs` 中所有“块几何”相关测试都从 `block_left = (cols - W) / 2` 改为 `block_left = 0`，并加上 `assert!(bg_cols.iter().all(|c| *c < W))` 之类的锁住“不超出块右”的断言。

如果以后有人把 `block_left` 又改成居中实现，下面这些测试会立刻变红：

- `test_aside_width_50pct`
- `test_aside_width_fixed_px`
- `test_aside_text_align_center`
- `test_aside_text_align_right`
- `test_nested_p_in_aside_with_fixed_width_centered`
- `test_div_width_50pct_left`（原名 `test_div_width_50pct_centered`，已重命名）
- `test_div_width_fixed`
- `test_div_text_align_center`
- `test_div_text_align_right`
- `test_header_width_50pct`
- `test_header_width_fixed`
- `nested_paragraph_uses_parent_containing_block_for_text_align`