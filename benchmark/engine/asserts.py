r"""断言库 —— 全部是纯函数，输入是采集到的行，输出是判定结果。

两个约定，写 spec 时必须知道：

1. **正则按行匹配**，不是整段文本匹配。所以写 ``HELLO (\d+)`` 而不是
   ``(?:^|\\n)HELLO (\\d+)``。
2. **默认只看本步骤窗口内的行**（``scope = "all"`` 才看全场）。理由：
   case06 是 FAIL→PASS→FAIL 的三段戏，若第二段能看见第一段的 PASS，
   断言就变成了永远绿。
"""

from __future__ import annotations

import re
import statistics
from dataclasses import dataclass, field
from typing import Any, Iterable

from hal.serialbus import Line


@dataclass
class AssertResult:
    id: str
    point: str
    type: str
    kind: str = "machine"  # machine | static | ai | skipped
    ok: bool | None = None  # None = 无法判定
    confidence: str = "high"  # high | medium | low
    want: Any = None
    got: Any = None
    evidence: list[str] = field(default_factory=list)
    note: str = ""

    def as_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "point": self.point,
            "type": self.type,
            "kind": self.kind,
            "ok": self.ok,
            "confidence": self.confidence,
            "want": self.want,
            "got": self.got,
            "evidence": self.evidence,
            "note": self.note,
        }


@dataclass
class Ctx:
    """断言执行上下文。"""

    lines: list[Line] = field(default_factory=list)
    sends: list[str] = field(default_factory=list)
    send_times: list[float] = field(default_factory=list)
    log_ref: str = ""
    static: dict[str, Any] = field(default_factory=dict)
    bloodline: dict[str, Any] = field(default_factory=dict)
    # 无硬件时（sim/dry-run）把不会真实测量的断言标成 skipped 而不是 fail
    hardware_simulated: bool = False
    # 本次 run 真实具备的仪器（决定 LA/相机断言是"采集后判定"还是"没接就跳过"）
    has_la: bool = False
    has_camera: bool = False
    sent_frames: int = 0
    # 这个串口在本局**此前**收到过多少行。只给一种情况用：`allow_silent` 的
    # "什么都没发生"断言。空窗口分不清"设备沉默"与"没在听"，但"此前收到过行"
    # 证明链路是活的 —— 两者合起来才是观测到的沉默（case14 的 3 分钟耐力窗）。
    link_alive: bool = False
    link_lines: int = 0
    # 仪器算出来的测量值，键是 "类型:通道"（如 "frequency:PA8"、"blink:"）。
    # 采集与判定分开：断言库保持纯函数，不碰任何硬件。
    measure: dict[str, dict[str, Any]] = field(default_factory=dict)

    # -- 符号引用：断言里可以写 ref = "sent_bytes_excluding_last" 这类 --------
    def symbol(self, name: str) -> float | None:
        total = sum(len(s.encode("utf-8")) for s in self.sends)
        last = len(self.sends[-1].encode("utf-8")) if self.sends else 0
        table = {
            "sent_bytes": float(total),
            "sent_bytes_excluding_last": float(total - last),
            "sent_chars": float(sum(len(s) for s in self.sends)),
            "sent_frames": float(self.sent_frames),
            "n_sends": float(len(self.sends)),
        }
        return table.get(name)


def _ref(ctx: Ctx, line: Line) -> str:
    """证据定位串。

    用"第 N 条"而不是"#L N"：日志文件开头有两行 `#` 表头，写 `#L0` 会让人
    去翻文件第 1 行（那是表头），对不上。时间戳是真正无歧义的定位，
    序号只作辅助（跳过表头后按条数找）。
    """
    where = f"{ctx.log_ref} 第{line.index + 1}条@+{line.t:.3f}s"
    return where if ctx.log_ref else where.lstrip()


