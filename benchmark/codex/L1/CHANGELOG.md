# Changelog

All notable changes to this project are documented in this file.

## [Unreleased] - 2026-08-06

### Fixed

- `parse_due("tomorrow")` now returns tomorrow instead of today.
- `TaskManager.mark_done` uses 1-based task numbers, matching the user-facing
  task numbering (`mark_done(1)` completes the first task).
- JSON persistence now writes the completion state under `done`, so completed
  tasks no longer come back as pending after a save/load round-trip.

### Added

- `add(title, due=None, priority="medium")` accepts `"high" | "medium" | "low"`
  and raises `ValueError` for any other priority.
- `list_tasks()` sorts tasks by priority (`high` > `medium` > `low`), keeping
  insertion order within the same priority. `include_done` semantics are
  unchanged.
- `Task.next()` returns the highest-priority pending task, or `None` when no
  pending task remains.
- Priority is preserved through JSON save/load round-trips; existing files
  without a `priority` field load with the `"medium"` default.

### Changed

- `TaskManager.complete(n)` was renamed to `TaskManager.mark_done(n)`. The old
  `complete` name remains as a backward-compatible alias.
- Tests were updated to cover the new priority behavior, `next()`, and the
  renamed method.
