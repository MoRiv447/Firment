"""冻结校验：判分器 / 规格有没有被改过。

用法（在 eval/ 目录下）：

    <venv python> freeze_check.py          # 退出码 0 = 与冻结清单一致
    <venv python> freeze_check.py -v       # 逐文件打印

为什么要有它：`r5` 之后测试集**冻结**。分数只有在同一把尺子下才可比，
而"我改的只是注释"这种说法，只有逐字节比对能证伪。
任何一次真实改动都必须：① 改 `engine/rev.py` 的 REV；② 重跑全部题；
③ 重写 `frozen-manifest.json`（并接受旧分数不可再比）。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
MANIFEST = ROOT / "frozen-manifest.json"


def digest(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main(argv: list[str] | None = None) -> int:
    # 控制台默认 GBK，而下面的结论句里有 `⇒` —— **判它通过的那一行**正是崩点，
    # 退出码却已经算好了。门禁在"通过"时抛异常比没有门禁更糟：看起来像坏了，
    # 于是下一次没人再跑它。同 `campaign.py` 的 `main()` 一个处理。
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8")
        except (AttributeError, ValueError):
            pass
    ap = argparse.ArgumentParser()
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args(argv)

    doc = json.loads(MANIFEST.read_text(encoding="utf-8"))
    files: dict[str, str] = doc["files"]
    merkle = hashlib.sha256()
    changed: list[str] = []
    missing: list[str] = []
    for rel, want in files.items():
        p = ROOT / rel
        if not p.exists():
            missing.append(rel)
            merkle.update(rel.encode())
            continue
        b = p.read_bytes()
        merkle.update(rel.encode())
        merkle.update(b)
        got = hashlib.sha256(b).hexdigest()
        if got != want:
            changed.append(rel)
            if args.verbose:
                print(f"CHANGED {rel}: {want[:12]} -> {got[:12]}")
        elif args.verbose:
            print(f"ok      {rel}")

    ok = not changed and not missing and merkle.hexdigest() == doc["merkle"]
    print(f"冻结版本 {doc['revision']}（{doc['frozen_at']}，{len(files)} 个文件）")
    if missing:
        print("缺失：" + ", ".join(missing))
    if changed:
        print("已改动：" + ", ".join(changed))
    if not ok:
        print("⇒ 与冻结清单不一致。改判分器必须同时改 REV 并重跑全部题；旧分数不可再比。")
        return 1
    print(f"⇒ 一致（merkle {doc['merkle'][:16]}…）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
