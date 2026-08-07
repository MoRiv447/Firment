"""JSON persistence for the task manager.

The file I/O and dict-conversion logic formerly inlined in todo.py lives
here.  It works on duck-typed task objects (attributes: title, due, done)
so it never imports todo.py — todo.py delegates to this module and keeps
its public interface unchanged.
"""

import json
from datetime import date


def to_dict(task):
    """Serialize one task object into a JSON-able dict.

    Keys: "title" (str), "due" (ISO date string or None), "done" (bool).
    """
    return {
        "title": task.title,
        "due": task.due.isoformat() if task.due else None,
        "done": task.done,
    }


def from_dict(d):
    """Deserialize one task dict into a dict of (title, due, done).

    "due" is parsed back into a datetime.date (or None).
    """
    raw_due = d.get("due")
    due = date.fromisoformat(raw_due) if raw_due else None
    return {"title": d["title"], "due": due, "done": d.get("done", False)}


def save(tasks, path):
    """Write a list of task objects to `path` as UTF-8 JSON."""
    with open(path, "w", encoding="utf-8") as f:
        json.dump([to_dict(t) for t in tasks], f, ensure_ascii=False)


def load(path):
    """Read a list of task dicts from `path`; returns [] if the file is missing."""
    try:
        with open(path, "r", encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        return []
