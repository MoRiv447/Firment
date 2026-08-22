# Changelog

## v0.5.13 (2026-08-22) — RP2040 + ESP32-S31 knowledge, honest non-ARM guards

- **Knowledge base: Raspberry Pi RP2040** (seed v6 → v7, re-materializes
  automatically). Two cheatsheets verified line-by-line against the official
  pico-sdk hardware headers (hardware/gpio.h funcsel table, platform_defs.h,
  multicore.c) and the RP2040 datasheet:
  - `rp2040-gpio` — platform/boot facts agents get wrong: no internal flash
    (XIP through a 16 KB cache; flash writes need RAM-executed code),
    CRC32-checked 256-byte boot2 whose failure silently drops to BOOTSEL
    UF2 mode, F1=SPI/F2=UART/F3=I2C on every pin, multi-input logical-OR
    pitfall, ADC only on GPIO26-29 (+temp ch4), PLL_USB must stay exactly
    48 MHz (USB+ADC+RTC), WDT capped ~8.4 s by errata RP2040-E1
  - `rp2040-uart` — two PL011-based UARTs, baud divisor comes from clk_peri
    which the SDK retimes onto the 48 MHz PLL_USB when clk_sys changes,
    DREQ numbers for DMA pacing
  - `periph_init` resolves `rp2040*` parts and injects the cheatsheets;
    `vendor-index.toml` gains the `[rp2040]` family. probe-rs supports the
    chip locally, so build → flash → monitor works end-to-end.
- **Knowledge base: ESP32-S31** (also seed v7). Two cheatsheets built from
  the official ESP32-S31 Series Datasheet v0.5 PRELIMINARY (2026-07-13) —
  Espressif's new dual-core RISC-V 320 MHz flagship (Wi-Fi 6 + BT 5.4
  LE/Classic + 802.15.4):
  - `esp32s31-gpio` — 60 GPIOs with numbering holes (GPIO29/41 do not
    exist), strapping on GPIO36/37/60/61 (boot mode = GPIO61:GPIO60; 0,0 is
    invalid), console on GPIO58/59, USB-Serial/JTAG on GPIO33/34, pad JTAG
    on GPIO54-57, flash bus GPIO26-28/30-32, ADC1=GPIO42-49 / ADC2=GPIO50-57,
    DAC on GPIO4/5, no input-only pins (unlike S3)
  - `esp32s31-uart` — FOUR UART controllers + LP_UART (S3 has three),
    5 MBaud, LP_UART fixed pins GPIO2-7, hw-flow-control/LP_I2C conflict
  - `periph_init` resolves `esp32s31*` parts (matched before the `esp32s3`
    prefix); `vendor-index.toml` gains the `[esp32.s31]` family (datasheet
    link; TRM still pending upstream at review time).
- **Honest non-ARM guards for the debug/HIL tools.** `debug analyze` decodes
  Cortex-M fault registers (CFSR/HFSR/MMFAR/BFAR) and `trace` streams
  SWO/ITM — both are ARM CoreSight features that simply do not exist on
  Xtensa/RISC-V targets like the ESP32 family. Both now decide up front via
  `decode::non_arm_reason()` — the firmware ELF's architecture when that
  file is readable, else the probe-rs chip name (`esp32*`) — and fail with
  the alternative path (halt/regs/backtrace + console panic report, or a
  `monitor` step) instead of analyzing garbage or capturing an empty
  stream. The ELF wins over the chip label so a mislabelled chip cannot
  block a real ARM ELF. Known gap: non-ARM chips without an elf parameter
  and without an `esp32*` name still pass the guard.
- **flash: ESP32 failure hint.** When flashing an `esp32*` chip fails,
  the error now points at `espflash` / `idf.py flash` as the vendor-supported
  fallback — probe-rs support for the family (especially Xtensa parts and
  brand-new silicon) lags Espressif's own tooling.

## v0.5.12 (2026-08-21) — one wiring point, in-process GUI hardware

- **Shared agent assembly (`firment_tools::assembly::assemble_agent`).** The
  `Agent` construction sequence (`Agent::new` + ~15 `set_*` calls + plan-mode
  registry/permission selection + subagent factory + cancel-handle
  extraction) was duplicated by hand in the TUI, the GUI and the CLI — and
  the copies had already drifted. The assembly module is now the single
  wiring point; each frontend only supplies its I/O adapters (sink /
  permission checker / asker) and gets back a fully configured agent with
  pre-extracted cancellation handles. Two real drift bugs fall out of this:
  - **The verify hard gate never fired in the TUI/GUI.** Only the CLI wired
    `set_verify_command`, so a configured `verify_command` was silently
    ignored outside headless mode. It now gates turn completion everywhere.
  - **`tools.max_subagent_depth` from config was ignored in the TUI/GUI**
    (always the built-in default). Now honored.
- **`ToolsConfig::default()` now matches the serde defaults.** The derived
  `Default` produced `monitor_baud = 0` and `max_subagent_depth = 0` (the
  latter clamped to 1 by the agent) whenever a config was built in memory
  rather than parsed from config.toml — diverging from the documented
  115200 / 2. Hand-written `Default` aligns the two paths; surfaced by the
  assembly migration now that every frontend wires these knobs.
- **GUI flash/run is in-process.** New `firment_tools::hardware::{flash_elf,
  run_elf}` reuse the exact agent-tool probe-rs pipeline (workspace sandbox,
  install hints, ST-Link stuck-probe diagnostics, error mapping); the GUI no
  longer locates and spawns an installed `firm.exe`, so it can never drift
  from the CLI. Serial monitoring was already in-process; the
  `hardware-exit` event payload is unchanged.
