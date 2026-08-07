"""Reference test suite. Covers the documented API plus the priority feature.
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


# --- priority feature ---


def test_add_default_priority_is_medium():
    tm = TaskManager()
    t = tm.add("default")
    assert t.priority == "medium"


def test_add_invalid_priority_raises():
    tm = TaskManager()
    for bad in ("urgent", "High", "", None, 1):
        try:
            tm.add("bad", priority=bad)
        except ValueError:
            pass
        else:
            raise AssertionError(f"expected ValueError for {bad!r}")


def test_list_tasks_sorts_by_priority():
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("medium", priority="medium")
    tm.add("high", priority="high")
    assert [t.title for t in tm.list_tasks()] == ["high", "medium", "low"]


def test_list_tasks_stable_within_same_priority():
    tm = TaskManager()
    tm.add("high1", priority="high")
    tm.add("med1", priority="medium")
    tm.add("high2", priority="high")
    tm.add("med2", priority="medium")
    assert [t.title for t in tm.list_tasks()] == ["high1", "high2", "med1", "med2"]


def test_list_tasks_include_done_sorts_all_not_done_last():
    # Done tasks are NOT pushed after pending ones; everything sorts by priority.
    tm = TaskManager()
    tm.add("high done", priority="high")
    tm.add("low pending", priority="low")
    tm.complete(1)
    assert [t.title for t in tm.list_tasks()] == ["low pending"]
    assert [t.title for t in tm.list_tasks(include_done=True)] == ["high done", "low pending"]


def test_priority_round_trip(tmp_path):
    tm = TaskManager()
    tm.add("high", priority="high")
    tm.add("default")
    tm.add("low", priority="low")
    p = tmp_path / "tasks.json"
    tm.save(str(p))
    tm2 = TaskManager()
    tm2.load(str(p))
    assert [t.priority for t in tm2.tasks] == ["high", "medium", "low"]


def test_priority_round_trip_keeps_ordering(tmp_path):
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("high", priority="high")
    p = tmp_path / "tasks.json"
    tm.save(str(p))
    tm2 = TaskManager()
    tm2.load(str(p))
    # Completion state AND priority survive; task order is still insertion order.
    assert [t.title for t in tm2.tasks] == ["low", "high"]
    assert [t.title for t in tm2.list_tasks()] == ["high", "low"]


def test_next_returns_highest_priority_pending():
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("high", priority="high")
    tm.add("medium", priority="medium")
    assert tm.next().title == "high"


def test_next_respects_insertion_order_within_priority():
    tm = TaskManager()
    tm.add("high2", priority="high")
    tm.add("medium", priority="medium")
    tm.add("high1", priority="high")
    assert tm.next().title == "high2"


def test_next_skips_done_tasks():
    tm = TaskManager()
    tm.add("high done", priority="high")
    tm.add("low pending", priority="low")
    tm.complete(1)
    assert tm.next().title == "low pending"


def test_next_returns_none_when_all_done():
    tm = TaskManager()
    tm.add("only", priority="high")
    tm.complete(1)
    assert tm.next() is None


def test_next_returns_none_when_empty():
    assert TaskManager().next() is None
