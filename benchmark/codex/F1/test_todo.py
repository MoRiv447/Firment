"""Reference test suite.
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


def test_add_default_priority():
    tm = TaskManager()
    task = tm.add("default")
    assert task.priority == "medium"


def test_add_valid_priorities():
    tm = TaskManager()
    assert tm.add("h", priority="high").priority == "high"
    assert tm.add("m", priority="medium").priority == "medium"
    assert tm.add("l", priority="low").priority == "low"


def test_add_invalid_priority_raises_value_error():
    tm = TaskManager()
    try:
        tm.add("bad", priority="urgent")
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError")
    assert tm.tasks == []  # nothing should have been added


def test_list_tasks_orders_by_priority():
    tm = TaskManager()
    tm.add("m1", priority="medium")
    tm.add("h1", priority="high")
    tm.add("l1", priority="low")
    tm.add("h2", priority="high")
    assert [t.title for t in tm.list_tasks()] == ["h1", "h2", "m1", "l1"]


def test_list_tasks_include_done_keeps_priority_order():
    tm = TaskManager()
    tm.add("m1", priority="medium")
    tm.add("h1", priority="high")
    tm.add("l1", priority="low")
    tm.complete(2)  # 1-based: completes h1
    assert [t.title for t in tm.list_tasks(include_done=True)] == ["h1", "m1", "l1"]


def test_next_returns_highest_priority_pending():
    tm = TaskManager()
    tm.add("m", priority="medium")
    tm.add("h", priority="high")
    tm.add("l", priority="low")
    assert tm.next().title == "h"
    tm.complete(2)  # 1-based: completes h
    assert tm.next().title == "m"
    tm.complete(1)
    tm.complete(3)
    assert tm.next() is None


def test_next_empty_manager():
    assert TaskManager().next() is None


def test_save_load_roundtrip_priority(tmp_path):
    tm = TaskManager()
    tm.add("high task", priority="high")
    tm.add("low task", priority="low")
    p = tmp_path / "priority_tasks.json"
    tm.save(str(p))
    raw = json.loads(p.read_text(encoding="utf-8"))
    assert raw[0]["priority"] == "high"
    assert raw[1]["priority"] == "low"
    tm2 = TaskManager()
    tm2.load(str(p))
    assert [t.priority for t in tm2.tasks] == ["high", "low"]


def test_load_legacy_data_defaults_priority(tmp_path):
    p = tmp_path / "legacy.json"
    p.write_text(
        json.dumps(
            [
                {"title": "old", "due": None, "is_done": True},
            ],
            ensure_ascii=False,
        ),
        encoding="utf-8",
    )
    tm = TaskManager()
    tm.load(str(p))
    assert tm.tasks[0].priority == "medium"
    assert tm.tasks[0].done is True
