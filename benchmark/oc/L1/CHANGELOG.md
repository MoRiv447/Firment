# Changelog

## [Unreleased]

### Fixed
- `parse_due("tomorrow")` returned today (off-by-one). It now returns `date.today() + timedelta(days=1)`.
- `TaskManager.complete(n)` treated `n` as 0-based. Completion is now 1-based from the user's perspective, matching the documented behavior; out-of-range indices raise `IndexError`.
- Save/load round-trip lost completion state: `Task.to_dict` wrote the flag under `is_done` while `Task.from_dict` read `done`. Both now consistently use `done`.

### Added
- Priority support per `FEATURE_BRIEF.md`:
  - `TaskManager.add(title, due=None, priority="medium")` accepts `"high" | "medium" | "low"`; invalid values raise `ValueError`.
  - `TaskManager.list_tasks()` now sorts tasks by priority (`high` > `medium` > `low`), with insertion order preserved for equal priorities. The `include_done` parameter semantics are unchanged.
  - `priority` is included in the JSON round-trip (`to_dict` / `from_dict`).
  - New `TaskManager.next()` returns the most actionable pending task (first per the priority sort), or `None` if none remain.

### Changed
- `TaskManager.complete` renamed to `TaskManager.mark_done`; the test suite was updated accordingly.

### Tests
- Fixed 3 failing tests seeded by the bugs above.
- Added coverage for default/invalid priority, priority ordering, stable order within a priority, priority persistence round-trip, and `next()` behavior.
- `python -m pytest test_todo.py -q` → 11 passed.
