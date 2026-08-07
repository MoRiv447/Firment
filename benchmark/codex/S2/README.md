# Todo Manager

A minimal, dependency-free task manager library for Python.

## Features

- Task IDs are 1-based from the user's perspective (`task #1` is the first added task)
- Due dates accept `datetime.date`, ISO strings (`YYYY-MM-DD`), or the shorthands `"today"` / `"tomorrow"`
- Complete tasks, list pending or all tasks, and find overdue tasks
- JSON persistence with UTF-8 encoding

## Status

> This repository is released with three intentionally seeded bugs in `todo.py`
> (`BUG-1`, `BUG-2`, `BUG-3`). The test suite in `test_todo.py` documents the
> intended behavior and currently has 3 failing tests. Fix the bugs before
> cutting a release, or keep this note in your README.

## Installation

```bash
pip install -e .
```

The library has no runtime dependencies. For development:

```bash
pip install -e ".[dev]"
```

## Usage

```python
from todo import TaskManager

tm = TaskManager()
tm.add("Write release notes", due="2026-08-10")
tm.add("Ship v0.1.0", due="tomorrow")
tm.complete(1)

print(tm.overdue())
print(tm.list_tasks())

tm.save("tasks.json")
tm.load("tasks.json")
```

## Configuration

All configuration is read from environment variables (see `config.py`):

| Variable       | Default    | Purpose                |
|----------------|------------|------------------------|
| `TODO_API_KEY` | *(empty)*  | Optional API key       |
| `TODO_API_BASE`| *(empty)*  | Optional API base URL  |
| `TODO_DEBUG`   | `0`        | Debug output (`1`/`true`) |

## Tests

```bash
pytest -q
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
