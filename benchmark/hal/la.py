"""逻辑分析仪：抓波形 + 测频率/占空比/边沿 + 协议解码。

**状态：未经真机验证。** 本机没装 sigrok-cli，用户的分析仪型号也还没确认
（plan §11 待确认项 1）。所以这里的代码是照着 sigrok-cli 的接口与 srzip
容器格式写的，第一次接上仪器时请按 README 的"首次标定"流程走一遍。

两个刻意的设计：

* **抓包落 srzip，自己解析。** ``-O srzip`` 出的就是一个 zip：``metadata``
  给出采样率与通道布局，``logic-1-*`` 是位打包的样本。自己解析比解析
  sigrok 的文本输出稳，也不受版本间格式微调的影响。
* **先标定再判定。** 用 ESP32 输出一个已知频率，看分析仪读数偏多少 ——
  这个偏差要计入 case05 那种 ±1.5% 的容差，否则是拿未校准的尺子量人。
"""

from __future__ import annotations

import struct
import zipfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from hal import proc

# 常见廉价分析仪的驱动优先级。fx2lafw 覆盖绝大多数 8ch 24MHz 克隆板。
DRIVER_CANDIDATES = (
    "fx2lafw",
    "dreamsourcelab-dslogic",
    "saleae-logic16",
    "saleae-logic-pro",
    "asix-sigma",
    "hantek-6xxx",
    "uni-t-ut32x",
    "demo",
)


class LaError(RuntimeError):
    pass


@dataclass
class Capture:
    path: str
    samplerate_hz: float
    channels: list[str]
    n_samples: int
    driver: str
    bits: dict[str, list[int]] = field(default_factory=dict)
    activity: dict[str, dict[str, Any]] = field(default_factory=dict)

    def as_dict(self) -> dict[str, Any]:
        return {
            "path": self.path,
            "driver": self.driver,
            "samplerate_hz": self.samplerate_hz,
            "n_samples": self.n_samples,
            "channels": self.channels,
            "activity": self.activity,
        }


