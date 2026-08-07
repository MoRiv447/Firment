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
    tm.mark_done(1)
    assert tm.tasks[0].done is True
    assert tm.tasks[1].done is False


def test_complete_out_of_range():
    tm = TaskManager()
    tm.add("only")
    try:
        tm.mark_done(2)
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
    tm.mark_done(2)
    p = tmp_path / "tasks.json"
    tm.save(str(p))
    raw = json.loads(p.read_text(encoding="utf-8"))
    assert raw[0]["title"] == "写周报"
    tm2 = TaskManager()
    tm2.load(str(p))
    assert tm2.tasks[0].title == "写周报"
    assert tm2.tasks[0].due == date(2026, 8, 7)
    assert tm2.tasks[1].done is True  # completion state must survive round-trip


def test_add_default_priority():
    tm = TaskManager()
    t = tm.add("default")
    assert t.priority == "medium"


def test_add_invalid_priority():
    tm = TaskManager()
    try:
        tm.add("bad", priority="urgent")
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError")


def test_list_sorted_by_priority():
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("high", priority="high")
    tm.add("medium", priority="medium")
    titles = [t.title for t in tm.list_tasks()]
    assert titles == ["high", "medium", "low"]


def test_list_same_priority_keeps_add_order():
    tm = TaskManager()
    tm.add("first", priority="high")
    tm.add("second", priority="high")
    titles = [t.title for t in tm.list_tasks()]
    assert titles == ["first", "second"]


def test_priority_roundtrip(tmp_path):
    tm = TaskManager()
    tm.add("urgent", priority="high")
    p = tmp_path / "tasks.json"
    tm.save(str(p))
    tm2 = TaskManager()
    tm2.load(str(p))
    assert tm2.tasks[0].priority == "high"


def test_next_returns_most_actionable():
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("high", priority="high")
    tm.add("medium", priority="medium")
    assert tm.next().title == "high"
    tm.mark_done(2)
    assert tm.next().title == "medium"
    tm.mark_done(3)
    tm.mark_done(1)
    assert tm.next() is None