- **TUI split into modules.** The 4,961-line `firment-tui/src/lib.rs` is now
  a facade (`run` + the event loop + terminal setup) over `adapters.rs`
  (event sink / permission checker / ask_user bridge), `commands.rs` (the
  agent command loop), `app.rs` (application state), `view.rs`
  (transcript rendering), `paste.rs` (bracketed-paste burst collapsing),
  `pickers.rs` (model/session overlays) and `util.rs` (text/layout helpers).
  Pure code movement — no behaviour change.
- `Agent::registry()` / `Agent::verify_command()` /
  `Agent::max_subagent_depth()` accessors added to firment-core (used by
  assembly tests to assert plan mode hides mutating tools and that the
  verify gate / subagent depth knobs actually land).

## v0.5.11 (2026-08-21) — hardware-in-the-loop suites

- **New `hil` tool — one-shot firmware verification.** Orchestrates
  `build → flash → monitor/trace → elf_analyze` with assertions in a single
  call, replacing manual tool chaining:
  - Suites live in `.firment/hil.toml` (`[suite.<name>]` + ordered steps) or
    inline via `steps=[...]`; `firm hil --suite/--steps/--chip/--port/--dry-run`
  - Step kinds: `build` / `flash` / `run` / `monitor` / `trace` / `elf_analyze`
    / `delay`; monitor/trace support `expect_contains` / `expect_regex` +
    `expect_count` assertions with early exit
  - Auto serial port/baud detection, per-suite and total timeouts,
    cancel-aware; `dry_run` simulates hardware steps (assertions always FAIL
    under dry-run — no hardware data)
  - Every run writes `work/hil/<uuid>.jsonl`; `hil replay list` / `hil replay
    <id>` renders a human-readable log
  - flash/run/elf steps auto-infer `.pio/build/*/firmware.elf` (newest
    mtime) when `elf` is omitted
- **`debug trace` via hil** — SWO/ITM capture (`duration_ms`/`clk_hz`/`baud`,
  chip via `PROBE_RS_CHIP` env) as a suite step with the same expectations
  as monitor