class LogicAnalyzer:
    def __init__(self, cfg: Any) -> None:
        self.cfg = cfg
        self.bin: str | None = proc.which(cfg.get("la.bin", "sigrok-cli"))
        self.driver: str = cfg.get("la.driver", "auto")
        self.samplerate: str = cfg.get("la.samplerate", "4m")
        # 引脚名 → 分析仪通道号。接好线之后在 config 里填，别在代码里写死。
        self.channel_map: dict[str, int] = {
            str(k): int(v) for k, v in (cfg.get("la.channel_map", {}) or {}).items()
        }
        # `driver="auto"` 的探测要起 6 次 `sigrok-cli --scan`（约 1.1s，实测 2026-10-18）。
        # 这件事**必须**发生在采集线程启动之前：窗内激励的时刻表是从线程启动算的，
        # 探测若在采样真正开始前占掉 1.1s，前两条激励就落在采集窗之外 ——
        # case08 F2d 的 3 个边沿（应为 5）就是这么来的。探测结果缓存，一轮只做一次。
        self._resolved: str | None = None

    def available(self) -> bool:
        return self.bin is not None

    def resolve_driver(self) -> str:
        """把 `auto` 解析成一个具体驱动（并缓存）。"""
        if self.driver != "auto":
            return self.driver
        if self._resolved is None:
            found = self.detect_driver()
            if not found:
                raise LaError("没有扫到任何分析仪驱动：确认仪器插好、驱动装对")
            self._resolved = found[0]
        return self._resolved

    def detect_driver(self) -> list[str]:
        """列出本机 sigrok-cli 能扫到的驱动（不接仪器也能跑，用于确认工具装好了）。"""
        if not self.bin:
            return []
        found: list[str] = []
        for drv in DRIVER_CANDIDATES:
            res = proc.run([self.bin, "-d", drv, "--scan"], timeout_s=15)
            text = res.stdout + res.stderr
            if res.ok and "no devices" not in text.lower() and text.strip():
                found.append(drv)
        return found

    def channel_of(self, pin: str) -> int:
        """引脚名 → 分析仪通道号（**0 起**，对应板上丝印 CH0..CH7）。

        刻意**不做**"从引脚名里抠数字"的猜测：`PA8` → 8 在 8 通道分析仪上
        根本不存在，而缺失通道会被当成"没信号"静默掉。宁可在这里报错，
        让人去填 `[la.channel_map]`。
        """
        if pin in self.channel_map:
            return self.channel_map[pin]
        raise LaError(
            f"config.toml 的 [la.channel_map] 里没有 {pin} 的映射。"
            f"已配：{sorted(self.channel_map) or '（空）'}；"
            "分析仪板上丝印是 CH0..CH7，填 0..7。"
        )

    # -- 抓包 -----------------------------------------------------------
    def capture(self, seconds: float, out: Path, *, driver: str | None = None,
                samplerate: str | None = None, channels: str | None = None,
                timeout_s: float = 60.0) -> Capture:
        if not self.bin:
            raise LaError("sigrok-cli 不在 PATH 上（见 README 的安装与标定）")
        drv = driver or self.driver
        if drv == "auto":
            drv = self.resolve_driver()
        sr = samplerate or self.samplerate
        samples = _samples_for(sr, seconds)
        out.parent.mkdir(parents=True, exist_ok=True)
        argv = [
            self.bin, "-d", drv,
            "--config", f"samplerate={sr}",
            "--samples", str(samples),
            "-O", "srzip", "--output-file", str(out),
        ]
        if channels:
            argv += ["-C", channels]
        res = proc.run(argv, timeout_s=timeout_s)
        if not res.ok:
            raise LaError(f"抓包失败：{res.tail(6)}")
        return parse_srzip(out, drv)

    # -- 协议解码 -------------------------------------------------------
    def decode(self, capture: Capture, decoder: str, *, options: str = "") -> str:
        """``sigrok-cli -i x.sr -P i2c:scl=1:sda=2 -A i2c`` 的文本输出。"""
        if not self.bin:
            raise LaError("sigrok-cli 不在 PATH 上")
        spec = f"{decoder}:{options}" if options else decoder
        res = proc.run([self.bin, "-i", capture.path, "-P", spec, "-A", decoder], timeout_s=60)
        return res.stdout + res.stderr



def parse_rate(text: str) -> float:
    """把 sigrok 写的采样率字符串转成 Hz。

    **srzip 的 metadata 里 samplerate 是给人看的字符串**，形如 ``"250 kHz"`` /
    ``"8 MHz"`` / ``"1000000"`` —— 直接 ``float()`` 会抛
    ``could not convert string to float: '250 kHz'``（实测踩过，抓包成功但解析炸）。
    """
    s = str(text).strip().lower().replace(" ", "")
    mult = {"k": 1e3, "m": 1e6, "g": 1e9}
    if s.endswith("hz"):
        s = s[:-2]
    if s and s[-1] in mult:
        return float(s[:-1]) * mult[s[-1]]
    return float(s)


def _samples_for(samplerate: str, seconds: float) -> int:
    mult = {"k": 1e3, "m": 1e6, "g": 1e9}
    s = samplerate.strip().lower()
    try:
        if s and s[-1] in mult:
            hz = float(s[:-1]) * mult[s[-1]]
        else:
            hz = float(s)
    except ValueError as exc:
        raise LaError(f"采样率写法不认识：{samplerate!r}") from exc
    return max(16, int(hz * seconds))


# ---------------------------------------------------------------------------
# srzip 解析
# ---------------------------------------------------------------------------


