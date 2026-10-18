"""图像上的纯测量函数 —— 纯 Python，不依赖 PIL/numpy。

灰度帧就是一段 ``bytes``（每像素一字节，行优先），由 ffmpeg 直接吐出来：
``-pix_fmt gray -f rawvideo``。这样既不用装图像库，也不用解码 PNG。

**为什么只用相对量**：笔记本内置摄像头的自动曝光与自动白平衡会随场景变化，
绝对亮度值不可比。所以这里量的是"帧间差异、明暗序列的幅度与周期、有无台阶"，
这些在自动曝光下依然稳定（它不会在 1/30 秒内把整个场景调没）。

对应 Firment 的 observe 工具那套（brightness/blink/motion/diff），但这里
刻意独立实现：Firment 是被测对象之一，裁判的尺子不该借被测对象的手。
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable, Sequence

Rect = tuple[int, int, int, int]


@dataclass
class Frame:
    t: float
    w: int
    h: int
    gray: bytes

    def clip(self, roi: Rect | None) -> Rect:
        if roi is None:
            return (0, 0, self.w, self.h)
        x, y, w, h = roi
        x = max(0, min(x, self.w - 1))
        y = max(0, min(y, self.h - 1))
        w = max(1, min(w, self.w - x))
        h = max(1, min(h, self.h - y))
        return (x, y, w, h)


def roi_values(f: Frame, roi: Rect | None = None) -> list[int]:
    x, y, w, h = f.clip(roi)
    stride = f.w
    out: list[int] = []
    base = f.gray
    for row in range(y, y + h):
        start = row * stride + x
        out.extend(base[start:start + w])
    return out


def roi_mean(f: Frame, roi: Rect | None = None) -> float:
    vals = roi_values(f, roi)
    return sum(vals) / len(vals) if vals else 0.0


def percentile(vals: Sequence[int], q: float) -> float:
    if not vals:
        return 0.0
    s = sorted(vals)
    idx = min(len(s) - 1, max(0, int(round((len(s) - 1) * q))))
    return float(s[idx])


def lit_fraction(f: Frame, roi: Rect | None, threshold: float) -> float:
    vals = roi_values(f, roi)
    if not vals:
        return 0.0
    return sum(1 for v in vals if v >= threshold) / len(vals)


def diff_fraction(a: Frame, b: Frame, roi: Rect | None, pixel_threshold: int = 25) -> float:
    """两帧之间"变了"的像素比例。要求同尺寸同场景。"""
    if (a.w, a.h) != (b.w, b.h):
        return 0.0
    x, y, w, h = a.clip(roi)
    stride = a.w
    changed = total = 0
    for row in range(y, y + h):
        start = row * stride + x
        pa = a.gray[start:start + w]
        pb = b.gray[start:start + w]
        for va, vb in zip(pa, pb):
            total += 1
            if abs(va - vb) >= pixel_threshold:
                changed += 1
    return changed / total if total else 0.0


def brightness_series(frames: Sequence[Frame], roi: Rect | None) -> list[float]:
    return [roi_mean(f, roi) for f in frames]


def auto_threshold(series: Sequence[float]) -> float:
    """在明暗序列上自动定一个阈值：取全局极值的中点。

    比固定 128 稳：不同光照下 ROI 的绝对亮度差很多，但"亮态"和"暗态"的
    中点总是分得开。幅度太小（<8 luma）时返回中点并让 confidence 说话。
    """
    if not series:
        return 128.0
    lo, hi = min(series), max(series)
    return (lo + hi) / 2.0


def blink_estimate(series: Sequence[float], fps: float) -> dict[str, Any]:
    """从亮度序列估闪烁频率。

    做法：二值化后数穿越次数。
    ``amplitude``（极差）用来判断这次测量到底可不可信 —— 幅度 2 个 luma 的
    "闪烁"是噪声，不是灯。
    """
    if len(series) < 4 or fps <= 0:
        return {"hz": 0.0, "amplitude": 0.0, "confidence": "low", "note": "序列太短"}
    thr = auto_threshold(series)
    amp = max(series) - min(series)
    bits = [1 if v >= thr else 0 for v in series]
    crossings = sum(1 for a, b in zip(bits, bits[1:]) if a != b)
    span = (len(series) - 1) / fps
    hz = (crossings / 2.0) / span if span > 0 else 0.0
    conf = "high" if amp >= 20 else ("medium" if amp >= 8 else "low")
    return {"hz": round(hz, 3), "amplitude": round(amp, 2), "crossings": crossings,
            "threshold": round(thr, 2), "confidence": conf,
            "note": "" if conf != "low" else "明暗幅度过小，可能是场景变化而非目标在亮灭"}


def smoothness(series: Sequence[float]) -> dict[str, Any]:
    """"平滑无阶梯"的量化：离散电平数 + 最长平台长度。

    呼吸灯若用软件步进 GPIO，亮度序列会呈现少量离散电平与长平台；真 PWM
    渐变则有几十个电平、平台极短。
    """
    if len(series) < 4:
        return {"distinct_levels": 0, "max_plateau": 0, "amplitude": 0.0}
    rounded = [round(v) for v in series]
    distinct = len(set(rounded))
    plateau = cur = 1
    for a, b in zip(rounded, rounded[1:]):
        cur = cur + 1 if a == b else 1
        plateau = max(plateau, cur)
    return {
        "distinct_levels": distinct,
        "max_plateau": plateau,
        "amplitude": round(max(series) - min(series), 2),
        "n": len(series),
    }


def motion_series(frames: Sequence[Frame], roi: Rect | None, pixel_threshold: int = 25) -> list[float]:
    return [diff_fraction(a, b, roi, pixel_threshold) for a, b in zip(frames, frames[1:])]


def first_lit(series: Sequence[float], fps: float, threshold: float | None = None) -> dict[str, Any]:
    """序列里第一次"亮起来"的时刻（相对采集起点，毫秒）。

    刻意不把这个当成"上电 N 秒内出画面"的判定依据：ffmpeg 起流本身有
    0.5–1s 的不确定度，拿它去卡 2 秒的判据，误差和判据同量级。所以这里
    只给时间线，判定交 AI/人。
    """
    if not series or fps <= 0:
        return {"first_lit_ms": None, "note": "序列为空"}
    thr = auto_threshold(series) if threshold is None else threshold
    for i, v in enumerate(series):
        if v >= thr:
            return {"first_lit_ms": round(i / fps * 1000.0, 1), "threshold": round(thr, 2),
                    "frame": i, "fps": fps}
    return {"first_lit_ms": None, "threshold": round(thr, 2),
            "note": "整个序列都没超过阈值"}


def summarize(frames: Sequence[Frame], roi: Rect | None, fps: float) -> dict[str, Any]:
    """一次采集的全部测量结果。"""
    series = brightness_series(frames, roi)
    thr = auto_threshold(series)
    diffs = motion_series(frames, roi)
    return {
        "n_frames": len(frames),
        "fps": fps,
        "roi": list(roi) if roi else None,
        "brightness": {
            "mean": round(sum(series) / len(series), 2) if series else 0.0,
            "min": round(min(series), 2) if series else 0.0,
            "max": round(max(series), 2) if series else 0.0,
            "lit_fraction": round(
                sum(lit_fraction(f, roi, thr) for f in frames) / len(frames), 4
            ) if frames else 0.0,
            "threshold": round(thr, 2),
        },
        "first_lit": first_lit(series, fps, thr),
        "series": [round(v, 2) for v in series],
        "blink": blink_estimate(series, fps),
        "smoothness": smoothness(series),
        "motion": {
            "max_changed_fraction": round(max(diffs), 4) if diffs else 0.0,
            "mean_changed_fraction": round(sum(diffs) / len(diffs), 4) if diffs else 0.0,
        },
    }


def diff_pair(before: Frame, after: Frame, roi: Rect | None, pixel_threshold: int = 25) -> dict[str, Any]:
    """两帧的差异。用于 case14 的"按键后第二行切换"。"""
    frac = diff_fraction(before, after, roi, pixel_threshold)
    delta = roi_mean(after, roi) - roi_mean(before, roi)
    return {
        "changed_fraction": round(frac, 4),
        "mean_delta": round(delta, 2),
    }
