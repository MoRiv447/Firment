# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.1.0] - 2026-08-06

### Added
- Priority support per `FEATURE_BRIEF.md`:
  - `add(title, due=None, priority="medium")` — `priority` accepts `"high"`,
    `"medium"`, or `"low"`; any other value raises `ValueError`.
  - `list_tasks()` now sorts tasks by priority (high > medium > low),
    preserving insertion order within the same priority. `include_done`
    semantics are unchanged; when `True`, all tasks are returned with the
    same priority ordering.
  - `next()` — returns the highest-priority pending task (the first item of
    the priority-sorted pending list), or `None` when nothing is pending.
  - `priority` participates in the JSON save/load round-trip.
- New tests covering priority defaults, validation, ordering, stability,
  JSON round-trip, `next()`, and the deprecated `complete()` alias.

### Changed
- `complete(n)` renamed to `mark_done(n)`. The behavior is identical
  (1-based indexing, `IndexError` on out-of-range). `complete()` remains as
  a deprecated alias so existing callers keep working.
- `Task.to_dict()` / `Task.from_dict()` now include and preserve `priority`.

### Fixed
- `parse_due("tomorrow")` returned today's date (off-by-one); it now returns
  `date.today() + timedelta(days=1)`.
- `complete(n)` (now `mark_done(n)`) treated `n` as 0-based; it now treats
  `n` as 1-based, so `mark_done(1)` marks the first task done.
- `Task.to_dict()` serialized the completion flag under `"is_done"` while
  `Task.from_dict()` read `"done"`, so completed tasks silently came back as
  pending after a save/load round-trip. Both now agree on `"done"`.

## [1.0.0] - 2026-08-06

### Added
- Initial release: `TaskManager` with `add`, `complete`, `list_tasks`,
  `overdue`, `save`, and `load`; `parse_due` accepts dates, ISO strings,
  and `"today"`/`"tomorrow"`.
