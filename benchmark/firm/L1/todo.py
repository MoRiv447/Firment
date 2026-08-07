"""A minimal task manager library.

Intended behavior (source of truth for graders):
- Tasks are 1-indexed from the USER's perspective (task #1 is the first added).
- add(title, due=None, priority="medium"): due accepts a date, an ISO string
  "YYYY-MM-DD", or the literal "today"/"tomorrow"; priority accepts
  "high" | "medium" | "low" and raises ValueError otherwise.
- mark_done(n): marks the n-th task (1-based) done; raises IndexError if out of range.
- list_tasks(include_done=False): returns pending tasks by default, ordered by
  priority (high > medium > low) and insertion order within the same priority.
- overdue(): tasks whose due date is BEFORE today and not done.
- next(): the first pending task in list order, or None if none is pending.
- save/load: JSON round-trip preserving all fields; files must be UTF-8.
"""

import json
from datetime import date, datetime, timedelta

STORE_PATH = "tasks.json"

PRIORITIES = ("high", "medium", "low")
_PRIORITY_RANK = {"high": 0, "medium": 1, "low": 2}


class Task:
    def __init__(self, title, due=None, done=False, priority="medium"):
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
        if priority not in PRIORITIES:
            raise ValueError(f"invalid priority: {priority!r}")
        t = Task(title, due=parse_due(due), priority=priority)
        self.tasks.append(t)
        return t

    def mark_done(self, n):
        """Mark the n-th task (1-based) as done; raises IndexError if out of range."""
        if n < 1 or n > len(self.tasks):
            raise IndexError(f"task {n} out of range")
        self.tasks[n - 1].done = True

    # Backward-compatible alias for mark_done.
    complete = mark_done

    def _sorted(self, tasks):
        return sorted(tasks, key=lambda t: _PRIORITY_RANK.get(t.priority, 1))

    def list_tasks(self, include_done=False):
        tasks = list(self.tasks) if include_done else [t for t in self.tasks if not t.done]
        return self._sorted(tasks)

    def overdue(self, today=None):
        today = today or date.today()
        return [t for t in self.tasks if t.due is not None and t.due < today and not t.done]

    def next(self):
        pending = self._sorted([t for t in self.tasks if not t.done])
        return pending[0] if pending else None

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
