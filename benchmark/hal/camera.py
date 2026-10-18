"""相机取帧：ffmpeg + dshow，直接拿灰度原始帧。

不写 PNG 再解码，是因为那样要么装 Pillow、要么自己写 PNG 解码器；而
``-pix_fmt gray -f rawvideo`` 出来的就是每像素一字节的裸数据，Python 直接切片
就能算，零依赖。

**自动曝光的坑**：内置摄像头的自动曝光/自动白平衡会随场景慢变。所以
* 采集前丢几帧（``warmup``），等曝光收敛；
* 判据只用相对量（明暗序列的幅度、周期、帧间差异），不用绝对亮度阈值；
* 每次 run 用当次序列自动定阈值，不沿用上次的。
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

from hal import proc
from hal.vision import Frame


class CameraError(RuntimeError):
    pass


@dataclass
class CaptureResult:
    frames: list[Frame]
    fps: float
    raw_path: str | None
    size: tuple[int, int]
    warmup: int
    note: str = ""

    def as_dict(self) -> dict[str, Any]:
        return {
            "n_frames": len(self.frames),
            "fps": self.fps,
            "size": list(self.size),
            "warmup_dropped": self.warmup,
            "raw": self.raw_path,
            "note": self.note,
        }


class Camera:
    def __init__(self, cfg: Any) -> None:
        self.cfg = cfg
        self.bin: str | None = proc.which(cfg.get("camera.ffmpeg", "ffmpeg"))
        self.device: str = cfg.get("camera.device", "Integrated Camera")
        self.warmup: int = int(cfg.get("camera.warmup_frames", 4))
        # 缩到 160x120 就够量明暗与差异了，还让纯 Python 的逐像素循环跑得动
        size = cfg.get("camera.size", [160, 120])
        self.w = int(size[0])
        self.h = int(size[1])

    def available(self) -> bool:
        return self.bin is not None

    def list_devices(self) -> list[str]:
        if not self.bin:
            return []
        res = proc.run([self.bin, "-hide_banner", "-f", "dshow",
                        "-list_devices", "true", "-i", "dummy"], timeout_s=20)
        text = res.stdout + res.stderr
        names: list[str] = []
        for line in text.splitlines():
            if '"' in line and line.strip().startswith("["):
                for part in line.split('"')[1::2]:
                    names.append(part)
        return names

    def _argv(self, seconds: float, fps: float, out: Path) -> list[str]:
        assert self.bin is not None
        return [
            self.bin, "-hide_banner", "-loglevel", "error",
            "-f", "dshow",
            "-framerate", str(int(fps)),
            "-i", f"video={self.device}",
            "-t", f"{seconds:.2f}",
            "-vf", f"scale={self.w}:{self.h}",
            "-pix_fmt", "gray",
            "-f", "rawvideo",
            "-y", str(out),
        ]

    def record(self, seconds: float, fps: float, *, out: Path | None = None,
               keep_raw: bool = True) -> CaptureResult:
        if not self.bin:
            raise CameraError("ffmpeg 不在 PATH 上")
        tmp = Path(out) if out else Path.home() / ".eval-camera-tmp.raw"
        tmp.parent.mkdir(parents=True, exist_ok=True)
        argv = self._argv(seconds, fps, tmp)
        res = proc.run(argv, timeout_s=max(20.0, seconds * 3 + 15))
        if not res.ok or not tmp.exists():
            raise CameraError(f"取帧失败：{res.tail(6)}")

        blob = tmp.read_bytes()
        fsz = self.w * self.h
        raw_frames = [blob[i:i + fsz] for i in range(0, len(blob) - fsz + 1, fsz)]
        if not raw_frames:
            raise CameraError(f"取到的数据不足一帧（{len(blob)} 字节，帧 {fsz} 字节）")

        note = ""
        if self.warmup and len(raw_frames) > self.warmup + 2:
            raw_frames = raw_frames[self.warmup:]
        elif self.warmup:
            note = f"帧数太少（{len(raw_frames)}），没有丢弃预热帧"

        # 实测教训：请求 20fps 只拿到 16 帧（ffmpeg 起流有延迟）。频率估计
        # 用的是"序列跨度"=(n-1)/fps，若按标称 fps 算，跨度会偏短约 20%，
        # 闪烁频率就被系统性高估。所以按"帧数/实际采集秒数"折算有效帧率。
        eff_fps = len(raw_frames) / seconds if seconds > 0 else fps
        if abs(eff_fps - fps) / fps > 0.1:
            note = (note + f"；标称 {fps:g}fps 实得 {eff_fps:.1f}fps，已按有效帧率折算").strip("；")

        frames = [Frame(t=i / eff_fps, w=self.w, h=self.h, gray=g) for i, g in enumerate(raw_frames)]
        raw_path = None
        if keep_raw:
            raw_path = str(tmp)
        else:
            try:
                tmp.unlink()
            except OSError:
                pass
        return CaptureResult(frames=frames, fps=eff_fps, raw_path=raw_path, size=(self.w, self.h),
                             warmup=self.warmup, note=note)

    def snapshot(self, out_png: Path) -> Path | None:
        """拍一张 PNG 存档 —— 给 AI 评委和人看的，不参与数值判定。"""
        if not self.bin:
            return None
        out_png.parent.mkdir(parents=True, exist_ok=True)
        res = proc.run([
            self.bin, "-hide_banner", "-loglevel", "error",
            "-f", "dshow", "-i", f"video={self.device}",
            "-frames:v", "1", "-y", str(out_png),
        ], timeout_s=30)
        return out_png if res.ok and out_png.exists() else None
