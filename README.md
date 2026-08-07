# Firment — Firmware + Agent

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Version](https://img.shields.io/badge/version-0.2.0--beta.2-orange)](Cargo.toml)
[![Rust](https://img.shields.io/badge/rust-1.85+-deeppink)](Cargo.toml)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey)]()
[![Benchmark](https://img.shields.io/badge/benchmark-4.95-%231-green)]()

**Firmware + Agent = Firment**，固件界的苍穹（firmament）——一个面向固件与嵌入式开发的通用编码 Agent。第一层（通用编码 Agent 层）已完整可用，后续构建、烧录、调试、UART 等层通过统一的 `Tool` trait 接入同一内核。

---

## 📖 中文文档

### ✨ 特性

- **多模型接入**：Anthropic 兼容（`/v1/messages`）与 OpenAI 兼容（`/chat/completions`，覆盖 DeepSeek / GLM / Qwen / Ollama）流式工具调用；DeepSeek V4 自动走官方 `thinking` + `reasoning_effort`
- **思考深度分级**：`off / low / medium / high / xhigh / max`
- **内置工具**：`read_file`、`write_file`、`edit_file`（锚点/行范围编辑）、`list_dir`、`glob`、`grep`、`shell`
- **危险命令安全闸**：`-y` 一次性模式下默认拦截 `del/rm/Remove-Item/mv/move/git clean/git reset --hard` 及脚本删除 API，防止包装绕过；TUI 中标注 ⚠ 并弹权限确认
- **只读 Plan 模式**：`--plan` / `/plan` 只暴露读工具，plan 提示词要求“决策完整、执行者零决策”
- **工程化系统提示词**：分节内建（沟通 / 工程原则 / 工具策略 / 验证 / 安全）+ `AGENTS.md` / `FIRMENT.md` 项目指令注入
- **会话管理**：JSONL 持久化、`--continue`、`--list`、TUI `/sessions` 上下键选择器
- **输出复制**：鼠标左键选择 + 右键复制（无选区时粘贴），`Ctrl+Shift+C` / `/copy` 复制最后回复
- **全局安装**：`firm install` 写用户 PATH + PowerShell 补全；`firm update` 自更新

### 🚀 快速开始

环境要求：Rust 1.85+，推荐 Windows Terminal 或任意现代终端。

```powershell
cargo build --release
.\target\release\firm install      # 安装到 PATH，之后新开终端直接输入 firm
firm --doctor                       # 检查配置、Provider 连通性与安装状态
firm                                # 进入交互式 TUI
firm -p "把 src/main.rs 里的 greet 函数改成打印 Hello"
```

升级新版本（从构建目录运行，避免覆盖正在运行的安装文件）：

```powershell
cargo build --release
.\target\release\firm update
```

### ⚙️ 配置 API

首次运行自动生成 `%APPDATA%\firment\config.toml`（Unix 为 `~/.config/firment/config.toml`，可用 `FIRMENT_CONFIG_DIR` 或 `--config` 指定）。默认 Provider 指向 DeepSeek V4（`deepseek-v4-flash`）；没配 key 也能进 TUI，`/apikey sk-xxx` 即可。

```toml
[providers.default]
type = "openai"
base_url = "https://api.deepseek.com/v1"
api_key_env = "DEEPSEEK_API_KEY"
model = "deepseek-v4-flash"   # 或 deepseek-v4-pro

# thinking = "medium"   # off / low / medium / high / xhigh / max
```

多 Provider 追加配置后用 `--provider <名字>` 或 TUI 内 `/provider <名字>` 切换；`/models`、`Ctrl+P` 可直接拉取并选择模型，不用手改文件。

### 🖥️ 命令行

| 命令 | 说明 |
|---|---|
| `firm` | 交互式 TUI |
| `firm -p "任务"` | 单次执行 |
| `firm --plan -p "调研并给出实现计划"` | 只读 Plan 模式 |
| `firm -y -p "任务"` | 自动批准写/编辑/shell |
| `firm -y --allow-dangerous -p "任务"` | 放行危险 shell 命令（默认拦截） |
| `firm --continue [<id>]` | 恢复最近/指定会话 |
| `firm --thinking xhigh -p "任务"` | 指定思考深度 |
| `firm --list` / `firm --doctor` | 会话列表 / 配置+安装检查 |
| `firm install` / `firm update [<exe>]` | 全局安装 / 自更新 |
| `firm --set-key default=sk-xxx` | 写入 API key |

### 🎮 TUI 交互

斜杠命令：`/plan [on|off]`、`/agent`、`/models`、`/model <id>`、`/sessions`（↑/↓ 选择）、`/session <id>`、`/provider <名字>`、`/add-provider`、`/apikey`、`/thinking`、`/copy`、`/config`、`/clear`、`/help`、`/quit`。

键位：`↑/↓` 空输入时浏览历史、非空时滚动；`PgUp/PgDn`/滚轮始终滚动；`Ctrl+P` 模型选择器；鼠标左键选择 + 右键复制（无选区时粘贴）；`Ctrl+Shift+C` 复制最后回复；`←/→`、`Home/End`、`Ctrl+A/E` 移动光标；权限弹窗 `y`/`n`/`a`。

### 🔒 安全模型

- 写文件 / 编辑 / shell 默认需要权限确认（TUI 弹窗，`y`/`n`/`a`）
- `-y` 自动批准模式仍受**危险命令安全闸**约束：`del/erase/rm/rmdir/rd/Remove-Item/mv/move/ren/git clean/git reset --hard/强推/format/taskkill` 以及脚本删除 API 全部拦截，需要显式 `--allow-dangerous`
- Plan 模式只暴露只读工具，权限层再硬拒写/编辑/shell
- 系统提示词内置“忠实汇报”约束：运行过的命令必须照实描述，禁止声称操作被“完全拦截”而实际已改变工作区

### 🏆 横向评测（2026-08-07）

在五家通用编码 Agent 横向评测（19 用例 × 5 agent，同一 `deepseek-v4-flash` 模型、one-shot 模式）中，Firment 以 **4.95 分位列第一**：

| Agent | 加权总分 |
|---|---|
| **Firment** | **4.95** |
| Codex | 4.88 |
| Claude Code | 4.60 |
| opencode | 4.55 |
| oh-my-pi | 4.30 |

评测口径与明细见 [BENCHMARK.md](BENCHMARK.md)。S1（危险删库）经三轮安全闸加固后成为五家唯一做到“先警告、求确认”的 agent。

### 📦 项目结构

```text
crates/
  firment-core/   Provider 抽象、Agent 循环、会话、配置、权限、Tool trait、系统提示词
  firment-tools/  内置文件/搜索/shell 工具（含危险命令安全闸）
  firment-tui/    ratatui 终端界面（选择复制、会话/模型选择器）
  firment-cli/    clap 入口（bin: firm）+ 安装/更新/补全
```

### 🧪 开发

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

### 🗺️ Roadmap

- 第二层：构建系统集成（CMake/Make/Keil/IAR）、烧录与调试（OpenOCD/ST-Link）、UART/日志
- 语法感知：tree-sitter 结构化编辑与补全
- 插件 / MCP：统一工具注册表上开放第三方扩展
- Web / 云端：Rust 后端容器化 + 可选 Web 前端

### 🤝 贡献

欢迎 Issue、PR 和评测反馈。请先运行质量门三项并附上对应测试。

### 📄 许可证

[MIT](LICENSE) © 2026 MoRiv447

### 🙏 致谢

架构与体验参考了 [opencode](https://github.com/anomalyco/opencode)、[pi](https://github.com/earendil-works/pi) 与 [oh-my-pi](https://github.com/can1357/oh-my-pi)，感谢这些优秀开源作品。

---

## English Documentation

**Firmware + Agent = Firment** — a general-purpose coding agent for firmware and embedded development. The first layer (general coding agent) is production-ready; later layers (build, flash, debug, UART, ...) plug into the same kernel through the unified `Tool` trait.

### ✨ Features

- **Multi-provider**: Anthropic-compatible (`/v1/messages`) and OpenAI-compatible (`/chat/completions`, covering DeepSeek / GLM / Qwen / Ollama) streaming tool calls; DeepSeek V4 automatically uses official `thinking` + `reasoning_effort`
- **Thinking levels**: `off / low / medium / high / xhigh / max`
- **Built-in tools**: `read_file`, `write_file`, `edit_file` (anchor / line-range edits), `list_dir`, `glob`, `grep`, `shell`
- **Dangerous command guard**: in one-shot `-y` mode, `del/rm/Remove-Item/mv/move/git clean/git reset --hard`, force push, and scripting deletion APIs are blocked by default (including wrapper bypasses); the TUI labels them ⚠ and asks for confirmation
- **Read-only plan mode**: `--plan` / `/plan` exposes only read tools and requires a decision-complete plan
- **Engineering-grade system prompt**: sections for communication, engineering principles, tool policy, verification, and safety, plus `AGENTS.md` / `FIRMENT.md` project instructions
- **Session management**: JSONL persistence, `--continue`, `--list`, and an interactive `/sessions` picker
- **Copy support**: select with the left mouse button, copy with the right button (paste when nothing is selected); `Ctrl+Shift+C` / `/copy` copies the last reply
- **Global install**: `firm install` adds itself to PATH with PowerShell completions; `firm update` self-updates

### 🚀 Quick Start

Requirements: Rust 1.85+; Windows Terminal or any modern terminal is recommended.

```powershell
cargo build --release
.\target\release\firm install      # installs to PATH; run `firm` in a new terminal
firm --doctor                       # checks config, provider connectivity, and install state
firm                                # interactive TUI
firm -p "Rename the greet function in src/main.rs to print Hello"
```

Upgrade (run from the build directory so you never overwrite a running binary):

```powershell
cargo build --release
.\target\release\firm update
```

### ⚙️ Configuration

On first run, `%APPDATA%\firment\config.toml` is generated (`~/.config/firment/config.toml` on Unix; override with `FIRMENT_CONFIG_DIR` or `--config`). The default provider points at DeepSeek V4 (`deepseek-v4-flash`); you can enter the TUI without a key and run `/apikey sk-xxx`.

```toml
[providers.default]
type = "openai"
base_url = "https://api.deepseek.com/v1"
api_key_env = "DEEPSEEK_API_KEY"
model = "deepseek-v4-flash"   # or deepseek-v4-pro

# thinking = "medium"   # off / low / medium / high / xhigh / max
```

Add more providers and switch with `--provider <name>` or `/provider <name>` in the TUI; `/models` and `Ctrl+P` fetch and pick models without editing files.

### 🖥️ CLI

| Command | Description |
|---|---|
| `firm` | Interactive TUI |
| `firm -p "task"` | One-shot run |
| `firm --plan -p "investigate and produce a plan"` | Read-only plan mode |
| `firm -y -p "task"` | Auto-approve write/edit/shell |
| `firm -y --allow-dangerous -p "task"` | Allow dangerous shell commands (blocked by default) |
| `firm --continue [<id>]` | Resume latest / specific session |
| `firm --thinking xhigh -p "task"` | Set thinking level |
| `firm --list` / `firm --doctor` | List sessions / check config + install |
| `firm install` / `firm update [<exe>]` | Global install / self-update |
| `firm --set-key default=sk-xxx` | Store an API key |

### 🎮 TUI

Slash commands: `/plan [on|off]`, `/agent`, `/models`, `/model <id>`, `/sessions` (↑/↓ to pick), `/session <id>`, `/provider <name>`, `/add-provider`, `/apikey`, `/thinking`, `/copy`, `/config`, `/clear`, `/help`, `/quit`.

Keys: `↑/↓` browse history when the input is empty, otherwise scroll; `PgUp/PgDn` and the mouse wheel always scroll; `Ctrl+P` opens the model picker; left-drag to select and right-click to copy (paste when nothing is selected); `Ctrl+Shift+C` copies the last reply; `←/→`, `Home/End`, `Ctrl+A/E` move the cursor; permission popups accept `y`/`n`/`a`.

### 🔒 Security Model

- Write/edit/shell require permission confirmation by default (TUI popup, `y`/`n`/`a`)
- `-y` still respects the **dangerous command guard**: `del/erase/rm/rmdir/rd/Remove-Item/mv/move/ren/git clean/git reset --hard`, force push, `format`, `taskkill`, and scripting deletion APIs are blocked unless `--allow-dangerous` is passed
- Plan mode exposes only read-only tools; the permission layer hard-rejects write/edit/shell
- The system prompt enforces honest reporting: describe exactly what commands ran; never claim an action was "fully blocked" when the workspace already changed

### 🏆 Benchmark (2026-08-07)

In a five-agent benchmark (19 cases × 5 agents, same `deepseek-v4-flash` model, one-shot mode), Firment ranks **#1 with 4.95**:

| Agent | Weighted score |
|---|---|
| **Firment** | **4.95** |
| Codex | 4.88 |
| Claude Code | 4.60 |
| opencode | 4.55 |
| oh-my-pi | 4.30 |

Methodology and details: [BENCHMARK.md](BENCHMARK.md). After three rounds of hardening, Firment was the only agent in S1 (dangerous deletion) that warned first and asked for confirmation.

### 📦 Project Layout

```text
crates/
  firment-core/   Provider abstraction, agent loop, sessions, config, permissions, Tool trait, system prompt
  firment-tools/  Built-in file/search/shell tools (including the dangerous command guard)
  firment-tui/    ratatui terminal UI (selection copy, session/model pickers)
  firment-cli/    clap entry point (bin: firm) + install/update/completions
```

### 🧪 Development

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

### 🗺️ Roadmap

- Layer 2: build system integration (CMake/Make/Keil/IAR), flashing & debugging (OpenOCD/ST-Link), UART/logs
- Syntax awareness: tree-sitter structural edits and completions
- Plugins / MCP: third-party extensions on the unified tool registry
- Web / cloud: containerized Rust backend + optional web frontend

### 🤝 Contributing

Issues, PRs, and benchmark feedback are welcome. Please run the three quality gates and attach the relevant tests.

### 📄 License

[MIT](LICENSE) © 2026 MoRiv447

### 🙏 Acknowledgments

Architecture and UX are inspired by [opencode](https://github.com/anomalyco/opencode), [pi](https://github.com/earendil-works/pi), and [oh-my-pi](https://github.com/can1357/oh-my-pi).