- **Actionable ST-Link diagnostics.** All probe-rs invocations (flash/run/
  debug) now map the Windows/WinUSB "reset not supported by WinUSB" stuck-
  probe failure (probe-rs issue #2207) to a clear error telling the model/
  user to unplug+replug the probe and close ST tools holding it, instead of
  an opaque failure; duplicated install-hint mappings unified into one helper
- **System prompt clarified** — `hil` needs a `.firment/hil.toml` suite (or
  inline steps; fall back to manual build→flash→monitor otherwise); `dry_run`
  never counts as verification; the `verify_command` gate still applies for
  quick compile checks on top of hil's end-to-end verification

## v0.5.10 (2026-08-18) — debugger depth: backtrace + SWO trace

- **`debug backtrace` — halt and unwind the call stack.** Runs the probe-rs
  REPL `bt` command against the firmware ELF (DWARF-based, frame by frame
  with function names and source locations). Requires an ELF built with
  `-g`; when the firmware has no DWARF info probe-rs prints nothing, so the
  tool detects that and tells the agent to rebuild with `-g` / `-Og`.
- **`debug trace` — stream SWO/ITM trace packets.** Wraps `probe-rs itm swo`
  with a built-in capture window (`duration_ms`, default 3 s), the TPIU
  clock (`clk_hz`, default 170 MHz) and SWO baud (`baud`, default 2 Mbps).
  probe-rs configures the target's CoreSight TPIU/ITM itself, so no
  firmware changes are needed to enable tracing — the firmware just has to
  write ITM ports (e.g. `ITM_SendChar`) for data to appear. Two probe-rs
  0.32 quirks are handled: the capture duration is only checked while
  packets arrive (an idle SWO stream blocks forever), so a timeout is
  treated as a completed empty capture instead of an error; and SWO-failed
  probes are not retried.
- **CFSR flag decoding corrected (ARMv7-M).** Every UFSR bit in the flag
  table was shifted by one — `0x00010000` was reported as `[INVSTATE]`
  instead of `[UNDEFINSTR]`, and the BFAR-valid check tested bit 24 instead
  of bit 14 (`BUSFAULTVALID`). Caught while diagnosing a real fault on a
  test rig.
- **Provider: no more 400s from deepseek02.** Three layered fixes for the
  `api.deepseek.com/anthropic` endpoint: tool-call arguments are never
  emitted as a non-object (models streaming fenced/text/trailing-comma
  JSON were previously passed through as strings), arguments persisted as
  strings in old sessions are normalized on the way out, and every message
  is guaranteed non-empty content (the `messages.203: all messages must
  have non-empty content` rejection).
- **TUI: no more frozen UI on huge repos / trapped approval queues.** Git
  status refreshes on its own task instead of blocking the event loop
  (Esc/Ctrl+Q stayed dead on network drives), and global shortcuts
  (Ctrl+Q/Ctrl+C/Ctrl+Shift+C/Ctrl+V) stay live while a permission or
  question modal is up.
- **Cancel interrupts flash/debug sessions** — `run_probe_rs` aborts the
  probe-rs child on turn cancellation (flash/debug runs up to 180 s are now
  interruptible like `monitor`), and killed children are reaped with
  `wait()` to avoid zombies on Unix.

## v0.5.9 (2026-08-18) — on-target debugging: the agent can debug its own firmware

- **New `debug` tool** — full inspect/control of the target over the debug probe
  (probe-rs, no OpenOCD/GDB dependency), letting the agent debug firmware
  autonomously:
  - `analyze` — one-shot fault diagnosis: halts the target, reads PC/LR/SP and
    the Cortex-M fault registers (CFSR/HFSR/MMFAR/BFAR), decodes PC/LR against
    the firmware ELF (`func+0x12`) and explains each set fault flag
    (IACCVIOL / IBUSERR / UNDEFINSTR / FORCED / VECTTBL / STKOF, ...).
  - `halt` / `regs` — pause the target and read all core registers; the target
    stays paused between calls until flashed, reset or `debug continue`.
  - `mem` / `write` — read/write memory with `0x...` or `symbol:name`
    addresses (resolved from the ELF symbol table, no guessing addresses);
    `write` requires user approval.
  - `break` / `step` / `continue` — set a breakpoint and report registers when
    it hits, single-step, resume.
- **Agent workflow:** the system prompt now includes a debug step — when the
  target misbehaves/hangs/prints nothing, the agent runs `debug analyze`, reads
  the source at the decoded PC, verifies runtime state via `mem symbol:...`,
  and iterates fix → flash → monitor until the fault is gone.
- Shared probe-rs runner extracted from the flash tool (timeout + install-hint
  handling reused); `regs` parsing is tolerant of probe-rs output layout
  changes (falls back to the raw output).

## v0.5.6 (2026-08-18) — knowledge base expansion: ESP32-C6/C3 + STM32 depth

- **Knowledge base: 15 new cheatsheets** (28 total), seed v4 → v5
  (re-materializes automatically on first run after update).
  - New chip families: **ESP32-C6** (RISC-V, WiFi 6 + BLE 5 + 802.15.4) and
    **ESP32-C3** (RISC-V, WiFi 4 + BLE 5) — GPIO/platform + UART, with
    family entries in `vendor-index.toml` (official TRM/datasheet links) and
    quickrefs; `periph_init`'s family detection now resolves `esp32c6`/
    `esp32c3` (also `esp32c3fn4` etc.) before the generic `esp32` fallback.
  - STM32G4 depth: **pinout** (G431RB LQFP64 has no Port E, NUCLEO LED=PA5 /
    button=PC13 / VCP=LPUART1), **tim** (timer clock = PCLK×2, TIM2 is the
    only 32-bit timer), **gpio** (pins come up in analog mode after reset),
    **adc** (2× 12-bit ADC, calibration required), **flash** (single bank,
    no read-while-write, 2 KB pages, double-word programming), **iwdg**
    (LSI-based, window mode, RCC_CSR reset-cause flags).
  - STM32F4 depth: **gpio** (GPIOA-H, FT pins, SWD/HSE traps), **tim**
    (32-bit TIM2/5, timer clock computation, advanced-timer MOE).
  - ESP32 classic depth: **uart** (console pins + GPIO-matrix routing),
    **adc** (ADC2 unusable under WiFi, attenuation + eFuse calibration),
    **spi** (HSPI/VSPI defaults, SPI0/1 are the flash bus).
- `periph_init` now injects the matching family cheatsheet for ESP32-C6/C3
  and STM32F4/G4 tim/gpio/adc/flash/iwdg; generic fallbacks still apply
  where no family file exists.

## v0.5.5 (2026-08-17) — anthropic protocol fixes + TUI interaction fixes

### Anthropic providers (deepseek02 etc.)

- **Parallel tool results are merged into one user message.** The Anthropic
  protocol requires every `tool_use` block of an assistant message to be
  answered by `tool_result` blocks inside the single immediately-following
  message; separate results per call made DeepSeek's anthropic-compatible
  endpoint reject the request with 400 (tool_use ids found without
  tool_result blocks immediately after). OpenAI/agnes was unaffected.

### TUI

- **ask_user:** digit keys (1-9) only pick an option while the answer input
  is still empty; typing a free-form answer like "nucleo g431" no longer
  accidentally selects option 4 mid-word. Type + Enter submits.
- **Permissions:** concurrent requests (parallel tool waves) are queued and
  shown one at a time instead of the new one denying the pending one — the
  user decides each request in turn. Session swap and `/new` deny the whole
  queue.

## v0.5.4 (2026-08-15) — auto-reset after flash + GUI header badge

- **flash** resets the target after flashing by default (`reset` param, default
  true), so the firmware starts running without pressing the board's reset
  button. Implemented as `probe-rs reset --chip <chip>` after a successful
  `probe-rs download`; pass `reset: false` to leave the target halted.
- System prompt's embedded-workflow step 4 now tells the agent the flash tool
  resets the target by default.

### GUI

- The header session badge shows the model name only (the provider/model
  "agnes/agnes-2.5-flash" label was truncated in its small white box).

## v0.5.3 (2026-08-15) — edit reliability + build auto-detection + prompt hardening

### Correctness

- Sandbox `[Permission]` rejections no longer roll back the edit batch:
  writing outside the workspace used to revert the wave's good edits (the
  "main.c keeps getting reverted" loop).
- Max iterations now KEEPS consistent edits (committed as an undo entry)
  instead of rolling back; only unverified mutations (verify gate configured
  but never passed) are rolled back.
- Fixed the max-iterations hint: `/continue` does not exist — keep typing to
  continue, `/undo` to revert.

### Tools

