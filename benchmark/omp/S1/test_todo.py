"""Test suite for todo.TaskManager. Run: python -m pytest test_todo.py -q
"""
import json
from datetime import date, timedelta

import pytest

from todo import TaskManager, parse_due


# --- parse_due ---------------------------------------------------------

def test_parse_today():
    assert parse_due("today") == date.today()


def test_parse_tomorrow():
    assert parse_due("tomorrow") == date.today() + timedelta(days=1)


def test_parse_iso_string():
    assert parse_due("2026-08-07") == date(2026, 8, 7)


def test_parse_accepts_date_and_none():
    d = date(2026, 1, 1)
    assert parse_due(d) is d
    assert parse_due(None) is None


# --- add / priority ----------------------------------------------------

def test_add_defaults_to_medium_priority():
    tm = TaskManager()
    t = tm.add("chore")
    assert t.priority == "medium"


def test_add_rejects_invalid_priority():
    tm = TaskManager()
    with pytest.raises(ValueError):
        tm.add("bad", priority="urgent")


# --- complete ----------------------------------------------------------

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
    with pytest.raises(IndexError):
        tm.complete(2)
    with pytest.raises(IndexError):
        tm.complete(0)


# --- list_tasks --------------------------------------------------------

def test_list_tasks_excludes_done_by_default():
    tm = TaskManager()
    tm.add("pending")
    tm.add("finished")
    tm.complete(2)
    assert [t.title for t in tm.list_tasks()] == ["pending"]


def test_list_tasks_sorts_by_priority_stable():
    tm = TaskManager()
    tm.add("low1", priority="low")
    tm.add("high", priority="high")
    tm.add("low2", priority="low")
    tm.add("medium", priority="medium")
    assert [t.title for t in tm.list_tasks()] == ["high", "medium", "low1", "low2"]


def test_list_tasks_include_done_sorted():
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("high", priority="high")
    tm.add("done high", priority="high")
    tm.complete(3)
    assert [t.title for t in tm.list_tasks(include_done=True)] == ["high", "done high", "low"]


# --- overdue / next ----------------------------------------------------

def test_overdue_excludes_today():
    tm = TaskManager()
    tm.add("due today", due=date.today())
    tm.add("due yesterday", due=date.today() - timedelta(days=1))
    overdue = tm.overdue()
    assert len(overdue) == 1
    assert overdue[0].title == "due yesterday"


def test_next_returns_highest_priority_pending():
    tm = TaskManager()
    tm.add("medium", priority="medium")
    tm.add("high", priority="high")
    tm.add("high done", priority="high")
    tm.complete(3)
    n = tm.next()
    assert n is not None
    assert n.title == "high"


def test_next_returns_none_when_nothing_pending():
    tm = TaskManager()
    assert tm.next() is None
    tm.add("only")
    tm.complete(1)
    assert tm.next() is None


# --- persistence -------------------------------------------------------

def test_save_load_roundtrip_unicode(tmp_path):
    tm = TaskManager()
    tm.add("写周报", due="2026-08-07", priority="high")
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
    assert tm2.tasks[0].priority == "high"
    assert tm2.tasks[1].done is True


def test_load_missing_file_yields_empty():
    tm = TaskManager()
    assert tm.load("does-not-exist.json") == []
