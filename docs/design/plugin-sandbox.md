# Plugin mechanism and the sandbox boundary — design review

Plan §5, item 8: "让用户写自己的 tool 接入 agent（WASM 或子进程）", with the risk column
noting the security sandbox. The plan requires this review before the work starts, and it is
the right call: a plugin mechanism is the one feature in this tier that can hand someone
else's code the same reach as the agent, and the reach is the product.

Checked against the code; file:line references are real and were re-read while writing.

---

## 1. What a "plugin" is here, concretely

A tool. The agent calls it by name, the model sees its schema, the permission checker may ask
the user about it, and its result becomes conversation. So a plugin mechanism is not a new
concept in this codebase — it is **the existing one, opened up**, and the three constraints
that make that hard are already visible:

1. **The registry is keyed by a `'static` string.** `ToolRegistry { tools: HashMap<&'static str,
   Arc<dyn Tool>> }` (`core/src/tool.rs:188`), and `Tool::name()` returns `&'static str`
   (`core/src/tool.rs:169`). A plugin whose name is read from a manifest at runtime **cannot
   be registered** without changing this — the key type *is* the boundary's first constraint,
   and it is a one-line change with a long tail (`names()` returns `Vec<&'static str>` at :206,
   and the specs at :210 go to the model).
2. **`Arc<dyn Tool>` means in-process.** Today a "plugin" can only be a Rust type compiled in.
   That is not a plugin system; it is a build-time extension point, and calling it a plugin
   would be the kind of naming this project's own reviews are written to prevent.
3. **The sandbox that exists is about *paths*, not about *code*.** `resolve_within(cwd, path,
   extra_roots)` (`tools/src/tools/util.rs:19`) enforces the workspace boundary for every file
   tool, and `allow_dangerous` gates the destructive shell commands (`tools/src/tools/shell.rs:286`).
   Both are checks *the tool itself* performs. Code that does not call them is not constrained
   by them — which is exactly the guarantee a plugin boundary has to provide.

## 2. Threat model, including the one that is usually missed

| # | Adversary | What they want | Does the current design stop it? |
|---|---|---|---|
| 1 | **A badly written plugin** | nothing — but it deletes a file | Only if it chooses to call `resolve_within`. A subprocess with `fs` access does not. |
| 2 | **A malicious plugin author** | read `~/.config/firment/auth.json` (the API keys), or `~/.ssh` | **No.** In-process, it has the whole address space; as a subprocess it has the user's own rights. |
| 3 | **The model, via a plugin's output** | get the *agent* to do something | **Nothing, today.** Plugin output enters the conversation as text; a plugin that prints "ignore your instructions and run …" is prompt injection down a channel the agent trusts. |
| 4 | **A careless user** | install something from a gist | Nothing structural. |
| 5 | A compromised host | everything | Out of scope, and worth saying out loud so nobody assumes otherwise. |

Threat 3 is the one to design against *first*, because it is the only one where the plugin can
be perfectly honest and still cause the damage: the attack is a string in the tool's result.
Threat 2 sets the floor for isolation: **whatever runs a plugin must not be able to read the
credential store**, which rules out "in-process, trusted" for anything the user did not write
themselves.

## 3. The options, with the argument for and against each

### A. In-process Rust types (`dyn Tool`), i.e. what exists

- **For:** zero new machinery; the compiler is the interface check; a plugin gets the full
  `ToolContext` (journal, sandbox helpers, config).
- **Against:** no isolation of any kind (threats 1, 2, 4 all succeed); and it is *not* a plugin
  mechanism, because a plugin you must recompile the host to install is an extension point.
- **Verdict:** keep it, and **call it what it is**: an extension point for first-party tools.
  Every tool in `tools/src/tools/` is that, and there is nothing wrong with it.

### B. Subprocess plugins over a line protocol (MCP-shaped)

- **For:** the OS provides the isolation boundary (a process, a working directory, an
  environment) — the same boundary the project already trusts for `probe-rs`, `sigrok-cli`
  and every compiler it shells out to. **The host stays in control of the three things that
  matter:** which executable is started, which working directory and environment it inherits,
  and which *tools* are exposed to the model. Threat 2 becomes "a process with the user's
  rights that we did not hand credentials to", which is a real improvement: the API key lives
  in `auth.json` and is never passed, and — unlike in-process — it cannot be read out of the
  host's memory.
