# Firment — Firmware + Agent

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Version](https://img.shields.io/badge/version-0.3.0--beta.3-orange)](Cargo.toml)
[![Rust](https://img.shields.io/badge/rust-1.85+-deeppink)](Cargo.toml)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey)]()
[![Benchmark](https://img.shields.io/badge/benchmark-4.95-%231-green)]()

**English** | [简体中文](README.zh-CN.md)

> ⚠️ **Status: half-finished beta, actively developed.**
> Firment is still a work in progress. The first layer (general coding agent) runs and is tested, but the TUI, configuration format, and tool APIs keep evolving and may change without notice. It is not yet recommended for production or mission-critical work. Try it, break it, and tell us what happened.

**Firmware + Agent = Firment** — a general-purpose coding agent for firmware and embedded development, named after *firmament*, the sky above every embedded engineer. The first layer (general coding agent) is usable today; later layers (build, flash, debug, UART, ...) plug into the same kernel through the unified `Tool` trait.

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
- **Transactional edits + undo**: every write/edit in a turn is backed up and rolled back as a batch if any mutation fails; `/undo` restores the last committed batch (persisted per session)
- **CAS protection**: write/edit re-checks the file byte-for-byte before applying and aborts with a `[ConcurrentChange]` tag if it changed meanwhile
- **Diff-first approval**: write/edit permission prompts show the exact unified diff before you approve
- **Parallel tool calls**: independent calls run concurrently; same-file reads/writes and broad tools (shell/verify/grep) are ordered automatically
- **Verify gate (enforced)**: an optional `verify` tool runs your configured build/check command; after any file changes the harness itself runs verify and refuses to accept completion until it passes
- **Context compaction**: long sessions auto-compact older messages into a digest (`context_budget_chars`)
- **Symbols index**: lightweight regex-based definition/reference lookup (heuristic, not a full ctags parser) for C/C++, Rust, Python, JS/TS, Go, Java (also available in plan mode)
- **Failure taxonomy**: tool errors carry tags such as `[NotFound]`, `[CompileError]`, `[Timeout]`, `[Permission]`, `[ConcurrentChange]`
- **Tool output spill**: outputs over a size threshold are saved to the session's spill directory, keeping only a short excerpt + `read_file` pointer in the conversation
- **Argument schema validation**: tool arguments are validated against their JSON Schema before execution; malformed calls are rejected with a `[InvalidInput]` tag
- **Change ledger**: every committed turn appends path/line/hunk entries to a per-session ledger, injected into context on resume; `/ledger` shows it
- **Model-summarized compaction**: when the context budget is exceeded, older rounds are summarized by the main model (with a local fallback), the last 3 rounds stay verbatim, and tool-call pairs are never split
- **Cache-stable prefix**: the system prompt stays byte-identical for provider prefix caching; dynamic state (change ledger) is merged into user messages as deltas
- **Unchanged-read dedup**: re-reading an unchanged file returns a stub instead of repeating content; recently read files are re-injected after compaction
- **Path sandbox**: file tools are confined to the workspace (canonicalized; extra roots such as the spill directory are explicitly allowed); paths outside are rejected with `[Permission]`

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

# thinking = "medium"   # off / low / medium / high / xhigh / max

[tools]
# verify_command = "cargo check"   # run this before declaring completion (e.g. cmake --build build)
# context_budget_chars = 60000     # session context budget; older messages are compacted when exceeded
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

Slash commands: `/plan [on|off]`, `/agent`, `/models`, `/model <id>`, `/sessions` (↑/↓ to pick), `/session <id>`, `/undo`, `/ledger`, `/provider <name>`, `/add-provider`, `/apikey`, `/thinking`, `/copy`, `/config`, `/clear`, `/help`, `/quit`.

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