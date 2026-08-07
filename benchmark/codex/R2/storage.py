"""JSON persistence helpers for the todo library.

All dict conversion and file I/O lives here so ``todo.py`` can delegate
without duplicating the on-disk format.
"""

import json
from datetime import date

STORE_PATH = "tasks.json"


def task_to_dict(task):
    """Serialize a Task to a plain JSON-compatible dict."""
    return {
        "title": task.title,
        "due": task.due.isoformat() if task.due else None,
        "done": task.done,
    }


def task_from_dict(data, task_type=None):
    """Build a Task from a serialized dict."""
    if task_type is None:
        # Local import avoids a module-load cycle: todo.py imports storage.py.
        from todo import Task

        task_type = Task

    due = date.fromisoformat(data["due"]) if data.get("due") else None
    # Accept the legacy "is_done" key so old JSON files still load correctly.
    return task_type(
        data["title"],
        due=due,
        done=data.get("done", data.get("is_done", False)),
    )


def save_tasks(tasks, path=STORE_PATH):
    """Write tasks to *path* as UTF-8 JSON."""
    data = [task_to_dict(task) for task in tasks]
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False)


def load_tasks(path=STORE_PATH):
    """Read tasks from *path*, returning [] if the file does not exist."""
    try:
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except FileNotFoundError:
        return []
    return [task_from_dict(d) for d in data]


# Aliases matching the method names used by Task/TaskManager.
save = save_tasks
load = load_tasks
to_dict = task_to_dict
from_dict = task_from_dict
