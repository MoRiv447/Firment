"""按 spec 执行一轮评测，产出证据与逐点结论。

三条流程约束，每条都有代价在背后：

1. **血统闸门先跑，违规就不碰硬件。** 判定违规的题本来就 0 分，再去烧录
   只是白烧一次 flash（case10 那类写 flash 的题还可能把板子写砖）。
2. **裁判只在 agent 结束后接管串口。** 所以本 runner 不是"和 agent 并行"
   的东西，它是 agent 声明 DONE / 时间盒到之后才启动的。
3. **证据落盘到报告目录，不落进被测树。** ``runs/<agent>/<case>/`` 是 agent
   的工作目录，把含阈值的 asserts.json 放进去等于泄题（哪怕它已经跑完，
   下一轮的 agent 也读得到）。
"""

from __future__ import annotations

import json
import random
import shutil
import string
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from engine import asserts as A
from engine import bloodline as BL
from engine import points as P
from engine import spec as SP
from engine.rev import REV
from engine.static_rules import rules_of, run_rules
from hal import caps as CAPS
from hal import ports
from hal.gpio import Fixture, FixtureUnavailable
from hal.probe import Probe
from hal.serialbus import SerialBus, SimBus, SimScript

# 固件产物的**优先级顺序**（先命中先用，不做时间戳比较）。
#
# 踩过的坑（2026-10-15 真板首跑）：原来把 `cubemx/Debug/*.elf` 也列进候选，
# 再用 `max(mtime)` 挑 —— 结果选中了**母本工程自己的 CubeIDE Debug 构建**，
# 烧进去的是空循环固件。现象是"串口一个字都没有"，而报告上看起来像
# "agent 没输出"，很容易被判成 agent 的错。
# 所以：PlatformIO 在工程根构建的 firmware.elf 才算 agent 的产物，优先；
# 母本那棵子树（cubemx/、start/、reference-pio/）一律排除。
ELF_GLOBS = (
    ".pio/build/*/firmware.elf",
    "platformio/.pio/build/*/firmware.elf",
    ".pio/build/*/*.elf",
    "build/*/firmware.elf",
    "*.elf",
)
ELF_EXCLUDE_PARTS = frozenset({"cubemx", "start", "reference-pio"})


def find_firmware(workdir: Path) -> Path | None:
    """按优先级找出 agent 构建出的固件 ELF（排除母本子树）。"""
    workdir = Path(workdir)
    for pattern in ELF_GLOBS:
        for cand in sorted(workdir.glob(pattern)):
            if not cand.is_file():
                continue
            try:
                parts = set(cand.relative_to(workdir).parts)
            except ValueError:
                continue
            if ELF_EXCLUDE_PARTS & parts:
                continue
            return cand
    return None


def assert_needs_activity(a: dict[str, Any]) -> bool:
    """这条仪器断言是不是**必须有跳变**才可能为真。

    关键区分在 `edges`：`min_count=4`（"报警在响"）零活动 = 测不出区别 ⇒ 记未观测；
    `max_count=2`（"报警已停"）零活动恰恰是它要看到的现象 ⇒ 照判不误。
    不加这层区分，防线会把 case14 alarm2 那种**合法绿灯**一起灭掉。
    """
    t = a.get("type")
    if t in ("frequency", "pulse_width", "decoded_contains"):
        return True
    if t == "edges":
        need = a.get("min_count", a.get("count"))
        if need is None:          # 只有 max_count 的断言（"报警已停"）：零活动就是它要看的
            return False
        try:
            return int(need) >= 2
        except (TypeError, ValueError):
            return False
    return False


def window_start(bus: Any, st: dict[str, Any], anchors: dict[str, int]) -> int:
    """串口窗口的**唯一起点算法**（runner、自测、离线重判都走这里）。

    默认起点 = 现在已有几行（旧语义）。带 `from_prev = true` 的步骤，起点回退到
    **上一个步骤开始执行前**已有的行数 —— 因为要找的那一行恰恰是上一步的硬件动作
    （按 RESET、发一帧）在**它自己执行期间**吐出来的：晚记一步就把它留在窗口外。

    case11 的 F3 就是这么丢的：`RESET=POR` 在 +41.0894s 到达，而窗口起点记在
    +41.094s，只差 3ms，一条确定性丢行把两边都判成红。
    """
    n = len(bus.capture.lines)
    if st.get("from_prev"):
        return int(anchors.get(str(getattr(bus, "label", "")), n))
    return n


@dataclass
class StepLog:
    op: str
    ok: bool
    detail: dict[str, Any] = field(default_factory=dict)
    skipped: bool = False
    note: str = ""


@dataclass
class PointOutcome:
    id: str
    kind: str
    max: float
    status: str  # pass | machine_fail | inconclusive | pending_ai | skipped | bloodline_zero
    score: float | None
    machine_evidence: list[str] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)

    def as_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "kind": self.kind,
            "max": self.max,
            "status": self.status,
            "score": self.score,
            "evidence": self.machine_evidence,
            "notes": self.notes,
        }


@dataclass
class RunOutcome:
    case: str
    agent: str
    sim: bool
    workdir: str
    started_at: float
    duration_s: float = 0.0
    bloodline: dict[str, Any] = field(default_factory=dict)
    caps: dict[str, Any] = field(default_factory=dict)
    steps: list[StepLog] = field(default_factory=list)
    asserts: list[dict[str, Any]] = field(default_factory=list)
    points: dict[str, PointOutcome] = field(default_factory=dict)
    static: dict[str, Any] = field(default_factory=dict)
    warnings: list[str] = field(default_factory=list)
    # 人工观测记录（`manual_observe` 步骤产出）。单独一列是因为它**不是机器证据**，
    # 评委引用它时必须能看出来自人眼，而不是"仪器测到了"。
    manual: list[dict[str, Any]] = field(default_factory=list)
    evidence_dir: str = ""
    not_run: bool = False
    # 这一轮是按哪一份规格跑的（`spec_fingerprint`）。激励脚本变过 ⇒ 旧证据与新判据不可比。
    spec_fp: str = ""

    # -- 汇总 -----------------------------------------------------------
    def summary(self) -> dict[str, Any]:
        auto = sum(p.score or 0.0 for p in self.points.values())
        in_scope = sum(p.max for p in self.points.values() if p.status != "skipped")
        # `pending_ai` 以前把**机器判红**（`machine_fail`）也塞进来，于是报告头那句
        # "待 AI 裁定：F1, F2, H2" 里其实有两项是已经判完的红。2026-10-17 补裁栏 A 就被它带偏过：
        # 照着这份清单裁完 10 条以为齐了，实际 `r3` 另外把 3 条判红降成了"未观测 ⇒ 交 AI"，
        # 而那 3 条不在冻结报告的旧清单上（状态是复判之后才变的）。现在两个数分开记：
        # **要不要人来裁**与**已经判成红**不许共用一个字段；待裁全集要从 `regrade/diff.json` 取。
        need_ai = [p.id for p in self.points.values() if p.status in ("pending_ai", "inconclusive")]
        reds = [p.id for p in self.points.values() if p.status == "machine_fail"]
        meta = P.CASE_META[self.case]
        return {
            "case": self.case,
            "title": meta["title"],
            "weight": meta["weight"],
            "agent": self.agent,
            "sim": self.sim,
            "not_run": self.not_run,
            "grader_rev": REV,
            "spec_fp": self.spec_fp,
            "bloodline_ok": bool(self.bloodline.get("ok")),
            "auto_score": round(auto, 2),
            "max_in_scope": round(in_scope, 2),
            "max_all": round(sum(p.max for p in self.points.values()), 2),
            "pending_ai": need_ai,
            "machine_fail": reds,
            "skipped": [p.id for p in self.points.values() if p.status == "skipped"],
            "duration_s": round(self.duration_s, 1),
            "warnings": self.warnings,
        }

    def to_dict(self) -> dict[str, Any]:
        return {
            "summary": self.summary(),
            "points": {k: v.as_dict() for k, v in self.points.items()},
            "asserts": self.asserts,
            "steps": [{"op": s.op, "ok": s.ok, "skipped": s.skipped, "note": s.note, **s.detail} for s in self.steps],
            "bloodline": self.bloodline,
            "caps": self.caps,
            "static": self.static,
            "manual": self.manual,
            "workdir": self.workdir,
            "evidence_dir": self.evidence_dir,
        }


