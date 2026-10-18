"""给 AI 评委组输入。

设计目标只有一个：**让评委没法只靠"agent 说自己做到了"给分。**

所以 bundle 里给的是四类**可引用**的东西：
1. 机器断言结果（每条带证据文件与行号）
2. 代码 diff（相对 start/cubemx，带文件名与行内容）
3. 会话摘要（工具调用序列 + 考点关键词命中，带行号）
4. judge_table 的判据原文

评委的输出契约要求每个给分点都引用其中之一。引用不到的，只能给低分并标
"需人工"。这条规则是整套自动评分可信度的地基 —— 松一厘米，整份成绩就
变成"agent 自述的转录"。
"""

from __future__ import annotations

import difflib
import json
from pathlib import Path
from typing import Any

from engine import points as P
from engine import spec as SP
from sessions import locate

# 每个 case 的考点关键词：用于在会话里定位"有没有查证/怎么答辩"的时刻。
# 不写成通用的（比如把全部判据词都塞进去），因为命中太杂等于没定位。
KEYWORDS: dict[str, list[str]] = {
    "case01": ["HELLO", "LPUART", "VCP", "PA2", "115200"],
    "case02": ["PE5", "PortE", "GPIOE", "LQFP64", "不存在", "替代", "PA8", "TIM1", "封装"],
    "case03": ["TIM2", "CH1", "PA5", "呼吸", "ARR", "Period", "AF1"],
    "case04": ["去抖", "debounce", "时间戳", "volatile", "EXTI", "临界区", "长按"],
    "case05": ["HSI", "170", "精度", "误差", "20k", "不是正好", "RC", "温漂"],
    "case06": ["输入捕获", "IC", "双边沿", "PA0", "TIM2", "17000", "SELFTEST"],
    "case07": ["Receive_IT", "环形", "ring", "溢出", "阻塞", "STATS"],
    "case08": ["ReceiveToIdle", "DMA", "IDLE", "GET_COUNTER", "CNDTR", "circular"],
    "case09": ["VREFINT", "TS_CAL", "3000", "3.0", "SampleTime", "周期", "折算"],
    "case10": ["DOUBLEWORD", "FLASH_TYPEPROGRAM", "页 63", "0x0801F800", "擦除", "Unlock"],
    "case11": ["RCC_CSR", "RMVF", "LSI", "32", "40", "IWDG 中断", "喂狗", "Prescaler"],
    "case12": ["0x78", "0x3C", "charge pump", "0x8D", "0xAF", "ACK", "NACK", "地址", "开漏"],
    "case13": ["9600", "AT", "配对", "USART1", "PC4", "PC5", "优先级"],
    "case14": ["状态机", "阻塞", "任务", "架构", "SET TH", "flash 写"],
}
DEFAULT_KEYWORDS = ["DONE", "完成", "验证", "实测"]

MAX_DIFF_CHARS = 14000
MAX_FILE_CHARS = 4000
EXCLUDE_PARTS = {".pio", "Drivers", ".git", "build", "Debug", ".metadata", "lib", "__pycache__"}
SRC_EXTS = {".c", ".h", ".cpp", ".hpp", ".ini", ".ld", ".s"}


