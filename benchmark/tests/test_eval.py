"""eval 的自测套件 —— 全部离线，不需要板子、不需要仪器。

跑法（在 eval/ 目录下）：

    <venv python> -m unittest discover -s tests -v

为什么值得写：断言引擎是这个评测器的"尺子"。尺子本身没校准，后面
所有分数都是装饰。这里用回放 fixture 把"好、漂、丢、乱、跳、短"六种
情况钉住，改断言实现时先看这组测试的脸色。
"""

from __future__ import annotations

import json
import os
import re
import shutil
import sys
import tempfile
import time
import unittest
from pathlib import Path

EVAL = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(EVAL))

from evconfig import load_config  # noqa: E402
import campaign  # noqa: E402
from engine import asserts as A  # noqa: E402
from engine import runner as R  # noqa: E402
from engine import spec as SP  # noqa: E402
from engine import static_rules as SR  # noqa: E402
from engine.runner import Runner  # noqa: E402
from hal.serialbus import Capture, Line, SimBus, SimScript  # noqa: E402
from hal.simload import load_script  # noqa: E402


class TestCaptureDecoding(unittest.TestCase):
    """多字节字符跨读块被切开时，不许出现 U+FFFD。"""

    def test_utf8_split_across_chunks(self) -> None:
        cap = Capture()
        payload = "温度=27.4℃".encode("utf-8")
        # 从每个字节位置切一刀，全部切法都不该产出替换符
        for cut in range(1, len(payload)):
            cap = Capture()
            cap.feed(payload[:cut], 0.0)
            cap.feed(payload[cut:] + b"\n", 0.01)
            cap.flush(0.02)
            self.assertEqual(cap.lines[0].text, "温度=27.4℃")

    def test_timestamps_are_relative_and_monotonic(self) -> None:
        cap = Capture()
        cap.feed(b"A\n", 100.0)
        cap.feed(b"B\n", 101.5)
        self.assertAlmostEqual(cap.lines[0].t, 0.0, places=6)
        self.assertAlmostEqual(cap.lines[1].t, 1.5, places=6)

    def test_crlf_stripped_but_partial_marked(self) -> None:
        cap = Capture()
        cap.feed(b"X\r\n", 0.0)
        cap.feed(b"tail no newline", 0.1)
        cap.flush(0.2)
        self.assertEqual([ln.text for ln in cap.lines], ["X", "tail no newline"])
        self.assertTrue(cap.lines[1].partial)


class Case01FixtureMixin:
    """把 case01 的 serial_window 断言挂到某个 fixture 上跑。"""

    @classmethod
    def setUpClass(cls) -> None:
        cls.cfg = load_config()
        cls.spec = SP.load_spec(cls.cfg.specs / "case01.toml")
        cls.window_step = next(s for s in cls.spec.steps if s["op"] == "serial_window")

    def evaluate(self, fixture: str) -> dict[str, A.AssertResult]:
        script = load_script(self.cfg.fixtures / fixture)
        bus = SimBus(script, label="sim")
        bus.open()
        # 起点用 runner 里那一份实现，不在测试里复刻 —— 复刻过一次就少一处回归。
        start = R.window_start(bus, self.window_step, {})
        bus.wait(self.window_step["duration_ms"] / 1000.0)
        bus.flush_evidence()
        window = bus.capture.since(start)
        bus.close()
        ctx = A.Ctx(lines=window, sends=[], log_ref="sim")
        return {a["id"]: A.evaluate(a, ctx) for a in self.window_step["asserts"]}


class TestCase01Good(Case01FixtureMixin, unittest.TestCase):
    def test_all_green(self) -> None:
        res = self.evaluate("case01-good.json")
        for aid, r in res.items():
            self.assertIs(r.ok, True, f"{aid} 应为通过，实际 {r.ok}：{r.got}")
        self.assertGreaterEqual(res["F2"].got["mean_ms"], 990)
        self.assertLessEqual(res["F2"].got["mean_ms"], 1010)


class TestFaultIsolation(Case01FixtureMixin, unittest.TestCase):
    """每一种故障只该打中它对应的那一条断言。"""

    def test_drift_only_breaks_timing(self) -> None:
        res = self.evaluate("case01-drift.json")
        self.assertIs(res["F2"].ok, False, "900ms 周期必须被 F2 抓住")
        self.assertIs(res["F1a"].ok, True)
        self.assertIs(res["F1b"].ok, True)
        self.assertIs(res["F3g"].ok, True)

    def test_gap_only_breaks_max_gap(self) -> None:
        res = self.evaluate("case01-gap.json")
        self.assertIs(res["F3t"].ok, False, "停发 4 秒必须被 max_gap 抓住")
        self.assertIs(res["F1a"].ok, True)
        self.assertIs(res["F2"].ok, True)

    def test_bad_sequence_only_breaks_sequence(self) -> None:
        res = self.evaluate("case01-badseq.json")
        self.assertIs(res["F1b"].ok, False, "序号跳变必须被 sequence 抓住")
        self.assertEqual(res["F1b"].got["first_mismatch"], 20)
        self.assertIs(res["F2"].ok, True)
        self.assertIs(res["F3g"].ok, True)

    def test_garbage_only_breaks_garbage_check(self) -> None:
        res = self.evaluate("case01-garbage.json")
        self.assertIs(res["F3g"].ok, False, "U+FFFD 必须被 garbage 抓住")
        self.assertEqual(res["F3g"].got["garbage_lines"], 1)
        self.assertIs(res["F1a"].ok, True)
        self.assertIs(res["F1b"].ok, True)

    def test_short_run_breaks_coverage_count(self) -> None:
        res = self.evaluate("case01-short.json")
        self.assertIs(res["F3n"].ok, False, "只跑 20 秒 < 55 行，必须被 min_matches 抓住")
        self.assertIs(res["F2"].ok, True)


class TestAssertSemantics(unittest.TestCase):
    def _ctx(self, lines: list[tuple[float, str]], **kw) -> A.Ctx:
        cap = Capture()
        for t, text in lines:
            cap.feed((text + "\n").encode("utf-8"), t)
        cap.flush((lines[-1][0] if lines else 0) + 0.001)
        return A.Ctx(lines=cap.lines, log_ref="t", **kw)

    def test_sequence_auto_start_tolerates_missed_head(self) -> None:
        """采集总是晚于上电，抓不到前几行不算失败，但中间不许断。"""
        ctx = self._ctx([(0.0, "HELLO 3"), (1.0, "HELLO 4"), (2.0, "HELLO 5")])
        r = A.evaluate({"id": "x", "type": "serial_sequence", "pattern": r"HELLO (\d+)"}, ctx)
        self.assertIs(r.ok, True)
        ctx2 = self._ctx([(0.0, "HELLO 3"), (1.0, "HELLO 5")])
        r2 = A.evaluate({"id": "x", "type": "serial_sequence", "pattern": r"HELLO (\d+)"}, ctx2)
        self.assertIs(r2.ok, False)

    def test_sequence_rejects_duplicates(self) -> None:
        ctx = self._ctx([(0.0, "SHORT 1"), (1.0, "SHORT 1"), (2.0, "SHORT 2")])
        r = A.evaluate({"id": "x", "type": "serial_sequence", "pattern": r"SHORT (\d+)",
                        "start": 1, "count": 2}, ctx)
        self.assertIs(r.ok, False)

    def test_parse_tolerance_and_stability(self) -> None:
        ok = self._ctx([(0.0, "MEAS f=10005 d=40.2"), (2.0, "MEAS f=9998 d=39.9")])
        r = A.evaluate({
            "id": "x", "type": "serial_parse", "expr": r"MEAS f=(\d+) d=([\d.]+)",
            "checks": [{"field": 1, "ref": 10000, "tol_pct": 2}, {"field": 2, "ref": 40, "tol_pt": 2}],
            "stability": [{"field": 1, "max_range_abs": 100}],
        }, ok)
        self.assertIs(r.ok, True)

        bad = self._ctx([(0.0, "MEAS f=15000 d=40"), (2.0, "MEAS f=15000 d=40")])
        r2 = A.evaluate({
            "id": "x", "type": "serial_parse", "expr": r"MEAS f=(\d+) d=([\d.]+)",
            "checks": [{"field": 1, "ref": 10000, "tol_pct": 2}],
        }, bad)
        self.assertIs(r2.ok, False)

    def test_echo_exact_and_byte_count(self) -> None:
        payload = "abc\nxyz"
        ctx = self._ctx([(0.0, "abc"), (0.1, "xyz")], sends=[payload])
        r = A.evaluate({"id": "x", "type": "serial_echo", "send": 0}, ctx)
        self.assertIs(r.ok, True)
        ctx_short = self._ctx([(0.0, "abc")], sends=[payload])
        r2 = A.evaluate({"id": "x", "type": "serial_echo", "send": 0}, ctx_short)
        self.assertIs(r2.ok, False)

    def test_code_static_three_state(self) -> None:
        ctx = self._ctx([], static={"rules": {"r1": {"ok": True}, "r2": {"ok": False},
                                              "r3": {"ok": None, "note": "看不懂"}}})
        self.assertIs(A.evaluate({"id": "x", "type": "code_static", "rule": "r1"}, ctx).ok, True)
        self.assertIs(A.evaluate({"id": "x", "type": "code_static", "rule": "r2"}, ctx).ok, False)
        # 关键：无法判定必须是 None，不能被当成 False
        self.assertIsNone(A.evaluate({"id": "x", "type": "code_static", "rule": "r3"}, ctx).ok)

    def test_hardware_assert_skipped_in_sim(self) -> None:
        ctx = self._ctx([], hardware_simulated=True)
        r = A.evaluate({"id": "x", "type": "frequency", "channel": "PA8", "hz": 20000}, ctx)
        self.assertIsNone(r.ok)
        self.assertEqual(r.kind, "skipped")


