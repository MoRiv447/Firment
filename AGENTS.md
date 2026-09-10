# AGENTS.md — rules for AI coding agents working in this repo

Scope: every agent surface (WorkBuddy, opencode, ZCode, Firment's own
agent, CI scripts). These rules exist because of real incidents on this
machine — read the "why" so you don't re-create the problem.

## Running cargo

- **Serial only.** Never run two cargo commands at the same time
  (e.g. `cargo clippy` while `cargo test` is still running). Cargo takes
  an exclusive lock on the build directory; a second command just blocks
  printing "Blocking waiting for file lock on build directory". Run
  fmt → clippy → test one after another, waiting for each to exit.
- **Prefer scoped runs while iterating**: `cargo test -p <crate>` or
  `cargo test -p <crate> <test_name>` instead of full-workspace runs.
  Full workspace runs are for the final check before committing.
- **`gui/src-tauri` is a SEPARATE cargo workspace** (own `Cargo.toml`,
  own `target/`, own `Cargo.lock`). Root-level cargo commands never cover
  it. After touching anything under `gui/src-tauri/src/`, run inside that
  directory:
  `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo check`
  (CI runs exactly this via `gui-check`.)

## Lock files: never delete, never force

- NEVER delete `target/**/.cargo-build-lock`, `.cargo-lock`,
  `.cargo-artifact-lock` or `~/.cargo/.package-cache` — not with `rm`,
  not "to clear a stuck lock". Deleting a lock file does not revoke the
  handle of a live holder; cargo then recreates a second lock file and
  two processes can write the build directory concurrently, corrupting
  incremental state.
- If cargo hangs on a lock: first check whether a real holder exists —
  `tasklist //FO CSV | grep -iE "cargo|rustc"`. If one is stuck for an
  unreasonable time, kill THAT PID (`taskkill /PID <pid>`). No live
  process + still failing ⇒ it is an environment problem (see below).
- `Blocking waiting for file lock` messages that resolve on their own are
  NORMAL (serial commands waiting for each other). Do not "fix" them.

## os error 5 (拒绝访问 / Access denied) is an environment failure

- If cargo fails with `error: failed to open ... .cargo-build-lock
  拒绝访问。 (os error 5)` — especially after a multi-minute stall, and
  `cargo fmt` (which never touches lock files) still works — the cause is
  security software intercepting file operations, NOT the code and NOT a
  cargo bug:
  - WorkBuddy's sandbox (`modify_backup` rule) intercepts every
    create/modify/delete and breaks cargo's lock-file open; its `rm -f`
    shim also silently no-ops there (stderr goes to /dev/null), so "rm
    the lock and retry" cannot work in-sandbox.
  - **`bypassPermissions` is NOT a reliable way to lift it.** This was
    recorded here as a verified fix and it does not hold: across 195
    `SandboxRuleSync` lines in `~/.workbuddy/logs/`,
    `permissionMode=bypassPermissions` landed on
    `ruleProfile=default-strict` **117 times** (116 file rules,
    `skippedRuleTypes=<none>` — nothing skipped, so `modify_backup`
    still fires) and on `sandbox-disabled` 78 times. Same switch, two
    opposite outcomes, so "switch to bypass and it works" is a coin
    flip.
  - **What the profiles actually do** (measured):

    | `permissionMode` | `ruleProfile` | sandbox | file rules |
    |---|---|---|---|
    | `fullAccess` | `fullAccess-relaxed` | on, relaxed | **6**, skips `read_only,no_access,network(denyAll+blacklist)` |
    | `bypassPermissions` | `sandbox-disabled` *or* `default-strict` | off *or* **still strict** | — / **116**, skips nothing |
    | `default` | `default-strict` | on, strict | 116 |

  - **Use `fullAccess`, then verify from the log rather than trusting the
    switch.** A working session logs
    `permissionMode=fullAccess configuredSandboxEnabled=true
    effectiveSandboxEnabled=true ruleProfile=fullAccess-relaxed`. If the
    line says `default-strict`, the wall is still up — change it, do not
    retry the build.
  - `permissionMode` is per SESSION (`[Startup] SessionMiddleware.handle:
    updatePermissionMode`), not a `settings.json` key. Editing
    `~/.workbuddy/settings.json` cannot change it. The
    `sandbox.extraAllowWrite` list there does NOT reach the shell sandbox
    either — `extend_rules` never appears in the sandbox logs — so
    editing it does not help.
  - Huorong (火绒) real-time shield also holds handles on freshly written
    files, as a SECOND layer: it is installed at
    `C:\Program Files\Huorong\Sysdiag\bin\HipsDaemon.exe` and runs
    independently of the sandbox. `D:\OldStudy66\Firment` (at least
    `target/` dirs and `.git/`) belongs in its trust zone. Note that
    Defender's Controlled Folder Access is *not* involved
    (`EnableControlledFolderAccess = 0`).
- When you hit this: report it once as an environment problem and stop.
  Do NOT loop retries, do NOT delete lock files, do NOT `cargo clean`
  (it will hit the same wall and wastes the whole build cache).

## Repo conventions agents must keep

- Commit style: conventional commits (`fix:`, `feat:`, `docs:`,
  `chore:`), as used throughout `git log`.
- CI gates a push must satisfy before you declare done:
  `cargo fmt --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  per-crate `cargo test` (see `.github/workflows/ci.yml`),
  and `web` + `gui` type-check/tests/build for frontend changes.
- `web/src/lib/tools/specs.json` is a committed snapshot of the Rust tool
  registry. If you change any tool's `input_schema()` or `description()`
  in `crates/firment-tools`, regenerate/verify the snapshot — CI diffs it
  against `firm tools` output and fails on drift.
- CHANGELOG.md gets an entry per release; the release workflow extracts
  the `## <tag> ...` section verbatim as GitHub release notes, so keep
  headings in the exact `## vX.Y.Z (date) — title` format.
- Counts and numbers quoted in changelogs/commit messages must be
  verified (a past entry said "18 unit tests" when there were 17).
- When borrowing IDEAS from other projects (features, doc structures,
  prompt concepts), implement and word them independently — never
  transplant prose or code from sources with attribution requirements
  (Apache-2.0, GPL, ...) without a license check. A README rewrite once
  lifted sentences from a peer project nearly verbatim; such passages
  must be reworded, not patched with attribution.
