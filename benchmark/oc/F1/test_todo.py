"""Reference test suite. Currently 3 failures, matching the 3 seeded bugs.
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


def test_priority_default_is_medium():
    tm = TaskManager()
    t = tm.add("default")
    assert t.priority == "medium"


def test_priority_invalid_value_raises():
    tm = TaskManager()
    try:
        tm.add("bad", priority="urgent")
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError")


def test_priority_sorting():
    tm = TaskManager()
    tm.add("lowest", priority="low")
    tm.add("first medium", priority="medium")
    tm.add("top", priority="high")
    tm.add("second medium", priority="medium")
    titles = [t.title for t in tm.list_tasks()]
    assert titles == ["top", "first medium", "second medium", "lowest"]


def test_priority_sorting_completed_last():
    tm = TaskManager()
    tm.add("done high", priority="high")
    tm.add("pending low", priority="low")
    tm.complete(1)
    assert [t.title for t in tm.list_tasks()] == ["pending low"]
    assert [t.title for t in tm.list_tasks(include_done=True)] == ["done high", "pending low"]


def test_next_returns_most_urgent_pending():
    tm = TaskManager()
    tm.add("later", priority="low")
    tm.add("now", priority="high")
    assert tm.next().title == "now"
    tm.complete(2)
    assert tm.next().title == "later"


def test_next_none_when_all_done():
    tm = TaskManager()
    tm.add("only")
    tm.complete(1)
    assert tm.next() is None


def test_priority_roundtrip(tmp_path):
    tm = TaskManager()
    tm.add("urgent", priority="high")
    p = tmp_path / "tasks.json"
    tm.save(str(p))
    raw = json.loads(p.read_text(encoding="utf-8"))
    assert raw[0]["priority"] == "high"
    tm2 = TaskManager()
    tm2.load(str(p))
    assert tm2.tasks[0].priority == "high"
