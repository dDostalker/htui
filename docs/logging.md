# htui 日志（`htui::logging`）

> 单一目标：把日志写到文件。TUI 模式下 stderr 会被终端抢走、窗口属于额外特性，所以这里只暴露一个 `init_file_logger`。

---

## 1. 公开 API

### `init_file_logger`

```rust
pub fn init_file_logger(
    path: impl AsRef<Path>,
    level: Option<LevelFilter>,
) -> Result<()>
```

- append 模式打开文件，不存在则创建。
- 自动创建父目录（`fs::create_dir_all`），所以 `"logs/htui.log"` 不需要先 `mkdir`。
- `level = None` 时用 `default_level()`。
- `RUST_LOG` 环境变量优先于 `level`。
- 全局记录器每个进程只能装一次；重复调用返回 `Err("global logger is already initialized")`。

### `default_level`

| profile | 返回值 |
| --- | --- |
| `debug_assertions`（`cargo build` / `cargo test`） | `LevelFilter::Debug` |
| release（`cargo build --release`） | `LevelFilter::Warn` |

---

## 2. `release_max_level_warn` 的语义

`Cargo.toml` 给 `log` 启用了 `release_max_level_warn` feature。效果：

| 调用 | release 构建 |
| --- | --- |
| `log::debug!(...)` | 调用点直接展开为 `if false { ... }`，**参数都不求值** |
| `log::info!(...)` | 同上 |
| `log::warn!` / `log::error!` | 正常编译 |

配合 `default_level() = Warn` 形成双重保险：即使有人忘记去掉 `info!`，运行期 filter 也会把它丢掉。

---

## 3. 用法

```rust
htui::logging::init_file_logger("logs/htui.log", None)?;
log::info!("...");
log::warn!("...");
log::error!("...");
```

可视测试默认路径：`%TEMP%/htui-visual-test.log`（见 `visual_test/src/main.rs`）。

---

## 4. 错误

| 场景 | 行为 |
| --- | --- |
| 路径无效 / 权限不足 | `Err`（`failed to open log file: ...`） |
| 父目录创建失败 | `Err`（`failed to create log directory: ...`） |
| 重复初始化 | `Err`（`global logger is already initialized`） |

可视测试在 `init_file_logger` 失败时打印 `warning:` 到 stderr 并继续运行（不阻断 TUI）。
