"""campaign —— 把"出题 → 计时 → 收工 → 判分"这一段自动化。

裁判只做两件事：坐在旁边、以及在 `manual_observe` 提问时回答。

为什么放在裁判资料夹而不是测试集里：driver 知道时间盒与判分入口，拷进
`D:\\OldStudy66\\ai test pro` 等于把评分表递到被测 agent 手边（README 铁律第 4 条）。

用法：

    PY -m campaign firm case01                 # 跑一题（跑完自动判分）
    PY -m campaign firm case01 case03 case07   # 跑几题
    PY -m campaign firm --all                  # 14 题连着跑
    PY -m campaign claude case01 --dry-run     # 只准备现场，不启动 agent
    PY -m campaign firm case01 --no-grade      # 只跑不判

三条硬规矩写死在这里，因为它们是"测试作废"级别：

1. **题面逐字不动**（铁律 1）：直接把 `prompt.md` 原文作为 `-p` 的参数，
   不拼接、不加前后缀、不改标点。两个 agent 拿到的是同一份字节。
2. **到点即停、不救场**（铁律 3）：时间盒一到就杀进程树，按当时的现场判分。
   不重试、不补提示、不替它把编译错误修好。
3. **工作区必须从零开始**：`prep.py` 只删 `cubemx/` 再重拷，根目录残留的
   `platformio.ini`/`src/` 会漏进下一题 —— 所以这里先把整个 case 目录清空。
"""
from __future__ import annotations

import argparse
import contextlib
import json
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from evconfig import load_config                       # noqa: E402
from engine import points as P                         # noqa: E402

AGENTS = ("firm", "claude")
# 归档区：手写的链路基准固件住在这里，永远不该被 driver 清掉
PROTECTED = ("_bench",)


def py_exe(cfg) -> str:
    """跑 prep.py / cli.py 用的解释器。`paths.python` 没配就用当前这个。"""
    return str(cfg.get("paths.python") or sys.executable)


# 端点/网络挂掉的特征。操作手册 §5 本来就写着"网络报错 → 重试几次"，
# 但重试必须**窄口径**，否则就成了救场：只在"这一下什么都没干成"时才重开。
_NET_ERR = re.compile(
    r"provider error|error decoding response body|强迫关闭|connection reset|"
    r"timed out|rate limit|overloaded|429|50[0-9]",
    re.IGNORECASE)


def needs_network_retry(log: Path, workdir: Path, timed_out: bool) -> bool:
    """判定"这次是被网络打断的、且 agent 还没真正开始干活"。

    两个条件缺一不可：
    ① 日志里有端点/网络错误的特征；
    ② 工作区里连 `platformio.ini` 都没有 —— 说明它没走到构建那一步。
    跑了 15 分钟才断网的情况**绝不重试**：那会抹掉真实工作、还偷偷把时间盒翻倍。
    """
    if timed_out or not workdir.exists():
        return False
    try:
        text = log.read_text(encoding="utf-8", errors="replace")
    except Exception:
        return False
    if not _NET_ERR.search(text):
        return False
    return not (workdir / "platformio.ini").exists()


def case_dir(testset: Path, cid: str) -> Path:
    hits = sorted((testset / "cases").glob(f"{cid}-*"))
    if not hits:
        raise SystemExit(f"测试集里没有 {cid}：{testset / 'cases'}")
    return hits[0]


def read_prompt(testset: Path, cid: str) -> str:
    """题面**原文**，一个字节都不改。"""
    p = case_dir(testset, cid) / "prompt.md"
    if not p.exists():
        raise SystemExit(f"缺题面：{p}")
    return p.read_text(encoding="utf-8")