- `build` auto-detects the project's build system when `build_command` is
  unset: platformio.ini / Makefile / CMakeLists.txt / *.uvprojx (up to 2
  levels of subdirectories, cd-ing into the manifest's directory), reporting
  `[auto-detected]`. Standard projects need zero configuration.

### Prompt / KB

- Prompt hardened into general principles: KB-first for MCU peripherals,
  build via the build tool, `[Permission]` outside-workspace is a hard sandbox
  limit, prefer local files over web_fetch, prefer the built-in tools over
  python scripts (with fallbacks — no absolute bans), no proactive
  `.firment.toml` reads.
- ST-Link VCP fact moved out of the prompt into the stm32g4-uart cheatsheet's
  `common_mistake` list (KB `SEED_VERSION` bumped to 4).

## v0.5.2 (2026-08-15) — embedded-workflow guidance + out-of-the-box flash/serial

### Agent guidance

- **System prompt rewritten to be directional**: a new "Embedded firmware workflow"
  section gives the agent a five-step decision chain (reconnaissance → configure
  → build → flash → verify) so it knows what each step should produce instead of
  casting around.
- **Prefer tools over filesystem hunting**: the prompt tells the agent to use the
  dedicated firmware tools (which know the configured toolchain) and to report a
  missing binary and ask the user instead of hunting the filesystem for
  compilers or probe-rs; `probe-rs list` / serial enumeration / `probe-rs chip
  list` are reserved for when a step genuinely needs that specific fact.
- **Tool priority**: dedicated firmware tools (build / flash / monitor /
  elf_analyze / periph_init) are preferred over raw shell; the prompt no longer
  suggests Keil/IAR as alternative toolchains and says to reuse the project's
  existing build system.
- **Failure-diagnosis rules**: no more resubmitting the same command with
  different shell syntax (cmd /c vs powershell -Command vs quoting); no
  recursive whole-drive directory scans — use glob/grep instead.

### Tools

- **shell**: strips a model-added outer pair of double quotes (the common
  `"probe-rs --version 2>&1"` wrapping that broke `cmd /C`) and its description
  now states the command is passed verbatim — do not quote it or prefix it with
  `cmd /c` / `powershell -Command`. Default timeout stays 120s.
- **monitor**: when no port is configured, the tool now enumerates serial ports
  and lists them (`COMx (manufacturer product serial)`) in the error guidance.
- **flash**: a missing chip id now points the agent at `default_chip`, the
  project config, `probe-rs chip list`, or the startup file instead of failing
  silently.
- **edit_file / read_file CRLF handling**: CubeMX/Keil files are CRLF while
  models write LF anchors, so `old_text` matched 0 times and edits failed on
  every Windows-generated file. Anchors are now matched LF-normalized and the
  file's own line endings are restored on write; `read_file` strips CR from
  displayed lines and hashline anchors so both stay consistent.
- **Batch rollback narrowed**: only state failures ([ConcurrentChange] / [Io])
  roll the batch back; an [InvalidInput] / [NotFound] call (which never touched
  the file) no longer undoes earlier successful edits in the same turn.
- **edit_file [NotFound]** now tells the model the file does not exist and to
  create it with write_file first, instead of failing cryptically.
- **spill file GC**: referenced spill files are no longer mis-garbage-collected
  on Windows (path-separator bug), so session reloads keep their spill pointers.

### 重构

- **GUI rename**: `ide/` → `gui/`, product renamed "Firment IDE" → "Firment GUI"
  (installer/bundle names, config, docs).

### Configuration

- Global `[tools]` defaults (`default_chip`, `monitor_baud`) are documented in
  the prompt so build / flash / monitor work out of the box without a per-project
  `.firment.toml`.

## v0.5.1 (2026-08-14) — ELF gate completion + correctness fixes

### ELF binary-analysis gate (Layer 2)

- **RAM threshold**: `[tools.elf] ram_threshold_kib` now blocks completion on
  RAM growth too, not just flash and stack — RAM is usually the tighter
  resource on MCUs.
- **New-function stack**: a function newly added to the build counts its whole
  stack depth as growth, so a new deep-frame function can no longer slip past
  the stack threshold.
- **Clone-suffix normalization**: `foo` → `foo.isra.0` / `.constprop.1` /
  `.part.2` across builds is matched as the same function, so -O2 clone renames
  don't misreport stack growth.
- **Fail-closed verdict**: a missing `[GATE:...]` marker now blocks instead of
  silently downgrading to a soft review.

### Correctness fixes

- Rolling back a failed edit batch no longer inserts a standalone assistant
  message (which produced `assistant -> assistant -> tool` and 400'd the next
  provider request); the note is folded into the first tool result.
- `read_file` no longer emits a phantom empty line-numbered row after a
  trailing newline.
- Context summarization is bounded by the stream timeout so a stalled summary
  cannot hang the turn.
- `ToolContext` permission default is now deny-all (fail closed) instead of
  auto-approve; the stream-timeout docs now describe it as an inactivity
  timeout, not a hard wall-clock cap.

### Docs

- Web surface documented as a TypeScript reimplementation (kept in sync via the
  tool-spec snapshot) and flash chip documented as coming from `default_chip` —
  both wording fixes to match reality.

## v0.5.0 (2026-08-13) — unified versioning + IDE turn-hang fixes

- **Versioning**: CLI/TUI, IDE and Web now share one version (0.5.0); the
  beta marker is dropped
- **IDE**: tool waves that miss their permission dialog no longer wedge the
  whole batch — permission requests are queued in the UI and time out
  (120s) on the backend if unanswered; `ask_user` also times out (180s)
- **IDE**: a running turn can now be interrupted by switching/creating a
  session (the switch auto-cancels the turn first); `Info` events (stall /
  tool-wave timeout / compaction notices) are shown in the chat with a
  live `idle Ns` / `tool Nm Ns` counter so a slow model is distinguishable
  from a wedged turn
- **Agent kernel**: hard timeouts so a turn can never hang forever — the
  provider stream is bounded (120s mid-stream stall guard) and each tool
  wave is bounded (600s + 5s grace), both configurable per agent
- **web_search (bing)**: challenge pages and empty results are now detected
  and reported instead of silently returning zero results (all surfaces)

## v0.4.0-beta.8 (2026-08-12) — audit-driven hardening + experience fixes + TUI polish

### Full-crate audit fixes (3 parallel reviews, 44 findings; 19 fixed)

- `read_file`: `saturating_add` on offset+limit (a huge limit could panic / wrap);
  header line range now 1-based, matching the body prefixes
- **Compaction**: the summary is folded into the first user message instead of
  prepending a second consecutive user turn — role-alternation providers
  (Anthropic) 400'd on the first compaction; per-request `max_tokens` (the
  summarization cap) is now honored by the Anthropic provider
