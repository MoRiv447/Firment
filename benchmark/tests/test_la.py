"""逻辑分析仪：用**合成抓包**验证 srzip 解析与波形测量。

真机没接之前，能先钉住的部分是：容器解析（位打包、通道序）、周期/占空比
算法、通道活动表。不能钉住的只有 sigrok-cli 的调用本身。
"""

from __future__ import annotations

import unittest
import zipfile
from pathlib import Path

import sys

EVAL = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(EVAL))

from hal import la as LA  # noqa: E402


def make_srzip(path: Path, *, samplerate: int = 4_000_000, n_channels: int = 8,
               freq_hz: float = 20_000.0, duty_pct: float = 30.0,
               duration_s: float = 0.005, high_channel_rest: bool = False,
               chunk_bytes: int = 1024) -> None:
    """造一个 srzip：**通道 0** = 指定频率/占空比的方波，其余通道恒定。

    **必须按真机的格式造**（这是踩过坑的地方）：
      - 一个采样占 `unitsize` 字节；8 通道时 1 字节/采样，**bit b = 通道 b（LSB first）**
      - 整条流按固定块大小切成 `logic-1-1`、`logic-1-2`…（**不是每通道一个文件**）
      - 通道名写在 metadata 的 `probeK=` 里

    早先这个 fixture 按"每文件一个通道"造 —— 那正是**我当时的错误理解**，
    于是它一直"通过"，而真机的抓包一进来就解析错了（5 块被当成 5 个通道）。
    **用同一个错误心智模型写的测试，只会确认这个 bug。**
    """
    n = int(samplerate * duration_s)
    period = samplerate / freq_hz
    high_len = period * duty_pct / 100.0
    chan_bits = [[1 if (i % period) < high_len else 0 for i in range(n)]]
    for _ in range(n_channels - 1):
        chan_bits.append([1 if high_channel_rest else 0] * n)

    # 交织：第 i 个采样的 8 个通道挤进 1 字节
    blob = bytearray()
    for i in range(n):
        byte = 0
        for b in range(n_channels):
            byte |= (chan_bits[b][i] & 1) << b
        blob.append(byte)

    meta = [
        "[device 1]",
        "capturefile=logic-1",
        f"total probes={n_channels}",
        f"samplerate={samplerate}",
        "unitsize=1",
        "",
    ] + [f"probe{k}=D{k - 1}" for k in range(1, n_channels + 1)]

    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("version", "2")
        zf.writestr("metadata", "\n".join(meta))
        for k in range(0, len(blob), chunk_bytes):
            zf.writestr(f"logic-1-{k // chunk_bytes + 1}", bytes(blob[k:k + chunk_bytes]))


class TestSrzipParsing(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(__file__).parent / "_la-tmp.sr"
        make_srzip(self.tmp)

    def tearDown(self) -> None:
        self.tmp.unlink(missing_ok=True)

    def test_parse_metadata_and_channels(self) -> None:
        cap = LA.parse_srzip(self.tmp, "fx2lafw")
        self.assertEqual(cap.samplerate_hz, 4_000_000)
        self.assertEqual(cap.n_samples, 20_000)
        # 通道号 0 起，8 通道 → D0..D7
        self.assertEqual(sorted(cap.bits), [f"D{i}" for i in range(8)])

    def test_frequency_and_duty_on_ch0(self) -> None:
        cap = LA.parse_srzip(self.tmp, "fx2lafw")
        f = LA.measure_frequency(cap.bits["D0"], cap.samplerate_hz)
        d = LA.measure_duty(cap.bits["D0"], cap.samplerate_hz)
        self.assertAlmostEqual(f["hz"], 20_000.0, delta=100.0)
        self.assertAlmostEqual(d["pct"], 30.0, delta=1.0)
        self.assertEqual(f["confidence"], "high")

    def test_flat_channels_report_no_signal(self) -> None:
        cap = LA.parse_srzip(self.tmp, "fx2lafw")
        act = cap.activity
        self.assertGreater(act["D0"]["edges"], 100)
        self.assertEqual(act["D1"]["edges"], 0)
        self.assertIn("无跳变", act["D1"]["hint"])

    def test_wrong_channel_fails_loudly_not_silently(self) -> None:
        """夹错通道的后果必须是"量不到"，而不是"通过" —— 这是标定安全的底线。"""
        cap = LA.parse_srzip(self.tmp, "fx2lafw")
        f = LA.measure_frequency(cap.bits["D3"], cap.samplerate_hz)
        self.assertEqual(f["hz"], 0.0)
        self.assertEqual(f["confidence"], "low")


class TestChannelMapping(unittest.TestCase):
    class _Cfg:
        def __init__(self, mapping):
            self._m = mapping

        def get(self, key, default=None):
            if key == "la.channel_map":
                return self._m
            if key == "la.bin":
                return "sigrok-cli"
            if key == "la.driver":
                return "fx2lafw"
            if key == "la.samplerate":
                return "4m"
            return default

    def test_explicit_map_wins(self) -> None:
        la = LA.LogicAnalyzer(self._Cfg({"PA8": 3}))
        self.assertEqual(la.channel_of("PA8"), 3)

    def test_no_digit_guessing(self) -> None:
        """PA8 绝不许被猜成通道 8：8 通道的板子上根本没有 CH8，
        猜错的后果是"量到一根没接的通道"，很难查。"""
        la = LA.LogicAnalyzer(self._Cfg({}))
        with self.assertRaises(LA.LaError):
            la.channel_of("PA8")


if __name__ == "__main__":
    unittest.main(verbosity=2)
