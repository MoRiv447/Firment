"""JSON persistence for the task manager.

All file I/O and dict (de)serialization lives here so todo.py can focus
on behavior. The storage layer only touches task *fields* via attribute
access, so it does not need to import the Task class (avoids a circular
import) and works with any object exposing ``.title`` / ``.due`` / ``.done``.
"""

import json
from datetime import date

STORE_PATH = "tasks.json"


def task_to_dict(task):
    """Serialize a task object into a plain, JSON-ready dict.

    The completion flag is written under the same key it is read back
    from, so a save/load round-trip preserves all fields.
    """
    return {
        "title": task.title,
        "due": task.due.isoformat() if task.due else None,
        "done": task.done,
    }


def task_from_dict(d):
    """Parse a saved dict back into ``(title, due, done)`` task fields."""
    due = date.fromisoformat(d["due"]) if d.get("due") else None
    return d["title"], due, d.get("done", False)


def save_tasks(tasks, path=STORE_PATH):
    """Write a list of task objects to ``path`` as UTF-8 JSON."""
    data = [task_to_dict(t) for t in tasks]
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False)


def load_tasks(path=STORE_PATH):
    """Read the saved task dicts from ``path``; ``[]`` if the file is missing."""
    try:
        with open(path, "r", encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        return []