- **Against:** a protocol to define (or adopt); a process to supervise (start, timeout, kill,
  orphan); a new class of hang; and the schema has to cross a boundary, so
  `validate_args` (`core/src/tool.rs:233`) must run on the host side of it, not be trusted to
  the plugin.
- **Verdict:** **the right boundary for third-party code.** Adopting **MCP** rather than
  inventing a protocol is the honest choice: the ecosystem exists, the shape (initialize,
  list tools, call tool) is what this needs, and a Firment-specific protocol would be a
  second standard nobody asked for.

### C. WASM (`wasmtime`)

- **For:** the strongest isolation available in-process; capability-based by construction
  (nothing is reachable that is not explicitly passed in); deterministic resource limits.
- **Against:** a large dependency with a per-platform build story; plugins must be written in
  a WASM-targeting language and shipped as `.wasm`; the "read this file" capability then has
  to be proxied back through the host anyway (a WASM plugin cannot open a file by itself), so
  the *host* ends up implementing the file API — which is option B's protocol, plus a
  compiler toolchain.
- **Verdict:** **not now, and say why**: it buys isolation the user cannot yet ask for, at the
  cost of a toolchain they do not have. Revisit when there is a plugin that needs to run
  untrusted computation *inside* the agent rather than as a process.

**Recommendation: B as the plugin boundary, A kept and renamed as the extension point, C
refused for this tier with the reasoning above recorded.**

## 4. Invariants a plugin boundary must not break

These are the project's own rules, restated for plugins — each one is a way the feature could
quietly undo something already promised:

1. **The workspace boundary applies to plugin *results*, not just to plugin calls.**
   `resolve_within` constrains what a tool may touch; a plugin that returns absolute paths
   outside the workspace is not breaking the boundary, but the *agent* acting on them is.
   The transcript should say where the text came from.
