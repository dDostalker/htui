# HTML解析器

## 概述

HTML解析器是htui项目中的一个核心模块，负责将HTML字符串解析为树形DOM结构（称为tdom）。解析器实现了递归下降解析算法，能够处理基本的HTML标签和文本内容。

## 核心数据结构

### Parser结构体

```rust
#[derive(Debug)]
struct Parser {
    point: usize,      // 当前解析位置
    context: String,   // 要解析的HTML字符串
    line: usize,       // 当前行号
    column: usize,     // 当前列号
}
```

Parser结构体维护解析状态，跟踪解析位置、行号和列号信息。

### EleBlockRead枚举

```rust
#[derive(Debug)]
enum EleBlockRead<'a> {
    Str(&'a str),      // 纯文本内容
    Block(&'a str),    // 开始标签（如<span>）
    EndBlock(&'a str), // 结束标签（如</span>）
    Enter,             // 换行符
}
```

EleBlockRead表示从HTML中读取的基本块类型，用于区分标签和文本内容。

### NodeType枚举

HTML解析器支持的节点类型包括：
- `Global`: 根节点类型
- `Span`: span标签
- `Div`: div标签  
- `P`: p标签
- `Text(String)`: 文本节点
- `None`: 空节点

## 解析流程

### 主解析入口

```rust
impl Parser {
    fn parse(&mut self) -> Result<Node> {
        self._parse(Some(NodeType::Global))
    }
}
```

解析过程从`parse()`方法开始，它调用内部的`_parse()`方法，并传入`NodeType::Global`作为根节点类型。

### 递归解析方法

`_parse()`方法是解析器的核心，采用递归下降算法：

1. **初始化节点**：创建新的Node实例
2. **循环读取**：不断调用`_element_content()`获取下一个HTML块
3. **处理不同类型块**：
   - `EleBlockRead::Block`: 识别标签类型，递归解析子节点
   - `EleBlockRead::Str`: 创建文本节点
   - `EleBlockRead::EndBlock`: 检查标签闭合是否匹配，返回当前节点
   - `EleBlockRead::Enter`: 跳过换行符
4. **结束条件**：当没有更多内容可读时，返回构建的节点

### 字符读取

`_next_char()`方法负责安全地读取下一个字符，处理UTF-8编码和边界检查。

### 元素内容提取

`_element_content()`方法根据当前字符决定如何读取：
- 遇到`<`：读取整个标签（直到`>`）
- 其他字符：读取文本内容（直到下一个`<`或结束）

## 标签类型检查

`check_ele_ty()`方法使用预定义的ELEMENT_FUNCTIONS数组来识别HTML标签对应的NodeType。这些函数定义在`tdom::element_fn`模块中。

## 使用示例

### 基本用法

```rust
use htui::html::parse::Parser;

let html = "<div><p>Hello World</p></div>";
let mut parser = Parser::new(html);
let dom = parser.parse().unwrap();
```

### 复杂HTML解析

```rust
let html = r#"
<div class="container">
    <h1>标题</h1>
    <span>内容<span>嵌套</span></span>
    <p>段落文本</p>
</div>
"#;

let mut parser = Parser::new(html);
match parser.parse() {
    Ok(dom) => println!("解析成功: {:?}", dom),
    Err(e) => println!("解析失败: {}", e),
}
```

## 错误处理

解析器使用`anyhow::Result`进行错误处理，可能出现的错误包括：
1. **标签不匹配**：开始标签和结束标签不匹配
2. **不支持的标签**：遇到未定义的HTML标签
3. **字符编码错误**：无效的UTF-8序列
4. **语法错误**：HTML格式不正确

## 测试

解析器包含单元测试，验证基本功能：

```rust
#[test]
fn test_check_ele_ty() {
    assert_eq!(Parser::check_ele_ty("<span>").unwrap(), NodeType::Span);
    assert_eq!(Parser::check_ele_ty("<div>").unwrap(), NodeType::Div);
}

#[test]
fn test_parse_html() {
    let html = "<div><p>Hello</p><p>World</p></div>";
    let mut parser = Parser::new(html);
    let node = parser.parse().unwrap();
    // 验证解析结果
}
```

## 注意事项

1. **标签属性**：当前版本主要关注标签结构，属性解析可能有限
2. **自闭合标签**：不支持自闭合标签（如`<br/>`）
3. **注释和脚本**：不支持HTML注释、`<script>`、`<style>`等特殊标签
4. **性能考虑**：采用递归算法，深度嵌套的HTML可能引起栈溢出

## 扩展性

要支持新的HTML标签，需要：
1. 在`NodeType`枚举中添加新的变体
2. 在`tdom::element_fn`模块中添加对应的识别函数
3. 更新相关的布局和渲染逻辑

## 与TUI集成

解析后的tdom结构可以传递给布局引擎进行屏幕布局计算，最终通过TUI框架渲染到终端。

---
# css 解析

首先需要明确，css的组成分为三部分：选择器 + 属性+ 值
```
p（选择器） {
  background-image（属性）: url('img_girl.jpg');（值）
}
```
## 选择器
选择器只有在完整使用css的时候才会使用，他决定了css的作用对象，如果是对某个标签内连则不需要使用，直接使用属性和值即可，如
```
 <p style="background-image: url('img_girl.jpg');"> 
```
其中选择器也分为一下几个部分
- 基础选择器
- 组合选择器
- 属性选择器
- 伪类
- 伪元素

### 基础选择器
```
*          /* 通配选择器 */
div        /* 标签选择器 */
.class     /* 类选择器 */
#id        /* ID选择器 */
```
### 组合选择器
```
div p      /* 后代选择器 */
div > p    /* 子选择器 */
div + p    /* 相邻兄弟 */
div ~ p    /* 普通兄弟 */
```

### 属性选择器
```
input[type="text"]
a[href]
```
### 伪类
```
:hover
:active
:focus
:first-child
:last-child
:nth-child()
```
## 属性和值
其属性和值格式如下
```
（属性）: （值）;
（属性）: （值）;
      ...
```

## 解析思路
解析的时候可以先设计一个函数用于分出选择器和属性值两个部分，效果如下
```
p:hover{
  background-image: url('img_girl.jpg')；
}

-> p:hover    第一部分
-> background-image:url('img_girl.jpg'); 第二部分   
```
然后对这两个部分进行分别解析


*文档最后更新：2026年 2月13日*

*相关文件：`src/html/parse.rs`, `src/tdom/node.rs`, `src/tdom/element_fn/`*
