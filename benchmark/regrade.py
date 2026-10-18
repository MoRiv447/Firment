"""离线重判：用**当前这把尺子**重放已经落盘的证据，不碰硬件、不花 token。

为什么必须先有它：判分口径每改一次，若只能靠重新烧板子来验证，改动就不可回归
（一次真板跑 = 时间 + 板子状态 + 那份 agent 成果已经被测过了）。有了重放，
"旧报告 vs 重判后" 才是纯软件对照，才量得出"修判分器值多少分"。

诚实边界（这条必须写在产物里，不能只写在注释里）：

* **串口 + 静态规则：可精确复判。** 证据就是当时那盘日志，重放的只是"切片 + 判定逻辑"。
* **逻辑分析仪 / 摄像头：不许复判。** 那些断言吃的是仪器当场采到的波形/帧，
  判分器改动（例如 case08 的开采时机）要的恰恰是**重新激励** —— 重放只能给出
  "离线不可复判"，绝不拿旧波形猜新结论。
* **窗口边界是从归档反推的**：每个 `serial_window` 步的 `save` 日志写的是**当时的整盘采集**
  （见 `runner._op_serial_window` 里 `bus.capture.write_log`），它的行数就是该窗口关闭时
  的采集长度。于是新语义下的窗口 = [上一步的采集长度, 本步关闭时的采集长度)。
  没有 `save` 的窗口步反推不出边界 ⇒ 该步断言记"不可复判"。

跑法：

    python cli.py regrade reports/_pre-fix/20261017-113903-firm-case11
    python cli.py regrade reports/_pre-fix --all
    python cli.py regrade <run-dir> --rule case04.H2
"""

from __future__ import annotations

import json
import re
import shutil
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from engine import asserts as A
from engine import spec as SP
from engine.rev import REV
from engine.runner import Runner
from hal import caps as CAPS
from hal.serialbus import Capture, Line

# `Capture.write_log` 写的是 `[+<9.4f>] <mark><text>`：`]` 后面那个空格是**分隔符**，
# 再后面那一格才是 partial 标记（空格或 ~）。少读一个空格，正文就整体多一个前导空格 ——
# 正则断言因为有 ^\s* 而照过，`serial_echo` 的逐字符比对则全挂（对照跑就是这样抓出来的）。
LOG_LINE = re.compile(r"^\[\+ *([0-9.]+)\] ([~ ])(.*)$")

# 这些 op 的证据不是"串口时间线"，重放不了（详见模块开头的诚实边界）。
NO_REPLAY_OPS = {"la_capture", "frame", "frame_burst", "vision", "manual_observe"}


def parse_capture_log(path: Path) -> Capture:
    """把归档的 `all-<role>.log` 读回一个 Capture（时间戳保持相对采集起点的原值）。"""
    cap = Capture()
    for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
        m = LOG_LINE.match(raw)
        if not m:
            continue
        cap.lines.append(
            Line(index=len(cap.lines), t=float(m.group(1)), text=m.group(3),
                 partial=m.group(2) == "~")
        )
    return cap


class ReplayBus:
    """一条"已经归档好的串口时间线"，只实现 Runner 收尾时要用的那几个方法。"""

    def __init__(self, role: str, cap: Capture) -> None:
        self.label = role
        self.port = "replay"
        self.capture = cap
        self.baud = 115200

    def close(self) -> None:
        pass

    def wait(self, seconds: float) -> None:
        pass

    def flush_evidence(self) -> None:
        pass

    def send_line(self, text: str, eol: str = "\r\n") -> None:
        pass


@dataclass
class ReplayReport:
    """重放过程中的"不确定"必须留痕，不然重判结果看起来和真跑一样可信。"""

    warnings: list[str] = field(default_factory=list)
    unrestorable: list[str] = field(default_factory=list)

    def note_unrestorable(self, what: str) -> None:
        if what not in self.unrestorable:
            self.unrestorable.append(what)


