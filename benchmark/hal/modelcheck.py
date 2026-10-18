"""**模型核验**：从 agent 自己的会话记录里读"实际用了什么模型"，与声明比对。

为什么要这个环节：`config.toml` 里 `[agents.*]` 的 model/thinking 是**我们写进去的
声明** —— 如果 agent 因自身配置错误被默认值顶掉，报告照样印声明的值，
**静默回退抓不到**。唯一可信的来源是 agent 自己的会话产物：

- **claude**：`~/.claude/projects/<把工作目录路径里的 \\ / : 换成 ->/*.jsonl`，
  每条 assistant 消息都带 `message.model`（实测确认）。
- **firm**：`~/AppData/Roaming/firment/sessions/<uuid>.jsonl`，**第一行是 `type=meta`**，
  里面直接带 `cwd / provider / model / thinking / mode / context_budget_chars`
  （2026-10-16 实测结构）。按 `cwd` 认出是哪道题的会话，取 mtime 最新的那次。
  同目录下的 `*.events.jsonl` 是事件流，**没有模型字段**，别去那儿找。

核不到就返回 `match=None` 并说明为什么，**绝不把"没核到"写成"核过了"**。
"""

from __future__ import annotations

import json
import os
import re
from pathlib import Path
from typing import Any

FIRM_SESSIONS = Path.home() / "AppData" / "Roaming" / "firment" / "sessions"


def _norm_dir(p: Any) -> str:
    """Windows 路径比对用：大小写与 `\\`/`/` 都不能成为差异。"""
    return os.path.normcase(os.path.normpath(str(p)))


def _munge_cwd(workdir: Path) -> str:
    """claude 把项目目录路径变成目录名：`C:\\x\\y` → `C--x-y`。

    **每个分隔符各变一个 `-`**（`:` 一个、`\\` 一个）—— 不是把连续分隔符合并成
    一个。我第一版用了 `[:\\\\/]+`（带 +），结果 `C--Users` 被我拼成 `C-Users`，
    整个目录差一个字符，"没找到会话记录"其实是"路径拼错了"。

    **空格也换成 `-`**（2026-10-17 实测）：题集路径里带空格（`ai test pro`），
    claude 落盘的目录名是 `...-ai-test-pro-runs-claude-case04`，
    而我们的 munge 保留了空格 ⇒ claude 侧 8 个 run 的模型核验全部假报"没找到会话记录"。
    核验拿不到就是拿不到，绝不能因为"路径拼错"而让整侧失去这道闸。
    """
    s = str(workdir)
    s = re.sub(r"[:\\/\s]", "-", s)
    return s.rstrip("-")


def claude_models_used(workdir: Path) -> list[str]:
    """读 claude 在该工作目录下**最近一次会话**里实际使用的模型（按出现顺序去重）。"""
    root = Path.home() / ".claude" / "projects" / _munge_cwd(workdir)
    files = sorted(root.glob("*.jsonl"), key=lambda f: f.stat().st_mtime, reverse=True)
    if not files:
        return []
    models: list[str] = []
    with files[0].open(encoding="utf-8", errors="replace") as fh:
        for line in fh:
            try:
                d = json.loads(line)
            except Exception:
                continue
            m = (d.get("message") or {}).get("model") or d.get("model")
            if m and m not in models:
                models.append(m)
    return models


def firm_session_meta(workdir: Path, root: Path | None = None) -> tuple[Path, dict] | None:
    """找该工作目录**最近一次** firment 会话的 meta 行，返回 (文件, meta)。

    匹配规则刻意保守：认"会话 `cwd` 就是这道题的目录"或"会话 `cwd` 在它里面"
    （agent 会 `cd cubemx` 再启动），但**会话跑在父目录时不认** —— 否则有人在
    `runs/firm/` 下起一次 firment，14 道题就都会把那次当成自己的会话，
    核验结果全是"看起来对得上"的假绿。
    """
    root = Path(root) if root else FIRM_SESSIONS
    if not root.is_dir():
        return None
    want = _norm_dir(workdir)
    best: tuple[float, Path, dict] | None = None
    for f in sorted(root.glob("*.jsonl"), key=lambda p: p.stat().st_mtime, reverse=True):
        if f.name.endswith(".events.jsonl"):
            continue                          # 事件流里没有模型字段
        try:
            with f.open(encoding="utf-8", errors="replace") as fh:
                first = fh.readline()
            meta = json.loads(first) if first.strip() else None
        except Exception:
            continue
        if not isinstance(meta, dict) or meta.get("type") != "meta":
            continue
        cwd = meta.get("cwd")
        if not cwd:
            continue
        got = _norm_dir(cwd)
        if got != want and not got.startswith(want + os.sep):
            continue
        mt = f.stat().st_mtime
        if best is None or mt > best[0]:
            best = (mt, f, meta)
    return (best[1], best[2]) if best else None


def verify_model(agent: str, workdir: Path, declared: str | None,
                 declared_thinking: str | None = None) -> dict[str, Any]:
    """返回 {"declared", "actual", "match", "source", "note", "thinking_actual"}。

    match 的取值：True（一致）/ False（不一致，**要在报告里标红**）/ None（无法核实）。
    无法核实时**绝不能**当成通过 —— 那和假绿灯是一回事。
    """
    workdir = Path(workdir)
    agent = "claude" if agent == "cc" else agent      # 命令行允许 `cc`
    if agent == "claude":
        models = claude_models_used(workdir)
        if not models:
            return {"declared": declared, "actual": None, "match": None,
                    "thinking_actual": None,
                    "source": "~/.claude/projects", "note": "没找到会话记录"}
        actual = models[-1]                     # 最后用的那个
        match = (declared is not None) and (actual == declared)
        return {"declared": declared, "actual": actual, "match": match,
                "thinking_actual": None, "source": files_str(workdir), "note": ""}
    if agent == "firm":
        hit = firm_session_meta(workdir)
        if hit is None:
            store = "会话目录不存在" if not FIRM_SESSIONS.is_dir() else \
                f"没找到 cwd 在 {workdir} 下的会话（firment 只在自己的目录里记 cwd）"
            return {"declared": declared, "actual": None, "match": None,
                    "thinking_actual": None,
                    "source": str(FIRM_SESSIONS), "note": store}
        path, meta = hit
        actual = meta.get("model")
        thinking = meta.get("thinking")
        note = ""
        if declared_thinking and thinking and declared_thinking != thinking:
            note = f"thinking 声明 `{declared_thinking}` 实际 `{thinking}` ⚠"
        return {"declared": declared, "actual": actual,
                "match": (declared is not None and actual is not None
                          and actual == declared),
                "thinking_actual": thinking, "source": str(path), "note": note}
    return {"declared": declared, "actual": None, "match": None,
            "thinking_actual": None, "source": "-", "note": f"未知 agent：{agent}"}


def files_str(workdir: Path) -> str:
    root = Path.home() / ".claude" / "projects" / _munge_cwd(workdir)
    files = sorted(root.glob("*.jsonl"), key=lambda f: f.stat().st_mtime, reverse=True)
    return str(files[0]) if files else "~/.claude/projects"
