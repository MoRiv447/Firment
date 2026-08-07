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


def test_rename_updates_title():
    tm = TaskManager()
    tm.add("old title")
    tm.rename(1, "new title")
    assert tm.tasks[0].title == "new title"


def test_rename_is_one_based():
    tm = TaskManager()
    tm.add("first")
    tm.add("second")
    tm.rename(2, "renamed")
    assert tm.tasks[0].title == "first"
    assert tm.tasks[1].title == "renamed"


def test_rename_out_of_range_raises():
    tm = TaskManager()
    tm.add("only")
    for n in (0, 2):  # 0 is below range, 2 is above — both must raise
        try:
            tm.rename(n, "x")
        except IndexError:
            pass
        else:
            raise AssertionError(f"expected IndexError for n={n}")


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