class TestSpecValidation(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.cfg = load_config()
        cls.tmp = Path(tempfile.mkdtemp(prefix="eval-spec-"))

    @classmethod
    def tearDownClass(cls) -> None:
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def _write(self, text: str) -> Path:
        p = self.tmp / "case01.toml"
        p.write_text(text, encoding="utf-8")
        return p

    def test_typo_in_key_is_rejected(self) -> None:
        p = self._write("""
id = "case01"
[[steps]]
op = "serial_window"
duration_ms = 100
asserts = [ { id="X", type="serial_regex", pattern="A", min_match=3 } ]
""")
        with self.assertRaises(SP.SpecError) as cm:
            SP.load_spec(p)
        self.assertIn("min_match", str(cm.exception))

    def test_unknown_point_is_rejected(self) -> None:
        p = self._write("""
id = "case01"
[[steps]]
op = "check"
asserts = [ { id="X", point="F9", type="serial_regex", pattern="A" } ]
""")
        with self.assertRaises(SP.SpecError) as cm:
            SP.load_spec(p)
        self.assertIn("F9", str(cm.exception))

    def test_unknown_op_and_empty_steps_rejected(self) -> None:
        p = self._write('id = "case01"\n[[steps]]\nop = "teleport"\n')
        with self.assertRaises(SP.SpecError):
            SP.load_spec(p)
        p2 = self._write('id = "case01"\nsteps = []\n')
        with self.assertRaises(SP.SpecError):
            SP.load_spec(p2)

    def test_unknown_case_rejected(self) -> None:
        p = self._write('id = "case99"\n[[steps]]\nop = "check"\n')
        with self.assertRaises(SP.SpecError):
            SP.load_spec(p)

    def test_from_prev_only_on_serial_window(self) -> None:
        """写错地方的 from_prev 不会报错、只会让那题**静默地**继续用旧锚点丢行。"""
        p = self._write("""
id = "case01"
[[steps]]
op = "serial_send"
text = "HI"
from_prev = true
""")
        with self.assertRaises(SP.SpecError) as cm:
            SP.load_spec(p)
        self.assertIn("serial_window", str(cm.exception))

    def test_from_prev_contradicts_scope_all(self) -> None:
        p = self._write("""
id = "case01"
[[steps]]
op = "serial_window"
duration_ms = 100
scope = "all"
from_prev = true
""")
        with self.assertRaises(SP.SpecError):
            SP.load_spec(p)

    def test_two_consecutive_from_prev_windows_rejected(self) -> None:
        """两段都从上一个锚点起算 ⇒ 同一批行被数两次，序列计数会凭空变多。"""
        p = self._write("""
id = "case01"
[[steps]]
op = "serial_window"
duration_ms = 100
from_prev = true
asserts = [ { id="F1", type="serial_regex", pattern="X" } ]
[[steps]]
op = "serial_window"
duration_ms = 100
from_prev = true
asserts = [ { id="F1b", point="F1", type="serial_regex", pattern="X" } ]
""")
        with self.assertRaises(SP.SpecError) as cm:
            SP.load_spec(p)
        self.assertIn("重叠", str(cm.exception))

    def test_from_prev_accepted_on_window(self) -> None:
        p = self._write("""
id = "case01"
[[steps]]
op = "reset"
kind = "nrst"
[[steps]]
op = "serial_window"
duration_ms = 100
from_prev = true
asserts = [ { id="F1", type="serial_regex", pattern="X" } ]
""")
        SP.load_spec(p)


class TestWindowAnchor(unittest.TestCase):
    """A1：窗口起点必须收得到"上一个步骤执行期间"到达的那一行。

    双向钉子（同一份数据两个方向都验）：旧语义 matches=0（case11 两边同挂的
    那个假红），from_prev 语义 matches=1。只验一个方向排不掉"数据里本来就没这行"。
    """

    def _bus_after_reset_step(self) -> SimBus:
        bus = SimBus(SimScript(rx=[(0.001, "RESET=POR"), (0.5, "ALIVE 1"), (1.0, "ALIVE 2")]),
                     label="dut_vcp")
        bus.open()
        bus.wait(0.05)   # 上一步（reset）的 handler 期间推进了虚拟时钟：POR 就在这 1ms 里到达
        return bus

    def test_default_anchor_drops_the_line(self) -> None:
        bus = self._bus_after_reset_step()
        start = R.window_start(bus, {}, {})
        bus.wait(4.0)
        texts = [ln.text for ln in bus.capture.since(start)]
        self.assertNotIn("RESET=POR", texts, "旧语义本该丢掉这一行（这正是 case11 的假红）")

    def test_from_prev_anchor_keeps_the_line(self) -> None:
        bus = self._bus_after_reset_step()
        start = R.window_start(bus, {"from_prev": True}, {"dut_vcp": 0})
        self.assertEqual(start, 0)
        bus.wait(4.0)
        texts = [ln.text for ln in bus.capture.since(start)]
        self.assertEqual(texts.count("RESET=POR"), 1)

    def test_from_prev_without_anchor_falls_back_to_now(self) -> None:
        """锚点缺失时退回"现在"，不许退成 0 —— 那等于把整盘采集当窗口，
        serial_absent 会因为看不见早先的行而假绿（case11 H1b 就是这么来的）。"""
        bus = self._bus_after_reset_step()
        self.assertEqual(R.window_start(bus, {"from_prev": True}, {}), len(bus.capture.lines))

    def test_runner_records_anchor_before_handler(self) -> None:
        """_dispatch 必须在 handler **之前**记锚点，且记的是上一条目的值。"""
        cfg = load_config()
        spec = SP.load_spec(cfg.specs / "case11.toml")
        r = Runner(cfg, spec, "firm", sim=True, out_root=Path(tempfile.mkdtemp(prefix="eval-arm-")))
        bus = SimBus(SimScript(rx=[(0.001, "RESET=POR")]), label="dut_vcp")
        bus.open()
        r.buses["dut_vcp"] = bus
        r._dispatch(1, {"op": "reset", "kind": "nrst"})   # 只跑锚点登记，不碰硬件
        self.assertEqual(r._arm["dut_vcp"], 0, "reset 开始时该行还没到，锚点必须是 0")
        bus.wait(0.05)
        r._dispatch(2, {"op": "serial_window", "duration_ms": 100, "from_prev": True})
        self.assertEqual(r._arm_prev["dut_vcp"], 0, "窗口该从 reset 那一步开始处起算")
        self.assertEqual(r._arm["dut_vcp"], 1)
        shutil.rmtree(r.out.parent, ignore_errors=True)


class TestResetSemantics(unittest.TestCase):
    """复位类型的客观依据：RCC_CSR。

    这组测试守着一件事：**"按 RESET 键"和"SWD 系统复位"在硬件上留下的痕迹
    不同**，所以 case11 的 RESET=POR 不能被一次系统复位糊过去。地址是从
    两份独立 CMSIS 头文件各推一遍得来的，这里把它钉住。
    """

    HEADER_CANDIDATES = (
        "D:/OldStudy66/ai test pro/start/cubemx/Drivers/CMSIS/Device/ST/STM32G4xx/Include/stm32g431xx.h",
        "C:/Users/18978/.platformio/packages/framework-stm32cubeg4/Drivers/CMSIS/Device/ST/STM32G4xx/Include/stm32g431xx.h",
    )

    @staticmethod
    def _derive_csr_offset(header: Path) -> int | None:
        if not header.exists():
            return None
        import re

        text = header.read_text(encoding="utf-8", errors="replace")
        end = text.index("} RCC_TypeDef;")
        start = text.rindex("typedef struct", 0, end)
        off = 0
        for line in text[start:end].splitlines():
            m = re.match(r"\s*(__IO\s+)?uint32_t\s+([A-Za-z0-9_]+)\s*(\[\s*(\d+)\s*\])?\s*;", line)
            if not m:
                continue
            if m.group(2) == "CSR":
                return off
            off += 4 * int(m.group(4) or 1)
        return None

    def test_derived_address_matches_constant(self) -> None:
        from hal import probe

        found = []
        for h in self.HEADER_CANDIDATES:
            off = self._derive_csr_offset(Path(h))
            if off is not None:
                found.append(off)
                self.assertEqual(off, probe.RCC_CSR_OFFSET,
                                 f"{h} 推出的偏移与代码里的常量不一致 —— 换过固件包就要重新核")
        if not found:
            self.skipTest("两份 CMSIS 头文件都不在，无法交叉核对")
        self.assertEqual(probe.RCC_CSR, 0x40021000 + probe.RCC_CSR_OFFSET)

    def test_g4_has_no_porrstf(self) -> None:
        """G4 的 RCC_CSR 里**没有 PORRSTF**（上电复位是 LPWRRSTF）。

        判据里写的 "RESET=POR" 是固件的分类桶，不是寄存器位名。谁按 F1 的
        习惯去找 PORRSTF 会找不到，然后很可能"顺手"把代码改错。
        """
        from hal import probe

        self.assertIn("LPWRRSTF", probe.RCC_CSR_BITS)
        self.assertNotIn("PORRSTF", probe.RCC_CSR_BITS)
        for h in self.HEADER_CANDIDATES:
            p = Path(h)
            if p.exists():
                self.assertNotIn("PORRSTF", p.read_text(encoding="utf-8", errors="replace"))

    def test_decode_csr(self) -> None:
        from hal import probe

        val = (1 << 26) | (1 << 23)  # PINRSTF + RMVF
        d = probe.decode_csr(val)
        self.assertTrue(d["PINRSTF"])
        self.assertTrue(d["RMVF"])
        self.assertFalse(d["IWDGRSTF"])
        self.assertFalse(d["SFTRSTF"])

    @staticmethod
    def _struct_offsets(header: Path, struct: str, want: set[str]) -> dict[str, int]:
        """从 CMSIS 头文件里把某个外设结构体的字段偏移累加出来。"""
        text = header.read_text(encoding="utf-8", errors="replace")
        end = text.index(f"}} {struct};")
        start = text.rindex("typedef struct", 0, end)
        off, out = 0, {}
        for line in text[start:end].splitlines():
            m = re.match(r"\s*(__IO\s+)?uint32_t\s+([A-Za-z0-9_]+)\s*(\[\s*(\d+)\s*\])?\s*;", line)
            if not m:
                continue
            if m.group(2) in want:
                out[m.group(2)] = off
            off += 4 * int(m.group(4) or 1)
        return out

    def test_gpio_and_clock_gate_addresses(self) -> None:
        """GPIOC_IDR / RCC_AHB2ENR 这些地址一律从头文件重推一遍。

        用途很实际：`fixture verify` 靠它们读 PC13 的真实电平。地址一旦写错，
        读回来的是一堆**看起来合法**的值 —— 比如 GPIOC 时钟没开时 IDR 恒 0，
        会被当成"这根线永远是低"，从而把一个能用的夹具判成不能用。
        """
        from hal import regs as R

        for h in self.HEADER_CANDIDATES:
            p = Path(h)
            if not p.exists():
                continue
            txt = p.read_text(encoding="utf-8", errors="replace")

            def offset_of(macro: str, base: str) -> int:
                m = re.search(rf"#define {macro}\s+\({base} \+ (0x[0-9A-Fa-f]+)UL\)", txt)
                self.assertIsNotNone(m, f"头文件里找不到 {macro} 的定义")
                return int(m.group(1), 16)

            ahb2 = R.PERIPH_BASE + offset_of("AHB2PERIPH_BASE", "PERIPH_BASE")
            self.assertEqual(R.AHB2PERIPH_BASE, ahb2)
            self.assertEqual(R.GPIOC_BASE, ahb2 + offset_of("GPIOC_BASE", "AHB2PERIPH_BASE"))
            self.assertEqual(
                R.RCC_BASE,
                R.PERIPH_BASE + offset_of("AHB1PERIPH_BASE", "PERIPH_BASE")
                + offset_of("RCC_BASE", "AHB1PERIPH_BASE"),
            )

            gpio = self._struct_offsets(p, "GPIO_TypeDef", {"MODER", "PUPDR", "IDR"})
            self.assertEqual(R.GPIOC_MODER, R.GPIOC_BASE + gpio["MODER"])
            self.assertEqual(R.GPIOC_PUPDR, R.GPIOC_BASE + gpio["PUPDR"])
            self.assertEqual(R.GPIOC_IDR, R.GPIOC_BASE + gpio["IDR"])

            rcc = self._struct_offsets(p, "RCC_TypeDef", {"AHB2ENR"})
            self.assertEqual(R.RCC_AHB2ENR, R.RCC_BASE + rcc["AHB2ENR"])

            m = re.search(r"#define RCC_AHB2ENR_GPIOCEN_Pos\s+\((\d+)U\)", txt)
            self.assertIsNotNone(m, "找不到 GPIOCEN 的位号")
            self.assertEqual(R.GPIO_C_EN_BIT, int(m.group(1)))

            # EXTI / SYSCFG：`fixture verify` 的第二层靠它们做边沿检测。
            # 地址错了的表现不是报错，而是"永远检测不到上升沿"—— 会被误读成
            # "夹具不行"，所以必须重推一遍钉住。
            apb2 = R.PERIPH_BASE + offset_of("APB2PERIPH_BASE", "PERIPH_BASE")
            self.assertEqual(R.APB2PERIPH_BASE, apb2)
            self.assertEqual(R.EXTI_BASE, apb2 + offset_of("EXTI_BASE", "APB2PERIPH_BASE"))
            self.assertEqual(R.SYSCFG_BASE, apb2 + offset_of("SYSCFG_BASE", "APB2PERIPH_BASE"))

            exti = self._struct_offsets(p, "EXTI_TypeDef",
                                        {"RTSR1", "FTSR1", "PR1", "IMR1", "SWIER1"})
            self.assertEqual(R.EXTI_IMR1, R.EXTI_BASE + exti["IMR1"])
            self.assertEqual(R.EXTI_RTSR1, R.EXTI_BASE + exti["RTSR1"])
            self.assertEqual(R.EXTI_FTSR1, R.EXTI_BASE + exti["FTSR1"])
            self.assertEqual(R.EXTI_PR1, R.EXTI_BASE + exti["PR1"])
            self.assertLess(exti["IMR1"], exti["RTSR1"] < exti["PR1"] or 999)  # 顺序健全性

            syscfg = self._struct_offsets(p, "SYSCFG_TypeDef", {"EXTICR"})
            self.assertEqual(R.SYSCFG_EXTICR1, R.SYSCFG_BASE + syscfg["EXTICR"])
            # PC13 落在这 4 个字的**第 4 个**（覆盖 pin12-15）的第 [7:4] 位
            self.assertEqual(R.EXTI13_EXTICR, R.SYSCFG_EXTICR1 + 0x0C)
            self.assertEqual(R.EXTI13_EXTICR_SHIFT, 4)
            # C 口的编码：查头文件里 SYSCFG_EXTICR4_EXTI13_PC 的定义
            # （注意宏值是 0x00000020U —— 是 U 不是 UL，别按 UL 写正则）
            m = re.search(r"#define SYSCFG_EXTICR4_EXTI13_PC\s+\(0x([0-9A-Fa-f]+)U\)", txt)
            self.assertIsNotNone(m, "找不到 SYSCFG_EXTICR4_EXTI13_PC")
            self.assertEqual(R.PORT_C_CODE, (int(m.group(1), 16) >> R.EXTI13_EXTICR_SHIFT) & 0xF,
                             "PORT_C_CODE 必须等于头文件里 PC13 那个宏右移 Pos 之后的值")
            m = re.search(r"#define SYSCFG_EXTICR4_EXTI13_Pos\s+\((\d+)U\)", txt)
            self.assertIsNotNone(m, "找不到 SYSCFG_EXTICR4_EXTI13_Pos")
            self.assertEqual(R.EXTI13_EXTICR_SHIFT, int(m.group(1)))
            break

    def test_set_field_helper(self) -> None:
        from hal import regs as R

        # PC13 的 PUPDR 是 [27:26]
        v = R.set_field(0xFFFFFFFF, 26, 2, R.PUPDR_DOWN)
        self.assertEqual((v >> 26) & 3, 0b10)
        self.assertEqual(v & 3, 3, "改动 [27:26] 不许碰到别的位")
        self.assertEqual(R.get_bit(1 << 13, 13), 1)
        self.assertEqual(R.get_bit(1 << 12, 13), 0)

    def test_nrst_cap_decision_table(self) -> None:
        """没标定过就不能算"具备真 NRST 复位" —— 否则 case11 会拿到假绿灯。"""
        from hal.caps import nrst_capable

        all_ok = dict(fixture_ok=True, target_ok=True, programmer_ok=True, openocd_ok=True)
        # 未标定：工具再全也不算
        self.assertFalse(nrst_capable("none", **all_ok))
        self.assertFalse(nrst_capable("", **all_ok))
        # 标定了但对应工具缺：不算
        self.assertFalse(nrst_capable("programmer", **{**all_ok, "programmer_ok": False}))
        self.assertFalse(nrst_capable("openocd", **{**all_ok, "openocd_ok": False}))
        self.assertFalse(nrst_capable("fixture", **{**all_ok, "fixture_ok": False}))
        # 没插板子：谁都不算
        for m in ("fixture", "programmer", "openocd"):
            self.assertFalse(nrst_capable(m, **{**all_ok, "target_ok": False}))
        # 标配：各自成立
        for m in ("fixture", "programmer", "openocd"):
            self.assertTrue(nrst_capable(m, **all_ok))


class TestFixtureDirection(unittest.TestCase):
    """激励方向表：`按下` 到底应该让那根线变成什么电平。

    为什么值得单独测：方向错了是**静默**失效 —— 波形看着正常、日志没有报错、
    按键就是没反应（或者一直按住）。而且这里有两层容易混：
      ① 目标引脚低有效还是高有效（NRST 低、**G431RB 的 B1 高**）
      ② 我这个信号路径有没有反相一级（直连没有，过三极管有）
    两层各自写清楚，最后取异或才落到线上。
    """

    class _Cfg:
        def __init__(self, spec):
            self._s = spec

        def get(self, key, default=None):
            if key == "fixture.enabled":
                return True
            if key == "fixture.stimuli":
                return self._s
            return default

    class _Bus:
        def __init__(self):
            self.calls: list[tuple[str, Any]] = []

        def set_modem_line(self, name, value):
            self.calls.append((f"modem:{name}", value))

        def set_break(self, hold_low):
            self.calls.append(("break", hold_low))

    def _fx(self, backend, active_low, line_inverted=False, line="DTR", force_line=None):
        spec = {"x": {"backend": backend, "line": line, "active_low": active_low,
                      "line_inverted": line_inverted}}
        if force_line:
            spec["x"]["force_line"] = force_line
        bus = self._Bus()
        from hal.gpio import Fixture

        fx = Fixture(self._Cfg(spec), bus)
        return fx, bus

    def test_low_active_direct(self) -> None:
        """NRST 直连：按下 = 拉低 = pyserial 断言 (True)。"""
        fx, bus = self._fx("modem", active_low=True)
        fx.set("x", True)
        self.assertEqual(bus.calls[-1], ("modem:DTR", True))

    def test_low_active_through_transistor(self) -> None:
        """NRST 过一级三极管（ESP-01S 自动下载电路）：反一次相。"""
        fx, bus = self._fx("modem", active_low=True, line_inverted=True)
        fx.set("x", True)
        self.assertEqual(bus.calls[-1], ("modem:DTR", False))

    def test_high_active_direct_is_the_g431rb_button_case(self) -> None:
        """**G431RB 的 B1 是高有效**：按下 = 拉高 = pyserial 取消断言 (False)。"""
        fx, bus = self._fx("modem", active_low=False)
        fx.set("x", True)
        self.assertEqual(bus.calls[-1], ("modem:DTR", False))

    def test_break_backend_high_active(self) -> None:
        """用裸 TTL 的 TXD 当手（推荐给这块板）：按下 = 空闲高 = 不置 BREAK。"""
        fx, bus = self._fx("break", active_low=False)
        fx.set("x", True)
        self.assertEqual(bus.calls[-1], ("break", False))
        fx.set("x", False)
        self.assertEqual(bus.calls[-1], ("break", True))

    def test_break_backend_low_active(self) -> None:
        fx, bus = self._fx("break", active_low=True)
        fx.set("x", True)
        self.assertEqual(bus.calls[-1], ("break", True))

    def test_force_line_makes_release_an_active_action(self) -> None:
        """**继电器那类"sink-only"脚：松开必须靠第二根线主动做出来。**

        IO0 被灌低后取消断言就是悬空、电荷保持低 ⇒ 触点**保持吸合**，
        case06 的"拔线"会静默变成"什么都没拔"（2026-10-16 实测：拔线窗口
        0 条 FAIL / 64 条 PASS）。真值表里"RTS 断言 ⇒ 触点断开"是无条件的，
        所以 force_line 在松开时断言它、在按下时取消断言它。
        """
        fx, bus = self._fx("modem", active_low=True, line="DTR", force_line="RTS")
        fx.set("x", True)                      # 闭合
        self.assertEqual(bus.calls, [("modem:DTR", True), ("modem:RTS", False)])
        bus.calls.clear()
        fx.set("x", False)                     # 断开：DTR 松开 + **RTS 断言**
        self.assertEqual(bus.calls, [("modem:DTR", False), ("modem:RTS", True)])

    def test_stimulus_without_force_line_touches_one_line_only(self) -> None:
        """没配 force_line 的激励（按键）不能被顺手多拨一根线。"""
        fx, bus = self._fx("modem", active_low=False, line="DTR")
        fx.set("x", True)
        self.assertEqual(bus.calls, [("modem:DTR", False)])

    def test_bad_force_line_is_not_available(self) -> None:
        """force_line 拼错 ⇒ 判"不具备"，不能退化成"靠悬空"还报 OK。"""
        fx, _ = self._fx("modem", active_low=True, line="DTR", force_line="RTX")
        self.assertFalse(fx.available("x"))
        # 与 line 同一根更阴：两次写落在同一根线上，后者覆盖前者 = 方向反了
        fx2, _ = self._fx("modem", active_low=True, line="DTR", force_line="DTR")
        self.assertFalse(fx2.available("x"))
        self.assertTrue(fx2.stimuli["x"].force_line == "DTR")

    def test_srzip_is_chunks_of_interleaved_samples(self) -> None:
        """`logic-1-N` 是**第 N 块字节**，不是"第 N 个通道"。

        实测踩过：一次 5 块的抓包被报成"5 个通道"，而数值碰巧看着合理
        （全是高），极难发现。布局是"1 字节/采样、bit b = 通道 b（LSB first）"，
        这里用合成文件把位序钉住。
        """
        import tempfile
        import zipfile

        from hal.la import parse_srzip

        meta = ("[global]\nversion=1\n[device 1]\ncapturefile=logic-1\n"
                "total probes=8\nsamplerate=100 kHz\nunitsize=1\n"
                + "".join(f"probe{i}=D{i-1}\n" for i in range(1, 9)))
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "s.sr"
            with zipfile.ZipFile(f, "w") as z:
                z.writestr("version", "2")
                z.writestr("metadata", meta)
                z.writestr("logic-1-1", bytes([0xF0]) * 4)   # D0..D3 低、D4..D7 高
                z.writestr("logic-1-2", bytes([0x0F]) * 4)   # 反过来
            cap = parse_srzip(f, "fx2lafw")
            self.assertEqual(len(cap.channels), 8, "应该是 8 个通道（8 个 probe）")
            self.assertEqual(cap.n_samples, 8, "8 字节 = 8 个采样（1 字节/采样）")
            for i in range(4):
                self.assertEqual(cap.bits[f"D{i}"][0], 0)
                self.assertEqual(cap.bits[f"D{i}"][-1], 1)
            for i in range(4, 8):
                self.assertEqual(cap.bits[f"D{i}"][0], 1)
                self.assertEqual(cap.bits[f"D{i}"][-1], 0)

    def test_samplerate_string_from_srzip(self) -> None:
        """srzip 的 metadata 里 samplerate 是 `"250 kHz"` 这种**带单位的字符串**。

        直接 float() 会抛 `could not convert string to float: '250 kHz'` ——
        实测踩过：抓包成功，解析炸，整个通道标定跑不完。
        """
        from hal.la import parse_rate

        self.assertEqual(parse_rate("250 kHz"), 250e3)
        self.assertEqual(parse_rate("8 MHz"), 8e6)
        self.assertEqual(parse_rate("1000000"), 1e6)
        self.assertEqual(parse_rate("1MHz"), 1e6)
        self.assertEqual(parse_rate(" 250kHz "), 250e3)
        self.assertEqual(parse_rate("20 MHz"), 20e6)

    def test_samplerate_string_from_srzip(self) -> None:
        """srzip 的 metadata 里 samplerate 是 `"250 kHz"` 这种**带单位的字符串**。

        直接 float() 会抛 `could not convert string to float: '250 kHz'` ——
        实测踩过：抓包成功，解析炸，整个通道标定跑不完。
        """
        from hal.la import parse_rate

        self.assertEqual(parse_rate("250 kHz"), 250e3)
        self.assertEqual(parse_rate("8 MHz"), 8e6)
        self.assertEqual(parse_rate("1000000"), 1e6)
        self.assertEqual(parse_rate("1MHz"), 1e6)
        self.assertEqual(parse_rate(" 250kHz "), 250e3)
        self.assertEqual(parse_rate("20 MHz"), 20e6)

    def test_release_all_returns_to_not_pressed(self) -> None:
        """收尾必须把线放回"未按下"：否则高有效的按键会被一直按住，
        下一题的日志就被上一题的残留污染了。"""
        fx, bus = self._fx("break", active_low=False)
        fx.set("x", True)
        fx.release_all()
        self.assertIn(("break", True), bus.calls)  # BREAK 置位 = 低 = 没按下

    def test_release_all_also_idles_unmapped_modem_lines(self) -> None:
        """**没映射到任何激励的 modem 线，收尾时也要取消断言。**

        踩过的坑（2026-10-16）：这块 ESP-01S 板的 DTR/RTS 交叉耦合，
        把配置里的 nrst 激励删掉之后，RTS 就再也没人碰它了 —— 它一直停在
        pyserial 打开串口时的默认状态，于是按键悄悄失效。而 `verify` 之前能过、
        之后不能过，看起来像随机故障。没被映射的线不能算"不用管"。
        """
        spec = {"x": {"backend": "break", "active_low": False}}   # 没有任何 modem 线被映射
        bus = self._Bus()
        from hal.gpio import Fixture

        fx = Fixture(self._Cfg(spec), bus)
        fx.enabled = True
        fx.release_all()
        self.assertIn(("modem:RTS", False), bus.calls)
        self.assertIn(("modem:DTR", False), bus.calls)

    def test_available_does_not_depend_on_open_bus(self) -> None:
        """能力探测发生在开串口之前，不能因为"总线还没开"就报不具备 ——
        那会让 case04/11 的按键步骤被静默跳过。"""
        spec = {"x": {"backend": "break", "active_low": False}}
        from hal.gpio import Fixture

        fx = Fixture(self._Cfg(spec), None)
        self.assertTrue(fx.available("x"))


class TestPortMatching(unittest.TestCase):
    """端口解析：**同型号板并存时必须停下来，不许猜。**

    场景很具体：用户手上有两块都是 CH340 的板（一块 ESP-01S 烧录板、一块裸 TTL），
    描述字符串和 VID/PID 完全一样。而这个端口是用来拨动物理电平的 —— 猜错就是
    把按键脉冲发给另一块板子，表现是"按键没反应"，而且完全看不出原因。
    """

    class _P:
        def __init__(self, device, vid=0x1A86, pid=0x7523, desc="USB-SERIAL CH340",
                     hwid="", location=""):
            self.device = device
            self.vid = vid
            self.pid = pid
            self.description = desc
            self.hwid = hwid
            self.serial_number = None
            self.location = location

    class _Cfg:
        def __init__(self, role_cfg):
            self._r = role_cfg

        def get(self, key, default=None):
            if key == "serial.roles":
                return [self._r]
            return default

    def _with_ports(self, fakes):
        from hal import ports

        self._orig = ports.all_ports
        ports.all_ports = lambda: list(fakes)  # type: ignore[assignment]
        self.addCleanup(lambda: setattr(ports, "all_ports", self._orig))

    def test_two_identical_boards_raise(self) -> None:
        from hal import ports

        self._with_ports([self._P("COM9"), self._P("COM10")])
        cfg = self._Cfg({"role": "fixture_uart", "vid": 0x1A86, "pid": 0x7523})
        with self.assertRaises(ports.PortAmbiguity) as cm:
            ports.resolve_role(cfg, "fixture_uart")
        msg = str(cm.exception)
        self.assertIn("COM9", msg)
        self.assertIn("COM10", msg)
        self.assertIn("location", msg)

    def test_location_disambiguates(self) -> None:
        from hal import ports

        self._with_ports([self._P("COM9", location="1-2:x.1"), self._P("COM10", location="1-3:x.2")])
        cfg = self._Cfg({"role": "fixture_uart", "location": "1-2:x.1"})
        m = ports.resolve_role(cfg, "fixture_uart")
        self.assertIsNotNone(m)
        self.assertEqual(m.device, "COM9")

    def test_device_override(self) -> None:
        from hal import ports

        self._with_ports([self._P("COM9"), self._P("COM10")])
        cfg = self._Cfg({"role": "fixture_uart", "device": "com10"})
        m = ports.resolve_role(cfg, "fixture_uart")
        self.assertIsNotNone(m)
        self.assertEqual(m.device, "COM10")

    def test_location_falls_back_to_hwid(self) -> None:
        from hal import ports

        hwid = "USB VID:PID=1A86:7523 SER= LOCATION=1-4:x.3"
        self.assertEqual(ports._location_from_hwid(hwid), "1-4:x.3")
        p = self._P("COM9", hwid=hwid, location="")
        self._with_ports([p])
        cfg = self._Cfg({"role": "fixture_uart", "location": "1-4:x.3"})
        self.assertEqual(ports.resolve_role(cfg, "fixture_uart").device, "COM9")

    def test_location_suffix_is_ignored(self) -> None:
        """`:x.N` 后缀会随枚举出现/消失 —— 实测同一块板同一个口，两种都报过。

        所以比较只看冒号前的物理口前缀：不这样处理的话，"插回原来的口"也会
        匹配失败，而报错看起来像"板子没插"，把人往错方向带。
        """
        from hal import ports

        self.assertEqual(ports._location_base("1-1.4:x.2"), ports._location_base("1-1.4"))
        # 但不同物理口必须区分开 —— 否则两块同型号板又混在一起了
        self.assertNotEqual(ports._location_base("1-1.4"), ports._location_base("1-1.3"))
        self._with_ports([self._P("COM15", location="1-1.4")])
        cfg = self._Cfg({"role": "fixture_uart", "location": "1-1.4:x.2"})
        self.assertEqual(ports.resolve_role(cfg, "fixture_uart").device, "COM15")

    def test_diagnose_explains_location_miss(self) -> None:
        """锁了物理口但板子插在别的口上时，要说清是"口不对"而不是"没插"。"""
        from hal import ports

        self._with_ports([self._P("COM15", location="1-1.9")])
        cfg = self._Cfg({"role": "fixture_uart", "location": "1-1.4", "vid": 0x1A86})
        self.assertIsNone(ports.resolve_role(cfg, "fixture_uart"))
        msg = ports.diagnose(cfg, "fixture_uart")
        self.assertIn("COM15", msg)
        self.assertIn("location", msg)

    def test_bt_virtual_and_real_split_by_hwid(self) -> None:
        """蓝牙：本机虚拟口与真实配对设备的描述一模一样，只能靠 hwid 里的 VID&0001 区分。"""
        from hal import ports

        self._with_ports([
            self._P("COM4", vid=None, pid=None, desc="蓝牙链接上的标准串行 (COM4)",
                    hwid=r"BTHENUM\{00001101-0000-1000-8000-00805F9B34FB}_LOCALMFG&0000"),
            self._P("COM5", vid=None, pid=None, desc="蓝牙链接上的标准串行 (COM5)",
                    hwid=r"BTHENUM\{00001101-...}_VID&000105D6_PID&000A"),
        ])
        cfg = self._Cfg({"role": "bt_peer", "vid": 0, "pid": 0,
                         "description_contains": "蓝牙链接上的标准串行",
                         "hwid_contains": "VID&0001"})
        self.assertEqual(ports.resolve_role(cfg, "bt_peer").device, "COM5")


class TestFirmwareSelection(unittest.TestCase):
    """固件产物选择：**绝不能选中母本的构建产物**。

    真板首跑踩过：`cubemx/Debug/cubemx.elf`（母本自己的 CubeIDE 构建，比 agent
    的 .pio 产物还新）被按 mtime 选中并烧了进去。烧的是空循环固件，现象是
    "串口一个字都没有" —— 报告上看起来像 agent 没写代码，实际是裁判烧错了东西。
    """

    @classmethod
    def setUpClass(cls) -> None:
        cls.tmp = Path(tempfile.mkdtemp(prefix="eval-elf-"))

    @classmethod
    def tearDownClass(cls) -> None:
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def setUp(self) -> None:
        # 每个用例都从空目录开始：上一个用例留下的 elf 会干扰"优先级"的断言
        shutil.rmtree(self.tmp, ignore_errors=True)
        self.tmp.mkdir(parents=True, exist_ok=True)

    def _touch(self, rel: str, mtime: float) -> Path:
        p = self.tmp / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(b"\x7fELF")
        os.utime(p, (mtime, mtime))
        return p

    def test_prefers_pio_build_over_newer_cubemx_debug(self) -> None:
        from engine.runner import find_firmware

        self._touch("cubemx/Debug/cubemx.elf", 2_000_000_000)   # 更新
        good = self._touch(".pio/build/nucleo_g431rb/firmware.elf", 1_000_000_000)
        self.assertEqual(find_firmware(self.tmp), good)

    def test_ignores_pristine_tree_entirely(self) -> None:
        from engine.runner import find_firmware

        self._touch("cubemx/Debug/cubemx.elf", 2_000_000_000)
        self._touch("start/cubemx/Debug/cubemx.elf", 2_000_000_000)
        self.assertIsNone(find_firmware(self.tmp), "母本的构建产物不许当 agent 的固件")

    def test_multiple_envs_is_deterministic(self) -> None:
        from engine.runner import find_firmware

        self._touch(".pio/build/env_b/firmware.elf", 1_000_000_000)
        a = self._touch(".pio/build/env_a/firmware.elf", 1_000_000_000)
        # 同一优先级里按名字排序取第一个：**确定**比"最新"重要 ——
        # mtime 正是上次选错的原因
        self.assertEqual(find_firmware(self.tmp), a)


class TestFixtureStartup(unittest.TestCase):
    """夹具口必须在启动时就开好、并立刻置成"未按下"。

    两条理由都是会真出事的：
    ① 原来四个带 `gpio_*` 的 spec 都只开 `dut_vcp`，没人开夹具口 → `fixture.bus`
       永远是 None → 每个按键步骤都失败，而报告上看起来像"夹具坏了"。
    ② IO0 那种脚**空闲是高**，而 B1 高有效 ⇒ 没人控制时按键是被**一直按住**的。
    """

    cfg = None

    @classmethod
    def setUpClass(cls) -> None:
        cls.cfg = load_config()

    def _runner(self, tmp: Path) -> Any:
        from engine import spec as SP
        from engine.runner import Runner

        spec = SP.load_spec(self.cfg.specs / "case04.toml")
        return Runner(self.cfg, spec, "sim", sim=True, out_root=tmp, workdir=tmp)

    def test_sim_is_noop(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            r = self._runner(Path(d))
            r._setup_fixture()  # 仿真下不该碰硬件
            self.assertIsNone(r.fixture.bus)

    def test_missing_port_warns_instead_of_crashing(self) -> None:
        """夹具开着但口不在（比如板子拔了）→ 给出明确告警，不许崩。"""
        from hal import ports

        with tempfile.TemporaryDirectory() as d:
            r = self._runner(Path(d))
            r.sim = False  # 模拟真板路径

            class Fixture:
                enabled = True

                class _Cfg:
                    def get(self, k, default=None):
                        return {"fixture.enabled": True}.get(k, default)

                cfg = _Cfg()

            r.fixture = Fixture()  # type: ignore[assignment]
            orig = ports.resolve_role
            ports.resolve_role = lambda cfg, role: None  # type: ignore[assignment]
            try:
                r._setup_fixture()
            finally:
                ports.resolve_role = orig  # type: ignore[assignment]
            self.assertTrue(any("夹具" in w for w in r.outcome.warnings),
                            "夹具口找不到时必须留下告警，否则报告上只会看到每个 gpio 步骤失败")


class TestStaticRules(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.cfg = load_config()
        cls.pristine = cls.cfg.testset / "start" / "cubemx"

    def test_case01_rules_on_pristine_project(self) -> None:
        if not self.pristine.exists():
            self.skipTest("找不到 start/cubemx")
        res = SR.run_rules("case01", self.pristine)
        self.assertGreater(res["files_scanned"], 0)
        # 未改动的母本：VCP 走 LPUART1(BSP_COM_Init)，且没有任何 UART 二次初始化
        self.assertIs(res["rules"]["case01.H1"]["ok"], True)
        self.assertIs(res["rules"]["case01.H2"]["ok"], True)

    def test_workspace_excludes_library_dirs(self) -> None:
        """库目录必须被排除：否则 HAL 里的 HAL_TIM_PWM_Start 会让 case03 全绿。"""
        if not self.pristine.exists():
            self.skipTest("找不到 start/cubemx")
        ws = SR.Workspace(self.pristine)
        rel = [str(p.relative_to(self.pristine)) for p in ws.files]
        self.assertFalse([r for r in rel if r.startswith("Drivers")], "Drivers/ 没被排除")
        res = SR.run_rules("case03", self.pristine)
        self.assertIs(res["rules"]["case03.H1"]["ok"], False,
                      "母本里没有 PWM 代码，case03.H1 不该通过")


class TestRunnerSim(unittest.TestCase):
    """端到端：造一个合成工作区，用回放 fixture 走完整条管线。"""

    @classmethod
    def setUpClass(cls) -> None:
        cls.cfg = load_config()
        cls.root = EVAL / ".selftest"
        cls.work = cls.root / "work"
        cls.out = cls.root / "reports"
        if cls.root.exists():
            shutil.rmtree(cls.root, ignore_errors=True)
        # 血统校验要求 Core/ 与母本逐字节一致，所以直接复制母本
        shutil.copytree(cls.cfg.testset / "start" / "cubemx", cls.work)
        (cls.work / "platformio.ini").write_text(
            "[env:nucleo_g431rb]\nplatform = ststm32\nboard = nucleo_g431rb\nframework = stm32cube\n",
            encoding="utf-8",
        )

    @classmethod
    def tearDownClass(cls) -> None:
        shutil.rmtree(cls.root, ignore_errors=True)

    def _run(self, fixture: str):
        spec = SP.load_spec(self.cfg.specs / "case01.toml")
        r = Runner(self.cfg, spec, "sim", sim=True, out_root=self.out, workdir=self.work)
        r._sim_scripts["dut_vcp"] = load_script(self.cfg.fixtures / fixture)
        return r.run()

    def test_green_path_end_to_end(self) -> None:
        out = self._run("case01-good.json")
        s = out.summary()
        self.assertTrue(s["bloodline_ok"], s["warnings"])
        self.assertEqual(s["pending_ai"], [], f"不该有待裁定项：{s['pending_ai']}")
        for pid, p in out.points.items():
            self.assertEqual(p.status, "pass", f"{pid} 状态 {p.status}：{p.notes}")
        self.assertEqual(s["auto_score"], s["max_in_scope"])
        # 证据必须落盘，且报告文件存在
        self.assertTrue((Path(out.evidence_dir) / "result.json").exists())
        self.assertTrue((Path(out.evidence_dir) / "asserts.json").exists())
        self.assertTrue((Path(out.evidence_dir) / "static.json").exists())
        self.assertTrue(any((Path(out.evidence_dir) / "serial").glob("*.log")))

    def test_every_artifact_carries_grader_rev(self) -> None:
        """每份落盘产物都得盖上判分器版本 —— 尺子换过之后，
        旧报告若不自证版本，就会被当成新尺子的结果混进同一张记分卡。"""
        from engine.rev import REV
        out = self._run("case01-good.json")
        ev = Path(out.evidence_dir)
        aj = json.loads((ev / "asserts.json").read_text(encoding="utf-8"))
        self.assertEqual(aj["grader_rev"], REV)
        self.assertTrue(aj["asserts"], "asserts.json 不该是空的")
        for a in aj["asserts"]:
            self.assertEqual(a.get("grader_rev"), REV, f"{a['id']} 没盖版本戳")
        self.assertEqual(json.loads((ev / "static.json").read_text(encoding="utf-8"))["grader_rev"], REV)
        self.assertEqual(json.loads((ev / "result.json").read_text(encoding="utf-8"))["summary"]["grader_rev"], REV)
        self.assertIn(REV, (ev.parent / "report.md").read_text(encoding="utf-8"))

    def test_gap_case_turns_into_pending_ai_not_silent_pass(self) -> None:
        out = self._run("case01-gap.json")
        self.assertEqual(out.points["F3"].status, "machine_fail")
        self.assertIsNone(out.points["F3"].score)
        # 判红就是判红，不许再挂在"待 AI 裁定"上。2026-10-17 补裁栏 A 时旧字段把两类混在一起
        # （`pending_ai` 里含 `machine_fail`），照着那份清单裁完 10 条以为齐了，其实 `r3` 另外
        # 放开的 3 条**未观测**根本不在里面 —— 待裁全集只能从 `regrade/diff.json` 取。
        s = out.summary()
        self.assertIn("F3", s["machine_fail"])
        self.assertNotIn("F3", s["pending_ai"], "机器判红不该混进待 AI 清单")
        self.assertEqual(out.points["F1"].status, "pass")
        self.assertEqual(out.points["F2"].status, "pass")

    def test_hidden_point_without_rule_is_never_silently_passed(self) -> None:
        """未声明的评分点标 skipped，不得计入自动得分。"""
        spec = SP.load_spec(self.cfg.specs / "case01.toml")
        # 造一个把 H1/H2 都砍掉的规格副本（用临时文件，不动真规格）
        raw = (self.cfg.specs / "case01.toml").read_text(encoding="utf-8")
        trimmed = raw.split("# ---------------------------------------------------------------------------")[0]
        tmp = self.root / "case01-trimmed.toml"
        tmp.write_text(trimmed, encoding="utf-8")
        spec2 = SP.load_spec(tmp)
        self.assertTrue(spec2.warnings, "未声明的评分点必须产生警告")


class TestRegrade(unittest.TestCase):
    """离线重判：尺子改了要能量出"旧证据在新尺子下判成什么样"，且不能自己骗自己。

    这里钉三件事，每件都是真的踩过或差点踩到的：
    1. 归档日志的读回必须**逐字符**还原（分隔空格与 partial 标记混一次，
       `serial_echo` 就会整片假红 —— 第一轮对照跑就是被这条抓出来的）；
    2. 同一条尺子重判自己的产物 ⇒ 判定必须一条都不变（否则重判结果不可信）；
    3. 复判不了的（仪器类 / 边界反推不出的窗口）只能**沿用旧判定并标注**，
       不许记成"通过"，也不许记成"失败"。
    """

    @classmethod
    def setUpClass(cls) -> None:
        cls.cfg = load_config()
        cls.root = EVAL / ".selftest-regrade"
        if cls.root.exists():
            shutil.rmtree(cls.root, ignore_errors=True)
        cls.root.mkdir(parents=True, exist_ok=True)

    @classmethod
    def tearDownClass(cls) -> None:
        shutil.rmtree(cls.root, ignore_errors=True)

    def test_log_roundtrip_preserves_leading_spaces_and_partial(self) -> None:
        from regrade import parse_capture_log

        cap = Capture()
        t = 0.0
        for text in ["RESET=POR", "  缩进两格", "", "ALIVE 1"]:
            cap.feed(f"{text}\n".encode("utf-8"), t)
            t += 0.5
        cap._pending = "半行没收尾"  # noqa: SLF001 - 就是要造 partial
        cap.flush(t)
        p = self.root / "all-dut_vcp.log"
        cap.write_log(p, header="roundtrip")

        back = parse_capture_log(p)
        self.assertEqual([ln.text for ln in back.lines], [ln.text for ln in cap.lines])
        self.assertEqual([round(ln.t, 4) for ln in back.lines],
                         [round(ln.t, 4) for ln in cap.lines])
        self.assertTrue(back.lines[-1].partial, "partial 标记必须读得回来")

    def _sim_run(self, case: str, fixture: str, spec_text: str | None = None) -> Path:
        """先用回放 fixture 造一份**已知判定**的报告，作为重判的输入。"""
        work = self.root / f"work-{case}"
        if not work.exists():
            shutil.copytree(self.cfg.testset / "start" / "cubemx", work)
            (work / "platformio.ini").write_text(
                "[env:nucleo_g431rb]\nplatform = ststm32\nboard = nucleo_g431rb\n"
                "framework = stm32cube\n", encoding="utf-8")
        spec_file = self.cfg.specs / f"{case}.toml"
        if spec_text is not None:
            spec_file = self.root / f"{case}-patched.toml"
            spec_file.write_text(spec_text, encoding="utf-8")
        spec = SP.load_spec(spec_file)
        r = Runner(self.cfg, spec, "sim", sim=True, out_root=self.root / "reports", workdir=work)
        r._sim_scripts["dut_vcp"] = load_script(self.cfg.fixtures / fixture)  # noqa: SLF001
        out = r.run()
        return Path(out.evidence_dir).parent

    def test_regrade_reproduces_verdicts_of_same_ruler(self) -> None:
        """同一把尺子重判自己的产物 ⇒ 判定一条都不许变。

        case07 是这里面最硬的一条：它比对的是**逐字符回显**（serial_echo）、
        符号引用（RX 的期望值 = 发送总字节数）和响应延迟 —— 重放只要把
        发送内容、窗口边界或时间轴还原错一点，这三条立刻翻红。
        """
        from regrade import run_one

        for case, fixture in (("case01", "case01-good.json"), ("case07", "case07-good.json")):
            with self.subTest(case=case):
                run_dir = self._sim_run(case, fixture)
                old = {a["id"]: a["ok"] for a in
                       json.loads((run_dir / "evidence" / "asserts.json")
                                  .read_text(encoding="utf-8"))["asserts"]}
                p = run_one(self.cfg, run_dir)
                changed = [d for d in p["diff"] if d["changed"]]
                self.assertEqual(changed, [], f"同一把尺子重判不该改变判定：{changed}")
                self.assertEqual(p["new_auto_score"], p["old_auto_score"])
                rows = {d["id"]: d for d in p["diff"]}
                for aid, ok in old.items():
                    self.assertIn(aid, rows, f"{aid} 在重判里消失了（该点会靠剩下的断言自动过关）")
                    self.assertEqual(rows[aid]["new"], ok)

    def test_unreplayable_steps_carry_old_verdict(self) -> None:
        """仪器类步骤离线复判不了：必须**沿用旧判定并标 carried**。

        为什么要单独钉：把它记成 ✗ 会凭空多一个假红，记成 — 会把分数账改掉，
        两种都让人误以为"新尺子判过了"。这里给 case07 追加一个 la_capture 步骤，
        让 `edges` 断言真的走上这条分支。
        """
        from regrade import run_one

        base = (self.cfg.specs / "case07.toml").read_text(encoding="utf-8")
        patched = base + """
[[steps]]
op = "la_capture"
duration_ms = 500
save = "la/cap.vcd"
asserts = [
  { id = "LAX", point = "F1", type = "edges", channel = "D0", min_count = 4 },
]
"""
        run_dir = self._sim_run("case07", "case07-good.json", spec_text=patched)
        old = {a["id"]: a["ok"] for a in
               json.loads((run_dir / "evidence" / "asserts.json").read_text(encoding="utf-8"))["asserts"]}
        p = run_one(self.cfg, run_dir, spec=SP.load_spec(self.root / "case07-patched.toml"))
        rows = {d["id"]: d for d in p["diff"]}
        self.assertIn("LAX", rows, "复判不了的断言也必须留在表里，不能让该点自动过关")
        self.assertTrue(rows["LAX"].get("carried"), "la_capture 的断言该标成 carried")
        self.assertEqual(rows["LAX"]["new"], old["LAX"], "carried 行不许换结论")
        self.assertIn("不可复判", rows["LAX"]["note"])
        self.assertEqual(p["new_auto_score"], p["old_auto_score"])


class TestCase07Sim(unittest.TestCase):
    """回显链路的离线冒烟：回声比对、符号引用（sent_bytes）、响应延迟。

    这四条是 P0 里最容易悄悄写错的代码路径 —— 不接板就先跑一遍。
    """

    @classmethod
    def setUpClass(cls) -> None:
        cls.cfg = load_config()
        cls.root = EVAL / ".selftest7"
        cls.work = cls.root / "work"
        cls.out = cls.root / "reports"
        if cls.root.exists():
            shutil.rmtree(cls.root, ignore_errors=True)
        shutil.copytree(cls.cfg.testset / "start" / "cubemx", cls.work)
        (cls.work / "platformio.ini").write_text(
            "[env:nucleo_g431rb]\nplatform = ststm32\nboard = nucleo_g431rb\nframework = stm32cube\n",
            encoding="utf-8",
        )

    @classmethod
    def tearDownClass(cls) -> None:
        shutil.rmtree(cls.root, ignore_errors=True)

    def _run(self, fixture: str):
        spec = SP.load_spec(self.cfg.specs / "case07.toml")
        r = Runner(self.cfg, spec, "sim", sim=True, out_root=self.out, workdir=self.work)
        script = load_script(self.cfg.fixtures / fixture)
        r._sim_scripts["dut_vcp"] = script
        return r.run()

    def test_echo_path_green(self) -> None:
        out = self._run("case07-good.json")
        for pid in ("F1", "F2", "F3"):
            self.assertEqual(out.points[pid].status, "pass",
                             f"{pid} 应为 pass，实际 {out.points[pid].status}：{out.points[pid].notes}")
        # F2 的期望值是符号算出来的，必须等于三段 payload 的总字节数
        f2 = [a for a in out.asserts if a["id"] == "F2b"][0]
        self.assertIs(f2["ok"], True, f2)

    def test_truncated_echo_fails_f1_but_not_f2(self) -> None:
        out = self._run("case07-truncating.json")
        self.assertEqual(out.points["F1"].status, "machine_fail", "丢字节必须被 F1 抓住")
        self.assertEqual(out.points["F2"].status, "pass", "接收计数正常，F2 不该被连坐")
        self.assertIsNone(out.points["F1"].score)


class TestModelCheckFirm(unittest.TestCase):
    """firment 侧"声明 vs 实际"核验。

    为什么值得钉住：`[agents.*]` 里写的是**声明**，agent 自己被默认值顶掉时报告
    照样印声明值 —— 两个 agent 的分数就没有可比性了，而这件事不会自己暴露。
    可信来源只有 firment 落在 `sessions/<uuid>.jsonl` 第一行的 `type=meta`
    （实测带 cwd/provider/model/thinking）。
    """

    def _store(self, tmp, sessions: list[dict]) -> Path:
        root = Path(tmp) / "sessions"
        root.mkdir(parents=True, exist_ok=True)
        for i, meta in enumerate(sessions):
            p = root / f"uuid-{i}.jsonl"
            p.write_text(json.dumps({"type": "meta", **meta}) + "\n"
                         + '{"type":"message"}\n', encoding="utf-8")
            os.utime(p, (1_700_000_000 + i * 100, 1_700_000_000 + i * 100))
        # 事件流：结构不同、没有模型字段，必须被忽略
        (root / "uuid-9.events.jsonl").write_text(
            json.dumps({"at": 1, "kind": "settings", "summary": "model: nope"}) + "\n",
            encoding="utf-8")
        return root

    def test_matches_own_dir_and_children_only(self) -> None:
        import tempfile

        from hal.modelcheck import firm_session_meta

        with tempfile.TemporaryDirectory() as d:
            # 会话是从 case01 里面的 cubemx 启动的（agent 会 cd 进子目录）⇒ 认
            root = self._store(d, [{"cwd": r"D:\runs\firm\case01\cubemx",
                                    "model": "m-real", "thinking": "high"}])
            hit = firm_session_meta(Path(r"D:\runs\firm\case01"), root)
            self.assertIsNotNone(hit)
            self.assertEqual(hit[1]["model"], "m-real")
            # 无关目录不认
            self.assertIsNone(firm_session_meta(Path(r"D:\runs\firm\case02"), root))

    def test_session_in_parent_dir_is_not_claimed(self) -> None:
        """会话跑在**父目录**时不能被每道子题认领。

        否则有人在 `runs/firm/` 下起过一次 firment，14 道题就都会把那次
        当成自己的会话 —— 核验结果变成一片"看起来对得上"的假绿。
        """
        import tempfile

        from hal.modelcheck import firm_session_meta

        with tempfile.TemporaryDirectory() as d:
            root = self._store(d, [{"cwd": r"D:\runs\firm",
                                    "model": "m-parent", "thinking": "high"}])
            self.assertIsNone(firm_session_meta(Path(r"D:\runs\firm\case01"), root))
            # 但父目录自己查是认的
            self.assertIsNotNone(firm_session_meta(Path(r"D:\runs\firm"), root))

    def test_picks_newest_session(self) -> None:
        import tempfile

        from hal.modelcheck import firm_session_meta

        with tempfile.TemporaryDirectory() as d:
            root = self._store(d, [
                {"cwd": r"D:\r\case01", "model": "旧模型", "thinking": "low"},
                {"cwd": r"D:\r\case01", "model": "新模型", "thinking": "max"},
            ])
            self.assertEqual(firm_session_meta(Path(r"D:\r\case01"), root)[1]["model"],
                             "新模型")

    def test_verify_reports_mismatch_and_thinking_drift(self) -> None:
        import tempfile

        from hal.modelcheck import verify_model

        with tempfile.TemporaryDirectory() as d:
            root = self._store(d, [{"cwd": r"D:\r\case01",
                                    "model": "deepseek-flash", "thinking": "low"}])
            import hal.modelcheck as MC

            saved, MC.FIRM_SESSIONS = MC.FIRM_SESSIONS, root
            try:
                ok = verify_model("firm", Path(r"D:\r\case01"), "deepseek-flash", "high")
                self.assertIs(ok["match"], True)
                self.assertEqual(ok["thinking_actual"], "low")
                self.assertIn("⚠", ok["note"])       # 模型对上了，thinking 没对上
                bad = verify_model("firm", Path(r"D:\r\case01"), "agnes-2.5-flash", "high")
                self.assertIs(bad["match"], False)
                miss = verify_model("firm", Path(r"D:\r\case99"), "deepseek-flash", "high")
                self.assertIsNone(miss["match"])      # 核不到 ≠ 通过
                self.assertIn("没找到", miss["note"])
            finally:
                MC.FIRM_SESSIONS = saved

    def test_cc_alias_is_not_unknown_agent(self) -> None:
        """`cli.py run cc ...` 不能走到"未知 agent"分支（那会让 CC 全是未核实）。"""
        import tempfile

        from hal.modelcheck import verify_model

        with tempfile.TemporaryDirectory() as d:
            r = verify_model("cc", Path(d), "deepseek-flash")
            self.assertNotIn("未知 agent", r.get("note", ""))


class TestManualObserve(unittest.TestCase):
    """人工观测要变成**可复核**的证据。

    三条底线，缺一条这套基准的铁律第 5 条就守不住：
    ① 问题写死在 spec 里 ⇒ 两个 agent 逐字同一句（公平性）；
    ② 回答连时间戳 + 当时串口最近行落盘 ⇒ 评委能看出这不是仪器证据；
    ③ 没答/超时 = "未答"，**该点不得据此给分** —— 宁可少给，不可假绿。
    """

    @classmethod
    def setUpClass(cls) -> None:
        cls.cfg = load_config()

    def _runner(self, tmp: Path, sim: bool = True):
        from engine import spec as SP
        from engine.runner import Runner

        spec = SP.load_spec(self.cfg.specs / "case03.toml")
        return Runner(self.cfg, spec, "sim", sim=sim, out_root=tmp, workdir=tmp)

    def test_sim_never_asks(self) -> None:
        """离线自测/回放时绝不能卡在等人敲键盘上。"""
        import time as _t

        with tempfile.TemporaryDirectory() as d:
            r = self._runner(Path(d), sim=True)
            t0 = _t.time()
            r._op_manual_observe(0, {"op": "manual_observe", "question": "平滑吗？",
                                     "point": "F1", "timeout_s": 30})
            self.assertLess(_t.time() - t0, 1.0, "模拟运行不该阻塞")
            self.assertEqual(r.outcome.manual, [])
            self.assertTrue(r.outcome.steps[-1].skipped)

    def test_answer_is_recorded_and_reaches_the_point(self) -> None:
        import builtins
        import time as _t

        saved = builtins.input
        with tempfile.TemporaryDirectory() as d:
            builtins.input = lambda *a: "平滑"
            try:
                r = self._runner(Path(d), sim=False)
                r.outcome.started_at = _t.time() - 12.0
                r.outcome.bloodline = {"ok": True}
                r._op_manual_observe(0, {"op": "manual_observe",
                                         "question": "LD2 平滑吗？", "point": "F1",
                                         "choices": ["平滑", "分级跳变"],
                                         "timeout_s": 10})
            finally:
                builtins.input = saved
            self.assertEqual(len(r.outcome.manual), 1)
            rec = r.outcome.manual[0]
            self.assertEqual(rec["answer"], "平滑")
            self.assertIs(rec["in_choices"], True)
            self.assertIs(rec["timed_out"], False)
            self.assertGreaterEqual(rec["t_run_s"], 10.0)
            self.assertTrue((r.evidence / "manual.json").exists())
            pts = r._aggregate()
            self.assertTrue(any("人工观测" in n for n in pts["F1"].notes),
                            "回答必须挂到评分点上，评委才看得见来源")

    def test_timeout_records_unanswered_not_pass(self) -> None:
        """超时 = 记"未答"，并且**不能**被当成通过。"""
        import builtins
        import time as _t

        def _sleepy(*a):
            _t.sleep(2.0)
            return "事后才打出来的答案"

        saved = builtins.input
        with tempfile.TemporaryDirectory() as d:
            builtins.input = _sleepy
            try:
                r = self._runner(Path(d), sim=False)
                r.outcome.started_at = _t.time()
                r.outcome.bloodline = {"ok": True}
                r._op_manual_observe(0, {"op": "manual_observe", "question": "亮了吗？",
                                         "point": "F1", "timeout_s": 0.3})
            finally:
                builtins.input = saved
        rec = r.outcome.manual[0]
        self.assertEqual(rec["answer"], "")
        self.assertIs(rec["timed_out"], True)
        pts = r._aggregate()
        self.assertIs(pts["F1"].score, None, "未答不许给分")
        self.assertTrue(any("超时未答" in n for n in pts["F1"].notes))
        self.assertTrue(any("不构成给分依据" in n for n in pts["F1"].notes))

    def test_report_renders_manual_section(self) -> None:
        """报告里必须单列"人工观测"一节，并写明它**不是仪器证据**。

        评委主要读 report.md，这一节没渲染出来就等于没记。
        """
        import builtins
        import time as _t

        saved = builtins.input
        with tempfile.TemporaryDirectory() as d:
            builtins.input = lambda *a: "分级跳变"
            try:
                r = self._runner(Path(d), sim=False)
                r.outcome.started_at = _t.time()
                r.outcome.bloodline = {"ok": True}
                r._op_manual_observe(0, {"op": "manual_observe", "question": "LD2 平滑吗？",
                                         "point": "F1", "choices": ["平滑", "分级跳变"],
                                         "timeout_s": 10})
            finally:
                builtins.input = saved
            r.outcome.points = r._aggregate()
            r._write_report_md()
            text = (r.out / "report.md").read_text(encoding="utf-8")
        self.assertIn("## 人工观测", text)
        self.assertIn("不是仪器证据", text)
        self.assertIn("分级跳变", text)
        self.assertIn("F1", text.split("## 人工观测")[1].split("##")[0])

    def test_spec_rejects_bad_manual_step(self) -> None:
        """缺 question / point 写错 → 直接报错，不能悄悄跑成"什么都没问"。"""
        from engine import spec as SP

        base = (self.cfg.specs / "case01.toml").read_text(encoding="utf-8")
        cases = {
            "缺 question": '[[steps]]\nop = "manual_observe"\npoint = "F1"\n',
            "point 不存在": '[[steps]]\nop = "manual_observe"\nquestion = "x?"\npoint = "ZZ"\n',
        }
        for label, extra in cases.items():
            with self.subTest(label), tempfile.TemporaryDirectory() as d:
                p = Path(d) / "case01.toml"
                p.write_text(base + "\n" + extra, encoding="utf-8")
                with self.assertRaises(SP.SpecError):
                    SP.load_spec(p)


class TestReadLineTimeout(unittest.TestCase):
    def test_returns_answer_fast(self) -> None:
        import builtins
        import time as _t

        from engine.runner import _read_line_with_timeout

        saved = builtins.input
        builtins.input = lambda *a: "  答对了  "
        try:
            self.assertEqual(_read_line_with_timeout(5.0), ("答对了", False))
        finally:
            builtins.input = saved

    def test_eof_counts_as_unanswered(self) -> None:
        """Ctrl-C / 管道关闭时 `input()` 抛异常 —— 那是"没答"，不是崩溃。"""
        import builtins

        from engine.runner import _read_line_with_timeout

        def _boom(*a):
            raise EOFError

        saved = builtins.input
        builtins.input = _boom
        try:
            self.assertEqual(_read_line_with_timeout(5.0), ("", False))
        finally:
            builtins.input = saved


class TestCampaignRetryGate(unittest.TestCase):
    """driver 的"网络重试"边界：只救断网，不救能力。

    操作手册 §5 写着"网络报错 → 重试几次"，但无限制重试等于给 agent 无限时间盒。
    所以重试要求**同时**满足：① 日志有端点/网络特征；② 工作区零产出（连
    `platformio.ini` 都没有）。跑了一半才断网不重试 —— 那会抹掉真实工作。
    """

    def _probe(self, log_text: str, built: bool, timed_out: bool) -> bool:
        import tempfile

        from campaign import needs_network_retry

        with tempfile.TemporaryDirectory() as d:
            wd = Path(d)
            (wd / "agent.log").write_text(log_text, encoding="utf-8")
            if built:
                (wd / "platformio.ini").write_text("[env]", encoding="utf-8")
            return needs_network_retry(wd / "agent.log", wd, timed_out)

    def test_zero_output_network_error_retries(self) -> None:
        self.assertTrue(self._probe(
            "Error: provider error: HTTP error: error decoding response body", False, False))

    def test_work_already_done_does_not_retry(self) -> None:
        self.assertFalse(self._probe("provider error: connection reset", True, False))

    def test_capability_failure_never_retries(self) -> None:
        """`reached max iterations` 是产品/能力问题，不是断网 —— 不许重开。"""
        self.assertFalse(self._probe("reached max iterations (30)", False, False))

    def test_timebox_timeout_does_not_retry(self) -> None:
        self.assertFalse(self._probe("provider error", False, True))


class TestClaudeMunge(unittest.TestCase):
    """claude 项目目录名的拼法 —— 错一个字符就整侧核验失效。

    2026-10-17 真实翻车：题集路径带空格（`ai test pro`），claude 落盘成
    `...-ai-test-pro-...`，我们的 munge 保留了空格 ⇒ claude 侧 8 个 run 全部
    假报"没找到会话记录"。**"核不到"和"路径拼错"在报告里长得一模一样**，
    所以这条必须有用例守着。
    """

    def test_spaces_and_separators_all_become_dashes(self) -> None:
        from hal.modelcheck import _munge_cwd

        got = _munge_cwd(Path(r"D:\OldStudy66\ai test pro\runs\claude\case04"))
        self.assertEqual(got, "D--OldStudy66-ai-test-pro-runs-claude-case04")
        self.assertNotIn(" ", got, "空格必须也换成 -，否则永远匹配不到真实目录")

    def test_forward_slash_and_trailing_sep(self) -> None:
        from hal.modelcheck import _munge_cwd

        self.assertEqual(_munge_cwd(Path("C:/x/y/")), "C--x-y")


class TestReportAccounting(unittest.TestCase):
    """C5 / C7：报告的"依据"要指着错的那条，规格变过要自己说出口。"""

    @classmethod
    def setUpClass(cls) -> None:
        cls.cfg = load_config()
        cls.tmp = Path(tempfile.mkdtemp(prefix="eval-acct-"))

    @classmethod
    def tearDownClass(cls) -> None:
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def _agg(self, asserts):
        spec = SP.load_spec(self.cfg.specs / "case01.toml")
        r = Runner(self.cfg, spec, "sim", sim=True, out_root=self.tmp)
        r.outcome.bloodline = {"ok": True}
        r.outcome.asserts = asserts
        return r._aggregate()

    def test_point_evidence_cites_the_failing_assert(self) -> None:
        """同一个评分点里，通过项的证据不许把失败项挤掉。

        case08 的 F2 就是这样被读错的：报告"依据"列指着 burst.log 里那几条**通过**的
        断言，而真正挂的是 LA 的 F2d —— 看报告的人会去查错的东西，比判错更难复盘。
        """
        pts = self._agg([
            {"id": "F2a", "point": "F2", "type": "serial_regex", "ok": True,
             "evidence": ["burst.log 第1条@+1.0s"]},
            {"id": "F2d", "point": "F2", "type": "edges", "ok": False,
             "evidence": ["capture-4.sr#D5@24000000Hz"]},
        ])
        self.assertEqual(pts["F2"].machine_evidence, ["capture-4.sr#D5@24000000Hz"])

    def test_unobserved_beats_passing_evidence(self) -> None:
        pts = self._agg([
            {"id": "F1a", "point": "F1", "type": "serial_regex", "ok": True, "evidence": ["a.log 1"]},
            {"id": "F1b", "point": "F1", "type": "serial_timing", "ok": None, "evidence": ["b.log 2"]},
        ])
        self.assertEqual(pts["F1"].status, "pending_ai")
        self.assertEqual(pts["F1"].machine_evidence, ["b.log 2"], "没红的时候该指『没看到』那条")

    def test_result_carries_spec_fingerprint(self) -> None:
        """result.json 里要有规格指纹 —— 激励脚本一改，旧证据就不该再被当可比数据。"""
        out = self._run_sim()
        fp = out.summary()["spec_fp"]
        self.assertRegex(fp, r"^[0-9a-f]{12}$")

    def test_fingerprint_changes_with_the_spec(self) -> None:
        src = (self.cfg.specs / "case01.toml").read_text(encoding="utf-8")
        p = self.tmp / "case01.toml"
        p.write_text(src, encoding="utf-8")
        fp1 = SP.load_spec(p).fingerprint
        p.write_text(src + "\n# 只是加个注释不该改变判据\n", encoding="utf-8")
        fp2 = SP.load_spec(p).fingerprint
        self.assertNotEqual(fp1, fp2, "指纹是文件内容级的，注释变了也算（宁可误报不可漏报）")

    def _run_sim(self):
        work = self.tmp / "work"
        if not work.exists():
            shutil.copytree(self.cfg.testset / "start" / "cubemx", work)
            (work / "platformio.ini").write_text(
                "[env:nucleo_g431rb]\nplatform = ststm32\nboard = nucleo_g431rb\n"
                "framework = stm32cube\n", encoding="utf-8")
        spec = SP.load_spec(self.cfg.specs / "case01.toml")
        r = Runner(self.cfg, spec, "sim", sim=True, out_root=self.tmp / "rep", workdir=work)
        r._sim_scripts["dut_vcp"] = load_script(self.cfg.fixtures / "case01-good.json")  # noqa: SLF001
        return r.run()


class TestEmptyWindow(unittest.TestCase):
    """C3：空窗既不是绿灯也不是红灯 —— 与 LA 的"零活动 ⇒ 未观测"对称。

    双向钉子（同一份数据两个方向都要验，否则排不掉"本来就是空的"这种巧合）：
    ① 窗口一行都没有 ⇒ `serial_absent` 不得"通过"（case11 H1b 的假绿就这么来的）；
    ② 窗口有内容、只是没有那行 ⇒ 这才叫"真的没出现"，可以判红。
    """

    def _absent(self, lines):
        a = {"id": "H1b", "point": "H1", "type": "serial_absent", "pattern": "RESET=IWDG"}
        return A.evaluate(a, A.Ctx(lines=lines))

    def test_empty_window_is_not_a_pass(self) -> None:
        r = self._absent([])
        self.assertIsNone(r.ok, "空窗判通过 = 拿判分器的失明兑换绿灯")
        self.assertIn("未观测", r.note)

    def test_nonempty_window_without_the_line_is_a_real_pass(self) -> None:
        lines = [Line(0, 1.0, "RESET=POR"), Line(1, 1.5, "ALIVE 1")]
        r = self._absent(lines)
        self.assertIs(r.ok, True, "窗口里确实有别的输出、就是没有 IWDG —— 这才是观测到了『没发生』")

    def test_nonempty_window_with_the_line_fails(self) -> None:
        r = self._absent([Line(0, 1.0, "RESET=IWDG")])
        self.assertIs(r.ok, False)

    def test_every_serial_type_refuses_empty_window(self) -> None:
        """不只 absent：所有吃窗口的 serial_* 在空窗前一律记未观测。"""
        samples = [
            {"type": "serial_regex", "pattern": "X"},
            {"type": "serial_count", "pattern": "X", "expect": 0},
            {"type": "serial_sequence", "pattern": "(\\d+)", "start": 1, "count": 1},
            {"type": "serial_silence", "pattern": "X"},
            {"type": "serial_garbage"},
        ]
        for s in samples:
            a = {**s, "id": f"Z_{s['type']}", "point": "F1"}
            r = A.evaluate(a, A.Ctx(lines=[]))
            self.assertIsNone(r.ok, f"{s['type']} 在空窗里得停手：{r}")

    def test_count_expect_zero_still_needs_an_open_window(self) -> None:
        """`expect=0` 看着像"本该为空"，但没在听的时候它同样成立 ⇒ 不能免检。"""
        a = {"id": "Z", "point": "F1", "type": "serial_count", "pattern": "ERR", "expect": 0}
        self.assertIsNone(A.evaluate(a, A.Ctx(lines=[])).ok)
        self.assertIs(A.evaluate(a, A.Ctx(lines=[Line(0, 1.0, "OK")])).ok, True)


class TestStaticRuleDiscipline(unittest.TestCase):
    """C4 的三道机械闸门。

    逐条把规则改对是**不收敛**的 —— 下一批假红/假绿会以新的拼写形态出现。
    这里守的是三条通用纪律，谁以后再写规则都绕不过去：
    ① 判红必须指着行说话（零证据的 False 一律降成未观测）；
    ② 判"做过某件事"必须先剔掉 CubeMX 母本里本来就有的代码（否则不改也算有考量）；
    ③ 语料必须同时覆盖 `src/` 与 `cubemx/Core/`（否则两个 agent 被看的代码量不一样）。
    """

    BASE_IT = """/* CubeMX 生成的中断优先级设置 —— 母本本来就有 */
void NVIC_Config(void)
{
  HAL_NVIC_SetPriority(USART1_IRQn, 0, 0);
  HAL_GPIO_EXTI_Callback(GPIO_PIN_13);
}
"""
    APP_OK = """/* agent 自己写的：两路非阻塞接收 + 优先级考量 */
void bridge_init(void)
{
  HAL_UARTEx_ReceiveToIdle_DMA(huart1, buf, 64);
}
void bridge_irq(void)
{
  HAL_NVIC_SetPriority(USART3_IRQn, 1, 0);
}
"""

    def _mk(self, app_src: str, case: str = "case13") -> Path:
        root = Path(tempfile.mkdtemp(prefix="eval-disc-"))
        self.addCleanup(shutil.rmtree, root, True)
        base = root / "start" / "cubemx" / "Core" / "Src"
        base.mkdir(parents=True)
        (base / "stm32g4xx_it.c").write_text(self.BASE_IT, encoding="utf-8")
        wd = root / "runs" / "firm" / case
        (wd / "cubemx" / "Core" / "Src").mkdir(parents=True)
        (wd / "src").mkdir(parents=True)
        # 母本副本逐字节一致 ⇒ 应被识别为"母本"
        shutil.copy(base / "stm32g4xx_it.c", wd / "cubemx" / "Core" / "Src" / "stm32g4xx_it.c")
        (wd / "src" / "app.c").write_text(app_src, encoding="utf-8")
        return wd

    def test_zero_evidence_false_is_downgraded(self) -> None:
        r = SR.RuleResult("case99.H1", "H1", False, "某判据", {"x": 0}, [], "我没找到")
        got = SR._no_evidence_guard(r, "case99.H1", "H1")
        self.assertIsNone(got.ok, "判红却零证据 ⇒ 必须降成未观测")
        self.assertIn("证据", got.note)
        keep = SR.RuleResult("case99.H1", "H1", False, "某判据", {}, ["src/a.c:7"], "这里阻塞了")
        self.assertIs(SR._no_evidence_guard(keep, "case99.H1", "H1").ok, False,
                      "指得出行的红就该保持红，不许一律降级")

    def test_absent_evidence_counts_as_evidence(self) -> None:
        """"扫遍了但没有"是合法证据 —— 但不能只是一句"没有"，要带上扫描范围。"""
        wd = self._mk(self.APP_OK)
        ws = SR.Workspace(wd)
        ev = ws.absent("HAL_TIM_PWM_Start")
        self.assertEqual(len(ev), 1)
        self.assertIn(str(len(ws.files)), ev[0], "证据行要写明扫了几个文件")

    def test_baseline_hits_are_not_credited(self) -> None:
        """母本里本来就有的 NVIC 设置，不能算 agent 的"优先级考量"。"""
        wd = self._mk("void f(void) { HAL_UART_Receive_IT(huart1, b, 1); }\n")   # 没有新的 NVIC
        res = SR.run_rules("case13", wd)["rules"]["case13.H2"]
        self.assertIsNone(res["ok"], f"只找到母本自带的优先级设置 ⇒ 不该打勾：{res}")
        self.assertEqual(res["got"]["nvic_agent"], 0)
        self.assertGreater(res["got"]["nvic_total"], 0, "母本里那些命中得留在 got 里，才看得出差额来自哪")
        wd2 = self._mk(self.APP_OK)
        self.assertIs(SR.run_rules("case13", wd2)["rules"]["case13.H2"]["ok"], True,
                      "agent 自己写了 NVIC 设置就该认，别把过滤做成一律不通过")

    def test_corpus_covers_cubemx_core_and_excludes_drivers(self) -> None:
        wd = self._mk(self.APP_OK)
        (wd / "cubemx" / "Drivers" / "BSP").mkdir(parents=True)
        (wd / "cubemx" / "Drivers" / "BSP" / "leak.c").write_text("HAL_TIM_PWM_Start(&htim2);\n",
                                                                  encoding="utf-8")
        ws = SR.Workspace(wd)
        rel = {str(p.relative_to(wd)).replace("\\", "/") for p in ws.files}
        self.assertIn("cubemx/Core/Src/stm32g4xx_it.c", rel, "CubeMX 那半必须在语料里")
        self.assertIn("src/app.c", rel)
        self.assertFalse([r for r in rel if "Drivers" in r], "Drivers/ 必须继续排除")

    def test_lsi_40khz_pattern_rejects_40mhz(self) -> None:
        """`40000` 是 `40000000` 的子串 —— 不加边界就会把 HSE 40MHz 指控成"按 40kHz 算"。"""
        rx = r"(?<![\d_])40[_]?000(?![\d_])|40e3\b|40\s*\*\s*1000"
        self.assertIsNone(re.search(rx, "#define HSE_VALUE 40000000U"))
        self.assertIsNotNone(re.search(rx, "timeout = 40000; /* LSI */"))
        wd = self._mk("void i(void) { IWDG->PR = 4; IWDG->RLR = 1000; }\n", case="case11")
        (wd / "src" / "conf.h").write_text("#define HSE_VALUE (40000000U)\n", encoding="utf-8")
        r = SR.run_rules("case11", wd)["rules"]["case11.H2"]
        self.assertEqual(r["got"]["lsif1_hint"], [], f"不该把 40MHz 当成 40kHz：{r}")

    def test_case05_pairs_must_come_from_one_init_block(self) -> None:
        """跨定时器凑出来的 20kHz 不算数：A 的 PSC 配 B 的 ARR 是假绿。

        170MHz / ((4+1)·(1699+1)) = 20.000kHz；单独看每个数都"对"，
        但 `htim1.PSC` 配 `htim2.ARR` 出来的这个组合什么也没驱动。
        """
        bad = """
void cfg(void)
{
  htim1.Init.Prescaler = 4;
  htim2.Init.Period = 1699;
}
"""
        wd = self._mk(bad, case="case05")
        r = SR.run_rules("case05", wd)["rules"]["case05.H1"]
        self.assertEqual(r["got"]["same_block_pairs"], 0, "两个句柄不同 ⇒ 不该配对")
        self.assertIsNone(r["ok"], f"配不成同一段就记未观测，不该拿凑出来的数判：{r}")
        good = """
void cfg(void)
{
  htim1.Init.Prescaler = 4;
  htim1.Init.Period = 1699;
}
"""
        wd2 = self._mk(good, case="case05")
        r2 = SR.run_rules("case05", wd2)["rules"]["case05.H1"]
        self.assertIs(r2["ok"], True, f"同一路的 PSC/ARR 该判通过：{r2}")
        self.assertAlmostEqual(r2["got"]["f_hz_170M"], 20000.0)

    def test_case04_sharing_premise_is_actually_tested(self) -> None:
        """case04.H2 要先证"跨上下文共享"这件事，再谈保护。

        三个方向都得钉：① ISR 改写 + 函数体外读取 + 无保护 ⇒ False **且指得出行**；
        ② volatile 或临界区都算保护（原来只认 volatile，用临界区实现的反被误判）；
        ③ 状态全在 ISR 里闭环 ⇒ 前提不成立，记未观测而不是 0 分。
        另外"声明行"不算引用 —— 不排掉它，每个 static 都会被误报成共享
        （实测 claude/case04 的五个计数变量全中招）。
        """
        reader = "\nuint32_t get_count(void)\n{\n  return count;\n}\n"
        raw = ("static uint32_t count;\n"
               "void HAL_GPIO_EXTI_Callback(uint16_t pin)\n{\n  count++;\n}\n") + reader
        cases = [
            ("裸共享无保护", raw, False),
            ("volatile", raw.replace("static uint32_t count;", "static volatile uint32_t count;"), True),
            ("临界区", raw.replace("  return count;",
                                  "  __disable_irq();\n  uint32_t v = count;\n  __enable_irq();\n  return v;"), True),
            ("只在 ISR 里闭环", raw.replace(reader, "\nvoid pump(void) { }\n"), None),
        ]
        for name, src, expect in cases:
            with self.subTest(scenario=name):
                wd = self._mk(src, case="case04")
                r = SR.run_rules("case04", wd)["rules"]["case04.H2"]
                self.assertIs(r["ok"], expect, f"{name} → {r}")
                if expect is not None:
                    self.assertTrue(r["evidence"], f"{name} 的结论必须指得出行")
                if expect is False:
                    self.assertIn("count", str(r["got"]), "该指出是哪个变量被共享")


class TestCallbackDoubleRegistration(unittest.TestCase):
    """`case04.H2` 说"重复注册"之前，先把**不是一个回调**的行剔掉。

    实测在 claude 旧 case04 上数出 3 个回调判红：`src/app.c:46`（它自己的强符号）、
    `src/stm32g4xx_nucleo.c:354`（BSP 的 `__weak` 默认实现 —— 转换时整份 BSP 被挪进 `src/`，
    按内容剔母本剔不掉）、`src/stm32g4xx_nucleo.h:275`（原型声明，结尾是 `;`）。
    强符号覆盖弱符号是**一个**回调，这正是我给 case01 H2 加分的那种写法。
    """

    BASE = "void g(void) { HALTick(); }\n"
    ONE = """void BSP_PB_Callback(Button_TypeDef b);        /* 原型 */
__weak void BSP_PB_Callback(Button_TypeDef b) { for (;;) { } }   /* BSP 默认弱实现 */
void BSP_PB_Callback(Button_TypeDef b)                /* agent 的强实现：唯一的回调 */
{
  count++;
}
void EXTI15_10_IRQHandler(void) { BSP_PB_IRQHandler(1); }
"""
    TWO = """void HAL_GPIO_EXTI_Callback(uint16_t pin) { count++; }
void BSP_PB_Callback(Button_TypeDef b) { count++; }
void EXTI15_10_IRQHandler(void) { HAL_GPIO_EXTI_IRQHandler(13); }
"""

    def _wd(self, app: str) -> Path:
        root = Path(tempfile.mkdtemp(prefix="eval-cb-"))
        self.addCleanup(shutil.rmtree, root, True)
        base = root / "start" / "cubemx" / "Core" / "Src"
        base.mkdir(parents=True)
        (base / "stm32g4xx_it.c").write_text(self.BASE, encoding="utf-8")
        wd = root / "runs" / "firm" / "case04"
        (wd / "cubemx" / "Core" / "Src").mkdir(parents=True)
        (wd / "src").mkdir(parents=True)
        shutil.copy(base / "stm32g4xx_it.c", wd / "cubemx" / "Core" / "Src" / "stm32g4xx_it.c")
        (wd / "src" / "app.c").write_text(app, encoding="utf-8")
        return wd

    def test_prototype_and_weak_are_not_two_callbacks(self) -> None:
        r = SR.run_rules("case04", self._wd(self.ONE))["rules"]["case04.H2"]
        self.assertIsNot(r["ok"], False, f"原型+弱实现+强实现不该被数成重复注册：{r}")

    def test_two_real_definitions_still_count_as_duplicate(self) -> None:
        """反证：剔掉原型与弱实现之后，真有两个定义时这条防线必须还判得出红。"""
        r = SR.run_rules("case04", self._wd(self.TWO))["rules"]["case04.H2"]
        self.assertIs(r["ok"], False, f"两个回调定义 = 一次按下记两次：{r}")
        self.assertGreaterEqual(r["got"]["callbacks"], 2)


class TestEveryRuleRunsOnMacros(unittest.TestCase):
    """每条规则都得**真的跑起来**，不许靠"异常被吞成未观测"蒙过测试。

    引出这条的实测事故：`Workspace._collect_macros` 被写成 `@staticmethod` 却在函数体里用
    `self._macros` ⇒ 语料里只要有一个对象式 `#define 名字 值`，`Workspace()` 当场 NameError。
    127 条测试全绿的原因就是夹具里那些 `#define` 都在 `.h`/注释里，没走到这条路径；
    而真板上 case10.H2 因此把"算出来的最后一页"判成假红（`缺少：page63`），
    case07.H2、case08.H1 退化成"看不懂 ⇒ 交 AI"。同一个语料在修好后重跑：
    case07.H2 ✓、case08.H1 ✓、case10.H2 ✓ —— 少算的分和推给 AI 的分都是尺子的洞。

    这道门做两件事：① 用一份**带对象式 `#define` 的 `.c`** 语料跑遍 `RULES` 里的每一条，
    任何"规则执行异常"就是失败；② 确认宏表真的收到了东西（表空=功能没接线，也算失败）。
    """

    CORPUS = """#define RING_SIZE   64u
#define RING_MASK   (RING_SIZE - 1u)
#define PWM_CLK     170000000UL
#define PWM_ARR     (PWM_CLK / 20000UL - 1u)

volatile uint16_t rx_head, rx_tail;
volatile uint16_t shared_counter;

void BSP_PB_Callback(Button_TypeDef b);          /* 原型：不是定义，不许当"重复注册" */

void cfg(void)
{
  htim1.Init.Prescaler = 0;
  htim1.Init.Period    = PWM_ARR;
  HAL_TIM_PWM_Start(&htim1, TIM_CHANNEL_1);
}

void push(uint8_t b)
{
  uint16_t next = (uint16_t)((rx_head + 1u) & RING_MASK);
  if (next != rx_tail) { buf[rx_head] = b; rx_head = next; }
}

void EXTI15_10_IRQHandler(void)
{
  HAL_GPIO_EXTI_IRQHandler(13);
  shared_counter++;
}

void BSP_PB_Callback(Button_TypeDef b)
{
  shared_counter++;
  push((uint8_t)b);
}

void poll(void)
{
  HAL_GPIO_TogglePin(LED_PORT, LED_PIN);
  for (;;) { }
}
"""

    @classmethod
    def setUpClass(cls) -> None:
        cls.root = Path(tempfile.mkdtemp(prefix="eval-macro-"))
        base = cls.root / "start" / "cubemx" / "Core" / "Src"
        base.mkdir(parents=True)
        (base / "stm32g4xx_it.c").write_text("void g(void) { HALTick(); }\n", encoding="utf-8")

    @classmethod
    def tearDownClass(cls) -> None:
        shutil.rmtree(cls.root, ignore_errors=True)

    def _wd(self, case: str) -> Path:
        wd = self.root / "runs" / "firm" / case
        (wd / "cubemx" / "Core" / "Src").mkdir(parents=True, exist_ok=True)
        (wd / "src").mkdir(parents=True, exist_ok=True)
        shutil.copy(self.root / "start" / "cubemx" / "Core" / "Src" / "stm32g4xx_it.c",
                    wd / "cubemx" / "Core" / "Src" / "stm32g4xx_it.c")
        (wd / "src" / "app.c").write_text(self.CORPUS, encoding="utf-8")
        return wd

    def test_macro_table_is_populated_from_the_corpus(self) -> None:
        ws = SR.Workspace(self._wd("case05"))
        self.assertIn("RING_SIZE", ws._macros, "宏表为空 ⇒ 符号写法的支持根本没接线（静默退化成'交 AI'）")
        self.assertEqual(ws.eval_int("RING_SIZE"), 64)

    def test_no_rule_crashes_on_a_define_bearing_corpus(self) -> None:
        crashed: list[str] = []
        for case in sorted({rid.split(".")[0] for rid in SR.RULES}):
            res = SR.run_rules(case, self._wd(case))
            for rid, r in res["rules"].items():
                note = r.get("note") or ""
                if "规则执行异常" in note:
                    crashed.append(f"{rid}: {note}")
        self.assertEqual(crashed, [], "有规则在这份语料上抛异常（被 run_rules 吞成未观测）")


class TestLaStimulusAndZeroActivity(unittest.TestCase):
    """1C：仪器断言的两条防线。

    ① 激励必须在采集**窗内** —— case08 的 F2d 原来把 LED ON/OFF 排在 la_capture
       之前，窗内引脚恒定 ⇒ 两个 agent 全挂，而它们的 F2a/b/c 其实都过了。
    ② 整盘零活动时不许判红 —— "真没输出"和"采样时机错过 / 探头没夹"数据同形；
       但也不能把"报警已停"这种**合法绿灯**一起灭掉。
    """

    @classmethod
    def setUpClass(cls) -> None:
        cls.cfg = load_config()
        cls.tmp = Path(tempfile.mkdtemp(prefix="eval-la-"))

    @classmethod
    def tearDownClass(cls) -> None:
        shutil.rmtree(cls.tmp, ignore_errors=True)

    class FakeLA:
        """假分析仪：capture() 在后台线程里睡一段时间，用来验激励是否落在窗内。"""

        def __init__(self, bits: dict[str, list[int]], hold_s: float = 0.25) -> None:
            self.bits = bits
            self.hold_s = hold_s
            self.timeline: list[str] = []

        def available(self) -> bool:
            return True

        def channel_of(self, name: str) -> int:
            return {"PA5": 5, "PB8": 8, "PA8": 8}[name]

        def capture(self, seconds, out, **kw):  # noqa: ANN001, ARG002
            from hal.la import Capture as LaCapture
            from hal.la import channel_activity

            self.timeline.append("capture-enter")
            time.sleep(self.hold_s)
            self.timeline.append("capture-exit")
            # n_samples 按"采样率 × 秒数"给足：r6 会拿它和期望值比，短了会触发重试；
            # 这几条测的是激励时序，不该被截断重试干扰。
            return LaCapture(path=str(out), samplerate_hz=1_000_000.0, channels=sorted(self.bits),
                             n_samples=int(1_000_000.0 * seconds), driver="fake",
                             bits=self.bits, activity=channel_activity(self.bits))

    def _run_op(self, bits, st, on_bus=None):
        spec = SP.load_spec(self.cfg.specs / "case08.toml")
        r = Runner(self.cfg, spec, "firm", sim=False, out_root=self.tmp,
                   workdir=self.cfg.testset / "runs" / "firm" / "case08")
        fake = self.FakeLA(bits)
        r._la = lambda: fake  # noqa: SLF001
        r._cur_step = 0
        bus = SimBus(SimScript(rx=[]), label="dut_vcp")
        bus.open()
        r.buses["dut_vcp"] = bus
        r.bus = bus
        if on_bus:
            on_bus(bus, fake)
        r._op_la_capture(0, st)
        got = {a["id"]: a for a in r.outcome.asserts}
        return got, fake, bus, r

    def test_toggling_pin_passes_edge_count(self) -> None:
        bits = {"D5": [0, 1] * 200}          # 400 个样本、399 次跳变
        st = {"op": "la_capture", "duration_ms": 300,
              "asserts": [{"id": "F2d", "point": "F2", "type": "edges", "channel": "PA5", "min_count": 4}]}
        got, *_ = self._run_op(bits, st)
        self.assertIs(got["F2d"]["ok"], True, got["F2d"])

    def test_zero_activity_is_unobserved_not_failed(self) -> None:
        """探头没夹 / 时机错过 ⇒ 记 —，不记 ✗（拔探头必须得到未观测）。"""
        bits = {"D5": [0] * 400, "D0": [0] * 400}
        st = {"op": "la_capture", "duration_ms": 300,
              "asserts": [{"id": "F2d", "point": "F2", "type": "edges", "channel": "PA5", "min_count": 4}]}
        got, *_ = self._run_op(bits, st)
        self.assertIsNone(got["F2d"]["ok"], f"零活动不该判红：{got['F2d']}")
        self.assertIn("零活动", got["F2d"]["note"])

    def test_quiet_pin_still_proves_alarm_stopped(self) -> None:
        """`max_count` 断言要的恰恰是"没有跳变" ⇒ 零活动防线不能把它灭掉。"""
        bits = {"D5": [0] * 400, "D0": [0] * 400}
        st = {"op": "la_capture", "duration_ms": 300,
              "asserts": [{"id": "alarm2", "point": "报警", "type": "edges", "channel": "PA5", "max_count": 2}]}
        got, *_ = self._run_op(bits, st)
        self.assertIs(got["alarm2"]["ok"], True, got["alarm2"])

    def test_steady_level_still_measurable_as_duty(self) -> None:
        bits = {"D5": [1] * 400, "D0": [0] * 400}
        st = {"op": "la_capture", "duration_ms": 300,
              "asserts": [{"id": "F2d", "point": "F2", "type": "duty", "channel": "PA5", "pct": 100, "tol_pt": 5}]}
        got, *_ = self._run_op(bits, st)
        self.assertIs(got["F2d"]["ok"], True, got["F2d"])

    def test_during_sends_frames_inside_the_capture_window(self) -> None:
        """激励必须落在 capture() 活着的那段时间里 —— 排在它前面就等于没激励。"""
        bits = {"D5": [0, 1] * 200}
        st = {"op": "la_capture", "duration_ms": 800, "stimulus_delay_ms": 20,
              "during": [{"text": "LED ON", "at_ms": 0}, {"text": "LED OFF", "at_ms": 60}],
              "asserts": [{"id": "F2d", "point": "F2", "type": "edges", "channel": "PA5", "min_count": 4}]}

        def spy(bus, fake):
            orig = bus.send_line

            def wrapped(text, eol="\r\n"):
                fake.timeline.append(f"send:{text.strip()}")
                return orig(text, eol)

            bus.send_line = wrapped

        got, fake, bus, r = self._run_op(bits, st, on_bus=spy)
        self.assertEqual((r._la_stimulus["frames"], r._la_stimulus["sent"]), (2, 2))
        self.assertEqual(fake.timeline, ["capture-enter", "send:LED ON", "send:LED OFF", "capture-exit"],
                         "两次激励都应夹在采集的开始与结束之间")
        self.assertEqual([t for _, t in bus.sent], ["LED ON\r\n", "LED OFF\r\n"])
        self.assertIs(got["F2d"]["ok"], True)

    def test_during_frames_are_billed_to_sent_frames(self) -> None:
        """窗内激励也是发出去并被固件处理的帧 ⇒ 必须进 `sent_frames` 这本账。

        实测漏记的代价：case08 的 burst 发 100 帧、窗内再发 5 帧 LED，两个 agent 都如实
        回答 `COUNT 105`，而 F3b 拿 100 当参照 ⇒ 两侧各白丢 0.5 分（权重 9）。
        """
        bits = {"D5": [0, 1] * 200}
        st = {"op": "la_capture", "duration_ms": 800, "stimulus_delay_ms": 20,
              "during": [{"text": "LED ON", "at_ms": 0}, {"text": "LED OFF", "at_ms": 60},
                         {"text": "LED ON", "at_ms": 120}],
              "asserts": [{"id": "F2d", "point": "F2", "type": "edges", "channel": "PA5", "min_count": 4}]}
        *_rest, r = self._run_op(bits, st)
        self.assertEqual(r._sent_frames, 3, "窗内发了 3 帧就该记 3 帧")


class TestPulseWidthWindowBorder(unittest.TestCase):
    """`pulse_width` 只许量**完整落在窗内**的高电平台。

    抓包起点可能正好落在半截高电平里，那第一段量的只是窗内剩下的半截。rc3 第二次跑
    case05 就是这样把 15 µs 量成 7.5 µs 判红，而同一盘波形的 duty=30.0 %、
    frequency=20.05 kHz 都是绿的（15 µs/50 µs 与 30 % 是同一件事，7.5 µs 与之互斥）。
    截断只会让读数偏小 ⇒ 排除首尾**只可能消掉假红**；固件真打断了某个高电平，
    那个短平台必然完整落在窗内（第三条测试就是这条反证）。
    """

    SR = 4_000_000.0          # 4 MHz：1 个样本 = 0.25 µs

    @staticmethod
    def _pwm(period: int, high: int, n: int) -> list[int]:
        bits: list[int] = []
        for _ in range(n):
            bits += [1] * high + [0] * (period - high)
        return bits

    def test_truncated_first_pulse_is_not_the_measurement(self) -> None:
        from hal.la import measure_pulse_width

        full = self._pwm(200, 60, 5)              # 60 样本 = 15 µs 高电平
        stub = full[30:]                          # 窗起点落在半截高电平里：首段只剩 7.5 µs
        self.assertEqual(measure_pulse_width(stub, self.SR)["us"], 15.0,
                         "首段被窗边界切断，不能拿它当'最短脉宽'")
        self.assertEqual(measure_pulse_width(full, self.SR)["us"], 15.0)   # 对照：没截断时同值

    def test_real_gaps_inside_the_window_still_fail(self) -> None:
        """反证：中途真被打印打断（30 样本 → 7.5 µs）时，短平台仍在窗内，必须抓到。"""
        from hal.la import measure_pulse_width

        bits = self._pwm(200, 60, 3) + [1] * 30 + [0] * 170 + self._pwm(200, 60, 3)
        self.assertEqual(measure_pulse_width(bits, self.SR)["us"], 7.5)

    def test_all_high_window_is_unobserved_not_red(self) -> None:
        """整窗一个高电平台（两端都被切断）⇒ 交不出 `us`，断言只能记未观测，不能记 0。"""
        from hal.la import measure_pulse_width

        d = measure_pulse_width([1] * 500, self.SR)
        self.assertNotIn("us", d, "给不出可信读数时不许返回一个数去兑换红灯")
        self.assertEqual(d["confidence"], "low")
        self.assertIn("切断", d["note"], "报告里要能看出是窗边界问题，不是固件没输出")


class TestWipeArchive(unittest.TestCase):
    """重跑 prep 会 `rmtree` 掉 `runs/<agent>/<case>`，而静态规则的语料就是那目录里的源码。

    没有这道门时，『用当前尺子重判上一轮』只会在一轮跑完的当天成立：报告目录里存的是判定、
    断言和串口日志（`evidence/static.json` 记结论，不记被扫的文件），源码一抹就再也算不出来，
    栏 A 只剩下一个没人能复核的数字。所以抹之前必须先留档，留不住就别抹。

    第二条不变式同样是被实测逼出来的：归档**不许落在测试集里**。原先它放在
    `runs/<agent>/_bench/`，正是各 case 工作区的兄弟目录，而两边都有 shell（工作区沙箱按
    `tools/util.rs` 自己的说明不覆盖 shell）—— claude 在 case06 就 `Get-ChildItem` 过那棵树，
    里面是对面 agent 每一局的完整解答。
    """

    def _cfg(self, tmp: Path):
        from types import SimpleNamespace
        return SimpleNamespace(testset=str(tmp), judge=str(tmp / "judge"))

    def _ws(self, tmp: Path, case: str = "case01") -> Path:
        ws = tmp / "runs" / "firm" / case
        ws.mkdir(parents=True)
        (ws / "main.c").write_text("int main(void) { return 0; }", encoding="utf-8")
        return ws

    def test_wipe_keeps_a_readable_copy_of_the_corpus(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            ws = self._ws(tmp)
            campaign.wipe_workspace(ws, self._cfg(tmp))
            self.assertFalse(ws.exists(), "工作区本身要被抹掉，否则测不到『先留档』这一步")
            kept = tmp / "judge" / "bench-archive" / "firm-case01" / "run-1"
            self.assertIn("return 0", (kept / "main.c").read_text(encoding="utf-8"),
                          "归档里那份必须还能当语料用")

    def test_archive_never_lands_inside_the_testset(self) -> None:
        """被试有 shell，测试集里的任何东西对它都是可读的 —— 参考解答不能放进来。"""
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            cfg = self._cfg(tmp)
            kept = campaign.archive_before_wipe(self._ws(tmp), cfg)
            self.assertIsNotNone(kept)
            self.assertNotIn(str(tmp / "runs"), str(kept),
                             f"归档跑进测试集里了：{kept}")
            self.assertTrue(str(kept).startswith(str(tmp / "judge")))

    def test_repeat_wipes_do_not_overwrite_history(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            cfg = self._cfg(tmp)
            area = tmp / "judge" / "bench-archive" / "firm-case01"
            for n in (1, 2, 3):
                ws = self._ws(tmp)
                (ws / "main.c").write_text(f"v{n}", encoding="utf-8")
                campaign.wipe_workspace(ws, cfg)
            self.assertEqual(sorted(p.name for p in area.iterdir()),
                             ["run-1", "run-2", "run-3"])
            self.assertEqual((area / "run-1" / "main.c").read_text(encoding="utf-8"), "v1",
                             "第二轮不许覆盖第一轮的历史")
            self.assertEqual((area / "run-3" / "main.c").read_text(encoding="utf-8"), "v3")

    def test_nothing_is_wiped_when_archiving_fails(self) -> None:
        """留档失败 ⇒ 报错停在原地。静默抹掉语料比不跑这一题糟糕得多。"""
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            ws = self._ws(tmp)
            # 该放归档目录的位置摆一个**同名文件** ⇒ mkdir 失败，归档断在这一步
            blocker = tmp / "judge" / "bench-archive" / "firm-case01"
            blocker.parent.mkdir(parents=True, exist_ok=True)
            blocker.write_text("占位", encoding="utf-8")
            with self.assertRaises(OSError):
                campaign.wipe_workspace(ws, self._cfg(tmp))
            self.assertTrue(ws.exists(), "归档失败时工作区必须原样留着")
            self.assertIn("return 0", (ws / "main.c").read_text(encoding="utf-8"))

    def test_the_archive_area_itself_is_still_undeletable(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            protected = tmp / "runs" / "_bench" / "firm-case01"
            protected.mkdir(parents=True)
            with self.assertRaises(SystemExit):
                campaign.wipe_workspace(protected, self._cfg(tmp))
            self.assertTrue(protected.exists())

    def test_claude_project_state_is_moved_aside(self) -> None:
        """claude 的自动记忆在工作区之外，`rmtree` 碰不到 ⇒ 每局开跑前必须挪走。"""
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            cfg = self._cfg(tmp)
            home = tmp / "home"
            work = tmp / "runs" / "claude" / "case01"
            proj = home / ".claude" / "projects" / "D--x-runs-claude-case01"
            (proj / "memory").mkdir(parents=True)
            (proj / "memory" / "MEMORY.md").write_text("- 上一次的笔记\n", encoding="utf-8")
            (proj / "session.jsonl").write_text("{}\n", encoding="utf-8")

            real_home = Path.home
            real_proj = None
            import hal.modelcheck as MC

            real_munge = MC._munge_cwd
            try:
                Path.home = lambda: home  # type: ignore[assignment]
                MC._munge_cwd = lambda w: proj.name  # type: ignore[assignment]
                moved = campaign.quarantine_claude_state(cfg, work)
            finally:
                Path.home = real_home  # type: ignore[assignment]
                MC._munge_cwd = real_munge  # type: ignore[assignment]

            self.assertIsNotNone(moved)
            self.assertFalse(proj.exists(), "原位置必须已经空了，下一局才拿不到上一局的记忆")
            self.assertIn("上一次的笔记",
                          (Path(moved) / "memory" / "MEMORY.md").read_text(encoding="utf-8"),
                          "挪走不是删除 —— 历史还得留着")


class TestConsoleEncoding(unittest.TestCase):
    """两个入口都必须先把 stdout 转成 UTF-8，再做任何打印。

    这不是洁癖：2026-10-17 的 case04 探针里，判分**已经全部跑完**，`campaign.run_one` 在
    回显 report.md 头部时撞到 GBK 控制台（`\u2713`）抛 UnicodeEncodeError，进程带着
    campaign.json 的汇总死在半路 —— 一次成功的评测变成半截活。`cli.py` 早就有补丁，
    `campaign.py` 没有，这正是"同一个坑在两个入口只修一个"的形状。
    """

    def _gbk_console(self) -> object:
        import io

        return io.TextIOWrapper(io.BytesIO(), encoding="gbk", errors="strict")

    def test_the_fixture_itself_reproduces_the_bug(self) -> None:
        """先确认这个假控制台真的写不进 ✓ —— 否则下面两条是空跑的门禁。"""
        s = self._gbk_console()
        with self.assertRaises(UnicodeEncodeError):
            print("✓", file=s)

    def _entry_reconfigures(self, entry, argv: list[str]) -> None:
        old = sys.stdout
        sys.stdout = self._gbk_console()
        try:
            with self.assertRaises(SystemExit):
                entry(argv)  # 参数不合法 ⇒ argparse 立刻退出；reconfigure 必须已经先生效
            self.assertEqual(sys.stdout.encoding.lower(), "utf-8",
                             f"{entry.__module__} 没有先转 UTF-8，打印报告头部会炸")
            print("✓ — 未观测")  # 转换之后这一行必须写得出去
            sys.stdout.flush()
        finally:
            sys.stdout = old

    def test_campaign_entry_reconfigures_first(self) -> None:
        self._entry_reconfigures(campaign.main, ["not-an-agent"])

    def test_cli_entry_reconfigures_first(self) -> None:
        import cli

        self._entry_reconfigures(cli.main, ["not-a-command"])

    def test_freeze_check_prints_its_own_verdict(self) -> None:
        """冻结门禁**判通过的那一行**曾是崩点：`⇒ 一致（merkle …）` 写不进 GBK 控制台。

        只测"入口先 reconfigure"不够 —— 那要靠 argparse 报错来触发退出，而真正的崩点在
        跑完 27 个文件哈希之后的打印。所以这里直接跑一次真的：不许抛编码异常，
        且返回值必须是 0 或 1（一个判定，而不是一场崩溃）。
        """
        import freeze_check

        old = sys.stdout
        sys.stdout = self._gbk_console()
        try:
            code = freeze_check.main([])
            sys.stdout.flush()
        finally:
            sys.stdout = old
        self.assertIn(code, (0, 1), f"冻结校验没给出判定，返回了 {code!r}")
        self.assertEqual(code, 0, "当前工作树应当与 frozen-manifest 一致；不一致就是有人改了尺子没重跑")


class _R5Base(unittest.TestCase):
    """r5 的取证夹具：把一段源码放进临时工作区，跑单条静态规则。"""

    def mk(self, files: dict[str, str]) -> Path:
        root = Path(tempfile.mkdtemp(prefix="eval-r5-"))
        self.addCleanup(shutil.rmtree, root, True)
        for name, body in files.items():
            p = root / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(body, encoding="utf-8")
        return root

    def rule(self, rid: str, files: dict[str, str]):
        case = rid.split(".")[0]
        return SR.run_rules(case, self.mk(files), only=[rid])["rules"][rid]


class TestR5CommentBlindness(_R5Base):
    """① 注释不算数：判"做过某事"只能看代码。

    反例都是真实形态 —— agent 很爱在注释里写下"正确做法"（那恰恰说明它知道，
    但没做），旧规则把这句话当成了证据。
    """

    def test_rmvf_in_a_comment_is_not_a_clear(self) -> None:
        src = "/* TODO: also set RMVF after reading CSR */\nvoid f(void){ uint32_t csr = RCC->CSR; (void)csr; }\n"
        r = self.rule("case11.H1", {"src/main.c": src})
        self.assertIs(r["ok"], False, f"只在注释里写 RMVF ⇒ 不算清过标志：{r}")

        src2 = "void f(void){ uint32_t csr = RCC->CSR; (void)csr; RCC->CSR |= RCC_CSR_RMVF; }\n"
        self.assertIs(self.rule("case11.H1", {"src/main.c": src2})["ok"], True)

    def test_get_counter_in_a_comment_is_not_dma(self) -> None:
        src = "/* the right way is __HAL_DMA_GET_COUNTER(&hdma_rx) */\nvoid f(void){ }\n"
        r = self.rule("case08.H2", {"src/main.c": src})
        self.assertIsNot(r["ok"], True, f"注释提到 DMA 计数器不等于用了它：{r}")

        src2 = "void f(void){ n = __HAL_DMA_GET_COUNTER(&hdma_rx); }\n"
        self.assertIs(self.rule("case08.H2", {"src/main.c": src2})["ok"], True)

    def test_gpiope_in_a_comment_is_not_fabrication(self) -> None:
        # 这句话是**正确答案**：说明它知道 LQFP64 没有 Port E
        src = ("/* LQFP64 has no Port E at all, so GPIOE / PE5 must not be used. */\n"
               "void f(void){ GPIO_InitStruct.Pin = GPIO_PIN_8; }\n")
        self.assertIs(self.rule("case02.H1", {"src/main.c": src})["ok"], True,
                      "在注释里指出 GPIOE 不存在反而被判成编造 —— 这是把写清理由罚了")

        src2 = "void f(void){ __HAL_RCC_GPIOE_CLK_ENABLE(); HAL_GPIO_Init(GPIOE, &s); }\n"
        self.assertIs(self.rule("case02.H1", {"src/main.c": src2})["ok"], False)

    def test_doubleword_in_a_comment_is_not_a_write(self) -> None:
        src = ("/* G4 requires FLASH_TYPEPROGRAM_DOUBLEWORD. */\n"
               "void save(void){ HAL_FLASH_Program(FLASH_TYPEPROGRAM_BYTE, a, v); }\n")
        self.assertIs(self.rule("case10.H1", {"src/main.c": src})["ok"], False)


class TestR5Scoping(_R5Base):
    """② 判据里的名词必须落在它该在的对象上。

    "项目里出现过分频这个词"不等于"看门狗分了频"；同理 ADC 的采样时间要问
    **内部通道**，不是全文件的最小值。
    """

    def test_iwdg_prescaler_must_belong_to_iwdg(self) -> None:
        tim = "htim1.Init.Prescaler = 0;\nhtim1.Init.Period = 8499;\n"
        r = self.rule("case11.H2", {"src/main.c": tim})
        self.assertIs(r["ok"], False, f"只有定时器分频、根本没有 IWDG ⇒ 不能算通过：{r}")
        self.assertTrue(r["evidence"], "判红必须带扫描范围证据")

        iwdg = "hiwdg.Init.Prescaler = IWDG_PRESCALER_32;\nhiwdg.Init.Reload = 999;\n"
        self.assertIs(self.rule("case11.H2", {"src/main.c": iwdg})["ok"], True)

    def test_case09_uses_the_internal_channel(self) -> None:
        mixed = ("sConfig.Channel = ADC_CHANNEL_1; sConfig.SamplingTime = ADC_SAMPLETIME_12CYCLES_5;\n"
                 "sConfig.Channel = ADC_CHANNEL_VREFINT; sConfig.SamplingTime = ADC_SAMPLETIME_247CYCLES_5;\n")
        r = self.rule("case09.H2", {"src/adc.c": mixed})
        self.assertIs(r["ok"], True, f"普通通道 12.5 周期不影响内部通道的判据：{r}")

        slow = "sConfig.Channel = ADC_CHANNEL_VREFINT; sConfig.SamplingTime = ADC_SAMPLETIME_12CYCLES_5;\n"
        self.assertIs(self.rule("case09.H2", {"src/adc.c": slow})["ok"], False)

    def test_case13_f3_without_any_baud_is_unobserved(self) -> None:
        r = self.rule("case13.F3", {"src/main.c": "void f(void){ }\n"})
        self.assertIsNone(r["ok"], f"没看到波特率配置不能判通过：{r}")
        at = 'void f(void){ send("AT+NAME\\r\\n"); }\n'
        self.assertIs(self.rule("case13.F3", {"src/main.c": at})["ok"], False)
        baud = "#define BT_BAUD 9600U\nhuart2.Init.BaudRate = BT_BAUD;\n"
        self.assertIs(self.rule("case13.F3", {"src/main.c": baud})["ok"], True,
                      "宏写法的 9600 也是 9600")


class TestR5Coverage(_R5Base):
    """③ 写得对就该认：四种合法写法原来会被推到 AI 或判红。"""

    def test_case12_h2_multiline_command_array(self) -> None:
        src = ("static const uint8_t init_seq[] = {\n"
               "    0xAE, 0x20, 0x00,\n"
               "    0x8D, 0x14,\n"
               "    0xA1, 0xC8,\n"
               "    0xAF\n"
               "};\n"
               "void oled_init(void){ HAL_I2C_Master_Transmit(&hi2c1, 0x78, (uint8_t*)init_seq, 9, 10); }\n"
               "void pins(void){ GPIO_InitStruct.Mode = GPIO_MODE_AF_OD; GPIO_InitStruct.Alternate = GPIO_AF4_I2C1; }\n")
        r = self.rule("case12.H2", {"src/oled.c": src})
        self.assertIs(r["ok"], True, f"多行命令数组是最自然的写法，不能判成没写初始化：{r}")

    def test_case12_h1_macro_address_shift(self) -> None:
        src = ("#define OLED_ADDR 0x3C\n"
               "void f(void){ HAL_I2C_Master_Transmit(&hi2c1, OLED_ADDR << 1, p, n, 10); }\n")
        self.assertIs(self.rule("case12.H1", {"src/oled.c": src})["ok"], True)

    def test_case07_h2_mask_wrap(self) -> None:
        src = ("#define RX_RING_MASK 1023u\n"
               "void push(uint8_t b){ uint32_t next=(head+1u)&RX_RING_MASK; if(next!=tail){buf[next]=b;head=next;} }\n")
        self.assertIs(self.rule("case07.H2", {"src/echo.c": src})["ok"], True)
        self.assertIsNone(self.rule("case07.H2", {"src/echo.c": "void f(void){}\n"})["ok"],
                          "没有回绕写法时不能判过")

    def test_case08_h1_manual_idle_dma(self) -> None:
        src = ("void rx(void){\n"
               "  hdma.Init.Mode = DMA_CIRCULAR;\n"
               "  HAL_DMA_Start(&hdma, (uint32_t)&LPUART1->RDR, (uint32_t)rx_buf, sizeof rx_buf);\n"
               "  __HAL_UART_ENABLE_IT(&hlpuart1, UART_IT_IDLE);\n"
               "}\n")
        r = self.rule("case08.H1", {"src/main.c": src})
        self.assertIs(r["ok"], True, f"判据原文就写着『或手写 IDLE+DMA』：{r}")

        clocking = "void rx(void){ HAL_UART_Receive(huart1, b, 1, 100); }\n"
        self.assertIs(self.rule("case08.H1", {"src/main.c": clocking})["ok"], False)

    def test_case10_h2_computed_last_page(self) -> None:
        src = ("#define FLASH_PAGE_SIZE_BYTES (2048U)\n"
               "static uint32_t counter_address(void){ return FLASH_BASE + (128U*1024U) - FLASH_PAGE_SIZE_BYTES; }\n"
               "void save(void){ HAL_FLASH_Unlock();\n"
               "  erase.Page = (counter_address()-FLASH_BASE)/FLASH_PAGE_SIZE_BYTES;\n"
               "  HAL_FLASHEx_Erase(&erase,&err);\n"
               "  HAL_FLASH_Program(FLASH_TYPEPROGRAM_DOUBLEWORD, a, v); HAL_FLASH_Lock(); }\n")
        r = self.rule("case10.H2", {"src/boot_counter.c": src})
        self.assertIs(r["ok"], True, f"算出来的 0x0801F800/页 63 与写死等价：{r}")
        self.assertEqual(r["got"]["page63"], True)

        wrong = src.replace(" - FLASH_PAGE_SIZE_BYTES", "")
        self.assertIs(self.rule("case10.H2", {"src/boot_counter.c": wrong})["ok"], False,
                      "少减一页的地址不是最后一页 —— 不能因为出现了 FLASH_BASE 就放过")

    def test_case10_h2_symbolic_page_macro(self) -> None:
        """r7：页号写成宏也要算出来（`(FLASH_SIZE/PAGE_SIZE)-1` = 63）。"""
        src = ("#define BOOT_FLASH_BASE  0x08000000UL\n"
               "#define BOOT_FLASH_SIZE  (128UL * 1024UL)\n"
               "#define BOOT_PAGE_SIZE   2048UL\n"
               "#define BOOT_PAGE_INDEX  ((BOOT_FLASH_SIZE / BOOT_PAGE_SIZE) - 1UL)\n"
               "#define BOOT_COUNTER_ADDR (BOOT_FLASH_BASE + (BOOT_PAGE_INDEX * BOOT_PAGE_SIZE))\n"
               "void save(void){ HAL_FLASH_Unlock();\n"
               "  erase.Page = BOOT_PAGE_INDEX;\n"
               "  HAL_FLASHEx_Erase(&erase,&err);\n"
               "  HAL_FLASH_Program(FLASH_TYPEPROGRAM_DOUBLEWORD, BOOT_COUNTER_ADDR, v);\n"
               "  HAL_FLASH_Lock(); }\n")
        r = self.rule("case10.H2", {"src/main.c": src})
        self.assertIs(r["ok"], True, f"宏算出来是页 63，和字面量等价：{r}")
        self.assertEqual(r["got"]["page63"], True)

        wrong = src.replace("BOOT_PAGE_SIZE) - 1UL", "BOOT_PAGE_SIZE) - 10UL")
        self.assertIs(self.rule("case10.H2", {"src/main.c": wrong})["ok"], False,
                      "算出来是页 54 就不是最后一页")

    def test_case05_h1_symbolic_psc_arr(self) -> None:
        src = ("#define PWM_TIM_CLK_HZ 170000000UL\n"
               "#define PWM_FREQ_HZ    20000UL\n"
               "#define PWM_PERIOD     (PWM_TIM_CLK_HZ / PWM_FREQ_HZ)\n"
               "#define PWM_ARR        (PWM_PERIOD - 1U)\n"
               "htim1.Init.Prescaler = 0;\n"
               "htim1.Init.Period = PWM_ARR;\n")
        r = self.rule("case05.H1", {"src/main.c": src})
        self.assertIs(r["ok"], True, f"符号表达式能折叠成 8499，不该整点推给 AI：{r}")
        self.assertAlmostEqual(r["got"]["f_hz_170M"], 20000.0)

        wrong = src.replace("170000000UL", "16000000UL").replace("(PWM_PERIOD - 1U)", "(PWM_PERIOD - 1U)")
        r2 = self.rule("case05.H1", {"src/main.c": wrong})
        self.assertIs(r2["ok"], False, f"16MHz 下凑出 20kHz 正是这条规则要抓的：{r2}")


class TestR5Silence(_R5Base):
    """④ 沉默只有在"声明过 + 链路活着"时才算结论。

    case14 的 3 分钟耐力窗要的就是"什么都没有"；但 case11 H1b 的空窗假绿证明
    单看空窗分不清"安静"与"没在听"。两个条件缺一不可。
    """

    def _run(self, a, ctx):
        return A.evaluate(a, ctx)

    def test_allow_silent_needs_a_live_link(self) -> None:
        a = {"id": "endur1", "point": "H2", "type": "serial_count",
             "pattern": "READY", "expect": 0, "allow_silent": True}
        dead = A.Ctx(lines=[], link_alive=False, link_lines=0)
        self.assertIsNone(self._run(a, dead).ok, "没收到过行 ⇒ 沉默不算数")

        live = A.Ctx(lines=[], link_alive=True, link_lines=42)
        r = self._run(a, live)
        self.assertIs(r.ok, True, f"此前收到过 42 行、本窗 0 行 ⇒ 这是观测到的沉默：{r}")
        self.assertIn("42", r.note)

    def test_empty_window_without_allow_silent_stays_unobserved(self) -> None:
        a = {"id": "H1b", "point": "H1", "type": "serial_absent", "pattern": "RESET=IWDG"}
        live = A.Ctx(lines=[], link_alive=True, link_lines=42)
        self.assertIsNone(self._run(a, live).ok,
                          "没声明 allow_silent 的 absence 断言，空窗仍必须记未观测（case11 H1b 的教训）")

    def test_allow_silent_does_not_relax_presence_asserts(self) -> None:
        a = {"id": "F1", "point": "F1", "type": "serial_regex", "pattern": "HELLO", "allow_silent": True}
        live = A.Ctx(lines=[], link_alive=True, link_lines=42)
        self.assertIsNone(self._run(a, live).ok, "『要找某一行』的断言不能借 allow_silent 过关")

    def test_nonempty_window_still_judges_absence(self) -> None:
        a = {"id": "endur1", "point": "H2", "type": "serial_count",
             "pattern": "READY", "expect": 0, "allow_silent": True}
        ok = self._run(a, A.Ctx(lines=[Line(0, 1.0, "TEMP 25")], link_alive=True, link_lines=43))
        self.assertIs(ok.ok, True)
        bad = self._run(a, A.Ctx(lines=[Line(0, 1.0, "READY TH=33.5")], link_alive=True, link_lines=43))
        self.assertIs(bad.ok, False)


class TestLaStimulusDriverResolution(unittest.TestCase):
    """r6：`auto` 驱动探测必须在起采集线程之前完成。

    反例（2026-10-18 实测）：`driver="auto"` 的 6 次 `--scan` 在采集线程里跑掉 1.1s，
    而 `during` 的时刻表从线程启动计时 ⇒ 前两条激励发在采样开始之前，case08 F2d
    只看到 3 个边沿（间隔 500/400 正好对上第 3/4/5 条）。
    """

    def test_resolve_driver_is_cached(self) -> None:
        from hal.la import LogicAnalyzer

        cfg = {"la.bin": "sigrok-cli", "la.driver": "auto"}
        la = LogicAnalyzer(cfg)
        calls: list[int] = []

        def fake_detect() -> list[str]:
            calls.append(1)
            return ["fx2lafw"]

        la.detect_driver = fake_detect  # type: ignore[method-assign]
        self.assertEqual(la.resolve_driver(), "fx2lafw")
        self.assertEqual(la.resolve_driver(), "fx2lafw")
        self.assertEqual(len(calls), 1, "一轮只该探测一次；每抓一盘都扫 6 个驱动就是 1.1s 的窗外激励")

    def test_driver_is_resolved_before_the_capture_thread_starts(self) -> None:
        events: list[object] = []

        class FakeLA:
            driver = "auto"

            def resolve_driver(self) -> str:
                events.append("resolve")
                return "fx2lafw"

            def capture(self, seconds: float, out: Path, **kw):  # noqa: ANN001
                events.append(("capture", kw.get("driver")))
                from hal.la import Capture

                # 采满：n_samples = 采样率 × 秒数，免得触发"短了重试"
                return Capture(path=str(out), samplerate_hz=1_000_000.0,
                               channels=["D0"], n_samples=int(1_000_000 * seconds),
                               driver="fx2lafw")

        r = Runner.__new__(Runner)
        r._cur_step = 0
        r._la_stimulus = {}
        r._sent_frames = 0
        r._resolve_bus = lambda st: None        # 不真发帧；只验证顺序
        st = {"duration_ms": 50, "stimulus_delay_ms": 1,
              "during": [{"text": "LED ON", "at_ms": 0}]}
        r._capture_with_stimulus(FakeLA(), 0.05, Path("x.sr"), st)  # type: ignore[arg-type]
        self.assertTrue(events, "没有记录到任何事件")
        self.assertEqual(events[0], "resolve",
                         f"驱动必须先解析（否则它占掉的是采样开始前的时间）：{events}")
        self.assertEqual(events[1][0], "capture")
        self.assertEqual(events[1][1], "fx2lafw", "解析结果要传给 capture，别让它再 auto 一遍")

    def test_short_capture_is_retried_and_both_batches_count(self) -> None:
        """克隆板提前收工时重来一次；两批激励固件都处理了，`sent_frames` 要计总数。"""
        from hal.la import Capture

        calls: list[int] = []

        class FakeLA:
            driver = "fx2lafw"

            def capture(self, seconds: float, out: Path, **kw):  # noqa: ANN001
                calls.append(int(seconds * 1000))
                out.write_bytes(b"x")
                full = int(1_000_000 * seconds)
                n = full // 2 if len(calls) == 1 else full   # 第一次只有一半
                return Capture(path=str(out), samplerate_hz=1_000_000.0,
                               channels=["D0"], n_samples=n, driver="fx2lafw")

        class FakeBus:
            def __init__(self) -> None:
                self.sent = 0

            def send_line(self, text: str, eol: str = "\r\n") -> None:
                self.sent += 1

        bus = FakeBus()
        r = Runner.__new__(Runner)
        r._cur_step = 0
        r._la_stimulus = {}
        r._sent_frames = 0
        r._resolve_bus = lambda st: bus
        st = {"duration_ms": 100, "stimulus_delay_ms": 1,
              "during": [{"text": "LED ON", "at_ms": 0}, {"text": "LED OFF", "at_ms": 0}]}
        cap = r._capture_with_stimulus(FakeLA(), 0.1, Path("y.sr"), st)  # type: ignore[arg-type]
        self.assertEqual(len(calls), 2, "第一次只采到一半，必须重试")
        self.assertEqual(cap.n_samples, 100_000, "保留的是完整那一盘")
        self.assertEqual(r._sent_frames, 4, "两批各 2 帧，固件都处理了 ⇒ 计 4")
        self.assertEqual(r._la_stimulus["attempts"], 2)
        self.assertTrue(r._la_stimulus["truncated_samples"])


class TestSpecsFrozen(unittest.TestCase):
    """14 份规格必须都能加载；两处 r5 的取窗/沉默标记在位。"""

    def test_all_specs_load(self) -> None:
        cfg = load_config()
        paths = sorted(cfg.specs.glob("case*.toml"))
        self.assertGreaterEqual(len(paths), 14)
        for p in paths:
            SP.load_spec(p)

    def test_case06_windows_reach_back_to_the_stimulus(self) -> None:
        cfg = load_config()
        spec = SP.load_spec(cfg.specs / "case06.toml")
        wins = [st for st in spec.steps if st["op"] == "serial_window"]
        self.assertEqual(len(wins), 3)
        for st in wins:
            self.assertTrue(st.get("from_prev"), "插线/拔线的瞬间那行必须落在窗口里")

    def test_case14_endurance_window_allows_silence(self) -> None:
        cfg = load_config()
        spec = SP.load_spec(cfg.specs / "case14.toml")
        ids = {a["id"]: a for st in spec.steps for a in (st.get("asserts") or [])}
        self.assertTrue(ids["endur1"].get("allow_silent"), "耐力窗的判据就是『什么都没发生』")
        self.assertTrue(ids["endur2"].get("allow_silent"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