def agent_argv(agent: str, workdir: Path, prompt: str) -> tuple[list[str], bool]:
    """返回 (argv, 是否需要关掉 stdin)。

    `claude -p` 不给 stdin 会一直等输入（实测），所以两个都关成 DEVNULL。
    权限：交互跑法是人坐在旁边一路放行，非交互要等价就必须自动批准，
    否则 agent 会卡在"等授权"上 —— 那测的是授权弹窗，不是写固件的能力。
    """
    if agent == "firm":
        return ["firm", "-p", prompt, "--yes", "--cwd", str(workdir)], True
    return ["claude", "-p", prompt, "--dangerously-skip-permissions",
            "--output-format", "text"], True


def archive_base(cfg) -> Path:
    """归档的根目录 —— **必须在被试够得着的那棵树之外**。

    原来它落在 `runs/<agent>/_bench/wiped/…`，也就是各 case 工作区的兄弟目录。两个 agent 都有
    shell，而 `resolve_within` 那套工作区沙箱按 `tools/util.rs` 自己的注释并不覆盖 shell。
    实测后果：claude 在 case06 跑过 `Get-ChildItem 'ai test pro\\_bench' -Recurse -Depth 3 -Force`
    —— 那棵树里躺着**对面 agent 每一局的完整解答**和 WorkBuddy 手写的参考固件。它只列了目录名、
    没有读文件（逐条查过 tool_use 入参），分数也没变（case06 两批都 5.0），但这扇门不该开着。

    位置由 `config.toml` 的 `paths.archive` 决定（现在指 `D:/Firment-bench-archive`）：既不在测试集
    里，也不在被测试集的父目录列举时顺手露出来，还避开 C 盘空间。没配就退回裁判资料下面。
    """
    root = None
    try:
        root = cfg.get("paths.archive")
    except Exception:  # noqa: BLE001 —— 测试里传的是 SimpleNamespace，没有 .get
        root = None
    root = root or getattr(cfg, "archive", None)
    if not root:
        root = Path(getattr(cfg, "judge", None) or HERE.parent) / "bench-archive"
    return Path(root)


def archive_before_wipe(dst: Path, cfg) -> Path | None:
    """抹掉工作区之前先留一份，返回归档路径（工作区不存在则 None）。

    为什么必须有这一步：静态规则的**语料就是工作区里的源码**，而报告目录只落判定、断言和
    串口日志（`evidence/static.json` 记的是结论，不是被扫的文件）。重跑 prep 会 `rmtree`
    整个 `runs/<agent>/<case>` ⇒ 抹掉之后"用当前尺子重判上一轮"就再也复现不出来了，
    只能引用一个没人能重新算出的数字。

    归档按 `run-1 / run-2 …` 递增、**永不覆盖**，所以每一轮的历史都留得下来。归档失败时
    直接抛出、不许继续抹 —— 宁可这一题不跑，也不要跑完才发现栏 A 复现不了。
    """
    if not dst.exists():
        return None
    area = archive_base(cfg) / f"{dst.parent.name}-{dst.name}"
    n = 1
    while (area / f"run-{n}").exists():
        n += 1
    final = area / f"run-{n}"
    partial = area / f"run-{n}.partial"
    if partial.exists():
        shutil.rmtree(partial)
    area.mkdir(parents=True, exist_ok=True)
    # `ignore_dangling_symlinks`：claude 的 platformio MCP 会在工作区里留
    # `.pio-mcp-workspace/logs/build/latest-build.log`，它是指向同一棵树里某个
    # 构建日志的**符号链接**；那个目标在后续构建里会被轮换掉 ⇒ 链接变悬空。
    # `copytree` 默认会去追目标、追不到就整棵归档失败 —— 而归档失败会直接
    # 中止这一题（宁可少跑一题，也不留一份不可复现的分数）。悬空链接对复现源码
    # 没有任何价值，跳过即可。
    shutil.copytree(dst, partial, ignore_dangling_symlinks=True)
    partial.replace(final)  # 先写 .partial 再改名：中途失败不会留下半份"完整"归档
    return final


