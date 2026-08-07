"""JSON persistence layer for tasks.

Owns the serialization format (to_dict/from_dict) and the file I/O
(save/load).  Kept independent of the Task class so todo.py can
delegate here without a circular import.
"""

import json
from datetime import date


def to_dict(task):
    """Serialize a task-like object into a JSON-friendly dict."""
    return {
        "title": task.title,
        "due": task.due.isoformat() if task.due else None,
        "done": task.done,
    }


def from_dict(d):
    """Deserialize a JSON dict into the Task fields (title, due, done)."""
    due = date.fromisoformat(d["due"]) if d.get("due") else None
    return {"title": d["title"], "due": due, "done": d.get("done", False)}


def save(tasks, path):
    """Write tasks as a UTF-8 JSON array."""
    with open(path, "w", encoding="utf-8") as f:
        json.dump([to_dict(t) for t in tasks], f, ensure_ascii=False)


def load(path, task_factory=None):
    """Read tasks from a UTF-8 JSON array; a missing file yields [].

    With task_factory (a callable accepting title/due/done keyword
    arguments) the returned items are Task instances; otherwise a list
    of plain field dicts.
    """
    try:
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except FileNotFoundError:
        data = []
    if task_factory is None:
        return [from_dict(d) for d in data]
    return [task_factory(**from_dict(d)) for d in data]
