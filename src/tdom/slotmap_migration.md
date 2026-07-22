# Slotmap 重构文档

## 变更日期: 2026-07-22

## 概述

将 DOM 树存储从 `Vec<Node>` 自有树 + `*const Node` 父指针 重构为 **slotmap arena** 方案。

### 变更动机

| 问题 | 旧方案 | 新方案 |
|------|--------|--------|
| 父节点引用 | `Option<*const Node>` — 原始指针，Vec 扩容后悬空 | `Option<NodeId>` — 类型安全索引，永不过期 |
| unsafe 代码 | 3 处 raw pointer deref | **0 处** |
| 节点增删 | 删除节点后指针悬空 | slotmap version 检测，访问已删节点 panic 而非 UB |
| 多子树操作 | `&mut root` 锁定整棵树 | 可通过 `get_disjoint_mut` 同时借多个不相交节点 |

## 核心类型

```rust
// src/tdom/node.rs

slotmap::new_key_type! {
    pub struct NodeId;  // Copy, Send, Sync, 带 version 检测
}

pub type NodeArena = SlotMap<NodeId, Node>;

pub struct Node {
    pub node_type: NodeType,
    pub children: Vec<NodeId>,     // 子节点 ID（保持顺序）
    pub parent: Option<NodeId>,    // 父节点 ID
    pub attributes: Attr,
    pub css_attr: CssAttr,
}
```

## 架构对比

### 旧架构（自有树）

```
root: Node {
    children: [             // Vec<Node> — 持有所有权
        div: Node {
            children: [
                Node::text("Hello"),
            ],
        },
    ],
    parent: Some(0x7fff...)  // *const Node — unsafe
}
```

- 遍历: `for child in &node.children { }`
- 父访问: `node.parent_ref()` → unsafe deref
- 创建: 直接在栈/Vec 中构建 `Node`

### 新架构（slotmap arena）

```
arena: SlotMap<NodeId, Node> {
    n#1: Node { children: [n#2], parent: None },        // root
    n#2: Node { children: [n#3], parent: Some(n#1) },   // div
    n#3: Node { children: [],    parent: Some(n#2) },   // text
}
```

- 遍历: `for &child_id in &node.children { let child = &arena[child_id]; }`
- 父访问: `let parent = &arena[node.parent.unwrap()];` — 安全
- 创建: `let id = arena.insert(Node::new());`

## 修改的文件

| 文件 | 变更 |
|------|------|
| `Cargo.toml` | 新增 `slotmap = "1.0"` |
| `src/tdom/node.rs` | 定义 `NodeId`、`NodeArena`；Node 移除 `*const Node`；新增 `append_child()`、`fix_parent_pointers()`、`Descendants` 迭代器 |
| `src/html/html_parse.rs` | `parse()` 返回 `(NodeId, NodeArena, String)`；`convert_children` 操作 arena |
| `src/html/css_parse.rs` | 匹配引擎改为 `selector_chain_matches(arena, node_id, ...)`；`apply_css_rule` / `css_into_node` 签名增加 arena |
| `src/layout/mod.rs` | `LayoutFn` 签名增加 `arena: &NodeArena` |
| `src/layout/layout.rs` | `draw(arena, root_id, ...)` 替代 `draw(&node, ...)` |
| `src/layout/layout_fn/p.rs` | 遍历改为 `for child_id in node.child_ids() { &arena[child_id] }` |
| `src/layout/layout_fn/head.rs` | 同上 |
| `src/layout/layout_fn/div.rs` | 同上 |
| `src/layout/layout_fn/img.rs` | img 不需 arena（无子节点遍历） |
| `src/lib.rs` | `parse()` 返回 `(NodeId, NodeArena)`；`render()` 持有 arena 生命周期 |

## 新增 API

### Node 树操作

```rust
// 追加子节点（同时设置 parent）
Node::append_child(&mut arena, parent_id, child_id);

// O(1) 按索引访问
let third = node.child(2);

// 子节点迭代
for child_id in node.child_ids() { let child = &arena[child_id]; }

// 兄弟节点
let prev = node.prev_sibling(&arena);
let next = node.next_sibling(&arena);

// 批量设置 parent（构建后调用一次）
Node::fix_parent_pointers(&mut arena, root_id);

// DFS 遍历
for (id, node) in Node::descendants(&arena, root_id) { }
```

### CSS 匹配

```rust
// 签名变化
fn selector_chain_matches(arena: &NodeArena, node_id: NodeId, selector: &CssSelector) -> bool;
fn apply_css_rule(arena: &mut NodeArena, node_id: NodeId, selector: &CssSelector, css: &CssAttr);
pub fn css_into_node(arena: &mut NodeArena, root_id: NodeId, css: &CssRule) -> Result<()>;
```

### HTML 解析

```rust
// 旧: pub fn parse(&mut self) -> Result<Node>
// 新: pub fn parse(&mut self) -> Result<(NodeId, NodeArena, String)>
let (root_id, arena, embedded_css) = parser.parse()?;
```

### Layout

```rust
// 旧: pub fn draw(&mut self, tdom: &Node, term_map: &TermMap) -> Result<()>
// 新: pub fn draw(&mut self, arena: &NodeArena, root_id: NodeId, term_map: &TermMap) -> Result<()>
```

## 迁移 checklist（如要扩展 Node）

1. 新增字段加到 `Node` struct，初始化为 `NodeArena::insert()` 中
2. 涉及子节点遍历的函数需 `arena: &NodeArena` 参数
3. 涉及子节点修改的函数需 `arena: &mut NodeArena` 参数
4. 对不相交节点同时 `&mut` 访问，使用 `arena.get_disjoint_mut([a, b])`
5. 不要手动 `arena.remove(id)` 后仍使用该 ID — 版本检测会 panic

## 与旧方案对比

| | Vec<Node> 自有树 | slotmap arena |
|---|---|---|
| unsafe 代码 | 3 处 | **0** |
| 父节点引用稳定性 | 扩容后失效 | **永久有效** |
| 随机访问 | O(1) | O(1) |
| 删除安全 | 手动维护，指针悬空 | version 检测 panic |
| 内存连续性 | 优（children 连续） | 一般（哈希表分散） |
| 多子树同时写 | 天然支持 | 需 `get_disjoint_mut` |
| 序列化 | 简单 | 需序列化 arena + root_id |
| 编译时检查 | 所有权检查强 | NodeId 是整数，编译时无检查 |

## 测试

```
running 24 tests
test result: ok. 24 passed; 0 failed
```
