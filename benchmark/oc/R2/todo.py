"""A minimal task manager library.

Intended behavior (source of truth for graders):
- Tasks are 1-indexed from the USER's perspective (task #1 is the first added).
- add(title, due=None): due accepts a date, an ISO string "YYYY-MM-DD",
  or the literal "today"/"tomorrow".
- complete(n): marks the n-th task (1-based) done; raises IndexError if out of range.
- list_tasks(include_done=False): returns pending tasks by default.
- overdue(): tasks whose due date is BEFORE today and not done.
- save/load: JSON round-trip preserving all fields; files must be UTF-8.
"""

from datetime import date, datetime, timedelta

from storage import STORE_PATH, load_tasks, save_tasks, task_from_dict, task_to_dict


class Task:
    def __init__(self, title, due=None, done=False):
        self.title = title
        self.due = due
        self.done = done

    def to_dict(self):
        return task_to_dict(self)

    @classmethod
    def from_dict(cls, d):
        return task_from_dict(cls, d)


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

    def add(self, title, due=None):
        t = Task(title, due=parse_due(due))
        self.tasks.append(t)
        return t

    def complete(self, n):
        idx = n - 1
        if idx < 0 or idx >= len(self.tasks):
            raise IndexError(f"task {n} out of range")
        self.tasks[idx].done = True

    def list_tasks(self, include_done=False):
        if include_done:
            return list(self.tasks)
        return [t for t in self.tasks if not t.done]

    def overdue(self, today=None):
        today = today or date.today()
        return [t for t in self.tasks if t.due is not None and t.due < today and not t.done]

    def save(self, path=STORE_PATH):
        save_tasks(self.tasks, path)

    def load(self, path=STORE_PATH):
        self.tasks = load_tasks(Task, path)
        return self.tasks
