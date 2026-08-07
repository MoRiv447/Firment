"""A minimal task manager library.

Public API (source of truth for graders):
- STORE_PATH: default JSON store path.
- Task: title, due (date | None), done (bool), priority ("high"|"medium"|"low").
- parse_due(value): accepts a date, an ISO string "YYYY-MM-DD", or the
  literals "today"/"tomorrow"; returns None for None.
- TaskManager:
  - add(title, due=None, priority="medium") -> Task; invalid priority raises
    ValueError.
  - complete(n): marks the n-th task (1-based) done; raises IndexError if out
    of range.
  - list_tasks(include_done=False): pending tasks by default; all tasks when
    include_done=True. Returned tasks are sorted by priority (high > medium >
    low), stable within the same priority (insertion order).
  - next(): the top pending task in list order, or None.
  - overdue(today=None): pending tasks whose due date is strictly BEFORE today.
  - save/load: JSON round-trip preserving all fields; files are UTF-8. load on
    a missing file yields an empty list.
"""

import json
from datetime import date, datetime, timedelta

STORE_PATH = "tasks.json"

PRIORITIES = ("high", "medium", "low")
_PRIORITY_RANK = {p: i for i, p in enumerate(PRIORITIES)}


class Task:
    def __init__(self, title, due=None, done=False, priority="medium"):
        if priority not in _PRIORITY_RANK:
            raise ValueError(f"priority must be one of {PRIORITIES}, got {priority!r}")
        self.title = title
        self.due = due
        self.done = done
        self.priority = priority

    def to_dict(self):
        return {
            "title": self.title,
            "due": self.due.isoformat() if self.due else None,
            "done": self.done,
            "priority": self.priority,
        }

    @classmethod
    def from_dict(cls, d):
        due = date.fromisoformat(d["due"]) if d.get("due") else None
        return cls(d["title"], due=due, done=d.get("done", False), priority=d.get("priority", "medium"))


def parse_due(value):
    """Parse a due-date spec into a date. Returns None for None."""
    if value is None or isinstance(value, date):
        return value
    if value == "today":
        return date.today()
    if value == "tomorrow":
        return date.today() + timedelta(days=1)
    return datetime.strptime(value, "%Y-%m-%d").date()


class TaskManager:
    def __init__(self):
        self.tasks = []

    def add(self, title, due=None, priority="medium"):
        t = Task(title, due=parse_due(due), priority=priority)
        self.tasks.append(t)
        return t

    def complete(self, n):
        if n < 1 or n > len(self.tasks):
            raise IndexError(f"task {n} out of range")
        self.tasks[n - 1].done = True

    def list_tasks(self, include_done=False):
        tasks = list(self.tasks) if include_done else [t for t in self.tasks if not t.done]
        # Stable sort: same priority keeps insertion order.
        tasks.sort(key=lambda t: _PRIORITY_RANK[t.priority])
        return tasks

    def next(self):
        pending = self.list_tasks()
        return pending[0] if pending else None

    def overdue(self, today=None):
        today = today or date.today()
        return [t for t in self.tasks if t.due is not None and t.due < today and not t.done]

    def save(self, path=STORE_PATH):
        data = [t.to_dict() for t in self.tasks]
        with open(path, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False)

    def load(self, path=STORE_PATH):
        try:
            with open(path, "r", encoding="utf-8") as f:
                data = json.load(f)
        except FileNotFoundError:
            data = []
        self.tasks = [Task.from_dict(d) for d in data]
        return self.tasks
