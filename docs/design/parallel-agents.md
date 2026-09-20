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

## 5. The smallest slice worth building, in dependency order

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

*Written 2026-09-20 against `7b5d08b`. The references in §1 are the ones that decide the
shape of the item; if `assembly.rs` or `subagent.rs` move, this review's premise changes.*