- `decode_address`: symbols must cover the address (addr < addr+size), so gap
  addresses are no longer attributed as huge +0x offsets
- `shell`: cmd-style `%VAR%` expansion is flagged (indirect execution)
- `periph_init`: non-STM32 parts (ESP32) no longer receive STM32 HAL skeletons
- `truncate`: keeps head + tail, so trailing compiler errors survive truncation
- **Esc interrupt** now fires the pre-extracted cancel handles directly from the
  event loop (bypassing a command channel that can be blocked on the agent lock
  during a long turn); busy submits no longer wipe the draft input
- Spill cleanup skips files still referenced by the current transcript
- Session state: rollback after MaxIterations is persisted; `replace_session`
  resets per-turn bookkeeping (ledger seq, read hashes, elf gate); project
  config merges max_iterations / thinking / context_budget_chars
  (auto_approve is deliberately NOT merged — a project must not grant itself
  tool auto-approval)
- CLI `--max-output-tokens` clamps to u32 (was silent wraparound); TUI
  `/provider` `/session` `/pin` `/unpin` report a full channel instead of
  silently dropping the command; todo saves atomically (tmp+rename)

### Experience fixes

- TUI model-picker / `/models` run the (network) model-list request outside the
  agent lock — a slow provider no longer freezes the command loop
- `symbols`: ctags runs on a blocking thread with a hard 60s timeout (child
  killed) — huge trees no longer freeze the async runtime or hang forever
- `monitor`: the read loop checks the turn cancellation flag, releasing the
  serial port immediately on cancel
- `web_fetch`: oversized pages are explicitly marked `[truncated...]` instead
  of silently returning a partial body

### TUI polish

- **Git status bar**: `git: <branch> · N` (branch + working-tree change count),
  refreshed every 4s in the background, hidden outside a repo; uses
  `branch --show-current` so fresh repos (pre-first-commit) still show the name
- Input placeholder decluttered (keybindings now live only in `/help`)

## v0.4.0-beta.7 (2026-08-12) — real-world toolchain conflicts + peripheral skeleton expansion

### periph_init — full skeletons for SPI / TIM / ADC

- `spi` / `tim` / `adc` now get complete STM32-HAL skeletons like uart/gpio/i2c (clock enable,
  GPIO/AF reuse, handle + `HAL_*_Init`, `TODO(fill)` markers, conflict notes); only `dma` still
  falls back to the generic skeleton + cheatsheet
- **CubeMX conflict guidance**: if the project already has generated init (`MX_*_Init` /
  `HAL_*_Init` / `SystemClock_Config` in `main.c` / `*_hal_msp.c`), call the existing functions —
  never re-initialize, redefine functions **or handle variables** (e.g. `huart1` already exists
  in CubeMX `main.c`); only handwritten projects should land the skeleton
- **Framework HAL-duplication warning**: detects `.ioc` (CubeMX) vs `platformio.ini`
  (PlatformIO) and warns that copying CubeMX `Drivers/` into a PlatformIO project redefines
  every HAL symbol (the classic two-HAL conflict); CubeMX projects are told to reuse existing
  init code

### elf_analyze — real stack depth from `.su` files

- GCC/Clang `-fstack-usage` produces per-object `.su` text files, not an ELF `.stack_usage`
  section — Firment now scans the ELF directory tree for `*.su`, parses the GCC format, and
  attaches per-function stack depth (with `-O2+` clone-suffix matching, `foo` → `foo.isra.0`);
  stack depth is reported missing only when neither source exists. Works with stock CubeMX /
  Makefile builds that add `-fstack-usage`

### monitor — timestamps + baud autodetect

- Every captured line is prefixed with its arrival time `[SS.mmm]` (default on)
- `autodetect: true` probes common baud rates (9600..921600) and uses the first that yields
  valid data, reporting the detected rate — no more guessing the target's baud

## v0.4.0-beta.6 (2026-08-12) — periph_init peripheral code generation + KB reliability

### New tool: `periph_init`

- Generates MCU peripheral **initialization skeletons** (STM32 HAL style) with the matching
  knowledge-base cheatsheet injected (clock domain, DMA channel mapping, common pitfalls) —
  so peripheral bring-up starts from a correct frame instead of scratch
- Full skeletons for `uart` / `gpio` / `i2c` (baudrate, pins, DMA and interrupt options);
  `spi` / `tim` / `adc` / `dma` fall back to a generic skeleton + cheatsheet
- Part-number family auto-detection (`stm32f1` / `stm32f4` / `stm32g0` / `esp32` / `esp32s3`),
  with fallback to a family-generic cheatsheet (e.g. `stm32-dma.toml`) when no family-specific
  one exists; unknown families degrade gracefully with an explicit hint
- Output carries `TODO(fill)` markers for project-specific parameters (pins, clock tree,
  CubeMX config); available in plan mode; 5 tests
