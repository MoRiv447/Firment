# Parallel subagents and the concurrency model — design review

Plan §5, item 2, with the risk column reading "并发编辑冲突（需文件级锁）". The plan requires
this review before the work starts. It is the review of the three that argues the most against
its own item's stated implementation, and it does so because the code says something the item's
one-line description does not.

Checked against the code; file:line references were re-read while writing.

---

## 1. Subagents exist, and they are read-only **by registry choice**

Item 2 reads as if subagents have to be built. They do not:

- `SubagentRunner` is real (`core/src/subagent.rs:65`), depth-bounded
  (`max_subagent_depth`, enforced in `tools/src/tools/task.rs:38`), and reachable from the
  model as the `task` tool.
- **The research subagent is given the read-only registry**:
  `SubagentRunner::new(…, plan_registry(), …)` (`tools/src/assembly.rs:167-169`), and the tool
  says so in its own description: "read-only research subagent … cannot modify the workspace
  or ask the user" (`tools/src/tools/task.rs:15`).
- There is already a **second** kind of runner with a different registry — the red-team one,
  `attacker_registry()` (`tools/src/assembly.rs:186-188`).
- Nested runs are already bracketed on the parent's sink, so a UI can attribute them
  (`core/src/subagent.rs:176-181`).

So the pattern "**a subagent is an agent plus a chosen registry**" is not a proposal; it is how
the feature works today. **Item 2 is therefore one specific change: give a parallel run a
*write-capable* registry.** That is a much smaller change than the item suggests — and it is
the entire risk of the item, which is why the plan flagged it.

Consequence for this review: the question is not "how do we add concurrency?" but "**what
makes handing several agents a writable workspace safe?**"

## 2. Two kinds of parallelism are being conflated, and only one is risky

| | What runs in parallel | Conflict possible? | Value |
|---|---|---|---|
| **Research parallelism** | several read-only subagents over different files | **No** — nothing writes | high, and it is what `task` is used for today |
| **Edit parallelism** | several writers, one working tree | **Yes**, by definition | high, and it is where every hazard lives |

The plan's item counts as one feature; they are two, with very different risk. Shipping the
first is nearly free (it is the current behaviour, made concurrent), and it delivers most of
what people mean by "faster". Shipping the second is the part that needs a model.

**Recommendation: split the item.** Parallel *research* first, with no new synchronisation at
all; parallel *edits* only behind declared scopes (§3).

## 3. The concurrency model, three candidates

### A. File-level locks (what the plan suggests)

- **For:** intuitive; allows two writers of unrelated files to proceed even when their plans
  interleave; familiar.
- **Against:** three costs that a project of this size pays and does not need to:
  1. **Deadlock** needs a lock order, and a lock order needs a global rule that every future
     tool respects — which is a policy the codebase does not currently have anywhere.
  2. **Stale locks** outlive a crashed child; the recovery path is "some files are unusable
     until a human or a timeout clears them", which is exactly the kind of state this project
     avoids elsewhere (the journal is per-turn and self-clearing; sessions are atomic writes).
  3. **A lock only binds those who take it.** The real guarantee today is the **CAS** —
     `edit_file` re-reads and refuses when the content changed (`tools/src/tools/edit_file.rs`,
     `[ConcurrentChange]`) — and that guarantee holds for *any* writer, including a human in
     another editor and a second Firment session in a GUI pane. A lock adds coordination on
     top of the one mechanism that already works without being asked.

### B. Declared partitions (recommended)

Each child is **assigned a set of paths at planning time**, and its file tools refuse anything
outside that set. This is the same shape as the existing workspace boundary — `resolve_within`
(`tools/src/tools/util.rs:19`) is already a scope check, so a child's scope is a second,
narrower one rather than a new mechanism.

- **For:** no deadlock (ownership is static, not acquired); no stale state (the scope dies with
  the child); no new protocol; and the *plan* becomes reviewable before anything runs — the
  parent can print "child A owns `crates/foo`, child B owns `docs/`" and a human can object.
- **Against:** no over-subscription (two children cannot both touch `Cargo.toml`), so the
  planner has to be a little smarter. That is a feature: the alternative is a lock, and a
  wait, and a deadlock.
