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
    create/modify/delete; its `rm -f` shim also silently no-ops there
    (stderr goes to /dev/null), so "rm the lock and retry" cannot work
    in-sandbox. The fix lives in WorkBuddy settings (`sandbox`
    extra-allow-write for `target/**`), not in more retries.
  - Huorong (火绒) real-time shield also holds handles on freshly written
    files. `D:\OldStudy66\Firment` (at least `target/` dirs and `.git/`)
    belongs in its trust zone.
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
