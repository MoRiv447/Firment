"""Reference test suite for the task manager, incl. priority support.
Graders run: python -m pytest test_todo.py -q
"""
import json
from datetime import date, timedelta

from todo import TaskManager, parse_due


def test_parse_tomorrow():
    assert parse_due("tomorrow") == date.today() + timedelta(days=1)


def test_complete_is_one_based():
    tm = TaskManager()
    tm.add("first")
    tm.add("second")
    tm.complete(1)
    assert tm.tasks[0].done is True
    assert tm.tasks[1].done is False


def test_complete_out_of_range():
    tm = TaskManager()
    tm.add("only")
    try:
        tm.complete(2)
    except IndexError:
        pass
    else:
        raise AssertionError("expected IndexError")


def test_overdue_excludes_today():
    tm = TaskManager()
    tm.add("due today", due=date.today())
    tm.add("due yesterday", due=date.today() - timedelta(days=1))
    overdue = tm.overdue()
    assert len(overdue) == 1
    assert overdue[0].title == "due yesterday"


def test_save_load_roundtrip_unicode(tmp_path):
    tm = TaskManager()
    tm.add("写周报", due="2026-08-07")
    tm.add("已搞定的事")
    tm.complete(2)
    p = tmp_path / "tasks.json"
    tm.save(str(p))
    raw = json.loads(p.read_text(encoding="utf-8"))
    assert raw[0]["title"] == "写周报"
    tm2 = TaskManager()
    tm2.load(str(p))
    assert tm2.tasks[0].title == "写周报"
    assert tm2.tasks[0].due == date(2026, 8, 7)
    assert tm2.tasks[1].done is True  # completion state must survive round-trip


def test_add_default_priority_is_medium():
    tm = TaskManager()
    t = tm.add("plain")
    assert t.priority == "medium"


def test_add_rejects_invalid_priority():
    tm = TaskManager()
    for bad in ("urgent", "High", "", None):
        try:
            tm.add("x", priority=bad)
        except ValueError:
            pass
        else:
            raise AssertionError(f"expected ValueError for priority={bad!r}")


def test_list_tasks_orders_by_priority_then_insertion():
    tm = TaskManager()
    tm.add("low1", priority="low")
    tm.add("high1", priority="high")
    tm.add("med1", priority="medium")
    tm.add("low2", priority="low")
    tm.add("high2", priority="high")
    assert [t.title for t in tm.list_tasks()] == ["high1", "high2", "med1", "low1", "low2"]


def test_list_tasks_include_done_sorts_all_by_priority():
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("high", priority="high")
    tm.complete(1)  # completes the low task (1-based)
    assert [t.title for t in tm.list_tasks()] == ["high"]
    assert [t.title for t in tm.list_tasks(include_done=True)] == ["high", "low"]


def test_next_returns_top_pending_task():
    tm = TaskManager()
    tm.add("med", priority="medium")
    tm.add("low", priority="low")
    tm.add("high", priority="high")
    assert tm.next().title == "high"
    tm.complete(3)  # complete the high task (1-based)
    assert tm.next().title == "med"
    tm.complete(1)
    tm.complete(2)
    assert tm.next() is None


def test_next_on_empty_manager_returns_none():
    assert TaskManager().next() is None


def test_next_tie_returns_first_added():
    tm = TaskManager()
    tm.add("first")
    tm.add("second")
    assert tm.next().title == "first"


def test_priority_survives_save_load(tmp_path):
    tm = TaskManager()
    tm.add("urgent", priority="high")
    tm.add("someday", priority="low")
    p = tmp_path / "tasks.json"
    tm.save(str(p))
    raw = json.loads(p.read_text(encoding="utf-8"))
    assert raw[0]["priority"] == "high"
    tm2 = TaskManager()
    tm2.load(str(p))
    assert [t.priority for t in tm2.tasks] == ["high", "low"]


def test_load_defaults_missing_priority_to_medium(tmp_path):
    tm = TaskManager()
    tm.add("old")
    p = tmp_path / "tasks.json"
    tm.save(str(p))
    data = json.loads(p.read_text(encoding="utf-8"))
    for d in data:
        d.pop("priority")
    p.write_text(json.dumps(data), encoding="utf-8")
    tm2 = TaskManager()
    tm2.load(str(p))
    assert tm2.tasks[0].priority == "medium"