# ---------------------------------------------------------------------------


def _read_line_with_timeout(timeout_s: float) -> tuple[str, bool]:
    """带超时的 `input()`，返回 (答案, 是否超时)。

    Windows 上 `select` 对 stdin 无效，只能用线程 + Queue。
    **超时后那个线程还卡在 `input()` 上**：此时人再打字会被这次已作废的提问吞掉，
    所以超时记录里带 `timed_out=True`，报告与证据都要写明，别让人以为答上了。
    """
    import queue
    import threading

    box: queue.Queue = queue.Queue()

    def _worker() -> None:
        try:
            box.put(input().strip())
        except Exception:                 # EOF / Ctrl-C 都算"没答"
            box.put("")

    t = threading.Thread(target=_worker, daemon=True)
    t.start()
    t.join(timeout_s)
    if not box.empty():
        return str(box.get()), False
    return "", True


class Runner:
    def __init__(self, cfg: Any, spec: SP.CaseSpec, agent: str, *, sim: bool = False,
                 out_root: Path | None = None, workdir: Path | None = None) -> None:
        self.cfg = cfg
        self.spec = spec
        self.agent = agent
        self.sim = sim
        # workdir 覆盖只给自测用：正常路径永远是测试集里的 runs/<agent>/<case>
        self.workdir = Path(workdir) if workdir else Path(cfg.testset) / "runs" / agent / spec.id
        run_id = time.strftime("%Y%m%d-%H%M%S")
        self.out = (out_root or Path(cfg.reports)) / f"{run_id}-{agent}-{spec.id}"
        self.evidence = self.out / "evidence"
        self.evidence.mkdir(parents=True, exist_ok=True)

        self.caps = CAPS.Caps()
        self.probe = Probe(cfg)
        self.fixture = Fixture(cfg, None)
        self.bus: SerialBus | SimBus | None = None
        self.buses: dict[str, SerialBus | SimBus] = {}
        self.bus_kind: str | None = None
        self._sends: list[str] = []
        self._send_times: list[float] = []
        self._sent_frames = 0
        self._measures: dict[str, dict[str, Any]] = {}
        self._la_stimulus: dict[str, Any] | None = None
        self._decoded_text: dict[str, str] = {}
        self._sim_scripts: dict[str, SimScript] = {}
        # 窗口锚点：_arm = 本步开始执行前的行数，_arm_prev = 上一步开始执行前的行数。
        # `from_prev` 的窗口用后者（见 window_start）。
        self._arm: dict[str, int] = {}
        self._arm_prev: dict[str, int] = {}
        # t_s / step 只为了"离线重判能对齐到原来那一步"：没有它们，重放时只能靠
        # 数行数猜窗口边界，而窗口边界正是最容易判错的东西（case11 就差 3ms）。
        self._t0: float | None = None
        self._cur_step: int | None = None

        self.outcome = RunOutcome(case=spec.id, agent=agent, sim=sim, workdir=str(self.workdir),
                                 started_at=time.time())
        self.outcome.spec_fp = spec.fingerprint
        self.outcome.evidence_dir = str(self.evidence)

    # ------------------------------------------------------------------ 工具
    def _write_evidence_json(self, name: str, payload: Any) -> Path:
        p = self.evidence / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
        return p

    def _log(self, op: str, ok: bool = True, *, skipped: bool = False, note: str = "", **detail: Any) -> StepLog:
        # 有的调用点直接展开 `**res.as_dict()`，而那种字典里自带 "ok" —— 会和
        # 这个参数撞成 "got multiple values for keyword argument 'ok'"，
        # 而且只在真板路径上出现（仿真会跳过 flash，永远跑不到）。
        # 统一在这里让"字典自己的结论"优先，别让每个调用点各写一遍。
        if "ok" in detail:
            ok = bool(detail.pop("ok"))
        if self._cur_step is not None:
            detail.setdefault("step", self._cur_step)
        if self._t0 is not None:
            detail.setdefault("t_s", round(time.monotonic() - self._t0, 3))
        s = StepLog(op=op, ok=ok, detail=detail, skipped=skipped, note=note)
        self.outcome.steps.append(s)
        return s

    def _cap_keys_for_spec(self) -> list[str]:
        return self.spec.required_caps()

    def _missing_caps(self) -> list[str]:
        return self.caps.require(self._cap_keys_for_spec())

    # ------------------------------------------------------------------ 主流程
    def run(self) -> RunOutcome:
        t0 = time.monotonic()
        self._t0 = t0
        cfg = self.cfg

        # 1) 能力探测
        self.caps = CAPS.probe_caps(cfg, check_hardware=not self.sim, fixture=self.fixture)
        self._write_evidence_json("caps.json", self.caps.to_dict())
        self.outcome.caps = self.caps.to_dict()
        missing = self._missing_caps()
        if missing and not self.sim:
            msg = f"缺少能力：{', '.join(missing)}（fallback.on_missing={self.spec.fallback['on_missing']}）"
            self.outcome.warnings.append(msg)
            self._log("caps", ok=False, note=msg, missing=missing)
        elif missing:
            self._log("caps", ok=True, note=f"模拟运行：真实能力 {', '.join(missing)} 未探测", missing=missing)
        else:
            self._log("caps", ok=True, caps=len(self.caps.caps))

        # 2) 夹具口：**在这里开，不等 spec 去开**（见 _setup_fixture 的注释）
        self._setup_fixture()

        # 3) 工作区就绪（不覆盖已有成果）
        self._step_prep()

        # 3) 血统闸门
        if not self._step_bloodline():
            self._finalize(t0)
            return self.outcome

        # 4) 静态规则（先跑：不依赖硬件，且能给后面的断言提供输入）
        self._step_static()

        # 5) 按序执行步骤
        for idx, st in enumerate(self.spec.steps):
            op = st["op"]
            try:
                self._dispatch(idx, st)
            except Exception as exc:  # noqa: BLE001 - 单步失败不该带走整轮，证据要留下来
                self._log(op, ok=False, note=f"{type(exc).__name__}: {exc}")
                self.outcome.warnings.append(f"steps[{idx}] {op} 失败：{exc}")
        self._finalize(t0)
        return self.outcome

    # ------------------------------------------------------------------ 步骤
    def _step_prep(self) -> None:
        if self.workdir.exists():
            self._log("prep", ok=True, note="工作区已存在，保持原样（不覆盖 agent 的成果）")
            return
        script = Path(self.cfg.testset) / "runner" / "prep.py"
        python = self.cfg.get("paths.python")
        from hal import proc

        res = proc.run([str(python), str(script), self.agent, self.spec.id], timeout_s=120)
        self._log("prep", ok=res.ok, note="由 runner/prep.py 生成干净副本" if res.ok else res.tail(6),
                  **res.as_dict())

    def _step_bloodline(self) -> bool:
        bl = BL.verify(self.cfg, self.workdir)
        self.outcome.bloodline = bl.as_dict()
        self._write_evidence_json("bloodline.json", bl.as_dict())
        (self.evidence / "bloodline_raw.txt").write_text(bl.raw, encoding="utf-8")
        self._log("verify_conversion", ok=bl.ok, note=bl.summary)
        if bl.not_run:
            self.outcome.not_run = True
            self.outcome.warnings.append(
                f"未运行：{bl.summary} —— 不按 0 分算，也不进分母"
            )
        elif not bl.ok:
            self.outcome.warnings.append("血统校验未通过 → 本题按 0 分处理，且不进行任何硬件操作")
        return bl.ok

    def _step_static(self) -> None:
        rules = rules_of(self.spec.id)
        if not rules:
            self._log("static_rules", ok=True, note="本题没有静态规则")
            return
        res = run_rules(self.spec.id, self.workdir)
        self.outcome.static = res
        self._write_evidence_json("static.json", res)
        n_ok = sum(1 for r in res.get("rules", {}).values() if r.get("ok") is True)
        n_bad = sum(1 for r in res.get("rules", {}).values() if r.get("ok") is False)
        n_inc = sum(1 for r in res.get("rules", {}).values() if r.get("ok") is None)
        self._log("static_rules", ok=True, rules=len(res.get("rules", {})), pass_=n_ok, fail=n_bad,
                  inconclusive=n_inc, files=res.get("files_scanned", 0))

    def _dispatch(self, idx: int, st: dict[str, Any]) -> None:
        op = st["op"]
        self._cur_step = idx
        # **在调用 handler 之前**记锚点。reset / send 期间到达的那一行往往就是
        # 本题要找的那一行；等 handler 跑完再记，它已经落在窗口外面了。
        for role, bus in self.buses.items():
            n = len(bus.capture.lines)
            self._arm_prev[role] = self._arm.get(role, n)
            self._arm[role] = n
        want_caps = st.get("require_caps") or []
        if want_caps:
            miss = self.caps.require(list(want_caps))
            if miss:
                self._log(op, ok=True, skipped=True, note=f"缺能力 {miss}，按 fallback 跳过", step=idx)
                self._record_unrun(idx, st, f"缺能力 {miss}")
                return

        handler = getattr(self, f"_op_{op}", None)
        if handler is None:
            self._log(op, ok=False, note="该 op 尚未实现", step=idx)
            self._record_unrun(idx, st, f"op {op} 尚未实现")
            return
        handler(idx, st)

    def _record_unrun(self, idx: int, st: dict[str, Any], reason: str) -> None:
        """步骤没跑：把它的断言全部记成"未观测"。

        为什么要这么做：断言若随着步骤一起消失，该评分点就会靠**剩下**的
        断言自动过关 —— 一道没做的检查变成了满分。这是本评测器最不能犯的错，
        所以"没跑"必须留下一条 ok=None 的记录，把该点推给 AI。
        """
        for a in st.get("asserts", []) or []:
            self._record_asserts([
                A.AssertResult(
                    id=a["id"], point=a.get("point", a["id"]), type=a["type"],
                    kind="skipped", ok=None, note=f"步骤未执行：{reason}",
                )
            ])

    # -- 步骤实现 --------------------------------------------------------
    def _op_prep(self, idx: int, st: dict[str, Any]) -> None:
        self._step_prep()

    def _op_verify_conversion(self, idx: int, st: dict[str, Any]) -> None:
        self._step_bloodline()

    def _op_build(self, idx: int, st: dict[str, Any]) -> None:
        elf = self._find_elf()
        self._log("build", ok=elf is not None, note="" if elf else "没找到固件产物（agent 可能没构建）",
                  elf=str(elf) if elf else None)

    def _find_elf(self) -> Path | None:
        return find_firmware(self.workdir)

    def _op_flash(self, idx: int, st: dict[str, Any]) -> None:
        if self.sim:
            self._log("flash", ok=True, skipped=True, note="模拟运行：不烧录")
            return
        elf = Path(st["elf"]) if st.get("elf") and st["elf"] != "auto" else self._find_elf()
        if elf is None:
            self._log("flash", ok=False, note="找不到固件产物")
            return
        res = self.probe.flash(elf)
        self._write_evidence_json(f"flash-{idx}.json", res.as_dict())
        detail = res.as_dict()
        detail.pop("ok", None)  # 结论走 ok= 参数，别再展开一遍
        self._log("flash", ok=res.ok, note=res.method, **detail)

    def _op_reset(self, idx: int, st: dict[str, Any]) -> None:
        kind = st.get("kind", "swd")
        if self.sim:
            self._log("reset", ok=True, skipped=True, note=f"模拟运行：跳过 {kind} 复位")
            return
        if kind == "auto":
            # 只在**已标定**具备真 NRST 时才用它；否则退到系统复位。
            # 差别不是学术性的：拉 NRST 置 PINRSTF，SWD 系统复位置 SFTRSTF，
            # 而 case11 的 RESET=POR 只对前者成立。
            kind = "nrst" if self.caps.has("reset:nrst") else "swd"

        if kind == "nrst":
            method = str(self.cfg.get("probe.nrst_method", "none"))
            if method == "fixture":
                try:
                    self.fixture.pulse("nrst", st.get("ms"))
                    self._log("reset", ok=True, kind="nrst", method="fixture")
                except FixtureUnavailable as exc:
                    self._log("reset", ok=False, kind="nrst", method="fixture", note=str(exc))
                return
            detail = self.probe.reset("nrst")
            self._log("reset", ok=bool(detail.get("ok")), kind="nrst", method=detail.get("method"),
                      note=detail.get("error") or "")
            return

        detail = self.probe.reset("swd")
        self._log("reset", ok=bool(detail.get("ok")), kind="swd", method=detail.get("method"))

    def _op_gpio_set(self, idx: int, st: dict[str, Any]) -> None:
        if self.sim:
            self._log("gpio_set", ok=True, skipped=True, note="模拟运行：无夹具")
            return
        value = st.get("value", "closed")
        asserted = value in (True, 1, "closed", "on", "press", "pressed", "asserted")
        try:
            self.fixture.set(st["name"], asserted)
            self._log("gpio_set", ok=True, name=st["name"], value=value)
        except FixtureUnavailable as exc:
            self._log("gpio_set", ok=False, name=st["name"], note=str(exc))

    def _op_gpio_pulse(self, idx: int, st: dict[str, Any]) -> None:
        count = int(st.get("count", 1))
        gap_ms = int(st.get("gap_ms", 0))
        ms = st.get("ms")
        if self.sim:
            self._log("gpio_pulse", ok=True, skipped=True, note="模拟运行：无夹具",
                      name=st["name"], count=count)
            return
        ok = True
        note = ""
        try:
            for i in range(count):
                if i and gap_ms:
                    time.sleep(gap_ms / 1000.0)
                self.fixture.pulse(st["name"], ms)
        except FixtureUnavailable as exc:
            ok = False
            note = str(exc)
        self._log("gpio_pulse", ok=ok, name=st["name"], ms=ms, count=count, gap_ms=gap_ms, note=note)

    def _resolve_bus(self, st: dict[str, Any]) -> SerialBus | SimBus | None:
        role = st.get("port")
        if role:
            return self.buses.get(role)
        return self.bus

    def _op_power_cycle(self, idx: int, st: dict[str, Any]) -> None:
        self._log("power_cycle", ok=False, skipped=True,
                  note="受控电源尚未接线（plan §0 的显式缺口），此项留给人工或 P1 夹具")

    def _setup_fixture(self) -> None:
        """启动时就打开夹具串口，并**立刻把所有激励置成"未按下"**。

        两件事都必须在这里做，理由都很硬：

        ① **不能让每个 spec 自己去开夹具口。** 原来四个带 `gpio_*` 的题都只开
           `dut_vcp`，于是 `fixture.bus` 永远是 None，每个按键步骤都会失败 ——
           而那种失败在报告上看起来像"夹具坏了"，排查方向完全是错的。
           夹具是基础设施，不是每道题该记着的事。

        ② **状态安全。** ESP-01S 板的 IO0 空闲是**高**，而 B1 是高有效 ——
           也就是说"没人控制这根线"的时候，按键是被**一直按住**的。
           所以开完串口立刻置成未按下，DUT 一开跑看到的就是干净的状态。
        """
        if self.sim or not self.fixture.enabled:
            return
        role = str(self.cfg.get("fixture.port_role", "fixture_uart"))
        if role in self.buses:
            self.fixture.bus = self.buses[role]  # type: ignore[assignment]
            return
        try:
            match = ports.resolve_role(self.cfg, role)
        except (ports.PortAmbiguity, KeyError) as exc:
            self.outcome.warnings.append(f"夹具已启用但口不可用：{exc}")
            self._log("fixture_open", ok=False, port=role, note=str(exc))
            return
        if match is None:
            self.outcome.warnings.append("夹具已启用但没找到夹具串口 → gpio 步骤会按缺失处理")
            self._log("fixture_open", ok=False, port=role, note="端口未找到")
            return
        try:
            real = SerialBus(match.device, int(self.cfg.get("serial.default_baud", 115200)), label=role)
            real.open()
        except RuntimeError as exc:
            self.outcome.warnings.append(f"夹具口打不开：{exc}")
            self._log("fixture_open", ok=False, port=role, device=match.device, note=str(exc))
            return
        self.buses[role] = real
        self.fixture.bus = real
        # 告诉夹具这道题**真的会用哪些激励**（spec 的 caps.gpio）。
        # 交叉耦合下，用不到的那根不能"顺手放回空闲" —— 会把在用的那根踩掉
        # （case06 实测：release_all 按旧语义断言了 RTS，继电器就再也合不上）。
        self.fixture.active_names = {
            k.split(":", 1)[1] for k in self._cap_keys_for_spec()
            if k.startswith("gpio:") and ":" in k
        } or None
        self.fixture.release_all()  # 立刻进入"未按下"，别让按键悬着
        resting = {k: v["state"] for k, v in self.fixture.evidence()["stimuli"].items()}
        self._log("fixture_open", ok=True, port=role, device=match.device, resting=resting,
                  note="已把所有激励置成未按下")

    def _op_serial_open(self, idx: int, st: dict[str, Any]) -> None:
        role = st["port"]
        if self.sim:
            bus: SerialBus | SimBus = SimBus(self._sim_scripts.get(role, SimScript()), label=role)
            bus.open()
            self.bus_kind = "sim"
            self.buses[role] = bus
            self.bus = bus
            self._log("serial_open", ok=True, port=role, kind="sim")
            return
        if role in self.buses:
            # 夹具口在 _setup_fixture 里已经开好了 —— 复用，别重复打开
            # （重复打开会先把 DTR 复位一次，等于在题目开始时抖一下按键）
            self.bus = self.buses[role]
            if role == str(self.cfg.get("fixture.port_role", "fixture_uart")):
                self.fixture.bus = self.bus  # type: ignore[assignment]
            self._log("serial_open", ok=True, port=role, reused=True)
            return
        try:
            match = ports.resolve_role(self.cfg, role)
        except ports.PortAmbiguity as exc:
            # 同型号板并存且没指名：停在这里，别把命令发给可能错的那一块
            self._log("serial_open", ok=False, port=role, note=str(exc))
            self._record_unrun(idx, st, f"串口角色有歧义：{exc}")
            return
        except KeyError as exc:
            self._log("serial_open", ok=False, port=role, note=str(exc))
            self._record_unrun(idx, st, str(exc))
            return
        if match is None:
            self._log("serial_open", ok=False, port=role, note="按 VID/PID 没找到端口")
            return
        real = SerialBus(
            match.device,
            int(st.get("baud", self.cfg.get("serial.default_baud", 115200))),
            label=role,
            open_retries=int(self.cfg.get("serial.open_retries", 5)),
            retry_delay_s=float(self.cfg.get("serial.retry_delay_ms", 400)) / 1000.0,
        )
        try:
            real.open()
        except RuntimeError as exc:
            self._log("serial_open", ok=False, port=role, device=match.device, note=str(exc))
            return
        self.buses[role] = real
        self.bus = real
        self.bus_kind = "real"
        if role == "fixture_uart":
            self.fixture.bus = real
        self._log("serial_open", ok=True, port=role, device=match.device, baud=real.baud)

    def _serial_ctx(self, st: dict[str, Any], window: list[Any], log_ref: str,
                    bus: Any = None) -> A.Ctx:
        link_lines = len(bus.capture.lines) if bus is not None else 0
        return A.Ctx(
            lines=window,
            sends=self._sends,
            send_times=self._send_times,
            log_ref=log_ref,
            static=self.outcome.static,
            bloodline=self.outcome.bloodline,
            hardware_simulated=self.sim,
            has_la=self.caps.has("la"),
            has_camera=self.caps.has("camera"),
            sent_frames=self._sent_frames,
            measure=self._measures,
            link_alive=link_lines > 0,
            link_lines=link_lines,
        )

    def _op_serial_send(self, idx: int, st: dict[str, Any]) -> None:
        bus = self._resolve_bus(st)
        if bus is None:
            self._log("serial_send", ok=False, port=st.get("port"), note="串口未打开")
            return
        text = st.get("text", "")
        payload = st.get("payload")
        if payload:
            text = _gen_payload(int(payload.get("chars", 1024)), int(payload.get("seed", 42)),
                                int(payload.get("line_chars", 63)))
        eol = st.get("eol", "\r\n")
        repeat = int(st.get("repeat", 1))
        gap_ms = int(st.get("gap_ms", 0))
        for i in range(repeat):
            if i and gap_ms:
                bus.wait(gap_ms / 1000.0)
            if st.get("pre_delay_ms"):
                bus.wait(float(st["pre_delay_ms"]) / 1000.0)
            bus.send_line(text, eol)
            self._sends.append(text)
            self._send_times.append(bus.capture.relative_now(bus.now()))
        self._log("serial_send", ok=True, port=st.get("port"), chars=len(text), repeat=repeat,
                  gap_ms=gap_ms, preview=text[:48].replace("\n", "\\n"))

    def _op_serial_burst(self, idx: int, st: dict[str, Any]) -> None:
        """按帧表循环发送（case08 的"50ms 间隔连发 100 帧"就是这个动作）。

        帧表里的每一帧都进 ctx.sends，所以 `sent_frames` 符号能对上
        COUNT? 的期望值。
        """
        bus = self._resolve_bus(st)
        frames = list(st.get("frames") or ([st["text"]] if st.get("text") else []))
        if not frames:
            self._log("serial_burst", ok=False, note="frames 为空")
            return
        if bus is None:
            self._log("serial_burst", ok=False, note="串口未打开")
            return
        count = int(st.get("count", len(frames)))
        gap_ms = int(st.get("gap_ms", 0))
        eol = st.get("eol", "\r\n")
        for i in range(count):
            if i and gap_ms:
                bus.wait(gap_ms / 1000.0)
            text = frames[i % len(frames)]
            bus.send_line(text, eol)
            self._sends.append(text)
            self._send_times.append(bus.capture.relative_now(bus.now()))
            self._sent_frames += 1
        self._log("serial_burst", ok=True, frames=len(frames), count=count, gap_ms=gap_ms)

    # -- 仪器：逻辑分析仪 / 相机 ------------------------------------------
    def _la(self) -> Any:
        from hal.la import LogicAnalyzer

        return LogicAnalyzer(self.cfg)

    def _camera(self) -> Any:
        from hal.camera import Camera

        return Camera(self.cfg)

    def _capture_with_stimulus(self, la: Any, seconds: float, out: Path, st: dict[str, Any]) -> Any:
        """采集窗内主动激励：sigrok 起进程之后再按时刻表发帧，让两边时间重叠。

        为什么必须重叠而不能排在前面：`edges` / `frequency` 这类判据看的是**窗内**
        有没有跳变。LED ON/OFF 若排在采集之前，窗内引脚是恒定的，边沿数必然 0 ——
        2026-10-17 实测两个 agent 的 case08 F2a/b/c 全过、引脚也选对了，却一起挂在
        F2d 上，判分器自己把一道题变成了假红。
        """
        kwargs = dict(driver=st.get("driver"), samplerate=st.get("samplerate"),
                      channels=st.get("channels"))
        frames = list(st.get("during") or [])
        if not frames:
            return la.capture(seconds, out, **kwargs)
        # **驱动解析必须在起采集线程之前做完。** `driver="auto"` 要起 6 次
        # `sigrok-cli --scan`（实测 ~1.1s），原来它发生在 `la.capture()` 内部、
        # 也就是线程里；而下面的时刻表从线程启动就开始计时 ⇒ 采样真正开始前
        # 的前两条激励直接落在窗外。2026-10-18 实测：case08 F2d 只量到 3 个边沿，
        # 间隔 500/400ms 正好对上第 3/4/5 条激励，前两条在采样开始前就发完了。
        if (kwargs.get("driver") or getattr(la, "driver", "")) == "auto":
            kwargs["driver"] = la.resolve_driver()

        # **这块克隆板会提前收工。** 同一命令实测到 12.0M（完整）、10.9M、10.27M、
        # 7.1M（只剩 1.78s）四种结果，间歇且与采样率无关 ⇒ 采集短了就重来一次。
        # 不重试的后果：窗内后两条激励落在被截掉的那段里，`edges` 少数两次 ——
        # 那是仪器丢数据，不是固件没做。重试的帧也要计入 `sent_frames`（固件确实处理了）。
        attempts = max(1, int(st.get("capture_retries", 2)))
        best: Any = None
        best_path: Path | None = None
        total_sent = 0
        short: list[int] = []
        for attempt in range(attempts):
            cand = out if attempt == 0 else out.with_name(f"{out.stem}-try{attempt + 1}{out.suffix}")
            box: list[Any] = []
            err: list[BaseException] = []

            def _run(target: Path = cand) -> None:
                try:
                    box.append(la.capture(seconds, target, **kwargs))
                except BaseException as exc:  # noqa: BLE001 - 线程里抛不出来，必须带回去
                    err.append(exc)

            th = threading.Thread(target=_run, daemon=True, name=f"la-step-{self._cur_step}")
            if st.get("pre_delay_ms"):
                time.sleep(float(st["pre_delay_ms"]) / 1000.0)
            th.start()
            # sigrok-cli 从启动到真正开始采样有几百毫秒，早发的帧会落在窗外
            time.sleep(float(st.get("stimulus_delay_ms", 400)) / 1000.0)
            bus = self._resolve_bus(st)
            t0 = time.monotonic()
            sent = 0
            for fr in frames:
                if err:
                    break
                wait_s = float(fr.get("at_ms", 0)) / 1000.0 - (time.monotonic() - t0)
                if wait_s > 0:
                    time.sleep(wait_s)
                if bus is None:
                    break
                try:
                    bus.send_line(str(fr["text"]), st.get("eol", "\r\n"))
                    sent += 1
                except Exception as exc:  # noqa: BLE001 - 激励失败别把整次采集毁了
                    err.append(exc)
                    break
            th.join(timeout=seconds + 30)
            if err:
                raise err[0]
            if not box:
                raise RuntimeError(f"分析仪采集线程没有返回（激励了 {sent}/{len(frames)} 帧）")
            total_sent += sent
            cap = box[0]
            expected = int(cap.samplerate_hz * seconds)
            if best is None or cap.n_samples > best.n_samples:
                best, best_path = cap, cand
            if cap.n_samples >= expected * 0.95:
                break
            short.append(cap.n_samples)

        assert best is not None and best_path is not None
        if best_path != out:
            shutil.copyfile(best_path, out)
            try:
                best_path.unlink()
            except OSError:
                pass
            best.path = str(out)
        self._la_stimulus = {"frames": len(frames), "sent": total_sent,
                             "attempts": attempt + 1,
                             "truncated_samples": short or None}
        # 窗内激励也是**发出去并被固件处理的帧**，必须进 `sent_frames` 这本账。
        # 不记的后果实测过：case08 的 F3b 拿 `sent_frames`（当时只有 burst 的 100）
        # 去比对固件的 `COUNT?`，而两个 agent 都如实答 105（burst 100 + 窗内 LED 5）
        # ⇒ 各自白丢 0.5 分。判分器少记了 5 帧，不是被测多数了 5 帧。
        # 重试时两批帧固件都处理了，所以计总数。
        self._sent_frames += total_sent
        return best

    def _op_la_capture(self, idx: int, st: dict[str, Any]) -> None:
        if self.sim:
            self._log("la_capture", ok=True, skipped=True, note="模拟运行：无分析仪")
            self._record_unrun(idx, st, "模拟运行：逻辑分析仪未采集")
            return
        la = self._la()
        if not la.available():
            self._log("la_capture", ok=False, skipped=True,
                      note="sigrok-cli 未安装（README『首次标定』一节）")
            self._record_unrun(idx, st, "逻辑分析仪不可用")
            return

        seconds = float(st["duration_ms"]) / 1000.0
        out = self.evidence / "la" / f"capture-{idx}.sr"
        try:
            from hal import la as la_mod

            cap = self._capture_with_stimulus(la, seconds, out, st)
        except Exception as exc:  # noqa: BLE001 - 仪器型号/驱动问题不该带走整轮
            self._log("la_capture", ok=False, note=f"{type(exc).__name__}: {exc}")
            self._record_unrun(idx, st, f"抓包失败：{exc}")
            return

        measures: dict[str, Any] = {}
        decoded_cache: dict[str, str] = {}
        unresolved: list[tuple[dict[str, Any], str]] = []
        for a in st.get("asserts", []) or []:
            ch = a.get("channel")
            key = A.measure_key(a)
            if a["type"] == "decoded_contains":
                spec = a.get("decoder", "")
                # 通道号取决于实际接线，写在 config 里（la.decoders.<name>）
                options = str(a.get("options") or self.cfg.get(f"la.decoders.{spec}", "") or "")
                cache_key = f"{spec}:{options}"
                if cache_key not in decoded_cache:
                    try:
                        decoded_cache[cache_key] = la.decode(cap, spec, options=options)
                    except Exception as exc:  # noqa: BLE001
                        decoded_cache[cache_key] = f"解码失败：{exc}"
                text = decoded_cache[cache_key]
                lines = __import__("re").findall(r"[^\n]+", text)
                self._measures[key] = {"hits": lines, "n": len(lines), "evidence": [str(out)]}
                self._decoded_text[cache_key] = text
                continue
            if not ch:
                unresolved.append((a, "断言没写 channel"))
                continue
            try:
                chidx = la.channel_of(str(ch))
            except Exception as exc:  # noqa: BLE001
                # 通道映射缺失/写错：**必须留下记录**，否则这条断言会消失，
                # 该点靠剩下的断言自动过关
                unresolved.append((a, f"通道映射失败：{exc}"))
                continue
            bits = cap.bits.get(f"D{chidx}")
            if bits is None:
                unresolved.append((a, f"分析仪没有 D{chidx} 这根通道（抓到的有 {sorted(cap.bits)}）"))
                continue
            ev = [f"{Path(out).name}#D{chidx}@{cap.samplerate_hz:.0f}Hz"]
            if a["type"] == "frequency":
                measures[key] = {**la_mod.measure_frequency(bits, cap.samplerate_hz), "evidence": ev}
            elif a["type"] == "duty":
                measures[key] = {**la_mod.measure_duty(bits, cap.samplerate_hz), "evidence": ev}
            elif a["type"] == "edges":
                measures[key] = {**la_mod.count_edges(bits), "evidence": ev}
            elif a["type"] == "pulse_width":
                measures[key] = {**la_mod.measure_pulse_width(bits, cap.samplerate_hz), "evidence": ev}
        self._measures.update(measures)
        for a, why in unresolved:
            self._record_asserts([A.AssertResult(
                id=a["id"], point=a.get("point", a["id"]), type=a["type"], kind="skipped",
                ok=None, note=f"未采集：{why}")])
        self._write_evidence_json(f"la-measure-{idx}.json",
                                  {"capture": cap.as_dict(),
                                   "measures": measures,
                                   "unresolved": [{"assert": a["id"], "why": why} for a, why in unresolved],
                                   "decoded": {k: v[:4000] for k, v in self._decoded_text.items()}})

        ctx = self._serial_ctx(st, [], "")
        # **零活动防线**：整盘一个跳变都没有时，"固件真没输出"和"采样时机错过 /
        # 探头没夹 / 没共地"在数据上完全同形 —— 这时候判红是把判分器的失明
        # 记到被测对象头上。记未观测，把该点推给 AI（拔探头就该得到 —，不是 ✗）。
        flat = bool(cap.activity) and all(int(v.get("edges", 0)) < 2 for v in cap.activity.values())
        # **截断防线**：克隆板会在 2.6–3.0s 之间随机收工（r6 实测，与采样率无关）。
        # 若最后一条窗内激励落在被截掉的那段里，"少了一个边沿"是仪器没看到，不是固件没做。
        # 截断只能**藏掉**边沿，所以已经通过的断言仍然成立；只有判红的那几条要降成未观测。
        during = list(st.get("during") or [])
        covered_s = cap.n_samples / cap.samplerate_hz if cap.samplerate_hz else 0.0
        need_s = (float(st.get("stimulus_delay_ms", 400))
                  + max((float(fr.get("at_ms", 0)) for fr in during), default=0.0)) / 1000.0
        truncated = bool(during) and covered_s + 0.02 < need_s
        results = []
        for a in st.get("asserts", []) or []:
            if flat and assert_needs_activity(a):
                results.append(A.AssertResult(
                    id=a["id"], point=a.get("point", a["id"]), type=a["type"], kind="skipped",
                    ok=None, confidence="low",
                    note="整盘采集零活动：分不清『真没输出』与『时机错过/探头没夹』 ⇒ 记未观测"))
            else:
                results.append(A.evaluate(a, ctx))
        if truncated:
            for i, r in enumerate(results):
                if r.ok is False:
                    results[i] = A.AssertResult(
                        id=r.id, point=r.point, type=r.type, kind="skipped", ok=None,
                        confidence="low", want=r.want, got=r.got,
                        note=(f"采集被截断：只覆盖 {covered_s:.2f}s，最后一条窗内激励在 "
                              f"{need_s:.2f}s ⇒ 缺的那几个跳变可能在窗外，记未观测（不判红）"))
        self._record_asserts(results)
        n_bad = sum(1 for r in results if r.ok is False)
        active = [k for k, v in (cap.activity or {}).items() if v.get("edges", 0) >= 2]
        self._log("la_capture", ok=n_bad == 0, samplerate=cap.samplerate_hz, samples=cap.n_samples,
                  asserts=len(results), failed=n_bad, file=str(out),
                  stimulus=self._la_stimulus,
                  active_channels=active or None,
                  note=("有跳变的通道：" + ", ".join(active)) if active else
                       "所有通道都没有跳变 —— 检查探头夹在哪根线上、有没有共地")

    def _op_frame(self, idx: int, st: dict[str, Any]) -> None:
        if self.sim:
            self._log("frame", ok=True, skipped=True, note="模拟运行：无相机")
            self._record_unrun(idx, st, "模拟运行：相机未采集")
            return
        cam = self._camera()
        if not cam.available():
            self._log("frame", ok=False, skipped=True, note="ffmpeg 不在 PATH 上")
            self._record_unrun(idx, st, "相机不可用")
            return
        name = str(st.get("save", f"frame-{idx}"))
        png = self.evidence / "frames" / f"{Path(name).stem}.png"
        cam.snapshot(png)
        seconds = float(st.get("duration_ms", 500)) / 1000.0
        fps = float(st.get("fps", 10))
        try:
            res = cam.record(seconds, fps, out=self.evidence / "frames" / f"{Path(name).stem}.raw")
        except Exception as exc:  # noqa: BLE001
            self._log("frame", ok=False, note=f"{type(exc).__name__}: {exc}")
            self._record_unrun(idx, st, f"取帧失败：{exc}")
            return
        from hal import vision

        roi = _roi(st, self.cfg)
        summary = vision.summarize(res.frames, roi, res.fps)
        self._measures["brightness:"] = {**summary["brightness"], "n": res.n_frames,
                                        "evidence": [str(png.relative_to(self.evidence))]}
        self._write_evidence_json(f"frame-{idx}.json", {"capture": res.as_dict(), "vision": summary})
        ctx = self._serial_ctx(st, [], "")
        results = [A.evaluate(a, ctx) for a in st.get("asserts", []) or []]
        self._record_asserts(results)
        self._log("frame", ok=True, frames=res.n_frames, fps=round(res.fps, 2), note=res.note or "",
                  png=str(png), asserts=len(results))

    def _op_frame_burst(self, idx: int, st: dict[str, Any]) -> None:
        if self.sim:
            self._log("frame_burst", ok=True, skipped=True, note="模拟运行：无相机")
            self._record_unrun(idx, st, "模拟运行：相机未采集")
            return
        cam = self._camera()
        if not cam.available():
            self._log("frame_burst", ok=False, skipped=True, note="ffmpeg 不在 PATH 上")
            self._record_unrun(idx, st, "相机不可用")
            return
        seconds = float(st["duration_ms"]) / 1000.0
        fps = float(st.get("fps", 20))
        name = str(st.get("save", f"burst-{idx}"))
        try:
            res = cam.record(seconds, fps, out=self.evidence / "frames" / f"{Path(name).stem}.raw")
        except Exception as exc:  # noqa: BLE001
            self._log("frame_burst", ok=False, note=f"{type(exc).__name__}: {exc}")
            self._record_unrun(idx, st, f"录制失败：{exc}")
            return
        from hal import vision

        roi = _roi(st, self.cfg)
        summary = vision.summarize(res.frames, roi, res.fps)
        png = self.evidence / "frames" / f"{Path(name).stem}.png"
        cam.snapshot(png)
        ev = [str(png.relative_to(self.evidence))]
        self._measures["brightness:"] = {**summary["brightness"], "n": res.n_frames, "evidence": ev}
        self._measures["blink:"] = {**summary["blink"], "n": res.n_frames, "evidence": ev}
        self._measures["motion:"] = {**summary["motion"], "n": res.n_frames, "evidence": ev}
        self._write_evidence_json(f"burst-{idx}.json", {"capture": res.as_dict(), "vision": summary})
        ctx = self._serial_ctx(st, [], "")
        results = [A.evaluate(a, ctx) for a in st.get("asserts", []) or []]
        self._record_asserts(results)
        n_bad = sum(1 for r in results if r.ok is False)
        self._log("frame_burst", ok=n_bad == 0, frames=res.n_frames, fps=round(res.fps, 2),
                  note=res.note or "", asserts=len(results), failed=n_bad)

    def _op_check(self, idx: int, st: dict[str, Any]) -> None:
        """不碰硬件的断言：静态规则、血统这类，只想把结论记进断言表。"""
        ctx = self._serial_ctx(st, [], "")
        results = [A.evaluate(a, ctx) for a in st.get("asserts", []) or []]
        self._record_asserts(results)
        n_bad = sum(1 for r in results if r.ok is False)
        n_inc = sum(1 for r in results if r.ok is None)
        self._log("check", ok=n_bad == 0, asserts=len(results), failed=n_bad, inconclusive=n_inc)

    def _op_serial_window(self, idx: int, st: dict[str, Any]) -> None:
        bus = self._resolve_bus(st)
        if bus is None:
            # 端口没开是**基础设施**问题，不是被测对象的失败：记成未观测，
            # 让该点落到 AI 手里，而不是拿"没输出"去判它失败
            self._log("serial_window", ok=False, port=st.get("port"), note="串口未打开，窗口未采集")
            self._record_unrun(idx, st, "串口未打开")
            return
        start = window_start(bus, st, self._arm_prev)
        bus.wait(float(st["duration_ms"]) / 1000.0)
        bus.flush_evidence()
        window = bus.capture.since(start) if st.get("scope", "window") == "window" else bus.capture.snapshot()
        save = st.get("save")
        log_ref = ""
        if save:
            p = self.evidence / str(save)
            bus.capture.write_log(p, header=f"{self.spec.id} {self.agent} 窗口 {idx}")
            log_ref = str(Path("evidence") / str(save)).replace("\\", "/")
        ctx = self._serial_ctx(st, window, log_ref, bus)
        results = [A.evaluate(a, ctx) for a in st.get("asserts", []) or []]
        self._record_asserts(results)
        n_bad = sum(1 for r in results if r.ok is False)
        self._log("serial_window", ok=n_bad == 0, port=st.get("port"), duration_ms=st["duration_ms"],
                  lines=len(window), asserts=len(results), failed=n_bad, log=log_ref or None,
                  anchor=start, from_prev=bool(st.get("from_prev")))

    def _op_probe_question(self, idx: int, st: dict[str, Any]) -> None:
        """脚本化的追问：对两个 agent 必须逐字同一句（公平性）。"""
        text = st["text"]
        bus = self._resolve_bus(st)
        if bus is None:
            self._log("probe_question", ok=False, note="串口未打开，无法投递追问")
            return
        bus.send_line(text, st.get("eol", "\r\n"))
        self._log("probe_question", ok=True, text=text, note="已按脚本投递（两 agent 同一句）")

    def _op_manual_observe(self, idx: int, st: dict[str, Any]) -> None:
        """把"人眼看到的现象"变成**可复核**的证据。

        铁律第 5 条说隐藏分只能来自"代码审查 + 现象"。有几项现象现在没有仪器能
        自动取（呼吸是否平滑、屏上显示的字 —— 摄像头机位受限），只能人看。
        但"口头说一句"不算证据，所以这里：

        - **问题写死在 spec 里**，两个 agent 逐字同一句（和 `probe_question` 一样
          受公平性约束），人只负责回答；
        - 回答连**时间戳 + 当时串口最近几行**一起落盘，评委看得见"这是人工观测"；
        - 超时/跳过就记"未答"，**该点不得据此给分**（宁可少给，不可假绿）。
        """
        q = str(st["question"])
        point = st.get("point")
        choices = [str(c) for c in (st.get("choices") or [])]
        timeout_s = float(st.get("timeout_s", self.cfg.get("manual.timeout_s", 300)))
        if self.sim:
            self._log("manual_observe", ok=True, skipped=True, question=q,
                      note="模拟运行：不提问，该点保持待 AI 裁定")
            return

        print("\n" + "=" * 72, flush=True)
        print(f"【人工观测】{q}", flush=True)
        if choices:
            print(f"    候选：{' / '.join(choices)}", flush=True)
        print(f"    作答后回车；{int(timeout_s)} 秒内不作答 = 记为未答", flush=True)
        print("=" * 72, flush=True)

        answer, timed_out = _read_line_with_timeout(timeout_s)
        tail: list[str] = []
        for bus in self.buses.values():
            cap = getattr(bus, "capture", None)
            if cap is not None:
                tail = [ln.text for ln in cap.snapshot()][-6:]
                break
        rec: dict[str, Any] = {
            "t_wall": time.strftime("%Y-%m-%d %H:%M:%S"),
            "t_run_s": round(max(0.0, time.time() - self.outcome.started_at), 1),
            "point": point,
            "question": q,
            "answer": answer,
            "timed_out": timed_out,
            "in_choices": (answer in choices) if choices else None,
            "serial_tail": tail,
            "source": "人工观测",
        }
        self.outcome.manual.append(rec)
        self._write_evidence_json("manual.json", {"observations": self.outcome.manual})

        if not answer:
            note = ("超时未答 —— 该点不得据此给分" if timed_out
                    else "未作答 —— 该点不得据此给分")
        elif choices and rec["in_choices"] is False:
            note = f"答：{answer}（不在候选里，按原话记录）"
        else:
            note = f"答：{answer}"
        # 往评分点上挂这条注记由 `_aggregate` 统一做（那时 points 才存在）
        self._log("manual_observe", ok=True, question=q, answer=answer,
                  point=point, timed_out=timed_out, note=note)

    def _op_ai_judge(self, idx: int, st: dict[str, Any]) -> None:
        self._log("ai_judge", ok=True, points=st["points"], evidence=st.get("evidence", []),
                  note="交由 AI 评委出分（P3）")

    def _op_skip_annotated(self, idx: int, st: dict[str, Any]) -> None:
        self._log("skip_annotated", ok=True, skipped=True, points=st["points"], reason=st["reason"])

    # -- 断言收集 --------------------------------------------------------
    def _record_asserts(self, results: list[A.AssertResult], *, carried: bool = False) -> None:
        for r in results:
            # grader_rev 逐条盖：判分口径换过版本后，同一份报告里不可能新旧混排。
            # carried=True 是离线重判专用：这条结论**没有被新尺子复算过**，
            # 只是把旧判定原样搬过来占位，对比表里必须能看出来。
            d = r.as_dict()
            d["grader_rev"] = REV
            if carried:
                d["carried"] = True
            self.outcome.asserts.append(d)

    # ------------------------------------------------------------------ 收尾
    def _finalize(self, t0: float) -> None:
        # **先把夹具放回"未按下"，再关串口。** 顺序反了的话，关口时驱动会
        # 把线电平还原（TXD 从 BREAK 恢复成空闲高），而 B1 是高有效 ——
        # 那就等于在评测结束的瞬间补按了一下，下一题的日志会被这一下污染。
        if not self.sim:
            try:
                self.fixture.release_all()
            except Exception:  # noqa: BLE001
                pass
        for role, bus in self.buses.items():
            try:
                bus.capture.write_log(self.evidence / "serial" / f"all-{role}.log",
                                      header=f"{self.spec.id} {self.agent} {role} 全部采集")
            except Exception:  # noqa: BLE001
                pass
            try:
                bus.close()
            except Exception:  # noqa: BLE001
                pass
        self.buses.clear()
        self.bus = None
        self.outcome.duration_s = time.monotonic() - t0
        self.outcome.points = self._aggregate()
        self._write_evidence_json("asserts.json", {"asserts": self.outcome.asserts,
                                                  "grader_rev": REV,
                                                  "unresolved": self.outcome.summary()["pending_ai"]})
        self._write_evidence_json("result.json", self.outcome.to_dict())
        self._write_report_md()

    def _aggregate(self) -> dict[str, PointOutcome]:
        out: dict[str, PointOutcome] = {}
        skipped = self.spec.skipped_points()
        ai_points = set(self.spec.ai_points())
        bl_ok = bool(self.outcome.bloodline.get("ok"))
        by_point: dict[str, list[dict[str, Any]]] = {}
        for a in self.outcome.asserts:
            by_point.setdefault(a["point"], []).append(a)

        for pt in P.points_of(self.spec.id):
            declared = pt.id in by_point or pt.id in ai_points or pt.id in skipped
            if not declared:
                out[pt.id] = PointOutcome(pt.id, pt.kind, pt.max, "skipped", None,
                                          notes=["规格里没有声明这个评分点（未覆盖）"])
                continue
            if self.outcome.not_run:
                out[pt.id] = PointOutcome(pt.id, pt.kind, pt.max, "skipped", None,
                                          notes=["这题还没跑（工作区不存在）—— 不计分也不进分母"])
                continue
            if not bl_ok:
                out[pt.id] = PointOutcome(pt.id, pt.kind, pt.max, "bloodline_zero", 0.0,
                                          notes=["血统违规 → 本题 0 分"])
                continue
            if pt.id in skipped:
                out[pt.id] = PointOutcome(pt.id, pt.kind, pt.max, "skipped", None,
                                          notes=[skipped[pt.id]])
                continue
            rel = by_point.get(pt.id, [])
            fails = [a for a in rel if a["ok"] is False]
            incon = [a for a in rel if a["ok"] is None]
            # **"依据"列必须指向造成这个结论的那几条断言。**
            # 原来是无序取该点所有断言的前 4 条证据 —— case08 的 F2 就因此指着
            # burst.log 里几条**通过**的断言，而真正挂掉的是 LA 的 F2d，
            # 看报告的人据此会去查错的东西（比判错更难复盘）。
            ev = [e for a in (fails or incon or rel) for e in a["evidence"]][:4]
            if fails:
                out[pt.id] = PointOutcome(
                    pt.id, pt.kind, pt.max, "machine_fail", None, ev,
                    notes=[f"机器断言失败：{fails[0]['id']}（{fails[0]['type']}）" + (f" — {fails[0]['note']}" if fails[0].get("note") else "")]
                    + ([f"另有 {len(fails) - 1} 条失败"] if len(fails) > 1 else []),
                )
                continue
            if incon or pt.id in ai_points:
                notes = [f"{a['id']} 无法机器判定（{a.get('note') or a['type']}）" for a in incon]
                if pt.id in ai_points:
                    notes.append("规格声明该点由 AI 判定")
                out[pt.id] = PointOutcome(pt.id, pt.kind, pt.max, "pending_ai", None, ev, notes)
                continue
            out[pt.id] = PointOutcome(pt.id, pt.kind, pt.max, "pass", pt.max, ev, notes=["全部机器断言通过"])

        # **人工观测挂到它支撑的点上。** 不在步骤里做：那时 `outcome.points`
        # 还没建（是 `_aggregate` 才建的），当场写会静默丢掉 —— 一处逻辑只留一份实现。
        for m in self.outcome.manual:
            pid = m.get("point")
            if not pid or pid not in out:
                continue
            ans = m.get("answer") or ("超时未答" if m.get("timed_out") else "未作答")
            tag = "人工观测" if m.get("answer") else "人工观测（**不构成给分依据**）"
            out[pid].notes.append(f"{tag}：{ans}")
        # judge_table 里写了"失败即功能分 0"的点：这里显式标注，避免被当成普通失败
        for pt in P.points_of(self.spec.id):
            if pt.zeroes_functional:
                o = out[pt.id]
                if o.status in ("machine_fail",):
                    o.notes.append("judge_table：该项失败 → 功能分直接 0（待 AI 确认）")
        return out

    def _agent_banner(self) -> str:
        """报告里那行"被测 agent 配置"。

        为什么要写进报告：两个 agent 的模型/思考档不一致时，**分数没有可比性**，
        而这件事不会自己暴露 —— 只有白纸黑字记下来，横向对比才站得住。
        `[agents]` 里没配就明确写"未声明"，而不是留空让人以为是默认值。

        **声明的配置不等于实际用的**：agent 因自身配置错误被默认值顶掉时，报告
        照样会印声明的值（静默回退）。所以这里还会从 agent 自己的会话记录里读
        实际模型并与声明比对（`hal/modelcheck.py`）；核不了就如实写"未核实"，
        **绝不当作通过**。
        """
        info = (self.cfg.get("agents", {}) or {}).get(self.agent)
        if not info and self.agent == "cc":
            # 命令行允许 `cli.py run cc ...`，而配置表里那一段叫 [agents.claude]。
            # 不做这层别名，CC 侧的 13 道题会全部静默印成"未声明"。
            info = (self.cfg.get("agents", {}) or {}).get("claude")
        if not info:
            return "**未声明**（config.toml 的 [agents.<名>] 里写上模型与思考强度）"
        model = info.get("model", "?")
        think = info.get("thinking", "?")
        try:
            from hal.modelcheck import verify_model

            chk = verify_model(self.agent, self.workdir, model, think)
        except Exception as exc:  # noqa: BLE001 - 核验失败不该毁掉报告
            chk = {"match": None, "actual": None, "note": str(exc)}
        if chk["match"] is True:
            verdict = f"实际 = `{chk['actual']}` ✓ 一致"
        elif chk["match"] is False:
            verdict = f"**实际 = `{chk['actual']}`，与声明不一致 ⚠**"
        else:
            verdict = f"**未核实**（{chk.get('note') or '会话记录里没有模型信息'}）"
        if chk.get("note") and chk["match"] is True:
            verdict += f"｜{chk['note']}"          # 模型对上了但 thinking 没对上
        return f"model = `{model}`｜thinking = `{think}`｜核验：{verdict}"

    def _write_report_md(self) -> None:
        s = self.outcome.summary()
        lines = [
            f"# {self.spec.id} {self.spec.title} — {self.agent}",
            "",
            f"- 权重 {s['weight']}｜时间盒 {self.spec.timebox_min}min｜本轮耗时 {s['duration_s']}s"
            + ("｜**模拟运行**" if self.sim else ""),
            # **模型与思考强度必须记录在案。** 两个 agent 用不同模型（或不同思考档、
            # 或某个 agent 悄悄回退到默认模型）时，跑出来的分没有任何可比性 ——
            # 而"看起来都跑了同一道题"这件事本身不会暴露差异。
            f"- 被测 agent 配置：{self._agent_banner()}",
            f"- 判分器版本：`{REV}`（版本不同 ⇒ 尺子不同，分数不可直接混排）",
            f"- 血统：{'合规' if s['bloodline_ok'] else ('**未运行**（工作区不存在）' if s['not_run'] else '**违规 → 本题 0 分**')}",
            f"- 机器自动判定得分：**{s['auto_score']} / {s['max_in_scope']}**（满分 {s['max_all']}，"
            f"跳过 {len(s['skipped'])} 项）",
            f"- 待 AI 裁定：{', '.join(s['pending_ai']) or '无'}",
            f"- 机器判红：{', '.join(s.get('machine_fail') or []) or '无'}",
            "",
            "| 点 | 类型 | 满分 | 状态 | 得分 | 依据 |",
            "|---|---|---|---|---|---|",
        ]
        for pid, o in self.outcome.points.items():
            ev = "; ".join(o.machine_evidence[:2]) if o.machine_evidence else (" ".join(o.notes)[:80] if o.notes else "")
            lines.append(f"| {pid} | {o.kind} | {o.max} | {o.status} | {'' if o.score is None else o.score} | {ev} |")
        if self.outcome.warnings:
            lines += ["", "## 警告", ""] + [f"- {w}" for w in self.outcome.warnings]
        if self.outcome.static.get("rules"):
            lines += ["", "## 静态规则", "", "| 规则 | 结论 | 说明 | 证据 |", "|---|---|---|---|"]
            for rid, r in self.outcome.static["rules"].items():
                verdict = {True: "通过", False: "**不通过**", None: "无法判定"}[r["ok"]]
                lines.append(f"| {rid} | {verdict} | {r.get('note') or r.get('want') or ''} | "
                             f"{'; '.join(r.get('evidence') or [])[:80]} |")
        if self.outcome.manual:
            # 单列一节并写明"不是仪器证据"：评委引用它时必须能看出来自人眼。
            lines += ["", "## 人工观测（**不是仪器证据**；评委引用时须注明来源）", "",
                      "| 支撑点 | 问题 | 回答 | 时刻 | 当时串口最近行 |", "|---|---|---|---|---|"]

            def _cell(x: Any, n: int = 46) -> str:
                return str(x).replace("|", "∣").replace("\n", " ")[:n]

            for m in self.outcome.manual:
                ans = m["answer"] or ("**超时未答**" if m["timed_out"] else "**未作答**")
                if m.get("in_choices") is False:
                    ans += "（不在候选里）"
                tail = " ⏎ ".join(m.get("serial_tail") or [])
                lines.append(f"| {m.get('point') or '—'} | {_cell(m['question'])} | {_cell(ans)} | "
                             f"+{m['t_run_s']}s | {_cell(tail, 60)} |")
        lines += ["", "## 执行轨迹", ""]
        for st in self.outcome.steps:
            mark = "跳过" if st.skipped else ("ok" if st.ok else "**失败**")
            lines.append(f"- `{st.op}` {mark}" + (f" — {st.note}" if st.note else ""))
        (self.out / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def _roi(st: dict[str, Any], cfg: Any) -> tuple[int, int, int, int] | None:
    """ROI 优先取步骤自己写的，其次取 config。空列表 = 全帧。"""
    raw = st.get("roi")
    if raw is None:
        raw = cfg.get("camera.roi", []) or []
    if not raw:
        return None
    if len(raw) != 4:
        raise ValueError(f"roi 必须是 [x, y, w, h]，收到 {raw!r}")
    return (int(raw[0]), int(raw[1]), int(raw[2]), int(raw[3]))


def _gen_payload(chars: int, seed: int, line_chars: int = 63) -> str:
    rnd = random.Random(seed)
    alphabet = string.ascii_letters + string.digits + " ,.;:-_()[]{}"
    out: list[str] = []
    remaining = chars
    while remaining > 0:
        n = min(line_chars, remaining)
        out.append("".join(rnd.choice(alphabet) for _ in range(n)))
        remaining -= n
    return "\n".join(out)


def cleanup_workspace(path: Path) -> None:
    """只在显式要求时清工作区 —— 平时绝不碰 agent 的成果。"""
    if path.exists():
        shutil.rmtree(path)
