"""JSON persistence helpers for tasks."""

import json
from datetime import date

STORE_PATH = "tasks.json"


def task_to_dict(task):
    return {
        "title": task.title,
        "due": task.due.isoformat() if task.due else None,
        "done": task.done,
    }


def task_from_dict(cls, data):
    due = date.fromisoformat(data["due"]) if data.get("due") else None
    return cls(data["title"], due=due, done=data.get("done", False))


def save_tasks(tasks, path=STORE_PATH):
    data = [task_to_dict(t) for t in tasks]
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False)


def load_tasks(task_cls, path=STORE_PATH):
    try:
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except FileNotFoundError:
        data = []
    return [task_from_dict(task_cls, d) for d in data]