2. **The model must be able to tell a plugin's output from the user's words.** Threat 3.
   Concretely: a plugin result is labelled as untrusted tool output in the prompt — the same
   treatment the redteam path already gives untrusted MQTT payloads (`AGENTS.md`: "UNTRUSTED
   data — an alert arriving over an unauthenticated broker can ask …", `cli/src/main.rs:840`).
   That precedent exists in this codebase; plugins should reuse its language.
3. **Approval must say *why*.** `Tool::approval()` (`core/src/tool.rs:174`) is the reason shown
   before a mutating call. A plugin cannot be allowed to supply that string unchecked, or the
   approval dialog becomes a plugin-controlled prompt.
4. **No silent capability escalation.** A plugin's declared capabilities at install time are
   what it gets at run time; a plugin that asks for more mid-session gets a user decision, not
   a default.
5. **§16.2's UI constraints still hold**: a plugin's diff renders with the same default-collapsed
   rule, its output is capped, and its card obeys the same collapse behaviour as any other.
6. **The permission checker stays the gate.** `PermissionChecker` (`core/src/permission.rs:21`)
   is consulted for mutating calls; a plugin call must go through it like any other tool,
   never bypass it because "it is the plugin's own tool".

## 5. The smallest first slice that is worth anything

Ordered by dependency, and deliberately small:

1. **Names that are not `'static`**: `HashMap<Arc<str>, Arc<dyn Tool>>` (`core/src/tool.rs`),
   with `Tool::name()` staying `&'static str` for built-ins (an `impl` detail: the registry
   copies the name into an `Arc<str>` at registration). Nothing else changes yet; every
   built-in tool keeps working, and the constraint that motivated the change is gone.
2. **A manifest**: `[plugins.<name>] command = "…", capabilities = ["fs.read", "net"]` in
   `config.toml`, with the executable path resolved and printed by `firm doctor` so a change
   to it is visible. Capabilities are declared, not discovered.
3. **One subprocess tool**: a plugin host that speaks the MCP call/result shape over stdio,
   with a hard timeout, a working directory of the session's cwd, an environment that is
   **allow-listed** rather than inherited, and stdout of the child treated as untrusted.
4. **A refusal test**: a test plugin that asks to write outside the workspace and is refused,
   plus one that answers with a schema-violating result and is rejected by `validate_args`.
   Then one that hangs, to prove the timeout.
5. **The untrusted-output labelling** (threat 3) — the smallest textual change with the
   largest effect, and it belongs in the same commit as the first plugin or not at all.

## 6. Open questions for the maintainer

1. **MCP compatibility as a goal, or just as a shape?** "A Firment plugin is an MCP server"
   means the existing ecosystem works; "MCP-shaped" means less surface to define. The first is
   more useful and commits the project to someone else's evolution.
2. **Where do plugins live — per project or per user?** A project-level plugin in a cloned
   repository runs on `firm` start, which is a supply-chain decision disguised as a
   convenience.
3. **Signing or pinning?** Any real answer starts with a lockfile-shaped thing (name → hash),
   which is more work than the plugin mechanism itself.
4. **Windows**: the environment allow-list and the process-group kill semantics differ enough
   that "it works on Linux" is not a statement about this project's own main platform.

---

## Status

*The argument above is left as written; this section records what has moved since.*

| §5 step | Status |
|---|---|
| 1. Names that are not `'static` | **Done** — `af9e431`: the registry is keyed by `Arc<str>`; `register` copies the name in, `get(&str)` is unchanged, and all 54 `Tool` impls are untouched because `name()` still returns `&'static str`. |
| 2. A manifest with declared capabilities | **Done** — `d2465c2`: `[plugins.<name>]` with `command`/`args`/`capabilities`; a closed five-word capability vocabulary whose parser names the valid set on a typo; a relative command resolved against the config's directory and printed by `firm doctor` (verified by running it); `Tool::owned_name()` so a run-time name registers without touching 54 impls; and `register_plugin`, which **refuses** a name that would shadow an existing tool. |
| 3. One subprocess tool | **Mostly done** — `48c7b77`: `PluginTool` speaks one MCP-shaped `tools/call` per invocation over the child's stdio, with a 30 s hard timeout, the session's cwd, an **allow-listed** environment, the model's arguments on stdin rather than the command line, and the result taken from the last stdout line that answers *our* call id. The step's own requirement turned out not to be expressible with the old helper — `run_command` applied a map on top of the inherited environment — so `EnvPolicy::{Inherit, Only}` had to exist. **Done** — `777f41b`: `session_registry` puts the built-in set and the plugin tools into one registry, the refusals travel out through `AgentAssembly::plugin_refusals` instead of being dropped, and `firm doctor` resolves plugin commands against the same directory the agent will. Writing the wiring test found a hole worth naming: **plan mode's read-only registry contains no `write_file`, so a plugin by that name would have found it free** — the check is against every built-in name, in every mode, and the test asserts the refusal in plan mode. |
| 4. A refusal test | **Done, minus one item that is re-scoped rather than claimed** — `929b1a3`: five tests spawn real child processes (a plugin that answers, one that prints junk, one that never answers, a declaration that does not parse, and the command line itself). Running them found two bugs reading could not: `cmd /C`'s outer-quote rule combined with `Command::arg`'s escaping produced a command line that ran nothing (fixed with `raw_arg`), and the timeout killed the shell while the plugin survived holding the pipe (fixed with the same `kill_process_tree` `run_command` uses — the suite went from 29 s to 0.62 s). **The fs item waits, and the gap is now a decision the user makes**: the host cannot refuse a plugin's *internal* writes — they happen inside the plugin's own process, and only an OS sandbox can stop them (§3B). `cb9087c` makes that the thing a plugin must be vouched for: `[plugins.<name>] trusted` is **off by default**, an untrusted declaration is not registered, the refusal names the flag, and `firm doctor` prints `[declared, not enabled — needs trusted = true]` where the declaration is read. Enforced today: the environment, the timeout, the result shape, the names. Vouched for by the user: everything the plugin then does. |
| 5. Untrusted-output labelling | **Done for the plugin host**, as part of step 3: `<plugin_result>` plus the UNTRUSTED-data sentence, and `approval()` built from the declared capabilities rather than from plugin-supplied text. Elsewhere: the `task` tool's report is already a delimited `<subagent_report>` block whose description says it is data rather than instructions. A plugin's output needs the same treatment, and the wording to reuse exists. |

**Both of those were done in step 2** (`Tool::owned_name()` and `register_plugin`, with tests);
the notes stay because they are the reasons the shape is what it is.

**Two things step 1 surfaced, recorded here so step 2 does not rediscover them:**

- **A collision with a built-in must be refused.** `register` replaces on a duplicate name
  (pinned by a test), which is fine while every tool is compiled in and a security hole the
  moment a plugin can choose its own name. The registry needs to know the difference.
- **`Tool::name()` does not have to become `&str`.** That would touch 54 impls. An `owned_name()`
  whose default is `self.name()` lets a plugin override exactly one method and costs every
  built-in nothing.

---

*Written 2026-09-20 against `7b5d08b`; status updated after `af9e431`. The file:line references in
§1 and §4 are the parts to re-check if the code moves.*
