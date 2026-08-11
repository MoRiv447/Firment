# Changelog

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
