# test_html 修改记录

## 变更日期: 2026-07-22

## 变更概述

为 test_html 视觉测试套件增加了 6 个新测试场景，并重构了 main.rs 支持 CLI 参数选择测试。

## 修改的文件

| 文件 | 变更 | 说明 |
|------|------|------|
| `src/main.rs` | 重写 | 支持 CLI 参数 `cargo run -p test_html -- <test_name>` 选择测试 |
| `src/all_in_one.html` | 新增 | 综合演示：标题/段落/嵌套/图片/颜色/Unicode |
| `src/headings.html` | 新增 | H1-H5 标题层级渲染 + CSS 颜色 |
| `src/image.html` | 新增 | 图片渲染：多种尺寸 (80~320px)、多图序列 |
| `src/colors.html` | 新增 | CSS 颜色渲染：8 种颜色 |
| `src/nested.html` | 新增 | 深度嵌套 div 结构 (1-4 层) |
| `src/text.html` | 新增 | 文本内容：Unicode/CJK/Emoji/特殊字符 |
| `README.md` | 新增 | 测试套件使用说明和覆盖矩阵 |

## 使用方式

```bash
cargo run -p test_html                    # 运行默认 all_in_one
cargo run -p test_html -- headings        # 运行标题测试
cargo run -p test_html -- image           # 运行图片测试
cargo run -p test_html -- colors          # 运行颜色测试
cargo run -p test_html -- nested          # 运行嵌套测试
cargo run -p test_html -- text            # 运行文本测试
```

## 测试覆盖新增

| 场景 | 之前 | 之后 |
|------|:---:|:---:|
| 综合演示 | - | ✓ all_in_one |
| 标题层级 | - | ✓ headings |
| 图片渲染 | - | ✓ image (6 种尺寸) |
| 颜色渲染 | - | ✓ colors (8 色) |
| 深度嵌套 | - | ✓ nested (4 层) |
| Unicode/CJK | - | ✓ text |
| Emoji 渲染 | - | ✓ text |
| 空段落 | - | ✓ text |
| 多图序列 | - | ✓ image |

## 保留的原有测试

- `css.html` → `cargo run -p test_html -- css`
- `helloworld.html` → `cargo run -p test_html -- helloworld`
- `pic.png` — 未修改

## 注意事项

- 所有测试为**肉眼观察测试**，无自动化断言
- 按 `q` 或 `Esc` 退出测试
- 调整终端大小自动刷新
- 图片测试依赖 `src/pic.png`
