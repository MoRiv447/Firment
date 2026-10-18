"""case 规格文件（TOML）的校验。

**为什么要拒绝未知键**：一个拼错的键（``expect_contains`` 写成
``except_contains``）在宽松解析下会静默变成"没有断言"，于是一道题
全程绿灯通过 —— 那比报错危险得多。所以任何未声明的键一律报错。

**为什么断言必须挂到注册表里的 point**：断言 id 只是标签，真正决定分数的
是它引用的 judge_table 条目。引用了不存在的 point 就说明规格写错了。
"""

from __future__ import annotations

import hashlib
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from engine import points as P


class SpecError(RuntimeError):
    pass


def spec_fingerprint(path: Path | None) -> str:
    """规格文件的 sha256 前 12 位。旧报告里没有 ⇒ 视为"未知"，不是"相同"。"""
    if not path:
        return ""
    try:
        return hashlib.sha256(Path(path).read_bytes()).hexdigest()[:12]
    except OSError:
        return ""


TOP_KEYS = {
    "id", "title", "weight", "timebox_min", "prompt", "notes",
    "caps", "fallback", "steps", "followup",
}
CAPS_KEYS = {"serial", "flash", "reset", "la", "camera", "gpio", "power"}
FALLBACK_KEYS = {"on_missing", "reason"}
STEP_KEYS = {
    "op", "asserts", "when", "note", "save", "scope", "port", "baud", "text", "eol",
    "duration_ms", "kind", "name", "value", "ms", "elf", "driver", "samplerate",
    "channels", "off_ms", "fps", "points", "evidence", "reason", "payload", "repeat",
    "gap_ms", "on_unavailable", "require_caps", "count", "frames", "pre_delay_ms", "roi",
    "question", "choices", "timeout_s", "point", "from_prev", "during", "stimulus_delay_ms",
}
OPS = {
    "prep", "verify_conversion", "flash", "reset", "power_cycle", "gpio_set",
    "gpio_pulse", "serial_open", "serial_send", "serial_burst", "serial_window",
    "la_capture", "frame", "frame_burst", "probe_question", "manual_observe",
    "ai_judge", "skip_annotated", "build", "check",
}
ON_MISSING = {"skip", "fail", "manual"}
SCOPES = {"window", "all"}
WHEN = {"always", "caps", "la_available"}

# 每个 op 必须带的键
OP_REQUIRED: dict[str, set[str]] = {
    "serial_open": {"port"},
    "serial_send": set(),
    "serial_burst": {"frames"},
    "serial_window": {"duration_ms"},
    "la_capture": {"duration_ms"},
    "frame": set(),
    "frame_burst": {"duration_ms"},
    "gpio_set": {"name"},
    "gpio_pulse": {"name"},
    "reset": set(),
    "flash": set(),
    "power_cycle": set(),
    "probe_question": {"text"},
    "manual_observe": {"question"},
    "ai_judge": {"points"},
    "skip_annotated": {"points", "reason"},
    "prep": set(),
    "verify_conversion": set(),
    "build": set(),
    "check": set(),
}


