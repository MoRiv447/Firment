"""血统闸门：包装测试集里那份 ``runner/verify_conversion.py``。

**刻意不重写规则**：判定以那份脚本的退出码为准，本模块只把它的输出解析成
结构化数据。理由是同一条规则存在两处实现，早晚会分叉，而分叉的那一天
没人会发现 —— 血统闸门一旦悄悄放宽，整份成绩就不可信了。
"""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from hal import proc


@dataclass
class Bloodline:
    ok: bool
    exit_code: int | None
    core_total: int = 0
    identical: int = 0
    modified: list[str] = field(default_factory=list)
    core_missing: list[str] = field(default_factory=list)
    bsp_modified: list[str] = field(default_factory=list)
    has_platformio_ini: bool = False
    firmware_elf: str | None = None
    raw: str = ""
    summary: str = ""
    # 「工作区不存在」「没做过转换」「血统违规」是三件事：
    # 前两者是"这题没跑成"，只有第三种才该按 0 分算。混在一起的话，一次没跑成的
    # 触发会把总分拉下来，而报告上看起来还像是被判了违规。
    # 注意：**合规与否的判定仍然完全来自那份脚本的退出码**（不重写规则），
    # 这里只是把"为什么不合规"分类。
    no_workspace: bool = False
    not_converted: bool = False

    @property
    def not_run(self) -> bool:
        return self.no_workspace or self.not_converted

    def as_dict(self) -> dict[str, Any]:
        return {
            "ok": self.ok,
            "no_workspace": self.no_workspace,
            "not_converted": self.not_converted,
            "exit_code": self.exit_code,
            "core_total": self.core_total,
            "identical": self.identical,
            "modified": self.modified,
            "core_missing": self.core_missing,
            "bsp_modified": self.bsp_modified,
            "platformio_ini": self.has_platformio_ini,
            "firmware_elf": self.firmware_elf,
            "summary": self.summary,
        }


def verify(cfg: Any, workdir: Path) -> Bloodline:
    workdir = Path(workdir)
    if not workdir.is_dir():
        return Bloodline(
            ok=False, exit_code=None, no_workspace=True,
            summary=f"工作区不存在：{workdir}（这题还没跑，不是血统违规）",
        )

    script = Path(cfg.testset) / "runner" / "verify_conversion.py"
    if not script.exists():
        return Bloodline(ok=False, exit_code=None, summary=f"找不到血统校验脚本：{script}")

    python = cfg.get("paths.python") or sys.executable
    res = proc.run([str(python), str(script), str(workdir)], timeout_s=180)
    out = res.stdout + res.stderr
    bl = _parse(out, res.returncode)
    if res.returncode not in (0, 1):
        bl.summary = f"血统脚本异常退出（code={res.returncode}）：{res.tail(5)}"
    return bl


def _parse(text: str, code: int | None) -> Bloodline:
    modified: list[str] = []
    missing: list[str] = []
    bsp_bad: list[str] = []
    core_total = identical = 0
    has_ini = False
    elf: str | None = None

    for line in text.splitlines():
        s = line.strip()
        if s.startswith("MODIFIED") and "Core/" in s:
            m = re.match(r"MODIFIED\s+Core/(\S+)\s+->\s+(.*)$", s)
            if m:
                modified.append(f"Core/{m.group(1)} -> {m.group(2).strip()}")
        elif s.startswith("MISSING") and "Core/" in s:
            m = re.match(r"MISSING\s+Core/(\S+)", s)
            if m:
                missing.append(f"Core/{m.group(1)}")
        elif s.startswith("MODIFIED") and "BSP/" in s:
            m = re.match(r"MODIFIED\s+BSP/(\S+)", s)
            if m:
                bsp_bad.append(f"BSP/{m.group(1)}")
        elif s.startswith("Core:") and "件" in s:
            m = re.search(r"Core:\s*(\d+)\s*件,\s*identical=(\d+)", s)
            if m:
                core_total, identical = int(m.group(1)), int(m.group(2))
        elif s.startswith("platformio.ini:"):
            has_ini = "OK" in s
        elif s.startswith("firmware.elf:"):
            val = s.split(":", 1)[1].strip()
            elf = None if "not built" in val else val

    ok = code == 0
    # 退出码说"不合规"，但成因要分清：没有 Core MISSING、没有 BSP 被改、
    # 也没有 platformio.ini ⇒ 这个工作区只是"还没被转换过"，不是违规。
    not_converted = (not ok) and not missing and not bsp_bad and not has_ini
    if not_converted:
        summary = "工作区里没有任何转换产物（无 platformio.ini、Core 无缺失）—— 这题还没跑成，不是血统违规"
    elif ok:
        summary = "合规"
    else:
        summary = "违规：" + ", ".join(missing + bsp_bad + ([] if has_ini else ["缺 platformio.ini"]))
    return Bloodline(
        ok=ok,
        exit_code=code,
        core_total=core_total,
        identical=identical,
        modified=modified,
        core_missing=missing,
        bsp_modified=bsp_bad,
        has_platformio_ini=has_ini,
        firmware_elf=elf,
        raw=text,
        summary=summary,
        not_converted=not_converted,
    )
