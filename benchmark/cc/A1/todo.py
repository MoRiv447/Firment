"""A minimal task manager library.

Intended behavior (source of truth for graders):
- Tasks are 1-indexed from the USER's perspective (task #1 is the first added).
- add(title, due=None): due accepts a date, an ISO string "YYYY-MM-DD",
  or the literal "today"/"tomorrow"; returns the new task's 1-based index.
- complete(n): marks the n-th task (1-based) done; raises IndexError if out of range.
- list_tasks(include_done=False): returns pending tasks by default.
- overdue(): tasks whose due date is BEFORE today and not done.
- save/load: JSON round-trip preserving all fields; files must be UTF-8.
"""

import json
from datetime import date, datetime, timedelta

STORE_PATH = "tasks.json"


class Task:
    def __init__(self, title, due=None, done=False):
        self.title = title
        self.due = due
        self.done = done

    def to_dict(self):
        # BUG-3: writes the completion flag under "is_done", but from_dict
        # reads "done" — completed tasks silently come back as pending after
        # a save/load round-trip.
        return {"title": self.title, "due": self.due.isoformat() if self.due else None, "is_done": self.done}

    @classmethod
    def from_dict(cls, d):
        due = date.fromisoformat(d["due"]) if d.get("due") else None
        return cls(d["title"], due=due, done=d.get("done", False))


def parse_due(value):
    """Parse a due-date spec into a date. Returns None for None."""
    if value is None or isinstance(value, date):
        return value
    if value == "today":
        return date.today()
    if value == "tomorrow":
        # BUG-1: off-by-one, returns today instead of tomorrow
        return date.today()
    return datetime.strptime(value, "%Y-%m-%d").date()


class TaskManager:
    def __init__(self):
        self.tasks = []

    def add(self, title, due=None):
        """Add a task and return its 1-based index in the task list."""
        t = Task(title, due=parse_due(due))
        self.tasks.append(t)
        return len(self.tasks)

    def complete(self, n):
        # BUG-2: treats n as 0-based; users pass 1-based indices.
        # complete(1) should complete the FIRST task but completes the second.
        if n < 0 or n >= len(self.tasks):
            raise IndexError(f"task {n} out of range")
        self.tasks[n].done = True

    def list_tasks(self, include_done=False):
        if include_done:
            return list(self.tasks)
        return [t for t in self.tasks if not t.done]

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