@dataclass
class CaseSpec:
    id: str
    title: str
    weight: float
    timebox_min: int
    prompt: Path | None
    caps: dict[str, Any]
    fallback: dict[str, Any]
    steps: list[dict[str, Any]]
    followup: str | None = None
    notes: str = ""
    path: Path | None = None
    # 规格内容的指纹。**激励脚本一改，旧证据就不可比了**（case08 的窗内激励是最好例子：
    # 拿新脚本去判旧波形，得到的既不是旧结论也不是新结论）。靠人工记版本容易忘，
    # 所以直接指纹化 spec 文件，落进 result.json，让 regrade 能自己发现"这题的脚本变过"。
    fingerprint: str = ""
    warnings: list[str] = field(default_factory=list)

    # -- 能力需求 → cap key -----------------------------------------------
    def required_caps(self) -> list[str]:
        need: list[str] = []
        for role in self.caps.get("serial", []) or []:
            need.append(f"serial:{role}")
        if self.caps.get("flash"):
            need.append("flash")
        reset = self.caps.get("reset")
        if reset and reset != "none":
            need.append(f"reset:{reset}")
        if self.caps.get("la") is True:
            need.append("la")
        if self.caps.get("camera") is True:
            need.append("camera")
        for name in self.caps.get("gpio", []) or []:
            need.append(f"gpio:{name}")
        if self.caps.get("power"):
            need.append("reset:power")
        return need

    def optional_caps(self) -> list[str]:
        out: list[str] = []
        if self.caps.get("la") == "optional":
            out.append("la")
        return out

    def ai_points(self) -> list[str]:
        out: list[str] = []
        for st in self.steps:
            if st["op"] == "ai_judge":
                out.extend(st.get("points", []))
        return out

    def skipped_points(self) -> dict[str, str]:
        out: dict[str, str] = {}
        for st in self.steps:
            if st["op"] == "skip_annotated":
                for p in st.get("points", []):
                    out[p] = st.get("reason", "")
        return out

    def declared_points(self) -> list[str]:
        """规格里声明过的全部 point（断言 + ai_judge + skip）。"""
        seen: list[str] = []
        for st in self.steps:
            for a in st.get("asserts", []) or []:
                pid = a.get("point", a.get("id"))
                if pid and pid not in seen:
                    seen.append(pid)
            for pid in st.get("points", []) or []:
                if pid not in seen:
                    seen.append(pid)
        return seen

    def coverage(self) -> dict[str, list[str]]:
        """哪些评分点没被任何断言/AI/跳过条目声明 —— 报告里要显式列出来。"""
        declared = set(self.declared_points())
        allp = P.point_ids(self.id)
        return {
            "undeclared": sorted(allp - declared, key=_point_sort),
            "unknown": sorted(declared - allp),
        }


def _point_sort(pid: str) -> tuple[int, str]:
    order = {"F": 0, "H": 1}
    return (order.get(pid[0], 2), pid)


def load_spec(path: Path) -> CaseSpec:
    with Path(path).open("rb") as fh:
        raw = tomllib.load(fh)
    return _validate(raw, Path(path))


def _validate(raw: dict[str, Any], path: Path) -> CaseSpec:
    def bad(msg: str) -> SpecError:
        return SpecError(f"{path.name}: {msg}")

    unknown = set(raw) - TOP_KEYS
    if unknown:
        raise bad(f"未知顶层键：{sorted(unknown)}（拼错会被静默忽略，故直接报错）")

    cid = raw.get("id")
    if cid not in P.POINTS:
        raise bad(f"未知 case id：{cid!r}；已知：{sorted(P.POINTS)}")

    meta = P.CASE_META[cid]
    caps = raw.get("caps", {})
    unknown_caps = set(caps) - CAPS_KEYS
    if unknown_caps:
        raise bad(f"caps 里未知键：{sorted(unknown_caps)}")

    fallback = raw.get("fallback", {})
    if set(fallback) - FALLBACK_KEYS:
        raise bad(f"fallback 里未知键：{sorted(set(fallback) - FALLBACK_KEYS)}")
    if fallback.get("on_missing", "skip") not in ON_MISSING:
        raise bad(f"fallback.on_missing 必须是 {sorted(ON_MISSING)} 之一")

    steps = raw.get("steps", [])
    if not steps:
        raise bad("steps 为空：一道没有任何步骤的题只会产出空报告，视为配置错误")
    for i, st in enumerate(steps):
        unknown_step = set(st) - STEP_KEYS
        if unknown_step:
            raise bad(f"steps[{i}] 未知键：{sorted(unknown_step)}")
        op = st.get("op")
        if op not in OPS:
            raise bad(f"steps[{i}] 未知 op：{op!r}；已知：{sorted(OPS)}")
        missing = OP_REQUIRED.get(op, set()) - set(st)
        if missing:
            raise bad(f"steps[{i}] ({op}) 缺必填键：{sorted(missing)}")
        if op == "serial_send" and not (st.get("text") or st.get("payload")):
            raise bad(f"steps[{i}] serial_send 需要 text 或 payload 之一")
        if st.get("during"):
            if op != "la_capture":
                raise bad(f"steps[{i}] during（采集窗内激励）目前只支持 la_capture，当前 op={op!r}")
            for k, fr in enumerate(st["during"]):
                if not isinstance(fr, dict) or not fr.get("text"):
                    raise bad(f"steps[{i}] during[{k}] 需要 text —— 没有内容的激励等于没激励")
        if "scope" in st and st["scope"] not in SCOPES:
            raise bad(f"steps[{i}] scope 必须是 {sorted(SCOPES)}")
        # from_prev 是取窗锚点开关，只对 serial_window 有意义；写错地方不会报错、
        # 只会让那一题**静默地**继续用旧锚点丢行 —— 所以在这里当成配置错误拦下。
        if st.get("from_prev"):
            if op != "serial_window":
                raise bad(f"steps[{i}] from_prev 只对在 serial_window 上有意义（当前 op={op!r}）")
            if st.get("scope", "window") == "all":
                raise bad(f"steps[{i}] from_prev 与 scope=\"all\" 矛盾：整盘采集本来就不丢行")
            prev = steps[i - 1] if i else {}
            if prev.get("op") == "serial_window" and prev.get("from_prev"):
                raise bad(f"steps[{i}] 与上一步都是 from_prev 窗口 —— 两段会重叠，同一行被数两次")
        if "when" in st and st["when"] not in WHEN:
            raise bad(f"steps[{i}] when 必须是 {sorted(WHEN)}")
        for j, a in enumerate(st.get("asserts", []) or []):
            _validate_assert(a, cid, path, i, j)
        for pid in st.get("points", []) or []:
            if pid not in P.point_ids(cid):
                raise bad(f"steps[{i}] 引用了不存在的评分点 {pid!r}；本题点是 {sorted(P.point_ids(cid))}")
        if st.get("point") and st["point"] not in P.point_ids(cid):
            raise bad(f"steps[{i}] point={st['point']!r} 不是本题的评分点")

    spec = CaseSpec(
        id=cid,
        title=raw.get("title", meta["title"]),
        weight=float(raw.get("weight", meta["weight"])),
        timebox_min=int(raw.get("timebox_min", meta["timebox_min"])),
        prompt=Path(raw["prompt"]) if raw.get("prompt") else None,
        caps=caps,
        fallback={"on_missing": "skip", **fallback},
        steps=steps,
        followup=raw.get("followup"),
        notes=raw.get("notes", ""),
        path=path,
        fingerprint=spec_fingerprint(path),
    )

    cov = spec.coverage()
    if cov["undeclared"]:
        spec.warnings.append(
            "以下评分点没有任何断言/AI/跳过条目，将不计分也不计入分母："
            + ", ".join(cov["undeclared"])
        )
    return spec


