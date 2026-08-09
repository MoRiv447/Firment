# Firment — Firmware + Agent

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Version](https://img.shields.io/badge/version-0.4.0--beta.1-orange)](Cargo.toml)
[![Rust](https://img.shields.io/badge/rust-1.85+-deeppink)](Cargo.toml)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey)]()
[![Benchmark](https://img.shields.io/badge/benchmark-4.95-%231-green)]()

**English** | [简体中文](README.zh-CN.md)

> ⚠️ **Status: half-finished beta, actively developed.**
> Firment is still a work in progress. The first layer (general coding agent) runs and is tested, but the TUI, configuration format, and tool APIs keep evolving and may change without notice. It is not yet recommended for production or mission-critical work. Try it, break it, and tell us what happened.

**Firmware + Agent = Firment** — a general-purpose coding agent for firmware and embedded development, named after *firmament* (the sky above every embedded engineer), with the second **a** dropped to fuse firmware + agent. The first layer (general coding agent) is usable today; later layers (build, flash, debug, UART, ...) plug into the same kernel through the unified `Tool` trait.

**Layer positioning.** Layer 1 (current release) is a general-purpose coding agent — the same category as other terminal coding agents, built with firmware/embedded workflows in mind. Embedded-specific capabilities (build, flash, debug, UART) land in **Layer 2**, currently in development; see the Roadmap.
### ✨ Features**Layer 1 — General**

- **Multi-provider**: Anthropic-compatible (`/v1/messages`) and OpenAI-compatible (`/chat/completions`, covering DeepSeek / GLM / Qwen / Ollama) streaming tool calls; DeepSeek V4 automatically uses official `thinking` + `reasoning_effort`
- **Thinking levels**: `off / low / medium / high / xhigh / max`
- **Built-in tools**: `read_file`, `write_file`, `edit_file` (anchor / line-range edits), `list_dir`, `glob`, `grep`, `shell`
- **Read-only plan mode**: `--plan` / `/plan` exposes only read tools and requires a decision-complete plan
- **Parallel tool calls**: independent calls run concurrently; same-file reads/writes and broad tools (shell/verify/grep) are ordered automatically
- **Engineering-grade system prompt**: sections for communication, engineering principles, tool policy, verification, and safety, plus `AGENTS.md` / `FIRMENT.md` project instructions
- **Session management**: JSONL persistence, `--continue`, `--list`, and an interactive `/sessions` picker
- **Copy support**: select with the left mouse button, copy with the right button (paste when nothing is selected); `Ctrl+Shift+C` / `/copy` copies the last reply
- **Global install**: `firm install` adds itself to PATH with PowerShell completions; `firm update` self-updates

**Context**

- **Model-summarized compaction**: when the context budget is exceeded, older rounds are summarized by the main model (with a local fallback), the last 3 rounds stay verbatim, and tool-call pairs are never split
- **Context compaction**: long sessions auto-compact older messages into a digest (`context_budget_chars`)
- **Cache-stable prefix**: the system prompt stays byte-identical for provider prefix caching; dynamic state (change ledger) is merged into user messages as deltas
- **Unchanged-read dedup**: re-reading an unchanged file returns a stub instead of repeating content; recently read files are re-injected after compaction
- **Pin files**: `/pin <path>` keeps a file's full content through compaction (re-injected verbatim); `/unpin <path>` removes it
- **Tool output spill**: outputs over a size threshold are saved to the session's spill directory, keeping only a short excerpt + `read_file` pointer in the conversation
- **Change ledger**: every committed turn appends path/line/hunk entries to a per-session ledger, injected into context on resume; `/ledger` shows it
- **Symbols index**: definition/reference lookup that auto-uses universal-ctags (JSON output) when available, falling back to the built-in regex scanner; `[tools] symbols_backend = auto | ctags | regex` (also available in plan mode)

**Safety & reliability**

- **Transactional edits + undo**: every write/edit in a turn is backed up and rolled back as a batch if any mutation fails; `/undo` restores the last committed batch (persisted per session)
- **CAS + SHA-256 anchoring**: write/edit re-checks the file byte-for-byte before applying; `read_file` returns a `[file-sha256: ...]` hash and `edit_file` / `write_file` accept `expected_sha256` as a stale-read guard (`[ConcurrentChange]` on mismatch); `read_file hashlines=true` exposes per-line content-hash anchors and `edit_file` supports `hashline` / `end_hashline` for precise placement
- **Diff-first approval**: write/edit permission prompts show the exact unified diff before you approve
- **Verify gate (enforced)**: an optional `verify` tool runs your configured build/check command; after any file changes the harness itself runs verify and refuses to accept completion until it passes
- **Path sandbox**: file tools are confined to the workspace (canonicalized; extra roots such as the spill directory are explicitly allowed); paths outside are rejected with `[Permission]`
- **Dangerous command guard**: in one-shot `-y` mode, `del/rm/Remove-Item/mv/move/git clean/git reset --hard`, force push, and scripting deletion APIs are blocked by default (including wrapper bypasses); the TUI labels them ⚠ and asks for confirmation
- **Argument schema validation**: tool arguments are validated against their JSON Schema before execution; malformed calls are rejected with a `[InvalidInput]` tag
- **Failure taxonomy**: tool errors carry tags such as `[NotFound]`, `[CompileError]`, `[Timeout]`, `[Permission]`, `[ConcurrentChange]`

**Layer 2 — Embedded-specific (basic build/flash available, rest in development)**

- Build / flash / run / monitor: `firm build`, `firm flash`, `firm run` (RTT logs via probe-rs) and `firm monitor` (serial + ELF symbol decoding) are available now
- Debugging (probe-rs), UART / serial log analysis with ELF symbol decoding
- MCU auto-detection (`.ioc` / CubeMX database)
- Register / peripheral awareness (chip register maps, `.ioc`, device trees)
### 🚀 Quick Start

Requirements: Rust 1.85+; Windows Terminal or any modern terminal is recommended.

One-line install (no local Rust toolchain needed):

```powershell
# Windows
irm https://raw.githubusercontent.com/MoRiv447/Firment/main/install.ps1 | iex
```

```bash
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/MoRiv447/Firment/main/install.sh | sh
```

For faster downloads in mainland China, set `FIRMENT_MIRROR` to a mirror base URL before running the installer (`{mirror}/{tag}/{asset}` layout, e.g. Alibaba Cloud OSS).

> **Security note:** the installer downloads the binary from GitHub Releases and verifies its SHA-256 against the release's `SHA256SUMS` before running it; the install scripts are small, HTTPS-served, and reviewable in this repository. To preview without executing anything, set `FIRMENT_DRY_RUN=1` before the one-liner; pin a specific version with `FIRMENT_VERSION`.
Build from source:

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

# thinking = "medium"      # off / low / medium / high / xhigh / max
# context_budget_chars = 60000       # session context budget; older messages are compacted when exceeded
# compaction_strategy = "summarize"  # default summarize; drop = discard oldest rounds, off = disable auto-compaction

[tools]
# verify_command = "cargo check"   # run this before declaring completion (e.g. cmake --build build)
# symbols_backend = "auto"         # auto / ctags / regex (symbol index backend)
# build_command = "cmake --build build"   # build tool (Keil: uv4 -j0 -b project.uvprojx)
# default_chip = "stm32f407vetx"          # default probe-rs chip for `firm flash`
# monitor_port = "COM3"                   # default serial port for `firm monitor`
# monitor_baud = 115200                   # default baud rate for `firm monitor`
```

Add more providers and switch with `--provider <name>` or `/provider <name>` in the TUI; `/models` and `Ctrl+P` fetch and pick models without editing files.

### 📚 硬件知识库（可选）

在固件项目里放 `docs/vendor-index.toml`（+ `docs/cheatsheets/` 原创速查表），Firment 会自动发现并在提示词中要求 agent 优先查询。涉及芯片/外设/寄存器/HAL 的问题，agent 会先查知识库再作答。模板见 [docs/vendor-index.toml](docs/vendor-index.toml)，说明见 [docs/vendor-index.md](docs/vendor-index.md)。

### 📁 项目级配置（让 AI 自己干活）

在项目根目录放 `.firment.toml`，把构建/烧录/串口配置写进 `[tools]`（可提交进版本库）：

```toml
[tools]
build_command = "cmake --build build"   # 或 uv4 -j0 -b project.uvprojx
default_chip = "stm32f407vetx"
monitor_port = "COM3"
```

项目配置会覆盖全局 `config.toml` 的对应项。进 TUI 后直接说“构建并烧录”，agent 会自己读取/修改这份文件并调用 `build` / `flash` / `run`；`build` 默认免确认，`flash` 始终弹确认。`monitor` 也是 agent 工具——target 通过物理 UART 输出日志时，agent 可打开串口监听一段时间（可配 `--elf` 解码地址），把日志拉回对话。

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
| `firm build` | Run the configured build command (`[tools] build_command`, e.g. CMake/Make/Keil/IAR CLI) |
| `firm flash [--chip <chip>] <elf>` | Flash a firmware ELF via probe-rs (ST-Link / J-Link / CMSIS-DAP / DFU) |
| `firm run [--chip <chip>] [--timeout <secs>] <elf>` | Flash, reset and run the target, streaming RTT logs |
| `firm monitor [--port <COMx>] [--baud <n>] [--elf <elf>]` | Monitor a serial port; decodes stack addresses with `--elf` |
| `firm --set-key default=sk-xxx` | Store an API key |

### 🎮 TUI

Slash commands: `/new`, `/plan [on|off]`, `/agent`, `/models`, `/model <id>`, `/sessions` (↑/↓ to pick), `/session <id>`, `/undo`, `/ledger`, `/pin <path>`, `/unpin <path>`, `/provider <name>`, `/add-provider`, `/apikey`, `/thinking`, `/copy`, `/config`, `/clear`, `/help`, `/quit`.

Keys: `↑/↓` browse history when the input is empty, otherwise scroll; `PgUp/PgDn` and the mouse wheel always scroll; `Ctrl+P` opens the model picker; left-drag to select and right-click to copy (paste when nothing is selected); `Ctrl+Shift+C` copies the last reply; `←/→`, `Home/End`, `Ctrl+A/E` move the cursor; permission popups accept `y`/`n`/`a`.

### 🔒 Security Model

- **Disclaimer**: the dangerous command guard is a best-effort heuristic (command-name scanning), not an OS sandbox. File tools are confined by the path sandbox; `shell` remains permission-gated only. For strong isolation, run Firment inside a container/VM.
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