def parse_srzip(path: Path, driver: str) -> Capture:
    """解析 sigrok 的 .srzip。

    **关键：`logic-1-N` 不是"第 N 个通道"，而是"第 N 块字节"。**
    **布局**：一个采样占 `unitsize` 字节；8 通道时正好 1 字节/采样，
    **bit b = 通道 b（LSB first）**。写入时按固定块大小（实测 1024 / 2560 字节）
    切成 `logic-1-1`、`logic-1-2`…，所以必须**按编号顺序拼回一条流**再拆通道。
    实测佐证：请求 5000 采样 → 拼接后正好 5000 字节 ✓

    我第一版把每个 `logic-1-N` 当成一个通道 → 一次 5 块的抓包被报成
    "5 个通道"，而数值碰巧看起来合理（都是高），极难发现。判据：抓包字节数
    应等于 `采样数 × unitsize`；对不上就说明结构理解错了。

    通道名以后端给的为准（metadata 里的 `probeK=名字`），不靠编号猜。
    """
    if not Path(path).exists():
        raise LaError(f"抓包文件不存在：{path}")
    with zipfile.ZipFile(path) as zf:
        names = zf.namelist()
        meta = _parse_metadata(zf.read("metadata").decode("utf-8", "replace")) if "metadata" in names else {}
        samplerate = parse_rate(meta.get("samplerate", "0") or "0")
        unitsize = int(meta.get("unitsize", 1) or 1)
        n_probes = int(meta.get("total probes", meta.get("total_probes", 0)) or 0)
        capturefile = meta.get("capturefile", "logic-1")

        chunks = sorted(
            (n for n in names if n == capturefile or n.startswith(capturefile + "-")),
            key=lambda s: int(s.rsplit("-", 1)[1]) if s.rsplit("-", 1)[1].isdigit() else 0,
        )
        if not chunks:
            raise LaError(f"{path} 里没有 {capturefile} 数据")
        blob = b"".join(zf.read(n) for n in chunks)

    # 通道名：metadata 的 probeK 最权威；没有就退回 D0..D(n-1)
    chan_names = [meta.get(f"probe{k}") for k in range(1, n_probes + 1)]
    if not all(chan_names):
        chan_names = [f"D{i}" for i in range(max(n_probes, 1))]

    n_samples = len(blob) // unitsize if unitsize else len(blob)
    bits: dict[str, list[int]] = {}
    for ch_i, name in enumerate(chan_names):
        byte_i = ch_i // 8
        bit_i = ch_i % 8
        if byte_i >= unitsize:
            continue                      # 该通道不在这一版数据里
        bits[name] = [
            (blob[i * unitsize + byte_i] >> bit_i) & 1 for i in range(n_samples)
        ]

    # **复用 channel_activity()**，别在这里另造一份 —— 我重写时另造过一份，
    # 结果丢掉了 hint 字段（测试立刻抓到）。活动表是标定用的，只有一处真相。
    activity = channel_activity(bits)
    return Capture(path=str(path), samplerate_hz=samplerate, channels=list(bits),
                   n_samples=n_samples, driver=driver, bits=bits, activity=activity)

def _parse_metadata(text: str) -> dict[str, str]:
    out: dict[str, str] = {}
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("[") or line.startswith(";"):
            continue
        if "=" in line:
            k, v = line.split("=", 1)
            out[k.strip()] = v.strip()
    return out


def _unpack_lsb(raw: bytes) -> list[int]:
    out: list[int] = []
    for byte in raw:
        for bit in range(8):
            out.append((byte >> bit) & 1)
    return out


# ---------------------------------------------------------------------------
# 波形测量
# ---------------------------------------------------------------------------


def measure_frequency(bits: list[int], samplerate_hz: float) -> dict[str, Any]:
    edges = _edge_indices(bits)
    if len(edges) < 3 or samplerate_hz <= 0:
        return {"hz": 0.0, "n": len(bits), "edges": len(edges), "confidence": "low",
                "note": "边沿太少，量不出周期"}
    periods = [(b - a) / samplerate_hz for a, b in zip(edges, edges[2:]) if b > a]
    if not periods:
        return {"hz": 0.0, "n": len(bits), "edges": len(edges), "confidence": "low"}
    mean_period = sum(periods) / len(periods)
    hz = 1.0 / mean_period if mean_period > 0 else 0.0
    jitter = max(abs(p - mean_period) / mean_period for p in periods)
    return {"hz": round(hz, 2), "n": len(bits), "edges": len(edges),
            "jitter_pct": round(jitter * 100, 3),
            "confidence": "high" if jitter < 0.02 else "medium"}


