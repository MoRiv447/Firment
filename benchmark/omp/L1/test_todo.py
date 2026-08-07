"""Reference test suite. Covers the 3 originally seeded bugs (all fixed),
the priority feature, and the complete -> mark_done rename.
Graders run: python -m pytest test_todo.py -q
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
        tm.mark_done(0)  # 0 is not a valid 1-based index


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


# --- priority feature ---


def test_add_default_priority():
    tm = TaskManager()
    t = tm.add("plain")
    assert t.priority == "medium"


def test_add_invalid_priority_raises():
    tm = TaskManager()
    with pytest.raises(ValueError):
        tm.add("urgent-ish", priority="urgent")


def test_list_sorts_by_priority():
    tm = TaskManager()
    tm.add("low task", priority="low")
    tm.add("high task", priority="high")
    tm.add("medium task", priority="medium")
    assert [t.title for t in tm.list_tasks()] == ["high task", "medium task", "low task"]


def test_list_stable_within_priority():
    tm = TaskManager()
    tm.add("first medium", priority="medium")
    tm.add("second medium", priority="medium")
    tm.add("first high", priority="high")
    assert [t.title for t in tm.list_tasks()] == [
        "first high",
        "first medium",
        "second medium",
    ]


def test_list_include_done_sorts_all():
    tm = TaskManager()
    tm.add("low done", priority="low")
    tm.add("high pending", priority="high")
    tm.mark_done(1)
    assert [t.title for t in tm.list_tasks(include_done=True)] == [
        "high pending",
        "low done",
    ]


def test_next_returns_highest_priority_pending():
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("high", priority="high")
    tm.add("medium", priority="medium")
    assert tm.next().title == "high"


def test_next_skips_done():
    tm = TaskManager()
    tm.add("done high", priority="high")
    tm.add("pending low", priority="low")
    tm.mark_done(1)
    assert tm.next().title == "pending low"


def test_next_empty_and_all_done():
    assert TaskManager().next() is None
    tm = TaskManager()
    tm.add("only")
    tm.mark_done(1)
    assert tm.next() is None


def test_priority_roundtrip(tmp_path):
    tm = TaskManager()
    tm.add("critical", priority="high")
    tm.add("meh", priority="low")
    p = tmp_path / "tasks.json"
    tm.save(str(p))
    raw = json.loads(p.read_text(encoding="utf-8"))
    assert raw[0]["priority"] == "high"
    tm2 = TaskManager()
    tm2.load(str(p))
    assert [t.priority for t in tm2.tasks] == ["high", "low"]


def test_load_legacy_without_priority(tmp_path):
    p = tmp_path / "tasks.json"
    p.write_text(
        json.dumps([{"title": "old", "due": None, "done": False}]),
        encoding="utf-8",
    )
    tm = TaskManager()
    tm.load(str(p))
    assert tm.tasks[0].priority == "medium"
