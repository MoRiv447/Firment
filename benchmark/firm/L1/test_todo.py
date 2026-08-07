"""Reference test suite. Graders run: python -m pytest test_todo.py -q
"""
import json
from datetime import date, timedelta

import pytest

from todo import TaskManager, parse_due


def test_parse_tomorrow():
    assert parse_due("tomorrow") == date.today() + timedelta(days=1)


def test_mark_done_is_one_based():
    tm = TaskManager()
    tm.add("first")
    tm.add("second")
    tm.mark_done(1)
    assert tm.tasks[0].done is True
    assert tm.tasks[1].done is False


def test_mark_done_out_of_range():
    tm = TaskManager()
    tm.add("only")
    with pytest.raises(IndexError):
        tm.mark_done(2)
    with pytest.raises(IndexError):
        tm.mark_done(0)


def test_complete_is_deprecated_alias():
    tm = TaskManager()
    tm.add("first")
    tm.add("second")
    tm.complete(1)
    assert tm.tasks[0].done is True


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


def test_add_validates_priority():
    tm = TaskManager()
    with pytest.raises(ValueError):
        tm.add("bad", priority="urgent")
    with pytest.raises(ValueError):
        tm.add("bad", priority=None)
    assert tm.add("ok", priority="high").priority == "high"


def test_list_tasks_sorted_by_priority_stable():
    tm = TaskManager()
    tm.add("low1", priority="low")
    tm.add("med1", priority="medium")
    tm.add("high1", priority="high")
    tm.add("low2", priority="low")
    tm.add("med2", priority="medium")
    titles = [t.title for t in tm.list_tasks()]
    assert titles == ["high1", "med1", "med2", "low1", "low2"]


def test_list_tasks_include_done_sorted_by_priority():
    tm = TaskManager()
    tm.add("done low", priority="low")
    tm.add("pending high", priority="high")
    tm.add("done high", priority="high")
    tm.mark_done(1)
    tm.mark_done(3)
    titles = [t.title for t in tm.list_tasks(include_done=True)]
    assert titles == ["pending high", "done high", "done low"]


def test_next_returns_highest_priority_pending():
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("high", priority="high")
    tm.add("medium", priority="medium")
    assert tm.next().title == "high"
    tm.mark_done(2)
    assert tm.next().title == "medium"
    tm.mark_done(3)
    assert tm.next().title == "low"
    tm.mark_done(1)
    assert tm.next() is None


def test_priority_roundtrip(tmp_path):
    tm = TaskManager()
    tm.add("urgent", priority="high")
    tm.add("later", priority="low")
    p = tmp_path / "tasks.json"
    tm.save(str(p))
    raw = json.loads(p.read_text(encoding="utf-8"))
    assert raw[0]["priority"] == "high"
    tm2 = TaskManager()
    tm2.load(str(p))
    assert [t.priority for t in tm2.tasks] == ["high", "low"]
    # legacy files without a priority field default to medium
    (tmp_path / "legacy.json").write_text(
        json.dumps([{"title": "old", "due": None, "done": False}]), encoding="utf-8"
    )
    tm3 = TaskManager()
    tm3.load(str(tmp_path / "legacy.json"))
    assert tm3.tasks[0].priority == "medium"