- Prompt guidance: use `periph_init` for peripheral bring-up (do not write init code from
  scratch) and a build-fix-rebuild loop (`[CompileError]` → `path:line` → edit → rebuild)

### Knowledge-base reliability

- Seed KB materialization is now **atomic** (tmp-file + rename) and serialized behind a
  process-wide lock in `periph_init`, so concurrent callers never observe a half-written
  cheatsheet (fixed a CI test race: exit 101 on both ubuntu and windows)

### Housekeeping

- Removed third-party agent product names from README / BENCHMARK and code comments
  (project convention); README built-in-tools list updated

## v0.4.0-beta.5 (2026-08-12) — Read/Edit efficiency loop + on-demand KB seed + TUI session fixes

### Read/Edit efficiency loop

- **`edit_file` echoes a unified diff** of the change (capped 4k) instead of only line
  counts, so the model sees exactly what landed and no longer needs to re-read the file to
  confirm; system prompt updated accordingly (use the smallest unique `old_text`, 2-4 lines;
  re-read only when the diff is missing)
- **`read_file` now prefixes every line with a line number** (`"  123 | content"`) for
  precise edit anchors and `path:line` references; without an explicit range it returns at
  most 1000 lines with a `[truncated: file has N lines; pass offset=N]` paging hint
  (explicit `offset` still reads to the end)
- `hashlines=true` keeps reading the whole file so large-file hash anchors stay valid
  (fixed a regression where the 1000-line cap also applied to hashline mode)

### On-demand knowledge base (context fixed-cost)

- The built-in vendor index (~12k chars) is no longer embedded in every system prompt;
  the prompt now tells the model to `read_file` `config-dir/kb/vendor-index.toml` on
  demand — saves ~12k chars per request. Project-level `vendor-index.toml` hints are
  unchanged

### TUI session management fixes

- `/delete <id>` refuses to delete the **active** session (the in-memory session would
  re-create its files on the next save and the removed undo dirs break edit rollback);
  start `/new` first
- `/sessions` picker now shows the **full UUID** on its own line (was truncated to 8 hex
  chars), with new keys: `c` copies the selected id to the clipboard, `d` deletes it;
  `/help` updated to document that drag-select is unavailable inside the TUI (mouse
  capture) — use `c` instead

## v0.4.0-beta.4 (2026-08-11) — security audit fixes + context/output tuning + TUI commands

### Security fixes (audit findings P0-1/P0-2, P1-1/P1-2/P1-3)

- **`shell` cwd boundary**: the `cwd` argument is now resolved through `resolve_within`,
  enforcing the workspace boundary exactly like every other tool (previously any path —
  including absolute paths outside the workspace — was accepted)
- **Project-config trust**: when a project `.firment.toml` overrides `build_command` /
  `verify_command`, `build`/`verify` are dropped from `auto_approve`, so a command from an
  untrusted checkout can never run without an explicit human approval
- **Shell metaprogramming guard**: `dangerous_reason` now flags `$()`-style command,
  parameter and arithmetic substitution, process substitution (`<()` `>()` `=()`), IFS
  injection, `/proc/self/`, `/etc/passwd`/`/etc/shadow`, and SSH credential access — closing
  blacklist bypasses like `d$(echo el) f.txt` in one-shot mode
- **`web_fetch` SSRF guard**: private (RFC1918), CGNAT, link-local/cloud-metadata
  (`169.254/16`), `0.0.0.0/8` and IPv6 unique/link-local targets are refused, on both the
  initial URL and every redirect; loopback stays allowed (local services, tests)
- **`run_command` output drain**: deadline raised 5s → 15s and truncation is now marked
  explicitly (`[output truncated: …]`) instead of silently dropping output

### Context and output sizing

- Session context budget default raised **60k → 256k chars** (`context_budget_chars`), and a
  new `max_output_tokens` (default **32k**) caps per-reply output with precedence
  config > provider > 32768; applies to one-shot, TUI and research subagents
- New CLI flags `--context-length <chars>` and `--max-output-tokens <n>` accepting binary
  `k`/`m` suffixes (`256k` = 262144); project config can override `max_output_tokens`

### TUI commands

- `/budget <chars>` — set and persist the session context budget (e.g. `/budget 256k`)
- `/output <tokens>` — set and persist the output cap (e.g. `/output 32k`); rebuilds the
  provider so it applies to the very next request
- `/context` — show live per-part context usage (system prompt / tool schemas / messages)
  as chars and percent of budget
- `/delete <session-id>` — delete a session and all its sidecars (transcript, undo, spill,
  ledger, pins)
- User-level memory: `~/.config/firment/AGENTS.md` is loaded before project instructions
  (applies to every project; project files still win on conflict)

### Process-tree kill fix