def build_bundle(cfg: Any, spec: SP.CaseSpec, agent: str, run_dir: Path,
                 workdir: Path | None = None) -> dict[str, Any]:
    run_dir = Path(run_dir)
    workdir = Path(workdir) if workdir else Path(cfg.testset) / "runs" / agent / spec.id
    ev = run_dir / "evidence"
    result = _read_json(ev / "result.json")
    asserts = _read_json(ev / "asserts.json")
    static = _read_json(ev / "static.json")

    rubric = None
    rub_path = Path(cfg.judge) / "cases"
    for d in sorted(rub_path.glob(f"{spec.id}-*")):
        p = d / "rubric.md"
        if p.exists():
            rubric = p.read_text(encoding="utf-8")
            break

    keywords = KEYWORDS.get(spec.id, DEFAULT_KEYWORDS)
    session = locate.load_for(agent, spec.id, workdir, keywords)
    diff = workspace_diff(workdir, Path(cfg.testset) / "start" / "cubemx")

    pts = []
    for p in P.points_of(spec.id):
        pts.append({"id": p.id, "kind": p.kind, "max": p.max, "text": p.text})

    bundle = {
        "case": spec.id,
        "title": spec.title,
        "weight": spec.weight,
        "agent_anonymous": "A",  # 评委不看 agent 身份；由 CLI 在汇总层映射回真名
        "points": pts,
        "machine": {
            "summary": (result or {}).get("summary", {}),
            "points": (result or {}).get("points", {}),
            "asserts": (asserts or {}).get("asserts", []),
            "static": (static or {}).get("rules", {}),
        },
        "diff": diff,
        "session": session,
        # **人眼观测**（`manual_observe` 步骤）。单列一个键而不是混进 machine，
        # 是为了让评委一眼看出这条证据不是仪器测来的。
        "manual_observations": (result or {}).get("manual", []),
        "rubric": rubric,
        "evidence_dir": str(ev),
        "notes": [
            "机器断言 ok=true 表示已判定通过；ok=false 表示已判定失败但**分档由你定**"
            "（judge_table 里有 0.5 这类中间档）；ok=null 表示没观测到，不要当成通过。",
            "每个给分点必须引用 code（diff 里的行）/ machine（断言 id 或证据文件）/ "
            "session（行号）三者之一。只引用 session 自述而无代码或现象支撑的，该点分数上限 0.5 并标 needs_human。",
            "截图与波形在 evidence 目录（frames/、la/）。若你无法查看图片，请在 uncertain 里说明，"
            "不要猜测屏幕上的文字。",
            "manual_observations 是人眼观测：问题写死在 spec 里（两个 agent 逐字同一句），"
            "回答连时间戳与当时串口最近行一起记录。可以据此给『现象』类判据分，"
            "但必须在 reason 里写明依据是人工观测；**answer 为空或 timed_out=true 不算证据**，"
            "那种情况按『未观测』处理。",
        ],
    }
    return bundle


