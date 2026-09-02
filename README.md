# Firment — Firmware + Agent

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Version](https://img.shields.io/badge/version-0.8.0-blue)](Cargo.toml)
[![Rust](https://img.shields.io/badge/rust-1.85+-deeppink)](Cargo.toml)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey)]()
[![CI](https://img.shields.io/badge/CI-Rust%20%2B%20Web%20%2B%20GUI-green)](.github/workflows/ci.yml)
[![Website](https://img.shields.io/badge/website-Firment%20official-ff5c9d)](https://moriv447.github.io/Firment-site/)

**English** | [简体中文](README.zh-CN.md)

<p align="center">
  <img src="docs/screenshots/cli.png" alt="Firment TUI: ask_user dialog + tool call stack + status bar with git branch, thinking level and Esc×2 interrupt hint" width="900">
</p>

**Firment is an AI engineering agent for firmware and embedded development.** Named after *firmament* — the sky above every embedded engineer — with the second **a** dropped to fuse *firmware + agent*. It takes a natural-language requirement and drives the whole loop from the same conversation: write code, build with the real toolchain, flash over the debug probe, monitor serial output, analyze the ELF, debug the target on-chip when it misbehaves, observe its physical behavior through photos and logic-analyzer waveforms, and — on request — red-team the firmware it just wrote.

The kernel is a Rust coding agent with an embedded-first toolchain. A firmware change is not "done" when the model prints code — it is done when it compiles, lands on the target, runs, and the observed output matches what was asked for; at the top of the evidence ladder, "observed" means a photo shows the LED lit or a logic-analyzer capture measures the waveform, never the model's say-so.

**One repo, three surfaces (monorepo):**

| Surface | Path | Stack |
|---|---|---|
| **CLI / TUI** | [`crates/`](crates/) | Rust (ratatui) — the core agent |
| **GUI client** | [`gui/`](gui/) | Tauri + React/Vite (TypeScript) |
| **Web** | [`web/`](web/) | Next.js + Tailwind (deployed on Vercel) — try it at [firment-web.vercel.app](https://firment-web.vercel.app) |

The CLI is the source of truth; the GUI client shares the same Rust agent kernel through the unified `Tool` trait and session format, while the Web surface is a TypeScript reimplementation kept in sync via a committed tool-spec snapshot (`web/src/lib/tools/specs.json`).

---

## 🔄 How it works

AI coding tools are very good at producing source code — but in embedded development, a plausible `main.c` is only the beginning. A firmware task is done when it builds, flashes, runs, and the observable output matches the expectation:

```text
Natural-language requirement
      │
      ▼
Understand board / chip / pins / peripherals
      │
      ▼
Generate or modify project files
      │
      ▼
Compile ───────────────┐
      │ success        │ failure
      ▼                │
Flash / deploy         └──► Read diagnostics ─► Patch ─► Rebuild
      │
      ▼
Observe serial / registers / runtime state
      │
      ├── expected evidence ─► elf_analyze gate ─► Done
      │
      └── mismatch / failure ─► Diagnose ─► Patch ─► Reflash ─► Re-check
```

Firment separates responsibilities instead of treating the model as a shell script generator:

- **The model reasons** about intent, failures, tradeoffs, and next actions.
- **Tools execute deterministic engineering operations**: compilation, flashing, serial monitoring, register access, ELF analysis, on-target debugging.
- **Evidence drives the next step** whenever the connected environment can provide it.

> **"The model wrote firmware" is not the finish line — "the firmware was verified on real hardware" is.**

---

## ✨ Key Features

### Agent core

- **Multi-provider**: Anthropic-compatible (`/v1/messages`) and OpenAI-compatible (`/chat/completions`, covering DeepSeek / GLM / Qwen / Ollama) streaming tool calls; DeepSeek V4 automatically uses official `thinking` + `reasoning_effort`
- **Thinking levels**: `off / low / medium / high / xhigh / max`
- **Built-in tools**: `read_file` (line-numbered pages), `write_file`, `edit_file` (anchor / line-range / hashline edits, unified diff echo), `list_dir`, `glob`, `grep`, `shell`, `web_search` (DuckDuckGo / Tavily / Brave), `web_fetch`, `task` (read-only research subagent), `todo`, `ask_user`, `hil`, `periph_init`, `elf_analyze`, `monitor`, `debug`, `observe`, `la`, `redteam`
- **Read-only plan mode**: `--plan` / `/plan` exposes only read tools and requires a decision-complete plan
- **Parallel tool calls**: independent calls run concurrently; same-file reads/writes and broad tools are ordered automatically
- **Engineering-grade system prompt**: communication, engineering principles, tool policy, verification, and safety sections, plus `AGENTS.md` / `FIRMENT.md` project instructions
- **Session management**: JSONL persistence, `--continue`, `--list`, interactive `/sessions` picker, change-ledger + `/undo`
- **Copy support**: left-drag select, right-click copy, `Ctrl+Shift+C` copies the last reply
- **Global install**: `firm install` adds PATH + completions; `firm update` self-updates

### Embedded toolchain

- **`periph_init`** — MCU peripheral init skeletons + knowledge-base cheatsheets. Full UART/GPIO/I2C/SPI/TIM/ADC HAL skeletons on STM32 (F1/F4/G0/**G4**/**H7**), ESP32/ESP32-S3 guidance, CubeMX/PlatformIO HAL-duplication warnings. The bundled KB covers real engineering traps: **G4's DMAMUX** (no fixed DMA channels — the classic F1→G4 migration trap) and **H7's D-Cache coherency** (clean before DMA TX, invalidate after DMA RX)
- **`elf_analyze`** — flash/RAM usage, function sizes, and real stack depth from `-fstack-usage` `.su` files; Firment auto-seeds a baseline and re-analyzes after each edited turn. Growth above the configured thresholds blocks completion until you approve it (or, headless + `strict`, until the model fixes it); below-threshold noise is swallowed by default
- **`monitor`** — serial monitor with per-line timestamps and baud-rate autodetect; cancelling a turn releases the port immediately
- **`la`** — logic analyzer over sigrok-cli (external binary, never linked): bounded captures stored under `.firment/la/`, deterministic measurements on the raw bits (frequency as a range, duty cycle, edge counts, pulse widths, bitrate) each with a confidence rating, and protocol decode through sigrok's own decoders (uart / spi / i2c / 1-wire / CAN …). HIL `la` steps assert `expect_frequency_hz` / `expect_duty` / `expect_edges` / `expect_decoded` at rung 5 — a low-confidence measurement can never pass
- **`hil`** — hardware-in-the-loop suite: one-shot `build → flash → monitor (with `expect_contains`/`expect_regex` assertions) → elf_analyze` via `.firment/hil.toml` suites or inline steps, with `dry_run` simulation, replayable JSONL logs (`hil replay`), auto serial port/baud, and total timeout — replaces manual build/flash/monitor chaining for firmware verification
- **`redteam`** — runtime adversarial verification: the agent attacks the firmware it wrote. Declarative suites in `.firment/redteam.toml` (same one-approval skeleton as HIL): uart interfaces + a valid baseline frame, a **seeded deterministic mutation corpus** (boundary / bitflip / oversize / format / delimiter / numeric — same seed, same byte sequence, so a finding's reproducer is `seed + case id`, no LLM needed), a crash oracle (fault signatures / boot-banner reappearance / heartbeat loss), budgets, and target recovery (reflash/reset — a board that cannot be revived aborts the run instead of poisoning later verdicts). Findings cite capture files; missing evidence caps severity to low/UNVERIFIED. An optional LLM attacker campaign explores on top of the corpus (interactive, target-locked to the suite's declared interfaces); headless live runs require an explicit `--live`
- **`build` / `flash` / `run`** — CMake/Make/Keil build commands, probe-rs flashing (chip from `[tools] default_chip`), all wired into the agent loop
- **`debug`** — full on-target debugging over the probe via probe-rs (no OpenOCD/GDB dependency), so the agent can debug its own firmware:
  - `analyze` — one-shot fault diagnosis: halts the target, reads PC/LR/SP and the Cortex-M fault registers (CFSR/HFSR/MMFAR/BFAR), decodes PC/LR against the firmware ELF (`func+0x12`) and explains each set fault flag (IACCVIOL / IBUSERR / UNDEFINSTR / FORCED / VECTTBL / STKOF, ...)
  - `halt` / `regs` — pause the target and read the full register table; the target stays paused between calls until flashed, reset or `debug continue`
  - `mem` / `write` — read/write memory with `0x...` or `symbol:name` addresses (resolved from the ELF symbol table); `write` requires approval
  - `break` / `step` / `continue` — set a breakpoint and report registers when it hits, single-step, resume
  - `backtrace` — halt and unwind the call stack against the firmware ELF (DWARF-based; the firmware must be built with `-g`)
  - `trace` — stream SWO/ITM trace packets (`probe-rs itm swo`); probe-rs configures CoreSight itself, the firmware just writes ITM ports
  - `forensic` — one-command hard-fault post-mortem: halts the target, captures the exception frame + Cortex-M fault registers + a 64-word stack window, decodes the fault site and candidate call chain against the ELF, re-reads PC to detect a corrupted scene (watchdog reset race), correlates the capture against the session change ledger (7-day window, newest first), and snapshots the report under the session dir. Approval-exempt: the scene is ephemeral

### Verification & evidence

A firmware task is not done on the model's say-so — the gates below enforce verification mechanically, and the system prompt keeps the model honest about *which* level of evidence it actually reached:

1. **Code-level** — files were generated or modified as intended.
2. **Build-level** — the real compiler/toolchain accepted the project.
3. **Deployment-level** — firmware was written to the target.
4. **Runtime-level** — serial output / registers match expectations (HIL `expect_*` assertions).
5. **Physical-behavior-level** — a real external effect was observed (sensor, probe, or explicit user confirmation).

A successful compile or flash does **not** automatically prove the physical task is correct. Three enforcement layers back this up:

- **`verify_command` gate**: a configured command (e.g. `cargo check`) runs before the agent may declare completion; if it fails, the agent must keep working
- **ELF regression gate**: `elf_analyze` baselines are re-checked after every edited turn — flash/RAM growth or stack-depth growth above threshold blocks completion
- **HIL end-to-end suites**: `hil` ties build → flash → monitored output (with `expect_contains`/`expect_regex` + `expect_count` assertions) → ELF analysis into one repeatable, replayable verification run, with `dry_run` for rehearsal
- **`observe` gate (physical level, automated)**: deterministic local CV on photos of the target answers "is the LED lit / where is the bright region" (`mode=brightness`, with an automatic ROI suggestion), "did anything move across a burst of shots" (`mode=motion`), "does it blink and how fast" (`mode=blink` — frequency as a range, not a false-precise number) and "did the change actually move pixels" (`mode=diff`, before/after). Every verdict carries a confidence rating, and HIL steps assert these verdicts (`expect_lit` / `expect_motion` / `expect_blink_hz` / ...) — a low-confidence answer can never pass an assertion — so rung 5 is verifiable by the agent itself without a vision model
- **Fault-signature markers**: captured serial/RTT output is scanned for fault signatures (panic, HardFault, BusFault, ...); on a hit the agent is pointed at `debug forensic` immediately — before a watchdog reset destroys the scene
- **`la` gate (physical level, automated)**: waveform evidence — a frequency assertion passes only when the wanted value sits inside the measured range and the estimate is not low-confidence; decoded protocol text (e.g. the UART bytes actually on the wire) counts as physical evidence too

### SBC edge data plane

Run the guard + small-model classification on a local single-board computer (Debian + systemd): mosquitto broker, ollama small models, the `sbc-guard` daemon, and PC-side provider config. From-zero walkthrough with a per-failure troubleshooting table: **[docs/sbc-setup.md](docs/sbc-setup.md)**. Acceptance is one command: `firm --doctor --sbc` (six checks, fix hints on every failure).

### Three surfaces, one kernel

| Surface | Screenshot |
|---|---|
| **GUI** — Tauri + React; tool cards stream live with success/failure, status pill tracks `idle Ns` / `tool Nm Ns`; the agent kernel lives in the same Rust binary as the CLI. Project workbench: session tree, pin registry, ADR decisions, device bindings, parallel chats | <img src="docs/screenshots/ide.png" alt="Firment GUI: tool cards (list_dir ✓, read_file ✕, todo ✓) with the running periph_init… tool 16s counter" width="900"> |
| **Web** — Next.js on Vercel (`firment-web.vercel.app`); the same agent kernel reachable from any browser, with the same `Tool` trait and tool-spec snapshot | <img src="docs/screenshots/web.png" alt="Firment Web: ask 'Search for GPIO configuration patterns' — agent invokes grep/glob/list_dir/read_file and answers that the workspace is Next.js, not firmware" width="900"> |

---

## 💬 Typical session

```text
You > PA0 has an LED. Make a 1 kHz PWM breathing-light demo on this STM32.

Firment > [read_file platformio.ini] → [periph_init tim2_pwm] → [edit_file main.c]
         → [build] ✓ → [flash] ✓ → [monitor] "LED ON" ×2 → [elf_analyze] ✓

You > The serial port is silent. Diagnose it.

Firment > [debug analyze] → target halted at HardFault_Handler, CFSR=IACCVIOL
         → decodes PC against the ELF → reads the source at the fault PC
         → finds a bad pointer → [edit_file] → [build] → [flash]
         → [monitor] "LED ON" ×2 → done
```

## 🚀 Quick Start

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

## ⚙️ Configuration

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

Project-scoped config (`.firment/config.toml` in a repo) is merged on top; the model is told to keep itself honest with `AGENTS.md` / `FIRMENT.md`.

## 📚 Hardware Knowledge Base (optional)

`periph_init` consults a bundled seed KB materialized into the config dir: `vendor-index.toml` (family ↔ reference-manual ↔ cheatsheet links) + `cheatsheets/*.toml` (original engineering experience, cross-checked against the reference manuals). Project repos can ship their own `vendor-index.toml` next to `.firment/` and the model merges both.

## 🖥️ CLI

```
firm            start a new session
firm --continue resume the last session
firm --plan     read-only plan mode
firm /sessions  interactive session picker
firm install    add to PATH + completions
firm update     self-update
firm config     interactively pick a provider from the neutral catalog (endpoints + key, writes config.toml)
firm build      run the configured build command
firm flash      flash a firmware ELF via probe-rs
firm run        flash and run the target, streaming RTT logs
firm monitor    serial monitor with optional ELF symbol decoding
firm hil        run a hardware-in-the-loop suite (--suite/--steps/--replay/--dry-run)
firm redteam    run a red-team attack suite (--suite/--replay/--list-suites/--dry-run/--live)
firm tools      print the tool registry specs as JSON (single source of truth)
firm --doctor   check config + provider connectivity
firm --doctor --sbc
                end-to-end check of the SBC edge-model data plane: broker link,
                guard heartbeat freshness, model endpoint (verifies the model is
                actually pulled), bound devices. Each failing stage prints a fix hint.
```

## 🎮 TUI

- Status bar shows mode, provider/model, thinking level, **git branch + working-tree change count** (refreshed every 4s, hidden outside a repo)
- `Esc` twice while a turn is running interrupts it (with a 5s confirmation window); a single `Esc` clears the draft input when idle
- Slash commands: `/new`, `/plan [on|off]`, `/agent`, `/models`, `/model <id>`, `/sessions`, `/session <id>`, `/undo`, `/ledger`, `/pin`, `/unpin`, `/provider`, `/add-provider`, `/apikey`, `/thinking`, `/budget`, `/output`, `/copy`, `/context`, `/config`, `/clear`, `/help`, `/quit` — run `/help` for the full command + key reference
- `Ctrl+P` opens the model picker; `↑/↓` history/scroll; `PgUp/PgDn` + wheel scroll; left-drag select + right-click copy; `Ctrl+Shift+C` copy last reply

## 🔒 Security Model

- **Hardware disclaimer**: Firment executes real commands against connected hardware, and its output is engineering work that still needs a human in the loop. Before anything touches power electronics, motors, heaters, batteries or a one-of-a-kind prototype: decide your own current/voltage/speed bounds, keep an independent way to recover and re-flash, and treat a green build or a finished flash as exactly that — neither of them tells you how the device actually behaves.
- **Evidence levels**: verification is a ladder — (1) code, (2) build, (3) deploy, (4) runtime, (5) physical behavior. Each level only counts when actually observed; passing one never implies the ones above. `build`/`verify` outputs carry an `[evidence: build]` tag and HIL suites report the highest level they attempted (`evidence: reached level N (…)`), so completion reports state what was proven, not assumed. On power electronics, motors, heaters or batteries, physical behavior must additionally be bounded by YOUR independent limits — the agent has no sense of current, torque or heat.
- **Disclaimer**: the dangerous command guard is a best-effort heuristic, not an OS sandbox. File tools are confined by the path sandbox; `shell` remains permission-gated. For strong isolation, run Firment inside a container/VM.
- Write/edit/shell require permission confirmation by default (TUI popup, `y`/`n`/`a`); `-y` still respects the dangerous command guard (`rm/rmdir/del/erase/Remove-Item/mv/ren/git clean/git reset --hard`, force push, `format`, `taskkill`, scripting deletion APIs, cmd-style `%VAR%` indirection — blocked unless `--allow-dangerous`)
- Plan mode exposes only read-only tools; the permission layer hard-rejects write/edit/shell
- Transactional edits + undo journal; content-addressed edits (SHA-256) so an edit can only be applied exactly once; path sandbox + spill quota
- The system prompt enforces honest reporting: describe exactly what ran, never claim an action was "fully blocked" when the workspace changed

## 📦 Project Layout

```text
crates/
  firment-core/   Provider abstraction, agent loop, sessions, config, permissions, Tool trait, system prompt, KB seeder
  firment-tools/  File/search/shell tools, dangerous command guard, periph_init/elf_analyze/monitor/hil/debug
  firment-tui/    ratatui terminal UI (git status bar, model/session pickers)
  firment-cli/    clap entry point (bin: firm) + install/update/completions
gui/              Tauri GUI client (React/Vite + src-tauri)
web/              Next.js marketing/docs frontend (Vercel)
sbc-guard/        SBC-side collector + deterministic guard (Python, MQTT + ollama)
docs/             vendor-index.toml + cheatsheets/*.toml (hardware KB)
.github/workflows/  CI: Rust (fmt/clippy/test) + web-check + gui-check
```

## 🧪 Development

```powershell
cargo test               # unit + integration tests (4 crates)
cargo clippy --all-targets -- -D warnings
cargo fmt --check
# web / gui
cd web && npm ci && npx tsc --noEmit && npm run build
cd gui && npm ci && npx tsc --noEmit && npm run build   # + npm run tauri build for installers
```

## 🗺️ Roadmap

- Debugger depth: variable & expression evaluation at a halted site (fault forensics shipped in v0.7.0)
- Logic analyzer phase 2: Saleae REST backend, SBC-side waveform nodes (the `CaptureBackend` seam is ready)
- Red team phase 2: rtt / device_cmd attack interfaces, corpus back-port tooling for campaign findings
- TUI command palette (fuzzy finder)
- SWO/trace streaming deeper into the agent loop
- Tree-sitter structural edits and completions
- Plugins / MCP on the unified tool registry
- Web backend: containerized Rust agent behind the web frontend
- Skills: installable tool packs (declarative external-command tools + schemas + prompts)

*(Streaming-token animation shipped in v0.6.3 — time-driven spinners, batched deltas, cached wrapping.)*

## 🤝 Contributing

Issues and PRs are welcome. Please run the three quality gates (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`) and attach the relevant tests.

## 📄 License

[MIT](LICENSE) © 2026 MoRiv447