def quarantine_claude_state(cfg, workdir: Path) -> Path | None:
    """把 claude 在这个题目录下的**项目状态**整体挪走，让它这一局从"失忆"开始。

    为什么只有 claude 需要：它有一套装在 `~/.claude/projects/<munged cwd>/` 的自动记忆，
    **存在工作区之外 ⇒ `rmtree` 删不到**，于是同一题的第 N 次会带着第 1 次的笔记。实测：
    case01 目录里有 8 月 16 日留下的 `MEMORY.md` 索引，而今天这局又新写了一份
    `serial-heartbeat-g431rb.md`（内含 COM3、LPUART1 PA2/PA3、115200、`--wrap=main` 等
    本题的解法要点）。今天的会话没加载它（`MEMORY.md` 在会话文本里出现 0 次），但下一局就会。
    Firment 没有这种按题累积的通道（它的 KB 对所有题、两侧都一样），所以这个动作是**把条件拉平**，
    不是照顾谁。挪走不删除：历史轨迹仍是判分后的证据。
    """
    if not cfg or not getattr(cfg, "judge", None):
        return None
    from hal import modelcheck

    src = Path.home() / ".claude" / "projects" / modelcheck._munge_cwd(workdir)
    if not src.exists():
        return None
    stamp = __import__("time").strftime("%Y%m%d-%H%M%S")
    dst = Path(cfg.judge) / "claude-state-quarantine" / f"{src.name}--{stamp}"
    dst.parent.mkdir(parents=True, exist_ok=True)
    src.rename(dst)
    return dst


def wipe_workspace(dst: Path, cfg) -> None:
    for bad in PROTECTED:
        if bad in dst.parts:
            raise SystemExit(f"拒绝删除归档区：{dst}")
    kept = archive_before_wipe(dst, cfg)
    if dst.exists():
        try:
            shutil.rmtree(dst)
        except OSError as exc:
            # **删不掉就整体挪走。** Windows 对悬空符号链接的 `unlink` 会返回
            # WinError 5（即使没有别的进程在用），`rmtree` 因此整棵失败 ——
            # 2026-10-18 第一轮 claude/case04 就是这么中断的（只跑完 case01）。
            # 判分只需要这个目录不在原位；归档上面已经留好了，挪走同样能保证复现。
            aside_base = (archive_base(cfg) / "wiped"
                          / f"{dst.parent.name}-{dst.name}-{time.strftime('%Y%m%d-%H%M%S')}")
            aside_base.parent.mkdir(parents=True, exist_ok=True)
            aside, n = aside_base, 2
            while aside.exists():
                aside = aside_base.with_name(f"{aside_base.name}--{n}")
                n += 1
            dst.rename(aside)
            print(f"  删不掉（{type(exc).__name__}: {exc}），已整体挪到：{aside}")
    if kept:
        print(f"  旧工作区已归档：{kept}")


def prep(cfg, agent: str, cid: str) -> Path:
    """生成干净工作区。先整目录清空，再走测试集自带的 prep.py（判定口径不变）。"""
    dst = Path(cfg.testset) / "runs" / agent / cid
    wipe_workspace(dst, cfg)
    if agent == "claude":
        # claude 的自动记忆装在 `~/.claude/projects/<cwd>` 里，工作区删不到它 ⇒ 每次开跑前先挪走
        moved = quarantine_claude_state(cfg, dst)
        if moved:
            print(f"  claude 的项目状态已挪出（防跨局记忆）：{moved}")
    r = subprocess.run([py_exe(cfg), str(Path(cfg.testset) / "runner" / "prep.py"),
                        agent, cid], capture_output=True, text=True, encoding="utf-8")
    if r.returncode != 0:
        raise SystemExit(f"prep.py 失败：{r.stdout}{r.stderr}")
    return dst


