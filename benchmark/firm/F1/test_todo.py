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


# ---- priority feature ----


def test_add_default_priority_is_medium():
    tm = TaskManager()
    assert tm.add("default").priority == "medium"


def test_add_invalid_priority_raises_value_error():
    tm = TaskManager()
    try:
        tm.add("bad", priority="urgent")
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError")


def test_list_tasks_sorted_by_priority():
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("high", priority="high")
    tm.add("medium", priority="medium")
    assert [t.title for t in tm.list_tasks()] == ["high", "medium", "low"]


def test_list_tasks_same_priority_keeps_insertion_order():
    tm = TaskManager()
    tm.add("a", priority="high")
    tm.add("b", priority="high")
    tm.add("c", priority="low")
    tm.add("d", priority="low")
    assert [t.title for t in tm.list_tasks()] == ["a", "b", "c", "d"]


def test_list_tasks_excludes_done_by_default():
    tm = TaskManager()
    tm.add("done high", priority="high")
    tm.add("pending low", priority="low")
    tm.tasks[0].done = True
    assert [t.title for t in tm.list_tasks()] == ["pending low"]


def test_list_tasks_include_done_sorted_by_priority():
    # Done status does not reorder the list; priority does.
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("high done", priority="high")
    tm.add("medium", priority="medium")
    tm.tasks[1].done = True
    assert [t.title for t in tm.list_tasks(include_done=True)] == ["high done", "medium", "low"]


def test_next_returns_first_pending_by_priority():
    tm = TaskManager()
    tm.add("low", priority="low")
    tm.add("high pending", priority="high")
    tm.add("high done", priority="high")
    tm.tasks[2].done = True
    assert tm.next() is tm.tasks[1]


def test_next_none_when_no_pending():
    tm = TaskManager()
    tm.add("only")
    tm.tasks[0].done = True
    assert tm.next() is None
    assert TaskManager().next() is None


def test_priority_save_load_roundtrip(tmp_path):
    tm = TaskManager()
    tm.add("a", priority="high")
    tm.add("b", priority="low")
    p = tmp_path / "tasks.json"
    tm.save(str(p))
    raw = json.loads(p.read_text(encoding="utf-8"))
    assert raw[0]["priority"] == "high"
    assert raw[1]["priority"] == "low"
    tm2 = TaskManager()
    tm2.load(str(p))
    assert tm2.tasks[0].priority == "high"
    assert tm2.tasks[1].priority == "low"


def test_load_missing_priority_defaults_medium(tmp_path):
    p = tmp_path / "tasks.json"
    p.write_text(json.dumps([{"title": "old", "due": None, "is_done": False}]), encoding="utf-8")
    tm = TaskManager()
    tm.load(str(p))
    assert tm.tasks[0].priority == "medium"
