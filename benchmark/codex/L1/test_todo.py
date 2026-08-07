"""Test suite for todo.py, including priority and mark_done behavior."""
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


def test_list_tasks_sorts_by_priority_then_insertion_order():
    tm = TaskManager()
    tm.add("low-first", priority="low")
    tm.add("medium", priority="medium")
    tm.add("high-first", priority="high")
    tm.add("low-second", priority="low")
    tm.add("high-second", priority="high")

    titles = [t.title for t in tm.list_tasks()]
    assert titles == [
        "high-first",
        "high-second",
        "medium",
        "low-first",
        "low-second",
    ]


def test_list_tasks_with_done_keeps_include_done_semantics_and_sorting():
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("high", priority="high")
    tm.add("medium", priority="medium")
    tm.mark_done(3)  # marks "medium" done

    assert [t.title for t in tm.list_tasks()] == ["high", "low"]
    assert [t.title for t in tm.list_tasks(include_done=True)] == [
        "high",
        "medium",
        "low",
    ]


@pytest.mark.parametrize("bad_priority", ["urgent", "", None])
def test_add_rejects_invalid_priority(bad_priority):
    tm = TaskManager()
    with pytest.raises(ValueError):
        tm.add("bad", priority=bad_priority)


def test_priority_roundtrip(tmp_path):
    tm = TaskManager()
    tm.add("top", priority="high")
    tm.add("normal")
    tm.add("later", priority="low")
    p = tmp_path / "tasks.json"
    tm.save(str(p))

    tm2 = TaskManager()
    tm2.load(str(p))
    assert [t.priority for t in tm2.tasks] == ["high", "medium", "low"]


def test_next_returns_highest_priority_pending_task():
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("high", priority="high")
    tm.add("another high", priority="high")

    assert tm.next() is tm.tasks[1]
    tm.mark_done(2)
    assert tm.next() is tm.tasks[2]
    tm.mark_done(3)
    assert tm.next() is tm.tasks[0]
    tm.mark_done(1)
    assert tm.next() is None