- **Overlap is not an error to hide:** if two scopes intersect, the run either refuses at
  planning time or **degrades to sequential** for the shared path. Silence is the only wrong
  answer.

### C. One writer at a time

Serialise every mutating call through a single owner. Trivially correct, and the honest
fallback when a partition cannot be found. It costs the parallelism the item exists for — but
for *edits*, which is the smaller half of the value (§2).

**Recommended combination: B for edit parallelism, C as the fallback, A not at all — with the
CAS staying the last word in every case.** A lock would be the right answer for coordinating
*independent processes that do not share this codebase's journal*; that is not a problem this
project has, and building for it now would buy the costs without the case.

## 4. Invariants a parallel run must not break

1. **One turn, one transaction.** A child's edits belong to the parent's batch, so a failed
   batch rolls back whole — the same all-or-nothing rule `rename_symbol` already implements
   and tests. Half-applied parallel edits are worse than no parallelism.
2. **The CAS is never bypassed.** A child's write goes through the same
   `[ConcurrentChange]` check as the parent's; a scope is a *permission*, not a substitute for
   detection.
3. **The journal stays single-owner.** `ToolContext { journal: Arc<Mutex<EditJournal>> }`
   (`core/src/tool.rs:22`) is per-turn state; parallel children sharing it is fine *because* it
   is a mutex, but the ordering of `begin`/`commit` across children must be defined, not
   incidental. (Today it cannot come up: the research child has no write tools.)
4. **Sink bracketing stays** (`core/src/subagent.rs:176-181`) — with N children the
   bracketing has to identify *which* child, not merely that a nested run happened.
5. **§16.2's UI constraints hold for a batch**: defaults collapse, a batch is not a wall of
   diffs, and progress that finishes in under two seconds still shows nothing.
6. **The permission checker stays the gate** for every child's mutating call, and approval
   prompts must name the child — a dialog that says "edit file" without saying *who* asked is
   a dialog nobody can answer.

## 5. Journal ownership: the decision step 2 needs

Step 2 gives a child write tools. Reading the code for this review found the hazard that makes
it a *decision* rather than a patch, and reading opencode found the shape of the answer.

**Firment's journal is per **turn**, not per agent.** `run_turn` creates
`Arc<Mutex<EditJournal>>` for the turn it is about to run (`core/src/agent.rs:986`), and that
one value is what a cancel rolls back and what `ToolContext` carries. A child spawned *during*
that turn is a different agent running a different turn, so it builds its own journal — in its
own temp store, which is discarded when the call ends. **So today a write-capable child's
edits would be un-undoable**, and worse, they would look undoable: the parent's `/undo` would
report success and touch none of them.

**What opencode does**, from `session/revert.ts` and `snapshot/index.ts`: a snapshot is
*tracked* workspace-wide, but the **patches are attached to message parts** (`part.type ===
"patch"`), so every message carries the record of what it changed. Revert takes a
`{sessionID, messageID}` and restores the snapshot for everything after that point. Two
consequences worth copying:

* **Ownership is per session, and it composes.** A child is a session, its patches live in its
  own history, and reverting the *parent* to a point before the call undoes whatever the child
  did to the workspace — because the snapshot is workspace-wide even though the records are
  not. Granularity and attribution fall out of the same design.
* **`assertNotBusy`** — revert refuses while the session is running. Firment has no such guard
  on `/undo`; with children in flight it will need one.

**The decision for Firment: the child shares the parent turn's journal.** Implemented in `04aabaa` (the plumbing below). Not a new journal,
not a second mechanism:

* One transaction is the invariant the review already set (§4.1). A child's writes belong to
  the turn that spawned it, so a failed batch rolls back whole and `/undo` after a batch is one
  step.
* The plumbing is small and already has a precedent: `SubagentFactory::run_subagent` takes
  `cancel: Cancellable` — a per-call value the `task` tool reads from its `ToolContext`. A
  `journal: Arc<Mutex<EditJournal>>` parameter is the same shape, read from `ctx.journal`,
  which the tool already holds. The nested agent then needs an override so `run_turn` uses the
  passed journal instead of creating one.
