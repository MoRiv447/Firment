"""Reference test suite for todo.TaskManager.
Graders run: python -m pytest test_todo.py -q
"""
import json
from datetime import date, timedelta

import pytest

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


def test_complete_alias_still_works():
    # complete() is a deprecated alias for mark_done(); existing callers
    # must keep working (FEATURE_BRIEF: don't break the public API).
    tm = TaskManager()
    tm.add("only")
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


# --- FEATURE_BRIEF: priority support ---


def test_add_default_priority_is_medium():
    tm = TaskManager()
    t = tm.add("plain task")
    assert t.priority == "medium"


def test_add_accepts_each_valid_priority():
    tm = TaskManager()
    assert tm.add("h", priority="high").priority == "high"
    assert tm.add("m", priority="medium").priority == "medium"
    assert tm.add("l", priority="low").priority == "low"


def test_add_rejects_invalid_priority():
    tm = TaskManager()
    with pytest.raises(ValueError):
        tm.add("bad", priority="urgent")


def test_list_tasks_sorts_by_priority():
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("high", priority="high")
    tm.add("medium", priority="medium")
    assert [t.title for t in tm.list_tasks()] == ["high", "medium", "low"]


def test_list_tasks_stable_within_same_priority():
    tm = TaskManager()
    tm.add("a", priority="medium")
    tm.add("b", priority="medium")
    assert [t.title for t in tm.list_tasks()] == ["a", "b"]


def test_list_tasks_include_done_sorts_by_priority():
    tm = TaskManager()
    tm.add("low-done", priority="low")
    tm.add("high-done", priority="high")
    tm.add("medium", priority="medium")
    tm.mark_done(1)  # low-done
    tm.mark_done(2)  # high-done
    titles = [t.title for t in tm.list_tasks(include_done=True)]
    assert titles == ["high-done", "medium", "low-done"]


def test_priority_roundtrips_through_json(tmp_path):
    tm = TaskManager()
    tm.add("urgent", priority="high")
    tm.add("normal", priority="medium")
    tm.add("someday", priority="low")
    p = tmp_path / "tasks.json"
    tm.save(str(p))
    tm2 = TaskManager()
    tm2.load(str(p))
    assert [t.priority for t in tm2.tasks] == ["high", "medium", "low"]


def test_next_returns_highest_priority_pending():
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("high", priority="high")
    assert tm.next().title == "high"


def test_next_returns_none_when_nothing_pending():
    tm = TaskManager()
    assert tm.next() is None
    tm.add("done", priority="high")
    tm.mark_done(1)
    assert tm.next() is None


def test_next_skips_done_tasks():
    tm = TaskManager()
    tm.add("done high", priority="high")
    tm.add("pending low", priority="low")
    tm.mark_done(1)
    assert tm.next().title == "pending low"