def kill_tree(proc: subprocess.Popen) -> None:
    """杀进程树。agent 会派生 pio/arm-none-eabi/串口工具，只杀父进程会留下
    占着 COM 口的孤儿 —— 下一题就会莫名其妙"串口打不开"。"""
    if os_is_win():
        subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"],
                       capture_output=True)
    else:
        proc.kill()
    try:
        proc.wait(timeout=20)
    except Exception:
        pass


def os_is_win() -> bool:
    return sys.platform.startswith("win")


@contextlib.contextmanager
def fixture_at_rest(cfg):
    """agent 干活期间，把夹具的激励线保持在"未按下"。

    为什么必须由 driver 来做：那根线的空闲电平是**断言**（= 按下）。`runner`
    只在**评测**开始时 `release_all()`（`fixture_open` 步骤），而 agent 是在评测
    之前干活的 —— 这段时间没有进程持有夹具口，按键就一直是被按住的。
    agent 一旦去读 IDR 量"静置电平"，量到的其实是"按下"：2026-10-18 实测，
    两次 run 读到 `bit13=1`，于是把 B1 判成低有效，固件全程不打印。

    这里只做一件事：开夹具口、release_all、**持有句柄**到 agent 结束，再关掉
    （评测要用同一个口）。夹具不可用时只打印一行，不阻断这一题。
    """
    fx = bus = None
    if not bool(cfg.get("fixture.enabled", False)):
        yield
        return
    try:
        from hal import ports
        from hal.gpio import Fixture
        from hal.serialbus import SerialBus

        role = str(cfg.get("fixture.port_role", "fixture_uart"))
        match = ports.resolve_role(cfg, role)
        if match is None:
            print("  夹具：没找到夹具串口 —— agent 期间线的电平不受控")
            yield
            return
        bus = SerialBus(match.device, int(cfg.get("serial.default_baud", 115200)), label=role)
        bus.open()
        fx = Fixture(cfg, bus)
        fx.active_names = None          # 没有题目上下文：全部放回空闲
        fx.release_all()
        print(f"  夹具已置为未按下并保持（{match.device}）")
    except Exception as exc:  # noqa: BLE001 —— 夹具是基础设施，坏了不该带走整轮
        print(f"  夹具未持有（{type(exc).__name__}: {exc}）—— agent 期间线的电平不受控")
    try:
        yield
    finally:
        for closer in (lambda: fx and fx.release_all(), lambda: bus and bus.close()):
            try:
                closer()
            except Exception:  # noqa: BLE001
                pass


def run_agent(agent: str, workdir: Path, prompt: str, box_s: float,
              log: Path) -> tuple[float, bool, int | None, str]:
    """跑一个 agent 到时间盒结束或它自己收工。返回 (用时, 是否超时, 退出码, 错误尾)。"""
    argv, close_stdin = agent_argv(agent, workdir, prompt)
    t0 = time.time()
    with log.open("wb") as fh:
        try:
            proc = subprocess.Popen(argv, cwd=str(workdir), stdout=fh, stderr=fh,
                                    stdin=subprocess.DEVNULL if close_stdin else None)
        except FileNotFoundError as exc:
            return time.time() - t0, False, None, f"启动失败：{exc}"
        timed_out = False
        try:
            proc.wait(timeout=box_s)
        except subprocess.TimeoutExpired:
            timed_out = True
            kill_tree(proc)
    tail = ""
    try:
        tail = log.read_text(encoding="utf-8", errors="replace")[-400:]
    except Exception:
        pass
    return time.time() - t0, timed_out, proc.returncode, tail


def grade(cfg, agent: str, cid: str) -> tuple[int, str]:
    """调 `cli.py run` 判分。**stdio 全部继承** —— manual_observe 要在这里问人。"""
    r = subprocess.run([py_exe(cfg), str(HERE / "cli.py"), "run", agent, cid])
    return r.returncode, ""


def latest_report(cfg, agent: str, cid: str) -> Path | None:
    root = Path(cfg.reports)
    hits = sorted(root.glob(f"*-{agent}-{cid}"), key=lambda p: p.stat().st_mtime)
    return hits[-1] if hits else None


