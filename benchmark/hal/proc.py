"""外部进程调用的统一外壳。

所有外部工具调用都走这里，顺便统一三件事：
* 超时（外部工具卡死不能拖垮整轮评测）
* stdout/stderr 分离捕获（证据要落盘）
* Windows 上不弹控制台窗口
"""

from __future__ import annotations

import os
import shutil
import subprocess
import time
from dataclasses import dataclass
from typing import Any


@dataclass
class ProcResult:
    argv: list[str]
    returncode: int | None
    stdout: str
    stderr: str
    duration_s: float
    timed_out: bool = False

    @property
    def ok(self) -> bool:
        return self.returncode == 0 and not self.timed_out

    def tail(self, n: int = 40) -> str:
        text = (self.stdout + self.stderr).strip().splitlines()
        return "\n".join(text[-n:])

    def as_dict(self) -> dict[str, Any]:
        return {
            "argv": self.argv,
            "returncode": self.returncode,
            "timed_out": self.timed_out,
            "duration_s": round(self.duration_s, 3),
            "stdout_tail": "\n".join(self.stdout.strip().splitlines()[-20:]),
            "stderr_tail": "\n".join(self.stderr.strip().splitlines()[-20:]),
        }


def which(name: str) -> str | None:
    return shutil.which(name)


def run(
    argv: list[str],
    *,
    timeout_s: float = 60.0,
    cwd: str | os.PathLike[str] | None = None,
    env: dict[str, str] | None = None,
    stdin_text: str | None = None,
) -> ProcResult:
    full_env = dict(os.environ)
    if env:
        full_env.update(env)
    t0 = time.monotonic()
    try:
        proc = subprocess.run(
            argv,
            cwd=str(cwd) if cwd else None,
            capture_output=True,
            timeout=timeout_s,
            input=stdin_text.encode("utf-8") if stdin_text is not None else None,
            env=full_env,
            # 别让子进程继承我们的 cwd 语义去解析相对路径
            text=False,
        )
        return ProcResult(
            argv=argv,
            returncode=proc.returncode,
            stdout=_decode(proc.stdout),
            stderr=_decode(proc.stderr),
            duration_s=time.monotonic() - t0,
        )
    except subprocess.TimeoutExpired as exc:
        return ProcResult(
            argv=argv,
            returncode=None,
            stdout=_decode(exc.stdout or b""),
            stderr=_decode(exc.stderr or b""),
            duration_s=time.monotonic() - t0,
            timed_out=True,
        )
    except FileNotFoundError as exc:
        return ProcResult(
            argv=argv,
            returncode=None,
            stdout="",
            stderr=f"可执行文件不存在：{exc}",
            duration_s=time.monotonic() - t0,
        )


def _decode(data: bytes) -> str:
    # 外部工具（尤其 ST 的 CLI）在本机会吐 GBK；errors=replace 别让证据丢在编码异常上
    for enc in ("utf-8", "gbk"):
        try:
            return data.decode(enc)
        except UnicodeDecodeError:
            continue
    return data.decode("utf-8", errors="replace")
