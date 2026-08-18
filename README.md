# Firment — Firmware + Agent

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Version](https://img.shields.io/badge/version-0.5.9-blue)](Cargo.toml)
[![Rust](https://img.shields.io/badge/rust-1.85+-deeppink)](Cargo.toml)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey)]()
[![CI](https://img.shields.io/badge/CI-Rust%20%2B%20Web%20%2B%20GUI-green)](.github/workflows/ci.yml)

**English** | [简体中文](README.zh-CN.md)

<p align="center">
  <img src="docs/screenshots/cli.png" alt="Firment TUI: ask_user dialog + tool call stack + status bar with git branch, thinking level and Esc×2 interrupt hint" width="900">
</p>

> ⚠️ **Status: v0.5.9.** Layer 1 (general coding agent) is
> production-usable and CI-green; Layer 2 (embedded toolchain loop) is
> shipped — build/flash/run, serial monitor, ELF analysis and on-target
> debugging via probe-rs all work — with deeper debugger features
> (backtrace, SWO trace) on the roadmap.

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
| **GUI client** | [`gui/`](gui/) | Tauri + React/Vite (TypeScript) |
| **Web** | [`web/`](web/) | Next.js + Tailwind (deployed on Vercel) — try it at [firment-web.vercel.app](https://firment-web.vercel.app) |

The CLI is the source of truth; the GUI client shares the same Rust agent
kernel through the unified `Tool` trait and session format, while the Web
surface is a TypeScript reimplementation kept in sync via a committed tool-spec
snapshot (`web/src/lib/tools/specs.json`).

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
  `ask_user`, `periph_init`, `elf_analyze`, `monitor`, `debug`
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
  re-analyzes after each edited turn. Growth above the configured thresholds
  blocks completion until you approve it (or, headless + `strict`, until the
  model fixes it); below-threshold noise is swallowed by default
- **`monitor`** — serial monitor with per-line timestamps and baud-rate
  autodetect; cancelling a turn releases the port immediately
- **`build` / `flash` / `run`** — CMake/Make/Keil build commands, probe-rs
  flashing (chip from `[tools] default_chip`), all wired into the agent loop
- **`debug`** — full on-target debugging over the probe via probe-rs
  (no OpenOCD/GDB dependency), so the agent can debug its own firmware:
  - `analyze` — one-shot fault diagnosis: halts the target, reads PC/LR/SP
    and the Cortex-M fault registers (CFSR/HFSR/MMFAR/BFAR), decodes PC/LR
    against the firmware ELF (`func+0x12`) and explains each set fault flag
    (IACCVIOL / IBUSERR / UNDEFINSTR / FORCED / VECTTBL / STKOF, ...)
  - `halt` / `regs` — pause the target and read the full register table; the
    target stays paused between calls until flashed, reset or `debug continue`
  - `mem` / `write` — read/write memory with `0x...` or `symbol:name`
    addresses (resolved from the ELF symbol table); `write` requires approval
  - `break` / `step` / `continue` — set a breakpoint and report registers when
    it hits, single-step, resume

### 🪟 Three surfaces, one kernel

| Surface | Screenshot |
|---|---|
| **GUI** — Tauri + React; tool cards stream live with success/failure, status pill tracks `idle Ns` / `tool Nm Ns`; the agent kernel lives in the same Rust binary as the CLI | <img src="docs/screenshots/ide.png" alt="Firment GUI: tool cards (list_dir ✓, read_file ✕, todo ✓) with the running periph_init… tool 16s counter" width="900"> |
| **Web** — Next.js on Vercel (`firment-web.vercel.app`); the same agent kernel reachable from any browser, with the same `Tool` trait and tool-spec snapshot | <img src="docs/screenshots/web.png" alt="Firment Web: ask 'Search for GPIO configuration patterns' — agent invokes grep/glob/list_dir/read_file and answers that the workspace is Next.js, not firmware" width="900"> |

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

# Full ELF gate policy (table form; string form above uses defaults):
# [tools.elf]
# glob = "build/fw.elf"
# stack_threshold = 32        # stack-depth growth (B) that blocks completion
# flash_threshold_kib = 1     # flash growth (KiB) that blocks completion
# report_benign = false       # surface below-threshold diffs (false = swallow)
# strict = false              # headless/CI: block until fixed, no soft downgrade
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
firm build      run the configured build command
firm flash      flash a firmware ELF via probe-rs
firm run        flash and run the target, streaming RTT logs
firm monitor    serial monitor with optional ELF symbol decoding
firm tools      print the tool registry specs as JSON (single source of truth)
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
  firment-tools/  File/search/shell tools, dangerous command guard, periph_init/elf_analyze/monitor/debug (Layer 2)
  firment-tui/    ratatui terminal UI (git status bar, model/session pickers)
  firment-cli/    clap entry point (bin: firm) + install/update/completions
gui/              Tauri GUI client (React/Vite + src-tauri)
web/              Next.js marketing/docs frontend (Vercel)
docs/             vendor-index.toml + cheatsheets/*.toml (hardware KB)
.github/workflows/  CI: Rust (fmt/clippy/test) + web-check + gui-check
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

- **Debugger depth**: stack unwinding / backtrace, variable & expression
  evaluation, SWO/trace streaming into the agent loop
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