def resolve_point(cid: str, assert_id: str, explicit: str | None = None) -> str:
    """把断言归到评分点上。

    允许写 ``F1a`` / ``F1b`` 这种细分 id，自动归到 ``F1``——但**不猜**：
    找不到就报错。归位结果会被写回断言本身（``point`` 键），所以下游
    （runner / 断言库 / 报告）看到的是同一份显式归属，不存在各处各猜一遍。
    """
    ids = P.point_ids(cid)
    if explicit:
        if explicit not in ids:
            raise SpecError(f"point={explicit!r} 不在 {cid} 的评分点里（{sorted(ids)}）")
        return explicit
    if assert_id in ids:
        return assert_id
    for k in range(len(assert_id) - 1, 0, -1):
        cand = assert_id[:k]
        if cand in ids:
            return cand
    raise SpecError(
        f"断言 id {assert_id!r} 无法归到任何评分点（{sorted(ids)}）；"
        "要么用评分点本身当 id，要么显式写 point="
    )


def _validate_assert(a: dict[str, Any], cid: str, path: Path, i: int, j: int) -> None:
    from engine.asserts import ASSERT_PARAMS, ASSERT_TYPES, COMMON, HARDWARE_TYPES

    where = f"{path.name}: steps[{i}].asserts[{j}]"
    if "id" not in a:
        raise SpecError(f"{where} 缺 id")
    if "type" not in a:
        raise SpecError(f"{where} 缺 type")
    known = set(ASSERT_TYPES) | HARDWARE_TYPES
    if a["type"] not in known:
        raise SpecError(f"{where} 未知断言类型 {a['type']!r}；已知：{sorted(known)}")
    allowed = COMMON | ASSERT_PARAMS.get(a["type"], set())
    unknown = set(a) - allowed
    if unknown:
        raise SpecError(
            f"{where} 有不允许的参数 {sorted(unknown)}；{a['type']} 允许：{sorted(allowed)}"
        )
    a["point"] = resolve_point(cid, a["id"], a.get("point"))


def discover_prompt(testset: Path, cid: str) -> Path | None:
    """题面按题号在测试集里找：cases/caseNN-*/prompt.md。"""
    for d in sorted((testset / "cases").glob(f"{cid}-*")):
        p = d / "prompt.md"
        if p.exists():
            return p
    return None
