# Subagents: Firment compared with opencode

Written after reading opencode's source rather than its documentation. The files are
`packages/opencode/src/tool/task.ts` (371 lines), `packages/opencode/src/agent/agent.ts` (453)
and `packages/opencode/src/agent/subagent-permissions.ts` (27 — the whole thing), from
`sst/opencode` on the `dev` branch, which now redirects to `anomalyco/opencode`.

The point of the comparison is not imitation: the two projects made different bets and some of
Firment's are better. It is to find the places where opencode has already paid for a lesson.

---

## What each one does

**opencode.** A subagent is a **session with a `parentID`**, and its capabilities come from a
**named agent type** (`subagent_type`) with its own permission ruleset. The launch itself is a
permission-gated tool call (`ctx.ask({permission: "task", patterns: [subagent_type]})`). The
child's ruleset is *derived*:

```ts
// parent denies and external_directory rules only — the parent's GRANTS do not travel
[...parentSessionPermission.filter(r => r.permission === "external_directory" || r.action === "deny"),
 // children cannot spawn children or touch the todo list unless their own rules allow it
 ...(canTodo ? [] : [{ permission: "todowrite", pattern: "*", action: "deny" }]),
 ...(canTask ? [] : [{ permission: "task",     pattern: "*", action: "deny" }])]
```

It also has: `task_id` to **resume** a previous subagent session; a `background: true` mode
that returns immediately and notifies on completion (behind an experimental flag); depth
computed by walking `parentID` up the session tree; and results returned in a delimited
`<task id state><task_result>…</task_result></task>` block, with explicit prompt text telling
the parent *not to poll, not to duplicate the work, and to avoid the same files*.

**Firment.** A subagent is an **agent plus a chosen registry**: research children get
`plan_registry` (read-only, mutating tools never advertised), the red-team campaign gets
`attacker_registry`, depth is a counter (`max_subagent_depth`, default 2), the child runs in a
temp session store, and the parent's sink is bracketed so a UI can attribute nested events.
Permission is inherited wholesale (`permission.clone()`), and the bound on parallelism is one
shared semaphore across the whole tree.

## The differences, and what to do about each

| # | opencode | Firment | Verdict |
|---|---|---|---|
| 1 | Child capabilities from a **rule-level derivation**; the parent's denies propagate | Child capabilities from a **whole registry swap** | **Firment's is stronger for its case**: a mutating tool is never *advertised*, so the model cannot even try. opencode's rule model still advertises and denies at call time. **Keep Firment's** — but opencode's deny-inheritance is the piece to remember if a *new* registry is ever built from scratch (the red-team one is built that way today, and it re-derives its own denials by hand). |
| 2 | `task` denied in a child by default | `task` is in the research registry, bounded by depth | **Deliberate difference, keep it**: nested research is useful and the depth counter bounds it. Worth noting that opencode's safer default is safer *because* its depth default is 1. |
| 3 | `todowrite` denied in a child by default | `todo` was advertised to children and **fails on every call** (`session_dir` is `None`) | **Fixed in this commit** — along with `ask_user`, which likewise has no asker. A tool that fails on every call is a wasted round trip and a misleading error; the registry now drops both (`subagent_registry`). |
| 4 | Results tagged `<task id state>`; the prompt says what to do with them | Results prefixed `[subagent report]` | **Adopted (the cheap half) in this commit**: the report is now in `<subagent_report>` tags and the tool description says it is data to verify rather than instructions to follow — the untrusted-input rule the plugin review asks for, applied to the mechanism that already exists. The `id`/`state` half needs the factory to return them, which is a bigger change than it is worth today. |
| 5 | `task_id` **resumes** a subagent session | None: a child's session is created fresh and its store is a temp directory | **Real gap.** Worth doing, and cheaper than it looks — but it is a design decision (a resumable child needs a session that outlives the call), not a patch. Left as a proposal. |
| 6 | `background: true` returns at once and notifies | Parallelism is a *wave*: several `task` calls in one turn run together, and the parent waits | **Different trade, both defensible.** Firment's parent always has the answer before it continues (simpler reasoning, no notification plumbing); opencode's keeps working. Adopt only if someone asks for "kick it off and keep going" — the notification channel is the whole cost. |
| 7 | A `description` (3–5 words) names the task | The label comes from elsewhere | Minor; worth adding when a UI wants a child's purpose as a title rather than its first line. |
| 8 | Depth derived by walking the session tree | Depth is a counter passed down | Equivalent for correctness; **derived** survives a session being reloaded, which a counter does not. Not worth changing today, worth remembering if sessions become resumable (#5), where the counter would be wrong. |

## What this commit changed

1. `subagent_registry()` (`crates/firment-tools/src/lib.rs`) — `plan_registry` minus `todo` and
   `ask_user`, with a test asserting both the exclusion and that **plan mode keeps them** (a
   plan-mode session has a directory and a user; that is why this is a separate function).
2. The `task` tool's result is a delimited `<subagent_report>` block, and its description says
   the block is data to verify rather than instructions — with the test asserting all three
   parts, so the label cannot be dropped silently later.

## What is deliberately not taken

- **The `effect` stack.** opencode's tools are `Effect` programs with dependency injection and
  typed errors. It buys composition that Firment's `async_trait` + `ToolContext` does not need
  yet, at the cost of a much larger surface for anyone reading the code to learn first.
- **Per-agent-types as first-class configuration.** Firment's registries are code, chosen at
  assembly; opencode's agent types are data. The second is more flexible and more to get wrong.
- **Background jobs** (#6) and **resume** (#5) — proposals, not patches, for the reasons above.

---

*Sources read on 2026-09-20: `sst/opencode@dev` → `anomalyco/opencode`. Line counts and the
permission snippet are quoted from those files as fetched.*