class RegradeRunner(Runner):
    """把归档证据喂回**当前**的断言与静态规则实现。

    继承 Runner 是故意的：聚合、报告、grader_rev 盖章这些逻辑一处只留一份实现，
    重判和真跑必须走同一条尾巴，否则"重判通过"和"真跑通过"就不是同一件事。
    """

    def __init__(self, cfg: Any, spec: SP.CaseSpec, agent: str, run_dir: Path,
                 out_root: Path, workdir: Path | None = None) -> None:
        src = {}
        rp = run_dir / "evidence" / "result.json"
        if rp.exists():
            src = json.loads(rp.read_text(encoding="utf-8"))
        wd = workdir or (Path(src.get("workdir")) if src.get("workdir") else None)
        super().__init__(cfg, spec, agent, sim=True, out_root=out_root, workdir=wd)
        self.run_dir = run_dir
        self.src_evidence = run_dir / "evidence"
        self.original = src
        self.rep = ReplayReport()
        # 重判的产物里必须写明它是重判：报告横幅与 summary 都靠它。
        self.outcome.warnings.append(
            "本轮是**离线重判**（旧证据 × 当前判分器），不是重新采集；仪器类断言不参与复判。")

    # -- 归档里的串口时间线 ------------------------------------------------
    def _load_captures(self) -> dict[str, Capture]:
        caps: dict[str, Capture] = {}
        for p in sorted((self.src_evidence / "serial").glob("all-*.log")):
            caps[p.stem.removeprefix("all-")] = parse_capture_log(p)
        return caps

    def _archived_spec_steps(self) -> list[dict[str, Any]]:
        """从旧 result.json 里挑出属于 spec.steps 的那段日志（用来做行数对账）。

        对齐方式：spec 步骤永远排在 `static_rules` 之后（`Runner.run` 的固定顺序）。
        旧报告没写 `step` 下标，只能这样认；认不出来就放弃对账（只降级警告信息）。
        """
        steps = self.original.get("steps") or []
        idx = max((i for i, s in enumerate(steps) if s.get("op") == "static_rules"), default=-1)
        tail = steps[idx + 1:]
        return tail if len(tail) == len(self.spec.steps) else []

    # -- 主流程 ------------------------------------------------------------
    def run(self) -> Any:
        t0 = time.monotonic()
        self._t0 = t0
        self.caps = CAPS.probe_caps(self.cfg, check_hardware=False, fixture=self.fixture)
        self.outcome.caps = self.caps.to_dict()
        self._log("caps", ok=True, note="离线重判：能力按不碰硬件探测", caps=len(self.caps.caps))

        self._step_prep()
        if not self._step_bloodline():
            self._finalize(t0)
            return self.outcome
        self._step_static()

        captures = self._load_captures()
        if not captures:
            self.outcome.warnings.append("旧报告里没有串口归档（evidence/serial/all-*.log）—— 串口断言无从复判")
        for role, cap in captures.items():
            self.buses[role] = ReplayBus(role, cap)  # type: ignore[assignment]
            self.bus = self.buses[role]

        archived = self._archived_spec_steps()
        cursor: dict[str, int] = {role: 0 for role in captures}
        for idx, st in enumerate(self.spec.steps):
            op = st["op"]
            self._cur_step = idx
            try:
                self._replay_step(idx, st, captures, cursor, archived)
            except Exception as exc:  # noqa: BLE001 - 单步失败不该带走整轮，证据要留下来
                self._log(op, ok=False, note=f"{type(exc).__name__}: {exc}")
                self.outcome.warnings.append(f"steps[{idx}] {op} 重放失败：{exc}")
        self._finalize(t0)
        return self.outcome

    @staticmethod
    def _send_text(st: dict[str, Any]) -> str:
        text = str(st.get("text", ""))
        payload = st.get("payload")
        if payload:
            from engine.runner import _gen_payload

            text = _gen_payload(int(payload.get("chars", 1024)), int(payload.get("seed", 42)),
                                int(payload.get("line_chars", 63)))
        return text

    def _carry_old(self, st: dict[str, Any], reason: str) -> None:
        """这一步的证据离线复判不了 ⇒ **原样搬旧判定**并标 `carried`，不是判它失败。

        为什么不能记 None：记 None 会把该点评判权推给 AI，重判分数就和旧分数不可比了。
        为什么不能沿用旧结论却不标记：那等于让旧尺子的结论冒充新尺子的结论。
        """
        by_id = {a["id"]: a for a in (self.original.get("asserts") or [])}
        for a in st.get("asserts", []) or []:
            o = by_id.get(a["id"])
            if o is None:
                self._record_asserts([A.AssertResult(
                    id=a["id"], point=a.get("point", a["id"]), type=a["type"],
                    kind="skipped", ok=None, note=f"离线不可复判（{reason}），旧报告里也没有这条")])
                continue
            self._record_asserts([A.AssertResult(
                id=o["id"], point=o["point"], type=o["type"], kind=o.get("kind", "machine"),
                ok=o.get("ok"), confidence="low", want=o.get("want"), got=o.get("got"),
                evidence=o.get("evidence") or [],
                note=f"离线不可复判（{reason}），沿用旧判定")], carried=True)
        self.rep.note_unrestorable(reason)

    def _replay_step(self, idx: int, st: dict[str, Any], captures: dict[str, Capture],
                     cursor: dict[str, int], archived: list[dict[str, Any]]) -> None:
        op = st["op"]
        old = archived[idx] if idx < len(archived) else {}

        if op in ("serial_open", "serial_close", "serial_config"):
            role = str(st.get("port") or (next(iter(captures)) if captures else "dut"))
            if op == "serial_open":
                if role not in self.buses and role in captures:
                    self.buses[role] = ReplayBus(role, captures[role])  # type: ignore[assignment]
                if role in self.buses:
                    self.bus = self.buses[role]
                    self.bus_kind = "replay"
                self._log(op, ok=True, port=role, kind="replay")
            else:
                self._log(op, ok=True, port=role)
            return

        if op in ("serial_send", "serial_burst", "probe_question"):
            bus = self.buses.get(str(st.get("port")) or "") or self.bus
            frames = [str(f) for f in (st.get("frames") or [])]
            texts = frames or [self._send_text(st)]
            # 发送时刻只能用"归档里最后一条已到达行的时间戳"近似 —— 旧报告没记每步时刻。
            cap = getattr(bus, "capture", None) if bus is not None else None
            t_send = cap.lines[-1].t if cap and cap.lines else 0.0
            n = int(st.get("repeat", 1)) if op != "serial_burst" else int(st.get("count", len(texts)))
            for i in range(max(1, n)):
                self._sends.append(texts[i % len(texts)])
                self._send_times.append(t_send)
                if op == "serial_burst":
                    self._sent_frames += 1
            self._log(op, ok=True, note="重放：只登记发送内容，时刻为近似值",
                      repeat=n, chars=sum(len(t) for t in texts))
            return

        if op == "serial_window":
            self._replay_window(idx, st, cursor, old)
            return

        if op == "check":
            ctx = self._serial_ctx(st, [], "")
            results = [A.evaluate(a, ctx) for a in st.get("asserts", []) or []]
            self._record_asserts(results)
            self._log("check", ok=all(r.ok is not False for r in results), asserts=len(results),
                      note="重放：静态/血统类断言按当前实现重算")
            return

        if op in ("ai_judge", "skip_annotated"):
            # 这两步不产生机器结论，只是把点交给 AI / 标注跳过 —— 照原样记一遍
            getattr(self, "_op_" + op)(idx, st)
            return

        if op in NO_REPLAY_OPS:
            # `la_capture` 的窗内激励（`during`）在真跑时也是**发出去并被固件处理的帧**，
            # runner 会把它们计入 `sent_frames`（r4）。重放跳过这一步，不补记就会让
            # `case08 F3b` 的参照值少 5 帧：归档里 agent 如实答 105，重判却拿 100 去比。
            # 这里按规格补记（规格指纹变过时 `spec_comparable=false` 会另行标注）。
            if op == "la_capture":
                self._sent_frames += len(st.get("during") or [])
            self._carry_old(st, f"{op}：仪器证据不重放")
            self._log(op, ok=True, skipped=True, note="离线重判：该步证据不复用（要重新激励才成立）")
            return

        # flash / reset / gpio_pulse / wait：真跑时它们只**产生**串口行，
        # 而那些行已经在归档日志里了。这里不重放硬件动作，只把它们对时间线的
        # 影响留给 serial_window 的边界链去处理。
        self._carry_old(st, f"{op}：硬件动作不重做")
        self._log(op, ok=True, skipped=True, note="重放：硬件动作不重做（其串口后果已在归档里）")

    def _replay_window(self, idx: int, st: dict[str, Any], cursor: dict[str, int],
                       old: dict[str, Any]) -> None:
        role = str(st.get("port") or getattr(self.bus, "label", "") or "")
        bus = self.buses.get(role)
        if bus is None:
            self._record_unrun(idx, st, "旧报告里没有这条串口的归档")
            self._log("serial_window", ok=False, note="没有可重放的串口归档", step=idx)
            return
        role = str(getattr(bus, "label", "dut"))
        cap = bus.capture
        prev = cursor.get(role, 0)
        save = st.get("save")
        log_ref = ""
        if not save:
            # 没落盘的窗口反推不出边界（旧报告里只有"这个窗口吃进几行"，没有"当时采集到哪一行"）。
            # 宁可沿用旧判定并说明，也不要猜一个边界去改判 —— 猜出来的红/绿都没法复核。
            self._carry_old(st, f"steps[{idx}] 窗口没有 save 日志，边界反推不出")
            self._log("serial_window", ok=True, skipped=True, step=idx,
                      note="离线重判：窗口边界反推不出，沿用旧判定")
            return
        log_ref = str(Path("evidence") / str(save)).replace("\\", "/")
        closed = len(parse_capture_log(self.src_evidence / str(save)).lines)

        if st.get("scope", "window") == "all":
            # 整盘采集：起点固定 0，不吃锚点（case11 的 stuck 窗口就是这个语义）
            start = 0
        else:
            if st.get("from_prev"):
                start = prev
            else:
                # 没有 from_prev 的窗口保持旧语义：起点 = 关闭时刻往前数 recorded lines。
                n = old.get("lines")
                start = closed - int(n) if n is not None else prev
            # 旧语义的起点不可能早于上一个窗口关闭处（采集只增不减）
            start = max(prev, min(start, closed))
        cursor[role] = closed
        window = cap.since(start)[: max(0, closed - start)]

        if old.get("lines") is not None and len(window) != old["lines"] and not st.get("from_prev"):
            self.rep.warnings.append(
                f"steps[{idx}] 窗口行数对不上：重放 {len(window)} vs 原跑 {old['lines']}")

        ctx = self._serial_ctx(st, window, log_ref)
        results = [A.evaluate(a, ctx) for a in st.get("asserts", []) or []]
        self._record_asserts(results)
        n_bad = sum(1 for r in results if r.ok is False)
        self._log("serial_window", ok=n_bad == 0, duration_ms=st.get("duration_ms"),
                  lines=len(window), asserts=len(results), failed=n_bad, log=log_ref or None,
                  note="重放（起点=%d，关闭于=%d）" % (start, closed))