def board_ok(cfg) -> tuple[bool, str]:
    """板子在不在。跑题之前必须问一次 —— 上一题（尤其 case10 的 flash 题）
    可能把芯片写坏，那样后面每一题都会"烧录失败 → 0 分"，
    而报告上的 0 分看起来像 agent 能力差，不像板子掉了。
    """
    from hal.probe import Probe

    try:
        p = Probe(cfg)
        if not p.probe_rs:
            return False, "probe-rs 不在 PATH 上"
        return (bool(p.has_target()), "probe-rs list")
    except Exception as exc:  # noqa: BLE001
        return False, str(exc)


def recover_board(cfg) -> dict:
    """整片擦除解砖（会顺带清掉 RDP）。不要求有母本 ELF：
    每次判分前都会重烧该题自己的固件，空芯片不影响下一题。"""
    from hal.probe import Probe

    try:
        return Probe(cfg).mass_erase_recover()
    except Exception as exc:  # noqa: BLE001
        return {"ok": False, "error": str(exc)}


def require_board(cfg) -> dict:
    """开跑一题前的闸门：连不上就恢复一次，还不行就**停下整轮**。

    停在这里比继续跑重要：继续跑只会产出一堆"0 分"，而它们和真实的 0 分
    在记分卡上长得一模一样。
    """
    ok, why = board_ok(cfg)
    if ok:
        return {"board": "ok"}
    print(f"⚠ 板子连不上（{why}）—— 试一次整片擦除恢复…")
    rec = recover_board(cfg)
    ok2, why2 = board_ok(cfg)
    print(f"   恢复 ok={rec.get('ok')} → 再探：{'连上了' if ok2 else why2}")
    if not ok2:
        raise SystemExit("板子仍然连不上 —— **本轮停在这里**。"
                         "继续跑只会把后面每题都判成 0，而那种 0 和真实能力的 0 分不出来。")
    return {"board": "recovered", "recover_ok": bool(rec.get("ok"))}


# 会往 flash 存储页写的题。芯片里的旧数据是**跨 agent 的不可控变量**：
# Firment 跑 case10 时页里是空的、Claude 再跑时页里已经有上一家的计数，
# 两边的"掉电计数"起点就不一样 —— 这种差异在记分卡上和真实能力差异长得一模一样。
# 所以这几题开跑前先把整片擦干净（顺带清掉 agent 可能设置的 RDP）。
FLASH_CASES = {"case10", "case14"}


def normalize_chip(cfg, cid: str) -> dict:
    if cid not in FLASH_CASES:
        return {}
    from hal.probe import Probe

    print(f"  本题会写 flash 存储页 ⇒ 先整片擦除，让两个 agent 从同一个空白芯片开始")
    try:
        r = Probe(cfg).mass_erase_recover()
        ok = bool(r.get("ok"))
        print(f"   擦除 ok={ok}")
        return {"chip": "erased" if ok else "erase_failed", "erase_ok": ok}
    except Exception as exc:  # noqa: BLE001
        print(f"   擦除异常：{exc}")
        return {"chip": "erase_error", "erase_error": str(exc)}


