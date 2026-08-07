# TaskManager

A minimal, dependency-free Python task manager library. Track tasks with due
dates, completion state, and JSON persistence in a few lines of code.

## Features

- 1-indexed tasks from the user's perspective (`complete(1)` marks the first task)
- Due dates accept `date` objects, ISO strings (`"2026-08-07"`), or `"today"` / `"tomorrow"`
- Overdue detection (due strictly before today, excluding tasks due today)
- UTF-8 JSON save/load round-trip preserving every field, including completion state
- No third-party dependencies; stdlib only

## Install

```bash
pip install -e .
```

or run directly from the checkout (the module has no dependencies):

```bash
python -c "from todo import TaskManager"
```

Requires Python 3.9+.

## Usage

```python
from datetime import date
from todo import TaskManager

tm = TaskManager()
tm.add("写周报", due="2026-08-07")
tm.add("water the plants", due="tomorrow")
tm.add("已搞定的事")

tm.complete(1)                 # marks the FIRST task done
tm.list_tasks()                # pending tasks only
tm.overdue()                   # due before today and not done

tm.save("tasks.json")          # persist to disk (UTF-8)
tm2 = TaskManager()
tm2.load("tasks.json")         # restore, including done state
```

## Configuration

Configuration lives in `config.py` and is read from environment variables, so
no secrets are committed. See `.env.example` for the full list.

```bash
export TODO_API_KEY="your-key"
export TODO_DEBUG=true
```

## Development

Run the test suite:

```bash
python -m pytest test_todo.py -q
```

## License

MIT. See [LICENSE](LICENSE).
