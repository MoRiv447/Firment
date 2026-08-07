# Changelog

All notable changes to this project are documented in this file.

## [Unreleased] — 2026-08-06

### Fixed
- `parse_due("tomorrow")` returned today instead of tomorrow (off-by-one in
  `todo.py`).
- `complete(n)` treated `n` as 0-based while the public contract is 1-based, so
  `complete(1)` marked the second task done.
- Save/load round-trip lost completion state: `Task.to_dict()` wrote the flag
  as `is_done` but `Task.from_dict()` read `done`, so completed tasks silently
  came back as pending.

### Added
- Task priorities: `add(title, due=None, priority="medium")` accepts
  `"high" | "medium" | "low"`; any other value raises `ValueError`.
- `list_tasks` now orders its results by priority (high > medium > low); equal
  priorities keep insertion order (stable sort). `include_done=True` applies
  the same ordering to all tasks.
- `TaskManager.next()` returns the highest-priority pending task — the first
  entry of `list_tasks()` — or `None` when no pending task exists.
- `priority` participates in the JSON save/load round-trip; legacy data without
  a `priority` field defaults to `"medium"` on load.

### Changed
- `TaskManager.complete(n)` renamed to `TaskManager.mark_done(n)`. Semantics
  unchanged: 1-based index, `IndexError` when out of range.

### Tests
- Updated the reference suite for the `complete` → `mark_done` rename.
- Added coverage for priority sorting (including stability and `include_done`),
  `next()`, priority round-trip, and legacy data without a `priority` field.