def measure_duty(bits: list[int], samplerate_hz: float) -> dict[str, Any]:
    if not bits:
        return {"pct": 0.0, "confidence": "low"}
    high = sum(bits)
    pct = high / len(bits) * 100.0
    # 窗口不是整周期时占空比会偏；用一个完整周期重算更稳
    edges = _edge_indices(bits)
    if len(edges) >= 3:
        a, b = edges[0], edges[-1]
        seg = bits[a:b]
        if seg:
            pct = sum(seg) / len(seg) * 100.0
    return {"pct": round(pct, 2), "n": len(bits), "confidence": "high" if len(edges) >= 3 else "medium"}


def count_edges(bits: list[int]) -> dict[str, Any]:
    return {"count": len(_edge_indices(bits)), "n": len(bits)}


def measure_pulse_width(bits: list[int], samplerate_hz: float) -> dict[str, Any]:
    """最短的**完整**高电平脉宽（µs）—— 用来判"PWM 输出没被打印打断"这类缺口。

    首尾两个平台不参与取最短：采集窗可能落在半截高电平里，那一段量到的只是**窗内**
    剩下的部分。实测 case05 F3c 就因此把 15 µs 量成 7.5 µs 判红，而同一盘波形的
    duty=30.0%、frequency=20.05 kHz 两条都是绿的 —— 三者不可能同时为真，红的是窗对齐
    造成的，不是固件。截断只会让读数偏小，所以排除首尾**只可能消掉假红，不会放跑假绿**：
    固件真的把某个高电平打断了，那个短平台必然完整落在窗内。
    """
    runs: list[tuple[int, int]] = []          # (起始下标, 长度)
    cur = 0
    start = 0
    for i, b in enumerate(bits):
        if b:
            if cur == 0:
                start = i
            cur += 1
        elif cur:
            runs.append((start, cur))
            cur = 0
    if cur:
        runs.append((start, cur))

    def whole(seg: tuple[int, int]) -> bool:
        s, ln = seg
        return s > 0 and s + ln < len(bits)

    complete = [seg for seg in runs if whole(seg)]
    if not complete or samplerate_hz <= 0:
        # 不给 us：宁可让断言记"未采集到完整平台"，也不要拿 0 兑换一枚红（零活动的
        # 另一层防线在 runner 的 assert_needs_activity 里）。
        return {"high_runs": len(runs), "complete_runs": 0, "confidence": "low",
                "note": f"窗内 {len(runs)} 段高电平全部被采集窗首尾切断 ⇒ 量不出真实最短脉宽"}
    min_run = min(ln for _, ln in complete)
    return {"us": round(min_run / samplerate_hz * 1e6, 3),
            "high_runs": len(complete),
            "truncated_at_border": len(runs) - len(complete),
            "confidence": "high"}


def _edge_indices(bits: list[int]) -> list[int]:
    return [i for i in range(1, len(bits)) if bits[i] != bits[i - 1]]


def channel_activity(bits: dict[str, list[int]]) -> dict[str, dict[str, Any]]:
    """每根通道的边沿数与高电平占比。

    **这是标定用的**：fx2lafw 的通道号与板上丝印 CH0..CH7 是否对得上、
    探头夹在哪一根上，看一眼这张表就知道 —— 有信号的那根会明显不同。
    没有它，夹错通道的表现只是"频率量出来是 0"，很难判断是固件没输出
    还是自己夹错了。
    """
    out: dict[str, dict[str, Any]] = {}
    for name, bits in bits.items():
        if not bits:
            out[name] = {"edges": 0, "high_pct": 0.0}
            continue
        edges = len(_edge_indices(bits))
        high = sum(bits) / len(bits) * 100.0
        out[name] = {"edges": edges, "high_pct": round(high, 2),
                     "hint": "有信号" if edges >= 2 else "无跳变（未接线或恒定电平）"}
    return out