* **Attribution is the follow-up, not the price.** opencode gets "which child changed this"
  for free because patches hang off sessions. Firment's ledger line has no such field; adding
  an optional label to `LedgerChange` is how `/ledger` and the transcript keep naming the
  child that made a change — after the sharing works, not before.

**Two smaller ideas from the same files, recorded rather than adopted:** un-revert (opencode
keeps the pre-revert snapshot, so undo has an undo), and the busy guard above.

---

## 6. The smallest slice worth building, in dependency order

1. **Parallel research** (§2): N read-only children, concurrent, with the existing sink
   bracketing extended to name the child. No new synchronisation, and it is testable without a
   provider (the runner is already injectable).
2. **One write-capable child, sequential** (depth 1): proves the registry/permission/journal
   path end to end before concurrency enters the picture at all. This is the step that would
   find the surprises, because it is the first time a child writes.
3. **Declared scopes** (§3B): a scope on the child's `ToolContext`, enforced where
   `resolve_within` is enforced, and printed by the planner.
4. **Two children, disjoint scopes**, then **overlapping scopes** (refused or serialised).
5. **Batch rollback test**: one child's write fails, every other child's write is rolled back —
   the same shape as `rename_symbol`'s read-only-file test, which is the pattern this project
   already has for proving a transaction.

## 6. Open questions for the maintainer

1. **Which half do you want first — parallel research, or parallel edits?** The recommendation
   is research, and it is worth confirming, because it is most of the value at a fraction of
   the risk.
2. **When two children's scopes overlap, refuse or serialise?** Refusing is honest and keeps
   the plan reviewable; serialising is friendlier and hides the concurrency from the user.
3. **What should `/undo` do after a parallel batch?** One step (the batch) matches the
   transaction rule; per-child steps match a mental model of "each agent did something". The
   first is consistent with everything else in this codebase.
4. **Do children ever ask the user?** Today they cannot (`task.rs:15`). Parallel children that
   can ask produce N simultaneous prompts, which is a UI problem before it is a concurrency one.

---

## Status

*The argument above is left as written; this section records what has moved since.*

| §6 step | Status |
|---|---|
| 1. Parallel research | **Done** — `3241cb5`. And the finding is that it already worked: a turn's tool calls run as one `join_all` wave (`core/src/agent.rs:1860`), so several `task` calls have always run beside each other. What was missing was **telling the model** (the description hinted at it and never said it) and **bounding it** (`subagent_slots`, four, held for the child's life — and shared by the whole tree rather than per level: `efcb28d` caught that the first version was four *per agent*, which multiplies by depth). The test pins both directions: four children overlap, one slot serialises them. |
| 2. One write-capable child, sequential | **Mostly done** (`f2b512b`): a child can write when `[tools] subagents_may_write` is on — default off — and its edits land in the spawning turn. What is left is step 3's scope (a writing child may touch anything `resolve_within` allows today) and step 5's batch-rollback test. Earlier: §5's journal plumbing landed in `04aabaa` — a child now writes inside the caller's transaction. What is left is the actual step: a registry that lets a child write, the permission story for it, and the batch-rollback test. |
| 3. Declared scopes | **Done** — `0db4461`: `ToolContext::write_scope`, one gate (`resolve_write_scope`) for the file-edit tools, a `scope` argument on `task` resolved through the workspace boundary, and `shell` dropped from the write-capable registry because a scope cannot constrain one. The scope narrows writes only — reads stay free — and that boundary is asserted, not just documented. |
| 4. Two children with disjoint scopes | Not started (the mechanism is in place: two `task` calls in one turn with different `scope` values) |
| 5. Batch rollback test | **Not started — the last one.** One child's write fails, every other child's write in the batch rolls back, the shape `rename_symbol` already proves for a single tool. |

---

*Written 2026-09-20 against `7b5d08b`; status updated after `3241cb5`. The references in §1 are
the ones that decide the shape of the item; if `assembly.rs` or `subagent.rs` move, this
review's premise changes.*
