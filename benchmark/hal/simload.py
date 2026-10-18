"""把 fixtures/*.json 读成可回放的设备脚本。

fixture 是**断言引擎的回归测试**：串口好、时序漂、丢行、乱码、序号跳变
这几种情况必须有确定的判定结果。所以 fixture 用"生成器 + 变异"写法，
既短又能一眼看出坏在哪：

    {
      "generator": {"pattern": "HELLO {i}", "start": 0, "count": 65, "period_ms": 1000},
      "mutations": [
        {"kind": "drop",   "at": 30},              # 丢一行
        {"kind": "delay",  "at": 40, "ms": 4000},  # 中途停发 4 秒
        {"kind": "drift",  "at": 10, "period_ms": 900},
        {"kind": "bump",   "at": 20, "by": 3},     # 序号跳变
        {"kind": "text",   "at": 12, "value": "HELLO \ufffd\ufffd"}
      ],
      "reply": {"STATS": [[50, "RX 1024"]]}
    }
"""

from __future__ import annotations

import json
from pathlib import Path

from hal.serialbus import SimScript


class FixtureError(RuntimeError):
    pass


def load_script(path: Path) -> SimScript:
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    rows = _generate(data.get("generator"))
    for m in data.get("mutations", []) or []:
        rows = _mutate(rows, m, data.get("generator"))
    rows.sort(key=lambda r: r[0])

    for item in data.get("rx", []) or []:
        rows.append((float(item[0]), str(item[1])))
    rows.sort(key=lambda r: r[0])

    reply = {
        str(k): [(float(d) / 1000.0, str(t)) for d, t in v]
        for k, v in (data.get("reply") or {}).items()
    }
    return SimScript(
        rx=[(off / 1000.0, text) for off, text in rows],
        reply=reply,
        echo=bool(data.get("echo", False)),
        echo_delay_ms=float(data.get("echo_delay_ms", 20.0)),
        truncate_chars=int(data.get("truncate_chars", 0)),
    )


def _generate(gen: dict | None) -> list[tuple[float, str]]:
    if not gen:
        return []
    start = int(gen.get("start", 0))
    count = int(gen.get("count", 10))
    period = float(gen.get("period_ms", 1000))
    first = float(gen.get("first_ms", 0))
    pattern = gen.get("pattern", "{i}")
    rows = []
    for k in range(count):
        idx = start + k
        rows.append((first + k * period, pattern.format(i=idx, n=idx, k=k)))
    return rows


def _mutate(rows: list[tuple[float, str]], m: dict, gen: dict | None) -> list[tuple[float, str]]:
    kind = m.get("kind")
    at = int(m.get("at", 0))
    if at < 0 or at >= len(rows):
        return rows
    if kind == "drop":
        return rows[:at] + rows[at + 1:]
    if kind == "dup":
        return rows[:at] + [rows[at]] + rows[at:]
    if kind == "text":
        return rows[:at] + [(rows[at][0], str(m["value"]))] + rows[at + 1:]
    if kind == "bump":
        by = int(m.get("by", 1))
        out = rows[:at]
        for off, text in rows[at:]:
            out.append((off, _renumber(text, by, gen)))
        return out
    if kind == "delay":
        add = float(m.get("ms", 1000))
        return rows[:at] + [(off + add, text) for off, text in rows[at:]]
    if kind == "drift":
        new_period = float(m.get("period_ms", 1000))
        base_off = rows[at][0]
        out = rows[:at]
        for k, (_, text) in enumerate(rows[at:]):
            out.append((base_off + k * new_period, text))
        return out
    raise FixtureError(f"未知 mutation：{kind}")


def _renumber(text: str, by: int, gen: dict | None) -> str:
    """把行里的第一个整数加上 by（用于制造序号跳变）。"""
    import re

    m = re.search(r"(-?\d+)", text)
    if not m:
        return text
    return text[:m.start()] + str(int(m.group(1)) + by) + text[m.end():]