def collect_transcript(cfg, agent: str, cid: str, workdir: Path, out: Path) -> dict:
    """把 agent **这一题的完整会话**收进本轮目录。

    为什么要单独收：driver 的 `-q` 日志只有"一行一个工具调用"，而复盘
    （尤其是拿它改进 Firment）要看的是完整对话、每次编辑的 diff、以及
    它在验证什么上撒了谎/偷懒了。终端日志会滚掉，会话文件才是原始事实。

    - firm：`firm share <id> --format md` 导出自包含文档（它自己会遮罩密钥），
      顺带把原始 jsonl 也拷一份 —— share 是渲染件，jsonl 才是可 grep 的源。
    - claude：会话本身就是 jsonl，直接拷。
    """
    from hal import modelcheck

    got: dict[str, str] = {}
    try:
        if agent == "firm":
            hit = modelcheck.firm_session_meta(workdir)
            if hit is None:
                return {"transcript": "没找到 firment 会话记录"}
            src = hit[0]
            sid = src.stem
            md = out / f"{cid}-session.md"
            r = subprocess.run(["firm", "share", sid, "--format", "md", "--out", str(md)],
                               capture_output=True, text=True, encoding="utf-8", timeout=120)
            if r.returncode != 0:
                md.write_text(f"<!-- firm share 失败 rc={r.returncode} -->\n"
                              f"{r.stdout}\n{r.stderr}\n", encoding="utf-8")
            raw = out / f"{cid}-session.jsonl"
            shutil.copyfile(src, raw)
            got = {"transcript": str(md), "session_jsonl": str(raw), "session_id": sid}
        else:
            root = Path.home() / ".claude" / "projects" / modelcheck._munge_cwd(workdir)
            files = sorted(root.glob("*.jsonl"), key=lambda f: f.stat().st_mtime, reverse=True)
            if not files:
                return {"transcript": "没找到 claude 会话记录"}
            raw = out / f"{cid}-session.jsonl"
            shutil.copyfile(files[0], raw)
            got = {"transcript": str(raw), "session_jsonl": str(raw)}
    except Exception as exc:  # noqa: BLE001 - 收 transcript 失败不该毁掉一轮跑题
        got = {"transcript": f"收集失败：{exc}"}
    print(f"  会话记录：{got.get('transcript')}")
    return got


def run_one(cfg, agent: str, cid: str, out: Path, *,
            dry: bool = False, do_grade: bool = True, retries: int = 2) -> dict:
    meta = P.CASE_META[cid]
    box_s = float(meta["timebox_min"]) * 60
    prompt = read_prompt(Path(cfg.testset), cid)
    out.mkdir(parents=True, exist_ok=True)

    print(f"\n{'=' * 72}\n{cid} {meta['title']} — {agent}"
          f"｜时间盒 {meta['timebox_min']}min｜题面 {len(prompt)} 字符\n{'=' * 72}")
    gate = ({"board": "skipped（dry-run）"} if dry
            else {**require_board(cfg), **normalize_chip(cfg, cid)})
    workdir = prep(cfg, agent, cid)
    print(f"工作区已重置：{workdir}")
    if dry:
        print("--dry-run：不启动 agent，也不判分")
        return {"case": cid, "agent": agent, "dry_run": True, "workdir": str(workdir)}

    tries: list[dict] = []
    total = 0.0
    timed_out = False
    code: int | None = None
    for attempt in range(1, retries + 2):
        log = out / f"{cid}-agent-try{attempt}.log"
        if attempt > 1:
            print(f"↻ 第 {attempt} 次尝试 —— 上一次是端点/网络错误且**没有任何产出**，"
                  f"重开一次（这只针对断网；agent 能力问题一律不救场）")
            workdir = prep(cfg, agent, cid)
        with fixture_at_rest(cfg):
            dur, timed_out, code, tail = run_agent(agent, workdir, prompt, box_s, log)
        total += dur
        tries.append({"try": attempt, "duration_s": round(dur, 1),
                      "timed_out": timed_out, "exit_code": code, "log": str(log)})
        print(f"结束：本次用时 {dur / 60:.1f}min｜超时={'是' if timed_out else '否'}"
              f"｜退出码={code}｜日志 {log.name}")
        if timed_out:
            print("⏰ 时间盒到 —— 按铁律不救场，直接判当前现场")
            break
        if attempt <= retries and needs_network_retry(log, workdir, timed_out):
            continue
        break
    if len(tries) > 1:
        print(f"（本题共尝试 {len(tries)} 次，累计 {total / 60:.1f}min）")
    log = Path(tries[-1]["log"])

    rec: dict = {"case": cid, "agent": agent, "started_at": time.strftime("%F %T"),
                 "duration_s": round(total, 1), "timed_out": timed_out,
                 "exit_code": code, "tries": tries, "agent_log": str(log), **gate}
    if tail:
        rec["agent_tail"] = tail[-200:]
    rec.update(collect_transcript(cfg, agent, cid, workdir, out))
    if do_grade:
        rc, _ = grade(cfg, agent, cid)
        rep = latest_report(cfg, agent, cid)
        rec["grade_exit"] = rc
        # **只存 run 目录名，不存绝对路径**：绝对路径在归档之后一定失效
        # （阶段0 把 16 份旧报告整体移进 `reports/_pre-fix/` 时，
        # 上一批 `campaign.json` 里的 `report` 字段全变成了指向空气的绝对路径 ——
        # 复现链就是这么断的）。目录名是稳定身份，配 `reports/` 下递归找就能定位。
        rec["report"] = rep.name if rep else None
        if rep and (rep / "report.md").exists():
            head = (rep / "report.md").read_text(encoding="utf-8").splitlines()[:8]
            print("\n".join(f"   {ln}" for ln in head if ln.strip()))
    return rec


