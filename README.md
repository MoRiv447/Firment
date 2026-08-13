# Firment — Firmware + Agent

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Version](https://img.shields.io/badge/version-0.4.0--beta.8-orange)](Cargo.toml)
[![Rust](https://img.shields.io/badge/rust-1.85+-deeppink)](Cargo.toml)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey)]()
[![CI](https://img.shields.io/badge/CI-Rust%20%2B%20Web%20%2B%20IDE-green)](.github/workflows/ci.yml)

**English** | [简体中文](README.zh-CN.md)

> ⚠️ **Status: beta, actively developed.** Layer 1 (general coding agent) is
> production-usable and CI-green; Layer 2 (embedded toolchain loop) is
> partially shipped. APIs and the TUI keep evolving.

**Firmware + Agent = Firment** — a general-purpose coding agent for firmware
and embedded development, named after *firmament* (the sky above every
embedded engineer), with the second **a** dropped to fuse *firmware + agent*.
The kernel is a Rust coding agent with an embedded-first toolchain loop:
write code, build, flash, run, monitor serial, analyze the ELF — all from the
same conversation.

**One repo, three surfaces (monorepo):**

| Surface | Path | Stack |
|---|---|---|
| **CLI / TUI** | [`crates/`](crates/) | Rust (ratatui) — the core agent |
| **IDE client** | [`ide/`](ide/) | Tauri + React/Vite (TypeScript) |
| **Web** | [`web/`](web/) | Next.js + Tailwind (deployed on Vercel) |

The CLI is the source of truth; the IDE and Web surfaces talk to the same
agent kernel through the unified `Tool` trait and session format.

### ✨ Features — Layer 1 (general coding agent)

- **Multi-provider**: Anthropic-compatible (`/v1/messages`) and
  OpenAI-compatible (`/chat/completions`, covering DeepSeek / GLM / Qwen /
  Ollama) streaming tool calls; DeepSeek V4 automatically uses official
  `thinking` + `reasoning_effort`
- **Thinking levels**: `off / low / medium / high / xhigh / max`
- **Built-in tools**: `read_file` (line-numbered pages), `write_file`,
  `edit_file` (anchor / line-range / hashline edits, unified diff echo),
  `list_dir`, `glob`, `grep`, `shell`, `web_search` (DuckDuckGo / Tavily /
  Brave), `web_fetch`, `task` (read-only research subagent), `todo`,
  `ask_user`, `periph_init`, `elf_analyze`, `monitor`
- **Read-only plan mode**: `--plan` / `/plan` exposes only read tools and
  requires a decision-complete plan
- **Parallel tool calls**: independent calls run concurrently; same-file
  reads/writes and broad tools are ordered automatically
- **Engineering-grade system prompt**: communication, engineering
  principles, tool policy, verification, and safety sections, plus
  `AGENTS.md` / `FIRMENT.md` project instructions
- **Session management**: JSONL persistence, `--continue`, `--list`,
  interactive `/sessions` picker, change-ledger + `/undo`
- **Copy support**: left-drag select, right-click copy, `Ctrl+Shift+C`
  copies the last reply
- **Global install**: `firm install` adds PATH + completions; `firm update`
  self-updates

### 🛠️ Layer 2 — embedded toolchain loop (shipping incrementally)

- **`periph_init`** — MCU peripheral init skeletons + knowledge-base
  cheatsheets. Full UART/GPIO/I2C/SPI/TIM/ADC HAL skeletons on STM32
  (F1/F4/G0/**G4**/**H7**), ESP32/ESP32-S3 guidance, CubeMX/PlatformIO
  HAL-duplication warnings. The bundled KB covers real engineering traps:
  **G4's DMAMUX** (no fixed DMA channels — the classic F1→G4 migration
  trap) and **H7's D-Cache coherency** (clean before DMA TX, invalidate
  after DMA RX)
- **`elf_analyze`** — flash/RAM usage, function sizes, and real stack depth
  from `-fstack-usage` `.su` files; Firment auto-seeds a baseline and
  re-analyzes after each edited turn
- **`monitor`** — serial monitor with per-line timestamps and baud-rate
  autodetect; cancelling a turn releases the port immediately
- **`build` / `flash` / `run`** — CMake/Make/Keil build commands, probe-rs
  flashing (chip auto-detection), all wired into the agent loop

### 🚀 Quick Start

```bash
# Build and run the CLI agent
cargo build --release
./target/release/firm            # start a session
./target/release/firm --continue # resume the last session
```

Windows one-liner install (adds `firm` to PATH + PowerShell completions):

```powershell
Set-ExecutionPolicy -Scope Process Bypass; iex (irm https://raw.githubusercontent.com/MoRiv447/Firment/main/install.ps1)
```

macOS / Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/MoRiv447/Firment/main/install.sh | sh
```

### ⚙️ Configuration

`firm config` opens the config file (created on first run, `%APPDATA%\firment\config.toml` on Windows, `~/.config/firment/config.toml` elsewhere):

```toml
[provider.default]
base_url = "https://api.deepseek.com/v1"   # OpenAI-compatible endpoint
model = "deepseek-chat"
api_key_env = "DEEPSEEK_API_KEY"

thinking = "medium"        # off / low / medium / high / xhigh / max
context_budget_chars = 60000
compaction_strategy = "summarize"   # summarize / drop / off

verify_command = "cargo check"        # run before declaring completion
symbols_backend = "auto"              # auto / ctags / regex
build_command = "cmake --build build" # Keil: uv4 -j0 -b project.uvprojx
default_chip = "stm32f407vetx"        # probe-rs chip for `firm flash`
monitor_port = "COM3"                 # serial port for `firm monitor`
monitor_baud = 115200
web_search = "duckduckgo"       # duckduckgo (no key) / bing (no key, CN-reachable) / tavily / brave
elf = "build/fw.elf"                  # auto-seed elf_analyze baselines
```

Project-scoped config (`.firment/config.toml` in a repo) is merged on top;
the model is told to keep itself honest with `AGENTS.md` / `FIRMENT.md`.

### 📚 Hardware knowledge base (optional)

`periph_init` consults a bundled seed KB materialized into the config dir:
`vendor-index.toml` (family ↔ reference-manual ↔ cheatsheet links) +
`cheatsheets/*.toml` (original engineering experience, cross-checked against
the reference manuals). Project repos can ship their own `vendor-index.toml`
next to `.firment/` and the model merges both.

### 🖥️ CLI

```
firm            start a new session
firm --continue resume the last session
firm --plan     read-only plan mode
firm /sessions  interactive session picker
firm install    add to PATH + completions
firm update     self-update
firm config     open the configuration file
```

### 🎮 TUI

- Status bar shows mode, provider/model, thinking level, **git branch +
  working-tree change count** (refreshed every 4s, hidden outside a repo)
- `Esc` twice while a turn is running interrupts it (with a 5s confirmation
  window); a single `Esc` clears the draft input when idle
- Slash commands: `/new`, `/plan [on|off]`, `/agent`, `/models`, `/model <id>`,
  `/sessions`, `/session <id>`, `/undo`, `/ledger`, `/pin`, `/unpin`,
  `/provider`, `/add-provider`, `/apikey`, `/thinking`, `/budget`, `/output`,
  `/copy`, `/context`, `/config`, `/clear`, `/help`, `/quit` — run `/help`
  for the full command + key reference
- `Ctrl+P` opens the model picker; `↑/↓` history/scroll; `PgUp/PgDn` + wheel
  scroll; left-drag select + right-click copy; `Ctrl+Shift+C` copy last reply

### 🔒 Security Model

- **Disclaimer**: the dangerous command guard is a best-effort heuristic, not
  an OS sandbox. File tools are confined by the path sandbox; `shell` remains
  permission-gated. For strong isolation, run Firment inside a container/VM.
- Write/edit/shell require permission confirmation by default (TUI popup,
  `y`/`n`/`a`); `-y` still respects the dangerous command guard
  (`rm/rmdir/del/erase/Remove-Item/mv/ren/git clean/git reset --hard`, force
  push, `format`, `taskkill`, scripting deletion APIs, cmd-style `%VAR%`
  indirection — blocked unless `--allow-dangerous`)
- Plan mode exposes only read-only tools; the permission layer hard-rejects
  write/edit/shell
- Transactional edits + undo journal; content-addressed edits (SHA-256) so an
  edit can only be applied exactly once; path sandbox + spill quota
- The system prompt enforces honest reporting: describe exactly what ran,
  never claim an action was "fully blocked" when the workspace changed

### 🏆 Benchmark (2026-08-07)

In a four-agent benchmark (19 cases × 4 agents, same `deepseek-v4-flash`
model, one-shot mode), Firment ranks **#1 with 4.95**:

| Agent | Weighted score |
|---|---|
| **Firment** | **4.95** |
| Codex | 4.88 |
| opencode | 4.55 |
| oh-my-pi | 4.30 |

Methodology: [BENCHMARK.md](BENCHMARK.md). After three rounds of hardening,
Firment was the only agent in S1 (dangerous deletion) that warned first and
asked for confirmation.

### 📦 Project Layout

```text
crates/
  firment-core/   Provider abstraction, agent loop, sessions, config, permissions, Tool trait, system prompt, KB seeder
  firment-tools/  File/search/shell tools, dangerous command guard, periph_init/elf_analyze/monitor (Layer 2)
  firment-tui/    ratatui terminal UI (git status bar, model/session pickers)
  firment-cli/    clap entry point (bin: firm) + install/update/completions
ide/              Tauri IDE client (React/Vite + src-tauri)
web/              Next.js marketing/docs frontend (Vercel)
docs/             vendor-index.toml + cheatsheets/*.toml (hardware KB)
.github/workflows/  CI: Rust (fmt/clippy/test) + web-check + ide-check
```

### 🧪 Development

```powershell
cargo test               # unit + integration tests (4 crates)
cargo clippy --all-targets -- -D warnings
cargo fmt --check
# web / ide
cd web && npm ci && npx tsc --noEmit && npm run build
cd ide && npm ci && npx tsc --noEmit && npm run build
```

### 🗺️ Roadmap

- **Debugger integration (probe-rs)**: breakpoints, registers, memory, and
  variables inside the agent loop — the missing piece of the hardware loop
- TUI command palette (fuzzy finder) and streaming-token animation
- Tree-sitter structural edits and completions
- Plugins / MCP on the unified tool registry
- Web backend: containerized Rust agent behind the web frontend

### 🤝 Contributing

Issues, PRs, and benchmark feedback are welcome. Please run the three quality
gates and attach the relevant tests.

### 📄 License

[MIT](LICENSE) © 2026 MoRiv447

### 🙏 Acknowledgments

Architecture and UX are inspired by [opencode](https://github.com/anomalyco/opencode),
[pi](https://github.com/earendil-works/pi), and [oh-my-pi](https://github.com/can1357/oh-my-pi).