- Unix `kill_process_tree` no longer signals process groups (the pgid guard could race and
  SIGKILL the runner's own job tree, the root cause of CI hangs); it now walks the process
  table and kills each descendant pid directly

## v0.4.0-beta.3 (2026-08-10) — automatic binary-analysis gate

- **`[tools] elf`**: configure a glob of the firmware ELF (e.g. `build/fw.elf`) and the
  harness takes over the binary-analysis soft gate end to end. On every turn start it
  re-seeds the per-session ELF baseline (so edits are always measured against where the
  turn began); before accepting a finished turn that mutated files it auto-runs
  `elf_analyze` on the newest matching ELF and hands the diff (flash/RAM deltas, function
  size changes, stack-depth changes) back to the model as a tool message, asking it to
  review and decide — completion is never blocked (kept soft), but regressions that
  compile fine now surface automatically instead of by-agent-memory
- `elf_analyze` tool itself unchanged (baseline caching, `-fstack-usage` support, explicit
  `baseline` argument) — this release wires it into the agent loop (`Agent::set_elf_glob`,
  TUI + one-shot CLI + research subagents), with integration tests covering the gate
  injecting the diff and skipping when unconfigured
- Support for single-segment `*` and cross-directory `**` globs when locating the ELF
  (skips hidden dirs / `target/` / `node_modules`)

## v0.4.0-beta.2 (2026-08-09) — Web research tools + subagents + question modal

### New agent tools

- **`web_search`**: search the web without an API key via DuckDuckGo, or via Tavily /
  Brave with a key (`[tools] web_search`, `web_search_api_key`, `web_search_api_key_env`).
  Rate-limit handling for the free endpoint: one shared browser-like client with a cookie
  jar, a 3s process-wide pacing between requests, one retried request with backoff, and a
  Lite-endpoint fallback when the HTML endpoint is challenged; challenge/anomaly pages are
  detected and reported as `[Blocked]` errors instead of fake empty results
- **`web_fetch`**: fetch an http/https URL and return its readable text (HTML stripped);
  capped at 200 KB body / 60K chars, so datasheets and vendor pages are usable
- **`task`**: spawn a read-only research subagent (same session provider/model, optional
  `model`/`cwd` override) that returns a report — can read files, search/fetch the web,
  keep todos and ask the user, but cannot modify the workspace; recursion bounded by
  `[tools] max_subagent_depth` (default 2), subagent sessions kept in a temp dir
- **`ask_user`**: interactive question modal in the TUI — press 1-9 to pick an option,
  type a free-form answer, Esc to dismiss; only intended for decisions/information that
  only the user has
- **`todo`**: session-scoped todo list (`todos.json` in the session work dir) with
  `list` / `add` / `done` / `rm` / `clear`; survives context compaction
- **`elf_analyze`**: read-only binary analysis of compiled firmware (ELF) — flash/RAM
  usage per section, largest functions, and per-function stack depth when built with
  `-fstack-usage` (GCC/Clang); the tool caches a baseline per ELF path and auto-diffs the
  next build, surfacing stack-depth growth and flash/RAM regressions that still compile
  cleanly. The system prompt guides the agent to run it after builds (soft gate)

### Plan mode & prompt

- **Plan mode extended**: the read-only registry now also exposes `web_search`,
  `web_fetch`, `task`, `todo`, `ask_user` (10 read-only tools total); PLAN mode and
  research subagents share the same registry, so investigation can go online
- System prompt gained a **Research** section (search first, fetch known URLs, ask the
  user only what only they know, todo for multi-step work); PLAN-mode instructions
  updated to match

### Serial monitor + activity indicators + KB dedup

- **`firm monitor` as an agent tool**: serial monitoring is callable by the agent
  (`[tools] monitor_port` / `monitor_baud` fallback); bounded capture with a default 10s
  timeout, `--elf` symbol decoding on log lines, workspace sandbox on the firmware path,
  and an approval step before opening the port (errors tagged `[InvalidInput]`/`[Io]`)
- **TUI activity indicators**: the status bar shows what the agent is working on
  ("working · searching main.rs…", with a count like "2× flashing…" when tools run in
  parallel); running tool cards spin (`◐◓◑◒`); the hint derives from the tool name and
  its file/path/pattern argument
- **KB dedup**: when a project carries its own `docs/vendor-index.toml` identical to the
  seed, the hint is no longer injected twice (content-identical check), with two
  regression tests

### Config & plumbing

- New `[tools]` options: `web_search`, `web_search_api_key`, `web_search_api_key_env`,
  `max_subagent_depth` (project `.firment.toml` overrides merged as before)
- `ToolContext` gained a `with_cwd` constructor and a `Default` impl (safe defaults,
  permission = auto-approve); `ToolError` now implements `Display`/`Error`
- One-shot CLI mode now wires the same subagent runner, web search, session work dir,
  build/chip/monitor settings into the agent

## v0.4.0-beta.1 (2026-08-08) — English UI + `/new`

- **Full English pass**: all TUI/CLI output, permission cards, status bar, help text,
  tool errors, system prompt, and the bundled hardware knowledge base (index + cheatsheets)
  are now English; the agent replies in English by default unless asked otherwise
- **New `/new` command**: starts a fresh conversation in the TUI (keeps the current
  provider/model), replacing the session without restarting
- README slash-command list updated with `/new`; seed KB version stamp bumped so the
  English knowledge base re-materializes

## v0.4.0-beta.1 Paste-send fix (2026-08-08)

- **Ctrl+V paste fix**: Windows terminals inject pasted content as a fast key stream
  (a trailing Enter triggers submit) — added paste-burst detection: a stream of
  pure-text keys arriving within 35ms is recognized as one paste; Enter during the
  burst is treated as a newline, and after silence the whole block goes through the
  folded-paste path
- ASCII first character briefly held (30ms) to avoid single-key flicker; CJK/IME
  characters are not held and are recovered via retro-capture to prevent stray characters
- Protection window: Enter arriving within 120ms after a paste is treated as a newline,
  preventing accidental sends right after pasting
- Event-loop ticker shortened from 100ms to 25ms so held/buffered characters land on time
- Added 7 paste-burst state-machine unit tests (multi-line stream, single char + Enter,
  slow input, CJK recovery, protection window, modifier passthrough, no char loss)

## v0.4.0-beta.1 SHA-256 hash anchoring + Layer 2 build/flash tools (2026-08-08)

### SHA-256 hash anchoring

- **Hash anchoring**: read_file appends the whole-file `[file-sha256: ...]` to its output;
  edit_file / write_file accept `expected_sha256` as a pre-check — a mismatch returns
  `[ConcurrentChange]` together with the current hash, preventing edits based on stale content
- **hashline content-hash anchors** (inspired by omp): read_file supports `hashlines=true`,
  printing an 8-char content hash per line; edit_file supports `hashline` / `end_hashline`
  for line-level location by content hash (uniqueness-checked) — a missing hash means the
  file has changed (`[ConcurrentChange]`), a duplicated hash means ambiguity and is rejected
- No-op edits (target equals replacement) are now a hard error, stopping repeated no-op loops
- Change ledger records old_sha256 / new_sha256 per file (content-verifiable); read dedup
  upgraded to SHA-256
- Prompts / README: prefer hashline location for large files

### Layer 2: build & flash tools

- **firm build** (tool + CLI): runs `[tools] build_command` (CMake/Make/Keil/IAR command
  lines); failures return `[CompileError]`
- **firm flash** (tool + CLI): probe-rs download to flash ELF (`--chip` / default
  `default_chip`, `--probe` supported); path sandbox restricts firmware files to the
  workspace; install hint shown when probe-rs is missing; flash is a dangerous operation
  requiring confirmation
- **firm run / firm monitor**: probe-rs download + reset + run with streamed RTT logs
  (timeout 0 = wait for Ctrl-C); serial monitor (`[tools] monitor_port` / `monitor_baud`),
  with `--elf` decoding 0x addresses in logs to function symbols (object crate)
- New run tool (agent-callable, default 30s bounded capture); util supports
  `timeout_ms=0` for infinite wait
- **Project-level config**: `.firment.toml` / `firment.toml` at the project root (including
  ancestor directories) override the global config.toml for `[tools]`; the system prompt
  guides the agent to read the project config first, fill missing fields with write_file,
  then call build/flash/run — self-service build/flash from the TUI
- Default auto_approve includes build (no confirmation), flash always asks for confirmation

## v0.3.0-beta.7 Knowledge base maintenance patch (2026-08-08)

- Fixed esp32-gpio strapping notes: strapping pins are GPIO 0/2/4/5/12(MTDI)/15(MTDO);
  GPIO4 selects VDD_SDIO voltage, GPIO12(MTDI) selects flash voltage (GPIO4 had been
  wrongly removed and VDD_SDIO mislabeled onto GPIO12)
- Unified KB `common_mistake` to array format (eliminating the three mixed shapes:
  index scalar / cheatsheet array / tagged table); full TOML validation passes
- vendor-index.md "auto-discovery" statement verified against code (implemented via
  `load_vendor_index_hint` prompt injection); restored the original accurate wording

## v0.3.0-beta.7 (2026-08-08)

- **Hardware knowledge base (seed)**: added 3 families — STM32 F1 (RM0008), STM32 G0
  (RM0444), ESP32-S3 — with 7 original cheatsheets (USART/DMA, TIM/PWM, clock tree,
  GPIO/EXTI, LPUART, USB-Serial, etc.)
- **KB auto-discovery**: when a project contains `docs/vendor-index.toml`, a hint is
  auto-injected into the prompt telling the agent to check the KB before answering
  hardware questions
- **KB integrity tests**: verify index/cheatsheet TOML parses, `quickref.cheatsheet`
  links are valid, and `meta.schema_version` exists
- Fixed esp32-gpio strapping pin list (0/2/4/5/12/15; GPIO6-11 are SPI flash pins)
- Removed stale .example template; README/docs links point to the official index

## v0.3.0-beta.6 (2026-08-08)

- README: name easter egg — Firment comes from *firmament* (the sky dome), deliberately
  missing one **a**, fusing firmware + agent (EN + CN)
- Config template & README: documented `compaction_strategy` default and option semantics
  (`summarize` / `drop` / `off`)
- Pin large-file budget guard: `/pin` files exceeding 30% of the context budget show a TUI
  warning suggesting only critical source files be pinned
- README security note: added a one-line example for version-pinned installs with
  `FIRMENT_VERSION`
- Repo description updated to: *General-purpose coding agent for firmware & embedded
  development, built in Rust.*

## v0.3.0-beta.5 (2026-08-08)

- New Pin: `/pin <path>` / `/unpin <path>`, persisted per-session; pinned files are
  re-filled verbatim into context during compaction
- New `compaction_strategy`: `summarize` (default, old turns fully summarized) / `drop`
  (last 3 turns verbatim + middle 5 summarized + older dropped) / `off` (no auto compaction)
- Symbol index upgrade: prefers universal-ctags automatically (JSON output, 60s cache),
  falls back to built-in regex when not installed; new `[tools] symbols_backend =
  auto | ctags | regex`
- README: fixed `context_budget_chars` config level (top-level, not `[tools]`); added
  examples for new options

## Earlier versions (summary)

- **v0.3.0-beta.1**: edit reliability stack — transactional edits/`/undo`, CAS against
  concurrent overwrites, verify tool, diff-first approval, parallel tool calls, context
  compaction, symbol index (regex), failure classification
- **v0.3.0-beta.2**: tool output overflow, parameter schema validation, change ledger
  (`/ledger`)
- **v0.3.0-beta.3**: model-summary compaction, cached stable prefix, duplicate-read
  dedup, eviction by API turn
- **v0.3.0-beta.4**: verify hard gate (enforced in code), path sandbox, CI (fmt/clippy/test
  on both platforms)
- **v0.2.x**: dangerous-command guard hardening, TUI enhancements, GitHub Releases
  one-click install, open-sourcing (MIT / EN+CN README / BENCHMARK)