def main(argv: list[str] | None = None) -> int:
    # 控制台默认 GBK，而 report.md 的第一行里就有 ✓ / — 这些字符。2026-10-17 实测：
    # case04 探针**判分已经跑完**，却在 `run_one` 回显报告头部时抛 UnicodeEncodeError，
    # 于是 campaign.json 的汇总没落盘 —— 一个纯打印的 bug 把一次成功的评测变成了半截活。
    # cli.py 早就打过这个补丁，这里补上，两个入口不再分叉。
    for s in (sys.stdout, sys.stderr):
        try:
            s.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[union-attr]
        except Exception:  # noqa: BLE001
            pass
    ap = argparse.ArgumentParser(prog="campaign", description=__doc__.split("\n")[0])
    ap.add_argument("agent", choices=AGENTS)
    ap.add_argument("cases", nargs="*", help="case01 case07 …；或 --all")
    ap.add_argument("--all", action="store_true", help="14 题全跑")
    ap.add_argument("--dry-run", action="store_true", help="只重置工作区，不启动 agent")
    ap.add_argument("--no-grade", action="store_true", help="只跑不判")
    ap.add_argument("--retries", type=int, default=2,
                    help="端点/网络错误且零产出时的重开次数（默认 2；agent 能力问题不重试）")
    a = ap.parse_args(argv)
    cfg = load_config()
    cids = [f"case{n:02d}" for n in range(1, 15)] if a.all else list(a.cases)
    if not cids:
        ap.error("给几个题号，或者 --all")
    bad = [c for c in cids if c not in P.CASE_META]
    if bad:
        ap.error(f"不认识的题号：{bad}")

    out = Path(cfg.reports) / f"campaign-{a.agent}-{time.strftime('%Y%m%d-%H%M%S')}"
    recs = [run_one(cfg, a.agent, c, out, dry=a.dry_run, do_grade=not a.no_grade,
                    retries=a.retries) for c in cids]
    (out / "campaign.json").write_text(
        json.dumps(recs, ensure_ascii=False, indent=1), encoding="utf-8")
    print(f"\n{'=' * 72}\n本轮汇总（{a.agent}）→ {out}")
    for r in recs:
        # 这个数是**判分进程的退出码**（0=跑完且无判红，1=有判红/2=出错），不是分数。
        # 原来写作 `判分=0`，被当成"这题得了 0 分"读过 —— 汇总行上最要紧的一个数，
        # 标签不能靠读者猜。
        print(f"  {r['case']}: 用时 {r.get('duration_s', 0) / 60:.1f}min"
              f"｜超时={r.get('timed_out')}｜判分器退出码={r.get('grade_exit', '—')}"
              f"｜{r.get('report') or ''}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
