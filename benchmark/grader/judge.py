"""AI 评委：把证据包交给一个 LLM，让它按判据逐点给分。

三种用法：

    python -m grader.judge bundle  <run 目录>          # 只生成证据包，不发请求
    python -m grader.judge run     <run 目录>          # 调模型出分（需要 config 里的端点）
    python -m grader.judge scorecard <reports 目录>    # 汇总成记分卡

**评委的中立性要求**（写在这里免得被忘掉）：
* 两个 agent 必须用同一端点、同一模型、同一提示、同一温度。
* 评委不知道 agent 身份；汇总层才把 A/B 映射回真名。
* 模型与档位要记进 scorecard —— 它是已知的不可控变量。
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from engine import points as P  # noqa: E402
from engine import spec as SP  # noqa: E402
from evconfig import load_config  # noqa: E402
from grader import bundle as B  # noqa: E402

PROMPT = Path(__file__).resolve().parent / "prompts" / "judge.md"
SCHEMA = Path(__file__).resolve().parent / "schemas" / "judge_output.json"


def _agent_of(run_dir: Path) -> str:
    """从运行目录名 `YYYYmmdd-HHMMSS-<agent>-<case>` 里取 agent 与 case。"""
    parts = run_dir.name.split("-")
    if len(parts) >= 4:
        return parts[-2], parts[-1]
    return "unknown", "unknown"


def cmd_bundle(args: argparse.Namespace) -> int:
    cfg = load_config()
    run_dir = Path(args.run_dir)
    agent, case = _agent_of(run_dir)
    spec = SP.load_spec(cfg.specs / f"{case}.toml")
    b = B.build_bundle(cfg, spec, agent, run_dir, Path(args.workdir) if args.workdir else None)
    out = run_dir / "bundle"
    out.mkdir(parents=True, exist_ok=True)
    (out / "bundle.json").write_text(json.dumps(b, ensure_ascii=False, indent=2), encoding="utf-8")
    md = B.render_markdown(b)
    (out / "bundle.md").write_text(md, encoding="utf-8")
    print(f"agent={agent} case={case} → {out}")
    print(f"  证据包 {len(md)} 字符；diff 文件 {b['diff']['n_files']} 个；"
          f"会话{'已找到' if b['session'].get('found') else '未找到'}")
    return 0


def _post(cfg: Any, system: str, user: str) -> str:
    base = cfg.get("grade.base_url")
    model = cfg.get("grade.model")
    key = os.environ.get(cfg.get("grade.api_key_env", "EVAL_JUDGE_API_KEY"), "")
    if not base or not model:
        raise RuntimeError("config.toml 的 [grade] 里没配 base_url / model")
    url = base.rstrip("/") + "/chat/completions"
    payload = {
        "model": model,
        "temperature": float(cfg.get("grade.temperature", 0.0)),
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
        "response_format": {"type": "json_object"},
    }
    req = urllib.request.Request(
        url, data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json", **({"Authorization": f"Bearer {key}"} if key else {})},
    )
    with urllib.request.urlopen(req, timeout=600) as resp:
        data = json.loads(resp.read().decode("utf-8"))
    return data["choices"][0]["message"]["content"]


def _validate(out: dict[str, Any], spec: SP.CaseSpec) -> list[str]:
    errs: list[str] = []
    seen: set[str] = set()
    for p in out.get("points", []):
        pid = p.get("id")
        meta = P.find_point(spec.id, str(pid))
        if meta is None:
            errs.append(f"未知评分点 {pid}")
            continue
        seen.add(str(pid))
        if not isinstance(p.get("score"), (int, float)):
            errs.append(f"{pid} 的 score 不是数字")
        elif not (0 <= float(p["score"]) <= meta.max + 1e-9):
            errs.append(f"{pid} 的 score={p['score']} 超出上限 {meta.max}")
        if not p.get("evidence"):
            errs.append(f"{pid} 没有引用证据")
        if p.get("confidence") == "low" and str(pid) not in (out.get("uncertain") or []):
            errs.append(f"{pid} 置信度 low 却没进 uncertain")
    missing = P.point_ids(spec.id) - seen
    if missing:
        errs.append(f"漏了评分点：{sorted(missing)}")
    return errs


def cmd_run(args: argparse.Namespace) -> int:
    cfg = load_config()
    run_dir = Path(args.run_dir)
    agent, case = _agent_of(run_dir)
    spec = SP.load_spec(cfg.specs / f"{case}.toml")
    b = B.build_bundle(cfg, spec, agent, run_dir, Path(args.workdir) if args.workdir else None)
    md = B.render_markdown(b)
    out = run_dir / "bundle"
    out.mkdir(parents=True, exist_ok=True)
    (out / "bundle.md").write_text(md, encoding="utf-8")
    (out / "judge_prompt.md").write_text(PROMPT.read_text(encoding="utf-8"), encoding="utf-8")

    try:
        raw = _post(cfg, PROMPT.read_text(encoding="utf-8"), md)
    except (RuntimeError, urllib.error.URLError, urllib.error.HTTPError, KeyError) as exc:
        print(f"调用评委失败：{type(exc).__name__}: {exc}")
        print(f"证据包已留下，可人工投给任何模型：{out / 'bundle.md'}")
        return 2

    text = raw.strip()
    if text.startswith("```"):
        text = text.split("```")[1]
        text = text[4:] if text.lower().startswith("json") else text
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError as exc:
        (out / "judge_raw.txt").write_text(raw, encoding="utf-8")
        print(f"评委输出不是合法 JSON（已存 judge_raw.txt）：{exc}")
        return 2

    errs = _validate(parsed, spec)
    (out / "judge.json").write_text(json.dumps(parsed, ensure_ascii=False, indent=2), encoding="utf-8")
    parsed["_validation_errors"] = errs
    if errs:
        (out / "judge.json").write_text(json.dumps(parsed, ensure_ascii=False, indent=2), encoding="utf-8")
        print("评委输出未通过契约校验：")
        for e in errs:
            print(f"  ! {e}")
        return 3

    total = sum(float(p["score"]) for p in parsed["points"])
    max_all = sum(p.max for p in P.points_of(spec.id))
    print(f"{case} / {agent}：AI 给分 {total:.2f} / {max_all:.2f}")
    for p in parsed["points"]:
        meta = P.find_point(spec.id, str(p["id"]))
        print(f"  {p['id']:<5} {p['score']}/{meta.max if meta else '?'}  "
              f"[{p['confidence']}] {' '.join(str(e.get('ref','')) for e in p.get('evidence', []))[:80]}")
    if parsed.get("summary"):
        print(f"\n{parsed['summary']}")
    return 0


def cmd_scorecard(args: argparse.Namespace) -> int:
    """把一批 run 目录汇总成 scorecard 格式（对齐裁判资料里的 scorecard.md）。"""
    cfg = load_config()
    root = Path(args.reports_dir)
    rows: dict[tuple[str, str, str], dict[str, Any]] = {}
    for report in sorted(root.glob("*/report.md")):
        run_dir = report.parent
        agent, case = _agent_of(run_dir)
        errs: list[str] = []
        j = run_dir / "bundle" / "judge.json"
        res_path = run_dir / "evidence" / "result.json"
        if not res_path.exists():
            continue
        res = json.loads(res_path.read_text(encoding="utf-8"))
        # 没有这个字段的报告都是校准前那一批 —— 缺省值必须是 pre-fix，不能假装是新尺子判的。
        rev = res["summary"].get("grader_rev") or "pre-fix"
        auto = res["summary"]["auto_score"]
        if j.exists():
            jd = json.loads(j.read_text(encoding="utf-8"))
            errs = jd.get("_validation_errors") or []
            ai = sum(float(p["score"]) for p in jd.get("points", []))
            total, source = ai, "AI"
        else:
            total, source = auto, "机器"
        rows[(case, agent, rev)] = {"total": total, "auto": auto, "source": source, "errors": errs,
                                    "pending": res["summary"]["pending_ai"],
                                    "skipped": res["summary"]["skipped"],
                                    "not_run": bool(res["summary"].get("not_run"))}

    cases = sorted(P.CASE_META)
    lines = ["# 记分卡（自动生成）", "",
             "| 题 | 权重 | agent | 尺子 | 总分(0-5) | 来源 | 机器自动分 | 待 AI | 跳过 | 校验 |",
             "|---|---|---|---|---|---|---|---|---|---|"]
    for case in cases:
        for key in sorted(k for k in rows if k[0] == case):
            r = rows[key]
            _, agent, rev = key
            label = f"{case} {P.CASE_META[case]['title']}"
            if r["not_run"]:
                lines.append(f"| {label} | {P.CASE_META[case]['weight']} | {agent} | {rev} | — | 未运行 | — | — | — | — |")
                continue
            lines.append(
                f"| {label} | {P.CASE_META[case]['weight']} | {agent} | {rev} | "
                f"{r['total']:.2f} | {r['source']} | {r['auto']} | {len(r['pending'])} | "
                f"{len(r['skipped'])} | {'**有错**' if r['errors'] else 'ok'} |")
    # **按尺子分组**：换过判分口径之后，同一张表里会出现两把尺子的行。
    # 加权总分必须分版本各算一次，否则"旧尺子判的 A"和"新尺子判的 B"会被加成一个数。
    weighted: dict[tuple[str, str], float] = {}
    scope: dict[tuple[str, str], float] = {}
    for (case, agent, rev), r in rows.items():
        if r["not_run"]:
            continue
        w = float(P.CASE_META[case]["weight"])
        weighted[(agent, rev)] = weighted.get((agent, rev), 0.0) + r["total"] / 5.0 * w
        scope[(agent, rev)] = scope.get((agent, rev), 0.0) + w
    lines += ["", "## 加权总分（满分 100 = Σ 题分/5 × 权重）", "",
              "只累加**已运行**的题；`未运行` 的行不参与（工作区不存在 ≠ 0 分）。",
              "**同一名 agent 在不同尺子下的分数分列，不可直接相加或互相替换。**", ""]
    for (agent, rev), v in sorted(weighted.items()):
        lines.append(f"- {agent}（尺子 `{rev}`，覆盖权重 {scope[(agent, rev)]:.0f}）：**{v:.2f}**")
    out = root / "scorecard.md"
    out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))
    print(f"\n已写入 {out}")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="grader.judge", description="AI 评委与记分卡")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("bundle", help="只生成证据包（不发请求）")
    p.add_argument("run_dir")
    p.add_argument("--workdir")
    p.set_defaults(func=cmd_bundle)

    p = sub.add_parser("run", help="调模型出分")
    p.add_argument("run_dir")
    p.add_argument("--workdir")
    p.set_defaults(func=cmd_run)

    p = sub.add_parser("scorecard", help="汇总记分卡")
    p.add_argument("reports_dir")
    p.set_defaults(func=cmd_scorecard)

    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