def _read_json(path: Path) -> dict[str, Any] | None:
    if not Path(path).exists():
        return None
    try:
        return json.loads(Path(path).read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return None


def workspace_diff(work: Path, pristine: Path) -> dict[str, Any]:
    """被测工程相对母本的变化。Core/ 之外的新增文件（src/ 等）也在这里出现。"""
    work, pristine = Path(work), Path(pristine)
    files: list[dict[str, Any]] = []
    total = 0
    truncated = False

    cands: list[Path] = []
    for base in ("Core", "src", "include", "Inc", "Src"):
        root = work / base
        if root.exists():
            cands.extend(p for p in root.rglob("*") if p.is_file() and p.suffix.lower() in SRC_EXTS)
    ini = work / "platformio.ini"
    if ini.exists():
        cands.append(ini)

    for p in sorted(set(cands)):
        rel = p.relative_to(work)
        if EXCLUDE_PARTS & set(rel.parts):
            continue
        try:
            text = p.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        orig = pristine / rel
        if orig.exists():
            try:
                base_text = orig.read_text(encoding="utf-8", errors="replace")
            except OSError:
                base_text = ""
            if base_text == text:
                continue
            d = "\n".join(difflib.unified_diff(
                base_text.splitlines(), text.splitlines(),
                fromfile=f"start/cubemx/{rel}", tofile=f"{rel}", lineterm="", n=2))
            kind = "modified"
        else:
            d = f"（新文件）\n" + "\n".join(text.splitlines()[:120])
            kind = "added"
        if len(d) > MAX_FILE_CHARS:
            d = d[:MAX_FILE_CHARS] + f"\n…（本文件 diff 已截断，共 {len(d)} 字符）"
            truncated = True
        total += len(d)
        if total > MAX_DIFF_CHARS:
            files.append({"path": str(rel).replace("\\", "/"), "kind": kind,
                          "diff": "…（整体 diff 已达上限，后续文件未纳入）"})
            truncated = True
            break
        files.append({"path": str(rel).replace("\\", "/"), "kind": kind, "diff": d})

    return {"files": files, "n_files": len(files), "truncated": truncated,
            "note": "只列 Core/、src/、include/、platformio.ini 的改动；库目录与构建产物不计"}


def render_markdown(bundle: dict[str, Any]) -> str:
    b = bundle
    out: list[str] = []
    out.append(f"# 评审任务：{b['case']} {b['title']}（权重 {b['weight']}）\n")
    out.append("## 评分点（判据原文，逐条给分）\n")
    for p in b["points"]:
        out.append(f"- **{p['id']}**（{p['kind']}，满分 {p['max']}）：{p['text']}")
    out.append("\n## 机器已判定的部分\n")
    for pid, po in (b["machine"]["points"] or {}).items():
        sc = "—" if po.get("score") is None else po["score"]
        out.append(f"- {pid}：status={po.get('status')} score={sc}"
                   + (f" ｜ {' '.join(po.get('notes') or [])[:200]}" if po.get("notes") else ""))
    out.append("\n### 断言明细\n")
    for a in b["machine"]["asserts"]:
        got = json.dumps(a.get("got"), ensure_ascii=False)[:200]
        want = json.dumps(a.get("want"), ensure_ascii=False)[:160]
        out.append(f"- [{a['id']} → {a['point']}] {a['type']} ok={a['ok']} "
                   f"kind={a.get('kind')} conf={a.get('confidence')}\n"
                   f"    want={want}\n    got={got}\n"
                   + (f"    证据={', '.join(a.get('evidence') or [])}\n" if a.get("evidence") else "")
                   + (f"    备注={a.get('note')}\n" if a.get("note") else ""))
    if b["machine"]["static"]:
        out.append("\n### 静态规则\n")
        for rid, r in b["machine"]["static"].items():
            out.append(f"- {rid}: ok={r.get('ok')} ｜ {r.get('note') or r.get('want') or ''} ｜ "
                       f"{', '.join(r.get('evidence') or [])}")

    if b.get("manual_observations"):
        out.append("\n### 人工观测（**不是仪器证据**；引用时须注明来源）\n")
        for m in b["manual_observations"]:
            ans = m.get("answer") or ("（超时未答）" if m.get("timed_out") else "（未作答）")
            out.append(f"- [{m.get('point') or '—'}] 问：{m.get('question')}")
            out.append(f"  答：{ans} ｜ 时刻 +{m.get('t_run_s')}s ｜ "
                       f"在候选内={m.get('in_choices')}")
            if m.get("serial_tail"):
                out.append(f"  当时串口最近行：{' ⏎ '.join(m['serial_tail'])}")

    out.append("\n## 代码改动（相对 CubeMX 母本）\n")
    if not b["diff"]["files"]:
        out.append("（没有检测到 Core/ 或 src/ 的改动 —— 这本身就是重要信号）")
    for f in b["diff"]["files"]:
        out.append(f"\n### {f['path']}（{f['kind']}）\n```diff\n{f['diff']}\n```")
    if b["diff"].get("truncated"):
        out.append("\n> 注意：diff 已截断。")

    out.append("\n## agent 会话摘要\n")
    s = b["session"]
    if not s.get("found"):
        out.append(f"（未找到会话记录：{s.get('note')}）")
    else:
        sm = s["summary"]
        out.append(f"- 文件：{', '.join(s['files'])}")
        out.append(f"- 条目 {sm['n_entries']}，工具调用 {sm['n_tool_calls']} 次"
                   f"（build={sm['did_build']} flash={sm['did_flash']} monitor={sm['did_monitor']}）")
        out.append(f"- 工具序列：{' → '.join(sm['tools'][:40])}")
        if sm["keyword_hits"]:
            out.append("\n### 考点关键词命中（引用时用这里行号）\n")
            for h in sm["keyword_hits"]:
                out.append(f"- 「{h['keyword']}」第 {h['line']} 行（{h['role']}）：{h['excerpt']}")
        out.append(f"\n### 会话结尾自述（第 {sm['tail_line']} 行）\n\n{sm['tail']}")

    if b.get("rubric"):
        out.append("\n## 裁判 rubric 原文（供对照，不要照抄措辞）\n")
        out.append(b["rubric"])
    out.append("\n## 注意事项\n")
    for n in b["notes"]:
        out.append(f"- {n}")
    out.append(f"\n- 证据目录：{b['evidence_dir']}")
    return "\n".join(out)