def diff_verdicts(old_asserts: list[dict[str, Any]], new_asserts: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """按断言 id 对齐，给出 旧判定 → 新判定 的变化表（含只消失或只新增的）。"""
    o = {a["id"]: a for a in old_asserts}
    n = {a["id"]: a for a in new_asserts}
    rows = []
    for aid in list(o) + [k for k in n if k not in o]:
        ov, nv = (o.get(aid) or {}).get("ok"), (n.get(aid) or {}).get("ok")
        rows.append({"id": aid, "point": (n.get(aid) or o.get(aid) or {}).get("point"),
                     "type": (n.get(aid) or o.get(aid) or {}).get("type"),
                     "old": ov, "new": nv, "changed": ov is not nv,
                     "carried": bool((n.get(aid) or {}).get("carried")),
                     "old_got": (o.get(aid) or {}).get("got"), "new_got": (n.get(aid) or {}).get("got"),
                     "note": (n.get(aid) or {}).get("note", "")})
    return rows


def run_one(cfg: Any, run_dir: Path, only_rule: str | None = None,
            spec: SP.CaseSpec | None = None) -> dict[str, Any]:
    """重判一个 run 目录，产物写在 `<run>/regrade/`，返回 diff。

    `spec` 只给自测用（临时改过的规格），正常路径永远按 case 名去 specs/ 取。
    """
    from engine.static_rules import run_rules

    if only_rule:
        case, _, rid = only_rule.partition(".")
        res_path = run_dir / "evidence" / "result.json"
        src = json.loads(res_path.read_text(encoding="utf-8")) if res_path.exists() else {}
        wd = Path(src.get("workdir")) if src.get("workdir") else \
            Path(cfg.testset) / "runs" / run_dir.name.split("-")[-2] / case
        return {"rule_only": True, "workspace": str(wd),
                "rules": run_rules(case, wd, only=[only_rule])["rules"]}

    agent = run_dir.name.split("-")[-2]
    case = run_dir.name.split("-")[-1]
    spec = spec or SP.load_spec(cfg.specs / f"{case}.toml")
    out_root = run_dir / "regrade"
    # 重判产物全部可再生成，且只由这个函数写 —— 每次清掉旧的，免得三次对照跑
    # 留下三份时间戳副本，之后谁也说不清哪份对应哪把尺子。
    if out_root.exists():
        shutil.rmtree(out_root)
    r = RegradeRunner(cfg, spec, agent, run_dir, out_root)
    out = r.run()
    new_asserts = out.asserts
    old_asserts = (r.original.get("asserts") or [])
    rows = diff_verdicts(old_asserts, new_asserts)
    # **规格变过 ⇒ 这一栏和旧结论不可比**（C7）。case08 的窗内激励就是典型：
    # 拿"新激励脚本"的判据去读"旧采集"的波形，得到的既不是旧结论也不是新结论。
    # 旧报告里没有 spec_fp（那批还没这字段）⇒ 一律按"未知"处理，宁可说不清，不许装作可比。
    old_fp = ((r.original.get("summary") or {}).get("spec_fp") or "") or None
    comparable = old_fp is not None and old_fp == spec.fingerprint
    payload = {
        "mode": "replay",
        "grader_rev": REV,
        "spec_fp_old": old_fp,
        "spec_fp_new": spec.fingerprint,
        "spec_comparable": comparable,
        "source_run": str(run_dir),
        "regrade_dir": str(r.out),
        "case": case,
        "agent": agent,
        "old_auto_score": (r.original.get("summary") or {}).get("auto_score"),
        "new_auto_score": out.summary()["auto_score"],
        "warnings": out.warnings + r.rep.warnings,
        "unrestorable_ops": r.rep.unrestorable,
        "diff": rows,
    }
    (out_root).mkdir(parents=True, exist_ok=True)
    (out_root / "diff.json").write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
    (out_root / "diff.md").write_text(render_diff_md(payload), encoding="utf-8")
    return payload


def render_diff_md(p: dict[str, Any]) -> str:
    v = {True: "✓", False: "✗", None: "—"}
    n_carried = sum(1 for d in p["diff"] if d.get("carried"))
    lines = [f"# 重判 {p['case']} {p['agent']}（尺子 `{p['grader_rev']}`）", "",
             f"- 原判定：`{p['source_run']}`（旧尺子）",
             f"- 重判产物：`{p['regrade_dir']}`",
             f"- 机器自动分：{p['old_auto_score']} → **{p['new_auto_score']}**",
             "- **这是旧证据 × 新尺子**，不是重新采集。",
             ("- 规格：**可比**（激励脚本没变）" if p.get("spec_comparable") else
              f"- 规格：**不可比** —— 旧 run 用的是 `{p.get('spec_fp_old') or '无记录'}`，"
              f"现在这份是 `{p.get('spec_fp_new')}`；步骤/激励变过，下表里由步骤改动带来的"
              "变化**不能当作重新采集的结论**，只能当作『新尺子会怎么读旧证据』的参考"),
             f"- 表里带 `(旧)` 的行是**离线不可复判**（仪器类 / 边界反推不出的窗口），"
             f"照搬旧判定只为让分数可比 —— 共 {n_carried} 条，它们**不代表新尺子认过**。", ""]
    if p.get("warnings"):
        lines += ["## 重放警告", ""] + [f"- {w}" for w in p["warnings"]] + [""]
    changed = [d for d in p["diff"] if d["changed"]]
    lines += [f"## 判定变化（{len(changed)} / {len(p['diff'])} 条）", "",
              "| 断言 | 点 | 类型 | 旧 | 新 | 说明 |", "|---|---|---|---|---|---|"]
    for d in p["diff"]:
        mark = "**→**" if d["changed"] else ""
        new = f"{v[d['new']]} (旧)" if d.get("carried") else v[d["new"]]
        lines.append(f"| {d['id']} | {d['point']} | {d['type']} | {v[d['old']]} {mark} | "
                     f"{new} | {d['note'][:70]} |")
    return "\n".join(lines) + "\n"