def _match_lines(ctx: Ctx, pattern: str) -> list[Line]:
    rx = re.compile(pattern)
    return [ln for ln in ctx.lines if rx.search(ln.text)]


def _skip(a: dict[str, Any], note: str) -> AssertResult:
    return AssertResult(
        id=a["id"], point=a.get("point", a["id"]), type=a["type"], kind="skipped", ok=None, note=note
    )


def _fail(a: dict[str, Any], want: Any, got: Any, note: str = "", evidence: list[str] | None = None) -> AssertResult:
    return AssertResult(
        id=a["id"],
        point=a.get("point", a["id"]),
        type=a["type"],
        ok=False,
        want=want,
        got=got,
        note=note,
        evidence=evidence or [],
    )


def _pass(a: dict[str, Any], want: Any, got: Any, note: str = "", evidence: list[str] | None = None,
          confidence: str = "high") -> AssertResult:
    return AssertResult(
        id=a["id"],
        point=a.get("point", a["id"]),
        type=a["type"],
        ok=True,
        want=want,
        got=got,
        note=note,
        evidence=evidence or [],
        confidence=confidence,
    )


# ---------------------------------------------------------------------------
# 串口类
# ---------------------------------------------------------------------------


def assert_serial_regex(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    hits = _match_lines(ctx, a["pattern"])
    need = int(a.get("min_matches", 1))
    want = {"pattern": a["pattern"], "min_matches": need}
    if len(hits) >= need:
        return _pass(a, want, {"matches": len(hits)}, evidence=[_ref(ctx, hits[0])])
    return _fail(a, want, {"matches": len(hits)})


def assert_serial_absent(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    hits = _match_lines(ctx, a["pattern"])
    want = {"pattern": a["pattern"], "matches": 0}
    if not hits:
        return _pass(a, want, {"matches": 0})
    return _fail(a, want, {"matches": len(hits)}, evidence=[_ref(ctx, hits[0])])


def assert_serial_count(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    hits = _match_lines(ctx, a["pattern"])
    expect = a["expect"]
    want = {"pattern": a["pattern"], "expect": expect}
    if isinstance(expect, list):
        lo, hi = int(expect[0]), int(expect[1])
        ok = lo <= len(hits) <= hi
    else:
        ok = len(hits) == int(expect)
    got = {"matches": len(hits)}
    if ok:
        return _pass(a, want, got)
    return _fail(a, want, got)


def assert_serial_sequence(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    """提取序号并要求逐个递增、无跳号无重复。

    这是 case04（SHORT 1..10）与 case10（BOOT #1..#n）的核心判据。

    不给 ``start`` 时按"从抓到的第一个值起连续"判：采集总是晚于上电
    （串口要花几百毫秒才打得开），头几行收不到属正常，不该因此判失败；
    但**中间不许断**，这才是"无跳号无丢行"的本意。
     """
    rx = re.compile(a["pattern"])
    field = int(a.get("field", 1))
    start = a.get("start")
    count = a.get("count")
    nums: list[int] = []
    evidence: list[str] = []
    for ln in ctx.lines:
        m = rx.search(ln.text)
        if not m:
            continue
        try:
            nums.append(int(m.group(field)))
            evidence.append(_ref(ctx, ln))
        except (IndexError, ValueError):
            continue

    base = int(start) if start is not None else (nums[0] if nums else 0)
    want: dict[str, Any] = {"pattern": a["pattern"], "start": start if start is not None else f"auto(={base})"}
    if count is not None:
        want["count"] = count
        expected = list(range(base, base + int(count)))
        ok = nums == expected
        got: dict[str, Any] = {"nums": nums[:40], "n": len(nums)}
        if not ok:
            # 给出最有用的一句话：缺了哪个 / 多了哪个 / 乱在哪
            if nums != expected[: len(nums)]:
                got["first_mismatch"] = next(
                    (i + base for i, v in enumerate(nums) if v != i + base), None
                )
            elif len(nums) < len(expected):
                got["missing_from"] = expected[len(nums)]
            else:
                got["extra"] = nums[len(expected):]
        return (_pass if ok else _fail)(a, want, got, evidence=evidence[:3])

    expected = list(range(base, base + len(nums)))
    ok = nums == expected
    got = {"nums": nums[:40], "n": len(nums)}
    if not ok:
        got["first_mismatch"] = next((i + base for i, v in enumerate(nums) if v != i + base), None)
    return (_pass if ok else _fail)(a, want, got, evidence=evidence[:3])


def assert_serial_timing(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    """相邻匹配行的时间间隔。用来量 1s 心跳、500ms 心跳。"""
    hits = _match_lines(ctx, a["pattern"])
    samples = a.get("samples")
    if samples:
        hits = hits[: int(samples) + 1]
    tol = float(a.get("tol_pct", 5.0)) / 100.0
    period = float(a["period_ms"]) / 1000.0
    want = {"period_ms": a["period_ms"], "tol_pct": a.get("tol_pct", 5.0), "samples": len(hits) - 1}
    if len(hits) < 2:
        return _fail(a, want, {"samples": len(hits) - 1, "error": "匹配行不足，无法计时"})

    deltas = [round(b.t - a_.t, 6) for a_, b in zip(hits, hits[1:])]
    up, lo = period * (1 + tol), period * (1 - tol)
    bad = [d for d in deltas if not (lo <= d <= up)]
    got = {
        "deltas_ms": [round(d * 1000, 1) for d in deltas],
        "mean_ms": round(statistics.fmean(deltas) * 1000, 1),
        "worst_dev_pct": round(max(abs(d - period) / period for d in deltas) * 100, 2),
        "out_of_tol": len(bad),
    }
    if not bad:
        return _pass(a, want, got, evidence=[_ref(ctx, hits[0])])
    return _fail(a, want, got, evidence=[_ref(ctx, hits[0])])


def assert_serial_max_gap(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    """任意两行之间的最大间隔（判"停发"/"卡死"）。"""
    hits = _match_lines(ctx, a["pattern"]) if a.get("pattern") else ctx.lines
    want = {"max_gap_ms": a["max_gap_ms"], "pattern": a.get("pattern")}
    if len(hits) < 2:
        return _fail(a, want, {"error": "行数不足"})
    gaps = [(b.t - a_.t) for a_, b in zip(hits, hits[1:])]
    worst = max(gaps)
    got = {"max_gap_ms": round(worst * 1000, 1), "n": len(hits)}
    if worst * 1000 <= float(a["max_gap_ms"]):
        return _pass(a, want, got)
    idx = gaps.index(worst)
    return _fail(a, want, got, evidence=[_ref(ctx, hits[idx]), _ref(ctx, hits[idx + 1])])


def assert_serial_latency(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    """从"第 k 次发送"到"匹配到指定行"的延迟。

    用来量化"回显风暴期间主循环还活着吗"（case07 F3）：风暴里发 STATS，
    RX 回得多快。没有发送时刻就没有这段延迟，所以要用 ctx.send_times。
    """
    idx = int(a.get("send", -1))
    if not ctx.send_times:
        return _fail(a, {"max_ms": a["max_ms"]}, {"error": "没有发送时刻记录"})
    t_send = ctx.send_times[idx] if idx >= 0 else ctx.send_times[-1]
    rx = re.compile(a["pattern"])
    for ln in ctx.lines:
        if ln.t >= t_send and rx.search(ln.text):
            lat = (ln.t - t_send) * 1000.0
            want = {"max_ms": a["max_ms"], "pattern": a["pattern"]}
            got = {"latency_ms": round(lat, 1)}
            if lat <= float(a["max_ms"]):
                return _pass(a, want, got, evidence=[_ref(ctx, ln)])
            return _fail(a, want, got, evidence=[_ref(ctx, ln)])
    return _fail(a, {"max_ms": a["max_ms"], "pattern": a["pattern"]}, {"error": "发送后没有匹配行"})


def assert_serial_gap(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    """从"匹配 from 的行"到之后第一条"匹配 to 的行"的间隔。

    case11 的"打印 STUCK 后 ~1s 内自动复位重启"就是这条：IWDG 超时的
    实际值只能从现象两端量。
    """
    from_lines = _match_lines(ctx, a["from_pattern"])
    to_lines = _match_lines(ctx, a["to_pattern"])
    want = {"from": a["from_pattern"], "to": a["to_pattern"],
            "min_ms": a.get("min_ms"), "max_ms": a.get("max_ms")}
    if not from_lines:
        return _fail(a, want, {"error": "没有 from 行"})
    src = from_lines[0]
    after = [ln for ln in to_lines if ln.t > src.t]
    if not after:
        return _fail(a, want, {"error": "from 之后没有 to 行"}, evidence=[_ref(ctx, src)])
    dst = after[0]
    gap = (dst.t - src.t) * 1000.0
    got = {"gap_ms": round(gap, 1)}
    ok = True
    if a.get("min_ms") is not None:
        ok = ok and gap >= float(a["min_ms"])
    if a.get("max_ms") is not None:
        ok = ok and gap <= float(a["max_ms"])
    res = (_pass if ok else _fail)(a, want, got, evidence=[_ref(ctx, src), _ref(ctx, dst)])
    return res


def assert_serial_garbage(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    """乱码检测：U+FFFD（解码失败）与不该出现的控制字符。"""
    U = "\ufffd"
    bad: list[Line] = []
    for ln in ctx.lines:
        if U in ln.text:
            bad.append(ln)
            continue
        for ch in ln.text:
            if ord(ch) < 0x20 and ch != "\t":
                bad.append(ln)
                break
            if 0x7F <= ord(ch) <= 0x9F:
                bad.append(ln)
                break
    want = {"garbage_lines": 0, "lines_seen": len(ctx.lines)}
    if not bad:
        return _pass(a, want, {"garbage_lines": 0}, confidence="medium" if a.get("confidence") else "high")
    return _fail(a, want, {"garbage_lines": len(bad)}, evidence=[_ref(ctx, bad[0])])


def _resolve_ref(value: Any, ctx: Ctx) -> float | None:
    """``ref`` 既可以是数字，也可以是符号名（例如 sent_bytes_excluding_last）。"""
    if isinstance(value, (int, float)):
        return float(value)
    if isinstance(value, str):
        return ctx.symbol(value)
    return None


def assert_serial_parse(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    """从行里抽数值做容差判定，并可判"数值是否稳定"。"""
    rx = re.compile(a["expr"])
    rows: list[list[float]] = []
    evidence: list[str] = []
    for ln in ctx.lines:
        m = rx.search(ln.text)
        if not m:
            continue
        try:
            rows.append([float(g) for g in m.groups()])
            evidence.append(_ref(ctx, ln))
        except ValueError:
            continue

    if not rows:
        return _fail(a, {"expr": a["expr"]}, {"rows": 0})

    want: dict[str, Any] = {"expr": a["expr"], "checks": a.get("checks", [])}
    detail: list[dict[str, Any]] = []
    all_ok = True
    for chk in a.get("checks", []):
        idx = int(chk["field"]) - 1
        vals = [r[idx] for r in rows if len(r) > idx]
        if not vals:
            all_ok = False
            detail.append({"field": chk["field"], "error": "捕获组缺失"})
            continue
        mean = statistics.fmean(vals)
        entry: dict[str, Any] = {"field": chk["field"], "mean": round(mean, 4), "n": len(vals)}
        ok = True
        if "ref" in chk:
            ref = _resolve_ref(chk["ref"], ctx)
            if ref is None:
                entry["ref"] = f"{chk['ref']}（符号无法解析）"
                detail.append(entry)
                all_ok = False
                continue
            entry["ref"] = ref
            if "tol_pct" in chk:
                lim = abs(ref) * float(chk["tol_pct"]) / 100.0
                entry["tol_abs"] = round(lim, 4)
                ok = abs(mean - ref) <= lim
            elif "tol_abs" in chk:
                entry["tol_abs"] = float(chk["tol_abs"])
                ok = abs(mean - ref) <= float(chk["tol_abs"])
        if "lo" in chk:
            lo = _resolve_ref(chk["lo"], ctx)
            ok = ok and lo is not None and mean >= lo
            entry["lo"] = chk["lo"]
        if "hi" in chk:
            hi = _resolve_ref(chk["hi"], ctx)
            ok = ok and hi is not None and mean <= hi
            entry["hi"] = chk["hi"]
        entry["ok"] = ok
        all_ok = all_ok and ok
        detail.append(entry)

    # 稳定性：某字段的极差
    for st in a.get("stability", []) or []:
        idx = int(st["field"]) - 1
        vals = [r[idx] for r in rows if len(r) > idx]
        if len(vals) < 2:
            continue
        rng = max(vals) - min(vals)
        ok = rng <= float(st["max_range_abs"])
        detail.append({"stability_field": st["field"], "range": round(rng, 4),
                       "max_range_abs": st["max_range_abs"], "ok": ok})
        all_ok = all_ok and ok

    if all_ok:
        return _pass(a, want, {"samples": len(rows), "checks": detail}, evidence=evidence[:2])
    return _fail(a, want, {"samples": len(rows), "checks": detail}, evidence=evidence[:2])


def assert_serial_echo(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    """原样回显：把发出去的 payload 和收到的文本比对（case07 的核心判据）。

    ``send`` 是 ctx.sends 里的下标；``mode`` = exact（逐字符相同）或
    ``bytes``（只比字节数，用于"丢了多少字节"的分档）。
    """
    idx = int(a.get("send", 0))
    if idx >= len(ctx.sends):
        return _fail(a, {"send": idx}, {"error": "没有对应的发送记录"})
    payload = ctx.sends[idx]
    received = "\n".join(ln.text for ln in ctx.lines)
    mode = a.get("mode", "exact")

    if mode == "bytes":
        want_bytes = len(payload.encode("utf-8"))
        got_bytes = len(received.encode("utf-8"))
        lost = want_bytes - got_bytes
        ok = lost <= int(a.get("max_lost_bytes", 0))
        return (_pass if ok else _fail)(
            a,
            {"want_bytes": want_bytes, "max_lost_bytes": a.get("max_lost_bytes", 0)},
            {"got_bytes": got_bytes, "lost": lost},
        )

    # exact：收到内容里必须出现完整 payload（行被拆开，故拼回来比对）
    ok = payload in received
    if ok:
        return _pass(a, {"exact": True, "bytes": len(payload.encode("utf-8"))}, {"exact": True})
    # 找最长公共前缀，便于人看"丢在哪"
    n = 0
    for x, y in zip(payload, received):
        if x != y:
            break
        n += 1
    return _fail(
        a,
        {"exact": True},
        {"matched_prefix_chars": n, "payload_chars": len(payload)},
        note="回显与发送内容不一致（第一个不一致字符的位置已在 got 里）",
    )


def assert_serial_silence(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    interesting = [ln for ln in ctx.lines if ln.text.strip()]
    want = {"nonempty_lines": 0}
    if not interesting:
        return _pass(a, want, {"nonempty_lines": 0})
    return _fail(a, want, {"nonempty_lines": len(interesting)}, evidence=[_ref(ctx, interesting[0])])


# ---------------------------------------------------------------------------
# 代码静态规则 / 血统
# ---------------------------------------------------------------------------


def assert_code_static(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    """查静态规则表。

    规则返回三态：True / False / **None（判定不出来）**。None 不许被当成
    False —— 把"静态规则看不懂"说成"没做到"会冤枉人，也会让分数取决于
    正则写得多聪明。None 一律转给 AI。
    """
    rule = a["rule"]
    rec = ctx.static.get("rules", {}).get(rule)
    if rec is None:
        return _skip(a, f"静态规则 {rule} 未执行")
    ok = rec.get("ok")
    if a.get("invert") and ok is not None:
        ok = not ok
    if ok is None:
        return AssertResult(
            id=a["id"], point=a.get("point", a["id"]), type=a["type"], kind="static", ok=None,
            want=rec.get("want"), got=rec.get("got"), evidence=rec.get("evidence", []),
            note=(rec.get("note") or "静态规则无法判定"), confidence="low",
        )
    return AssertResult(
        id=a["id"], point=a.get("point", a["id"]), type=a["type"], kind="static", ok=ok,
        want=rec.get("want"), got=rec.get("got"), evidence=rec.get("evidence", []),
        note=rec.get("note", ""), confidence=rec.get("confidence", "high"),
    )


def assert_bloodline(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    bl = ctx.bloodline or {}
    ok = bool(bl.get("ok"))
    return AssertResult(
        id=a["id"], point=a.get("point", a["id"]), type=a["type"], kind="bloodline", ok=ok,
        want={"core_missing": []}, got={"core_missing": bl.get("core_missing", [])},
        note=bl.get("summary", ""),
    )


# ---------------------------------------------------------------------------
# 分派
# ---------------------------------------------------------------------------

# 每种断言允许的参数（不含 id/type/point/note）。
# 用途是让 spec 校验拒绝拼错的参数名 —— 一个写错的键会静默变成"没有断言"，
# 于是整道题全程绿灯，这比报错危险得多。
COMMON = {"id", "type", "point", "note"}
ASSERT_PARAMS: dict[str, set[str]] = {
    "serial_regex": {"pattern", "min_matches"},
    "serial_absent": {"pattern", "allow_silent"},
    "serial_count": {"pattern", "expect", "allow_silent"},
    "serial_sequence": {"pattern", "field", "start", "count"},
    "serial_timing": {"pattern", "period_ms", "tol_pct", "samples"},
    "serial_max_gap": {"pattern", "max_gap_ms"},
    "serial_latency": {"send", "pattern", "max_ms"},
    "serial_gap": {"from_pattern", "to_pattern", "min_ms", "max_ms"},
    "serial_garbage": {"confidence", "allow_silent"},
    "serial_parse": {"expr", "checks", "stability"},
    "serial_echo": {"send", "mode", "max_lost_bytes"},
    "serial_silence": {"allow_silent"},
    "code_static": {"rule", "invert"},
    "bloodline": set(),
    # 以下留给 P2（LA / 相机）；现在先声明参数，便于把 spec 一次写全
    "frequency": {"channel", "hz", "tol_pct"},
    "duty": {"channel", "pct", "tol_pt"},
    "edges": {"channel", "count", "min_count", "max_count"},
    "pulse_width": {"channel", "us", "tol_pct"},
    "decoded_contains": {"decoder", "channel", "pattern", "options"},
    "brightness": {"roi", "threshold", "min_lit_fraction"},
    "blink": {"roi", "hz", "tol_pct"},
    "motion": {"roi", "min_changed_fraction"},
    "diff": {"roi", "min_changed_fraction"},
}

# ---------------------------------------------------------------------------
# 仪器类（LA / 相机）：只查表，不碰硬件
# ---------------------------------------------------------------------------

# 每个仪器断言类型：测量键 → 判据字段
MEASURE_KEYS = {
    "frequency": "hz",
    "duty": "pct",
    "edges": "count",
    "pulse_width": "us",
    "brightness": "lit_fraction",
    "blink": "hz",
    "motion": "changed_fraction",
    "diff": "changed_fraction",
    "decoded_contains": "hits",
}


def measure_key(a: dict[str, Any]) -> str:
    return f"{a['type']}:{a.get('channel', '') or ''}"


def assert_measured(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    """读仪器测出来的值并比对。

    仪器没接 → 键不存在 → 返回 None（未观测），由报告推给 AI。**不许**因为
    "没测到"就判失败，也不许因为"没测到"就跳过整点。
    """
    key = measure_key(a)
    data = ctx.measure.get(key)
    if data is None:
        why = "本次未采集该测量" + ("" if (ctx.has_la or ctx.has_camera) else "（仪器未接入）")
        return _skip(a, why)

    t = a["type"]
    field_name = MEASURE_KEYS[t]
    if field_name not in data:
        # 测量函数自己会说"为什么交不出这个数"（例如整窗只有一个被窗边界切断的高电平
        # 平台）—— 那句话要跟着进报告，不然读者只看到"测量 pulse_width 里没有 us"。
        why = f"测量 {key} 交不出 {field_name}" + (f"：{data['note']}" if data.get("note") else "")
        return _skip(a, why)

    got_v = data[field_name]
    want: dict[str, Any] = {k: v for k, v in a.items() if k not in COMMON}

    if t == "decoded_contains":
        hits = list(got_v or [])
        need = a["pattern"]
        hit = [h for h in hits if re.search(need, str(h))]
        ok = bool(hit)
        return (_pass if ok else _fail)(a, want, {"hits_matched": len(hit), "hits_total": len(hits)},
                                        evidence=[str(hit[0])] if hit else [])

    try:
        val = float(got_v)
    except (TypeError, ValueError):
        return _skip(a, f"测量值不是数字：{got_v!r}")

    ok = True
    detail: dict[str, Any] = {"measured": round(val, 4), "n": data.get("n")}
    if t in ("frequency", "pulse_width"):
        ref, tol = float(a.get("hz", a.get("us"))), float(a.get("tol_pct", 1.5))
        lim = abs(ref) * tol / 100.0
        ok = abs(val - ref) <= lim
        detail |= {"ref": ref, "tol_pct": tol, "dev_pct": round(abs(val - ref) / ref * 100, 3)}
    elif t == "duty":
        ref, tol = float(a["pct"]), float(a.get("tol_pt", 2.0))
        ok = abs(val - ref) <= tol
        detail |= {"ref": ref, "tol_pt": tol}
    elif t == "edges":
        if a.get("count") is not None:
            ok = int(val) == int(a["count"])
        if a.get("min_count") is not None:
            ok = ok and int(val) >= int(a["min_count"])
        if a.get("max_count") is not None:
            ok = ok and int(val) <= int(a["max_count"])
        detail |= {"min_count": a.get("min_count"), "count": a.get("count"),
                   "max_count": a.get("max_count")}
    elif t == "blink":
        ref, tol = float(a["hz"]), float(a.get("tol_pct", 10.0))
        lim = abs(ref) * tol / 100.0
        ok = abs(val - ref) <= lim
        detail |= {"ref": ref, "tol_pct": tol}
    elif t == "brightness":
        need = float(a.get("min_lit_fraction", 0.2))
        ok = val >= need
        detail |= {"min_lit_fraction": need}
    elif t in ("motion", "diff"):
        need = float(a.get("min_changed_fraction", 0.01))
        ok = val >= need
        detail |= {"min_changed_fraction": need}

    res = (_pass if ok else _fail)(a, want, detail, evidence=data.get("evidence", [])[:2])
    res.confidence = data.get("confidence", "high")
    return res


ASSERT_TYPES: dict[str, Any] = {
    "serial_regex": assert_serial_regex,
    "serial_absent": assert_serial_absent,
    "serial_count": assert_serial_count,
    "serial_sequence": assert_serial_sequence,
    "serial_timing": assert_serial_timing,
    "serial_max_gap": assert_serial_max_gap,
    "serial_latency": assert_serial_latency,
    "serial_gap": assert_serial_gap,
    "serial_garbage": assert_serial_garbage,
    "serial_parse": assert_serial_parse,
    "serial_echo": assert_serial_echo,
    "serial_silence": assert_serial_silence,
    "code_static": assert_code_static,
    "bloodline": assert_bloodline,
}

# 需要硬件（LA / 相机 / 夹具）才能跑的断言类型；这些在 sim/dry-run 下会被标 skipped
HARDWARE_TYPES = {
    "frequency",
    "duty",
    "edges",
    "pulse_width",
    "decoded_contains",
    "brightness",
    "blink",
    "motion",
    "diff",
}


# 吃"串口窗口"的断言类型：它们都从 ctx.lines 里找东西。
SERIAL_WINDOW_TYPES = {t for t in ASSERT_TYPES if t.startswith("serial_")}


def _is_absence_assert(a: dict[str, Any]) -> bool:
    """"判据要的是『什么都没发生』"这一类断言。

    只有它们有资格在空窗口上通过 —— 而且还得有 `allow_silent` + 链路此前收过行。
    其余（找某一行、数某个数 >0）在空窗口上一律没有结论。
    """
    t = a.get("type")
    if t in ("serial_absent", "serial_silence", "serial_garbage"):
        return True
    if t == "serial_count":
        exp = a.get("expect")
        return exp == 0 or exp == [0, 0]
    return False


def evaluate(a: dict[str, Any], ctx: Ctx) -> AssertResult:
    t = a["type"]
    if t in HARDWARE_TYPES:
        if ctx.hardware_simulated:
            return _skip(a, f"{t} 需要真实仪器，本次是模拟/试跑，未采集")
        return assert_measured(a, ctx)
    # **空窗不判过，也不判红。** 窗口里一行都没采到时，"设备确实没输出"和
    # "裁判没在听 / 开采时机错过 / 串口没打开"在数据上完全同形。
    # 原来 `serial_absent` 在这种情况下返回"通过" —— 那是拿判分器的失明兑换一枚绿灯
    # （实测 case11 的 H1b 正是这么假绿的，比假红危险：它看起来像有证据）。
    # 与逻辑分析仪那侧"零活动 ⇒ 未观测"是同一条原则的两个形状。
    #
    # 唯一的例外是**显式声明过的沉默**（`allow_silent = true`）：像 case14 的
    # 3 分钟耐力窗，"窗口里什么都没有"本身就是判据。但光声明不够 —— 还要求
    # 本局此前确实收到过行（链路活着）。否则 case11 那种"窗口被切歪"会借这条
    # 重新变成假绿。两个条件缺一不可：**声明**（spec 作者知道这里该沉默）
    # + **旁证**（这台串口这次真的通过）。
    if t in SERIAL_WINDOW_TYPES and not ctx.lines:
        if a.get("allow_silent") and ctx.link_alive and _is_absence_assert(a):
            return _pass(
                a,
                {"window_lines": 0, "link_lines": ctx.link_lines},
                {"window_lines": 0, "link_lines": ctx.link_lines},
                note=f"窗口内 0 行；本局此前已收到 {ctx.link_lines} 行（链路活着）⇒ 沉默是观测到的结论",
                evidence=[f"上一窗口/本局累计 {ctx.link_lines} 行"],
            )
        return _skip(a, "窗口内一行都没采到 ⇒ 分不清『设备没输出』与『没在听/时机错过』，"
                        "记未观测（不判通过，也不判失败）")
    fn = ASSERT_TYPES.get(t)
    if fn is None:
        return _skip(a, f"断言类型未实现：{t}")
    return fn(a, ctx)


def evaluate_all(specs: Iterable[dict[str, Any]], ctx: Ctx) -> list[AssertResult]:
    return [evaluate(a, ctx) for a in specs]
