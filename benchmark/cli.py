#!/usr/bin/env python3
"""eval —— 14 题嵌入式基准的自动评测器（裁判侧工具）。

用法（全部命令都在本目录下跑）：

    python cli.py caps                        # 看这台机器现在能测什么
    python cli.py spec                        # 校验全部 spec 并打印覆盖情况
    python cli.py spec case01                 # 看一道题的规格
    python cli.py rules case01 runs/firm/case01   # 只跑静态规则
    python cli.py run firm case01             # 真板跑一轮
    python cli.py run firm case01 --sim fixtures/case01-good.json   # 无板自测
    python cli.py report <run 目录>            # 重打印一份报告摘要

**这个目录不能进 D:\\OldStudy66\\ai test pro**：被测 agent 的工作目录就在那棵树
的 runs\\ 下，断言阈值放进去等于把评分表递给它。
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from evconfig import load_config  # noqa: E402
from engine import points as P  # noqa: E402
from engine import spec as SP  # noqa: E402
from engine.runner import Runner  # noqa: E402
from engine.static_rules import rules_of, run_rules  # noqa: E402
from hal import caps as CAPS  # noqa: E402
from hal import ports  # noqa: E402
from hal.gpio import Fixture  # noqa: E402
from hal.serialbus import SimScript  # noqa: E402


def _load_sim(path: Path) -> SimScript:
    from hal.simload import load_script

    return load_script(path)


def cmd_caps(args: argparse.Namespace) -> int:
    cfg = load_config()
    fixture = Fixture(cfg, None)
    caps = CAPS.probe_caps(cfg, check_hardware=not args.offline, fixture=fixture)
    d = caps.to_dict()
    print("== 能力 ==")
    for k in sorted(d["caps"]):
        mark = "OK " if d["caps"][k] else "-- "
        print(f"  {mark}{k}")
    print("\n== 串口 ==")
    for p in ports.describe_all():
        print(f"  {p['device']:<8} {p['vid'] or '?':>6}:{p['pid'] or '?':<6} {p['description']}")
    print("\n== 角色解析 ==")
    for role in ("dut_vcp", "fixture_uart", "bt_peer"):
        try:
            m = ports.resolve_role(cfg, role)
            print(f"  {role:<14} {m.device if m else '(未找到)'}")
        except KeyError:
            print(f"  {role:<14} (config 里没定义)")
        except ports.PortAmbiguity:
            print(f"  {role:<14} **有歧义**（多个同型号板，需在 config 里用 device=/location= 指定）")
    if args.json:
        print(json.dumps(d, ensure_ascii=False, indent=2))
    missing = [k for k, v in d["caps"].items() if not v]
    print(f"\n缺失 {len(missing)} 项：{', '.join(missing) if missing else '无'}")
    return 0


def cmd_spec(args: argparse.Namespace) -> int:
    cfg = load_config()
    names = [args.case] if args.case else sorted(p.stem for p in cfg.specs.glob("case*.toml"))
    if not names:
        print("specs/ 下没有文件")
        return 1
    rc = 0
    for name in names:
        missing_files = [p.name for p in [cfg.specs / f"{name}.toml"] if not p.exists()]
        if missing_files:
            print(f"{name}: 规格文件不存在（{cfg.specs / (name + '.toml')}）")
            if not args.case:
                continue
            return 1
        try:
            spec = SP.load_spec(cfg.specs / f"{name}.toml")
        except SP.SpecError as exc:
            print(f"{name}: **规格错误** {exc}")
            rc = 1
            continue
        prompt = spec.prompt if (spec.prompt and spec.prompt.exists()) else SP.discover_prompt(cfg.testset, name)
        cov = spec.coverage()
        print(f"== {spec.id} {spec.title} ==  权重 {spec.weight}  时间盒 {spec.timebox_min}min")
        print(f"   题面：{prompt if prompt else '**找不到 prompt.md**'}")
        print(f"   步骤 {len(spec.steps)} 个｜需要能力：{', '.join(spec.required_caps()) or '无'}")
        print(f"   声明点：{', '.join(spec.declared_points())}")
        if cov["undeclared"]:
            print(f"   **未声明（不计分）**：{', '.join(cov['undeclared'])}")
        for w in spec.warnings:
            print(f"   ! {w}")
        if args.case:
            pts = P.points_of(spec.id)
            print(f"\n   评分点明细（{P.max_functional(spec.id)} 功能 + "
                  f"{sum(p.max for p in pts if p.kind == 'hidden')} 隐藏）：")
            for p in pts:
                print(f"     {p.id:<4} {p.kind:<10} max={p.max:<4} {p.text[:70]}")
    return rc


def cmd_rules(args: argparse.Namespace) -> int:
    cfg = load_config()
    wd = Path(args.workdir) if args.workdir else Path(cfg.testset) / "runs" / args.agent / args.case
    if not wd.exists():
        print(f"工作目录不存在：{wd}")
        return 1
    res = run_rules(args.case, wd, only=args.rule or None)
    print(f"扫描 {res['files_scanned']} 个源文件：{wd}")
    if res.get("error"):
        print("! " + res["error"])
    for rid, r in res["rules"].items():
        verdict = {True: "通过", False: "不通过", None: "无法判定"}[r["ok"]]
        print(f"  [{verdict:<4}] {rid:<12} {r.get('note') or r.get('want') or ''}")
        if r.get("evidence"):
            print(f"           {', '.join(r['evidence'])}")
    if args.json:
        print(json.dumps(res, ensure_ascii=False, indent=2))
    return 0


def cmd_run(args: argparse.Namespace) -> int:
    cfg = load_config()
    cases = [args.case] if args.case != "all" else sorted(
        p.stem for p in cfg.specs.glob("case*.toml")
    )
    rc = 0
    for case in cases:
        spec_path = cfg.specs / f"{case}.toml"
        if not spec_path.exists():
            print(f"{case}: **规格文件不存在** {spec_path}")
            rc = 1
            continue
        try:
            spec = SP.load_spec(spec_path)
        except SP.SpecError as exc:
            print(f"{case}: 规格错误，跳过：{exc}")
            rc = 1
            continue

        sim_script = None
        if args.sim:
            sim_path = Path(args.sim)
            if not sim_path.exists():
                print(f"{case}: 找不到 sim fixture：{sim_path}")
                return 1
            sim_script = _load_sim(sim_path)

        runner = Runner(cfg, spec, args.agent, sim=bool(args.sim),
                        workdir=Path(args.workdir) if args.workdir else None)
        if sim_script is not None:
            # 串口角色名 → 同一个脚本；单口的题够用，多口的题再按角色拆
            for role in spec.caps.get("serial", []) or ["dut_vcp"]:
                runner._sim_scripts[role] = sim_script

        outcome = runner.run()
        s = outcome.summary()
        tag = "（模拟）" if s["sim"] else ""
        bl_text = ("合规" if s["bloodline_ok"]
                   else ("**未运行**（还没转换/工作区不存在）" if s["not_run"]
                         else "**违规 → 本题 0 分**"))
        print(f"\n== {s['case']} {s['title']} — {s['agent']}{tag} ==")
        print(f"   血统：{bl_text}")
        print(f"   自动判定：{s['auto_score']} / {s['max_in_scope']}（满分 {s['max_all']}）")
        for pid, o in outcome.points.items():
            score = "" if o.score is None else o.score
            print(f"     {pid:<4} {o.status:<16} {score}")
        if s["pending_ai"]:
            print(f"   待 AI 裁定：{', '.join(s['pending_ai'])}")
        if s.get("machine_fail"):
            print(f"   机器判红：{', '.join(s['machine_fail'])}")
        if s["skipped"]:
            print(f"   跳过（不计分）：{', '.join(s['skipped'])}")
        for w in s["warnings"]:
            print(f"   ! {w}")
        print(f"   报告：{runner.out / 'report.md'}")
        if not s["bloodline_ok"]:
            rc = 1
    return rc


def cmd_la(args: argparse.Namespace) -> int:
    """看分析仪能不能用、驱动的型号对不对、通道号怎么填。"""
    from hal import proc
    from hal.la import DRIVER_CANDIDATES, LogicAnalyzer

    cfg = load_config()
    la = LogicAnalyzer(cfg)
    if not la.available():
        print("sigrok-cli 不在 PATH 上 —— 先按 README『首次标定』第 2 步装好。")
        print("（winget 里只有 PulseView，不含 CLI；用 sigrok.org 的官方安装包）")
        return 1
    print(f"sigrok-cli：{la.bin}")
    res = proc.run([str(la.bin), "-V"], timeout_s=20)
    print("版本：", (res.stdout + res.stderr).strip().splitlines()[:1])

    print("\n== 驱动清单里有没有 fx2lafw（24MHz 8ch 克隆板）==")
    listed = "\n".join(la.detect_driver())
    candidates = "\n".join(proc.run([str(la.bin), "-L"], timeout_s=30).stdout.splitlines())
    for drv in DRIVER_CANDIDATES[:4]:
        print(f"  {'有' if drv in candidates else '无'}  {drv}")

    print("\n== 扫设备 ==")
    found = la.detect_driver()
    if found:
        for d in found:
            print(f"  发现可用驱动：{d}")
    else:
        print("  没扫到任何设备：确认分析仪插好、USB 线是数据线（不是纯充电线）")
    print(f"\n当前配置的驱动：{la.driver}；通道映射：{la.channel_map or '（空 —— 必须填！）'}")
    print("通道号 0 起，对应板上丝印 CH0..CH7。填进 config.toml 的 [la.channel_map]。")
    return 0


def cmd_fixture(args: argparse.Namespace) -> int:
    """夹具标定：自动测出 DTR/RTS 的极性（靠 probe-rs 能不能连上来判断）。

    原理：把一根线接到 NRST 并**主动拉低**时，目标芯片处于复位状态、SWD 口
    是死的，probe-rs 连不上。所以"哪个 (线, 值) 组合能让 probe-rs 连不上"，
    那个组合就是"拉低"。
    """
    import time as _time

    from hal import ports, proc
    from hal.probe import Probe
    from hal.serialbus import SerialBus

    cfg = load_config()
    role = cfg.get("fixture.port_role", "fixture_uart")
    _, match = _resolve_or_explain(role)
    if match is None:
        print(f"没找到夹具串口（角色 {role}）：检查 config.toml 的 [serial.roles] 与线材")
        return 1
    probe = Probe(cfg)
    if not probe.probe_rs or not probe.has_target():
        print("需要先插上板子、并且 `probe-rs list` 能看到目标 —— 标定靠它判断 NRST 有没有被拉低")
        return 1

    bus = SerialBus(match.device, int(cfg.get("serial.default_baud", 115200)), label=role)
    try:
        bus.open()
    except RuntimeError as exc:
        print(f"打不开夹具串口 {match.device}：{exc}")
        return 1

    def reachable() -> bool:
        # 超时刻意取短：NRST 被拉低时 probe-rs 会一直等，不值得等 45 秒
        return proc.run([str(probe.probe_rs), "reset", "--chip", probe.chip], timeout_s=8).ok

    try:
        print(f"夹具口：{match.device}（{match.description}）")
        print("基线（不拉任何线）：probe-rs", "可连 ✓" if reachable() else "**连不上**（先解决这个再标定）")
        table: list[tuple[str, bool, bool]] = []
        for line in ("RTS", "DTR"):
            for value in (True, False):
                bus.set_modem_line(line, value)
                _time.sleep(0.4)
                ok = reachable()
                table.append((line, value, ok))
                print(f"  {line}={str(value):<5} → probe-rs {'可连（线没拉低 NRST）' if ok else '连不上（这根线把 NRST 拉低了）'}")
        bus.set_modem_line("RTS", False)
        bus.set_modem_line("DTR", False)
    finally:
        bus.close()

    print("\n== 结论 ==")
    nrst_line = None
    for line, value, ok in table:
        if not ok:
            nrst_line = line
            # 让 probe-rs 挂掉的那个 pyserial 值 = 引脚真的被拉低了。
            # pyserial 断言(True) → DTR#/RTS# 引脚低；说明是**直连**，没有反相那一级。
            # 反过来则说明中间过了一级三极管（自动下载电路就是这种）。
            inverted = not value
            print(f"接 NRST 的是 **{line}**；pyserial 写 {value} 时把它拉低。")
            if inverted:
                print("  → 中间有一级反相（大概率是自动下载电路的三极管），填：")
            else:
                print("  → 直连，无反相，填：")
            print(f'     line = "{line}"')
            print(f"     line_inverted = {str(inverted).lower()}")
            print("     active_low = true        # NRST 本身是低有效，这一项不变")
            break
    if nrst_line is None:
        print("四组测试里 probe-rs 都连得上 —— NRST 没接上（或模块的两个控制脚没引出来）。")
        print("先确认：模块是否 6 针、DTR/RTS 是否直通排针、是否 3.3V 供电、是否共地。")
        return 1
    print("\n按压侧的方向**不能照搬**：B1 是高有效（拉高=按下），NRST 是低有效（拉低=复位），")
    print("所以 active_low 要各写各的。确认按键的命令：`cli.py run <agent> case04`，")
    print("10 次脉冲应恰好得到 SHORT 1..10。")
    return 0


def _resolve_or_explain(role: str):
    """解析串口角色；失败时把"该填什么 / 为什么"直接打出来。

    **一定不能静默返回 None** —— 之前没匹配到时不打任何东西，调用方只返回
    退出码 1，跑起来就是"命令跑完了、什么都没说"，比报错难查得多。
    """
    from hal import ports

    cfg = load_config()
    try:
        match = ports.resolve_role(cfg, role)
    except KeyError as exc:
        print(f"{exc}\n在 config.toml 的 [serial.roles] 里配上这块板子的 VID/PID。")
        return cfg, None
    except ports.PortAmbiguity as exc:
        print(f"{exc}\n当前所有串口：")
        for p in ports.describe_all():
            print(f"  {p['device']:<8} {p['description']}  vid={p['vid']} pid={p['pid']} "
                  f"location={p['location']}")
        print("\n把上面某个 location 抄进该角色的 location = \"...\" 即可锁定。")
        return cfg, None
    if match is None:
        print(ports.diagnose(cfg, role))
    return cfg, match


def cmd_fixture_wiggle(args: argparse.Namespace) -> int:
    """让某根 modem 线周期翻转，方便你用万用表/LED 直接看它到底是谁。

    场景：手上这块 USB-TTL 除了 TX/RX 还引出了 RST/EN/IO0/IO2 四个脚，
    不知道哪根对应 DTR、哪根对应 RTS、极性如何。跑这个命令，然后拿万用表
    （直流电压档）依次量那四个脚：**跟着一起翻转的就是它**，同时也能看出
    翻转时是高电平还是低电平。
    """
    import time as _time

    from hal.serialbus import SerialBus

    cfg = load_config()
    role = cfg.get("fixture.port_role", "fixture_uart")
    _, match = _resolve_or_explain(role)
    if match is None:
        print(f"没找到夹具串口（角色 {role}）。")
        print(ports.diagnose(cfg, role))
        return 1

    line = args.line.upper()
    period = float(args.period)
    is_break = line in ("BREAK", "TXD", "TX")
    # TXLOW：不依赖 BREAK 的"把 TX 压到低位"办法。连发 0x00 时，每个字节是
    # 起始位(低)+8 个数据位(低)+停止位(高) —— 10 位里 9 位是低，万用表读到
    # 接近 0V。这比 BREAK 更能说明问题：它证明"这根线确实是 TX、而且被我们驱动着"。
    is_txlow = line in ("TXLOW", "ZERO", "TX0")
    bus = SerialBus(match.device, int(cfg.get("serial.default_baud", 115200)), label=role)
    try:
        bus.open()
    except RuntimeError as exc:
        print(f"打不开 {match.device}：{exc}")
        return 1

    if is_txlow:
        what = f"TXD（连发 0x00：≈0V ／ 空闲：3.3V）"
    elif is_break:
        what = "TXD（推挽：BREAK=低，空闲=高）"
    else:
        what = f"{line} 脚"
    print(f"{match.device} 上的 {what} 每 {period:g}s 翻转一次，共 {args.count} 次。")
    print("拿万用表直流电压档量目标脚（黑表笔接 GND）：跟着一起跳的就是它。")
    if is_txlow:
        print("**读数会是「接近 0V」而不是干净的 0V**（停止位每 10 位占 1 位），这是正常的。")
    else:
        print("**两个读数都要记**：高电平时几伏（5V 就不能接 PC13），以及翻转时是高还是低。")
    try:
        for i in range(args.count):
            high = i % 2 == 0
            if is_txlow:
                if high:
                    _time.sleep(period)
                else:
                    # 在这一个 period 里持续灌 0x00，把线压住
                    end = _time.monotonic() + period
                    while _time.monotonic() < end:
                        bus.send_bytes(b"\x00" * 128)
                        _time.sleep(0.01)
                print(f"  [{i + 1}/{args.count}] " + ("空闲(高≈3.3V)" if high else "连发 0x00(低≈0V)"))
                continue
            if is_break:
                bus.set_break(not high)
            else:
                bus.set_modem_line(line, high)
            print(f"  [{i + 1}/{args.count}] " + (f"TXD 空闲(高)" if high else f"TXD 拉低(BREAK)") if is_break
                  else f"{line} = {high}")
            _time.sleep(period)
    except KeyboardInterrupt:
        print("\n中断。")
    finally:
        try:
            if is_break or is_txlow:
                bus.set_break(False)
            else:
                bus.set_modem_line(line, False)
        except Exception:  # noqa: BLE001
            pass
        bus.close()
    print("已复位该线（现在是空闲高）。")
    return 0


def cmd_probe(args: argparse.Namespace) -> int:
    """探针侧的操作。目前只有 reset-test：标定"哪条路才是真 NRST 复位"。

    做法很硬：先写 RMVF 清掉复位标志 → 用一种方式复位 → 读 RCC_CSR 看置起了
    哪个位。拉 NRST 该出 PINRSTF，SWD 系统复位该出 SFTRSTF。**这一步是
    case11 那项判据能不能自动判的前提** —— 没标定过就只能跳过它。
    """
    from hal.probe import RESET_KIND_EXPECTS, RCC_CSR, Probe

    cfg = load_config()
    probe = Probe(cfg)
    if not probe.probe_rs or not probe.has_target():
        print("需要先插上板子、并且 `probe-rs list` 能看到目标。")
        return 1
    if probe.probe_selector:
        print(f"使用探针：{probe.probe_selector}")

    base = probe.reset_flags()
    print(f"当前 RCC_CSR（0x{RCC_CSR:08X}）：{base.get('summary')}")
    if not base.get("ok"):
        print(f"读寄存器失败：{base.get('error') or base.get('stderr_tail')}")
        return 1
    if not base.get("plausible"):
        print("! 读到的值里 RMVF 与所有复位标志都是 0 —— 地址可能不对，先别信下面的结论")
        return 1

    methods = [("swd", "probe-rs 系统复位")]
    if str(cfg.get("probe.nrst_method")) == "fixture" or cfg.get("fixture.enabled"):
        methods.append(("nrst_fixture", "夹具 modem 线拉 NRST"))
    methods.append(("nrst_programmer", "STM32_Programmer_CLI -rst"))
    methods.append(("nrst_openocd", "OpenOCD stlink + srst_only"))
    # DAPLink 也是候选：它的 RST 脚同样由 OpenOCD 驱动
    methods.append(("nrst_openocd_dap", "OpenOCD cmsis-dap(DAPLink) + srst_only"))

    rows: list[tuple[str, str, str, bool]] = []
    for key, label in methods:
        cleared = probe.clear_reset_flags()
        if not cleared.get("ok"):
            rows.append((key, label, f"清标志失败：{cleared.get('error')}", False))
            continue
        if key == "swd":
            probe.reset("swd")
        elif key == "nrst_fixture":
            from hal.gpio import Fixture, FixtureUnavailable
            from hal.serialbus import SerialBus

            try:
                match = ports.resolve_role(cfg, cfg.get("fixture.port_role", "fixture_uart"))
            except ports.PortAmbiguity as exc:
                rows.append((key, label, f"夹具角色有歧义：{exc}", False))
                continue
            if match is None:
                rows.append((key, label, "夹具串口没找到", False))
                continue
            bus = SerialBus(match.device, int(cfg.get("serial.default_baud", 115200)), label="fixture")
            try:
                bus.open()
                fx = Fixture(cfg, bus)
                fx.pulse("nrst")
            except (RuntimeError, FixtureUnavailable) as exc:
                rows.append((key, label, f"夹具不可用：{exc}", False))
                continue
            finally:
                bus.close()
        elif key == "nrst_programmer":
            probe.reset_programmer_nrst()
        elif key == "nrst_openocd_dap":
            probe.reset_openocd_srst("cmsis-dap")
        else:
            probe.reset_openocd_srst("stlink")

        after = probe.reset_flags()
        if not after.get("ok"):
            rows.append((key, label, "复位后读寄存器失败", False))
            continue
        flags = {k: v for k, v in after["flags"].items() if v and k != "RMVF"}
        want = RESET_KIND_EXPECTS.get("swd" if key == "swd" else "nrst", "")
        ok = bool(flags.get(want))
        got = ", ".join(flags) or "（无）"
        rows.append((key, label, f"{got}（期望 {want}）", ok))

    print("\n| 方式 | 说明 | 复位置起的标志 | 是我们要的吗 |")
    print("|---|---|---|---|")
    for key, label, got, ok in rows:
        print(f"| {key} | {label} | {got} | {'✓' if ok else '✗'} |")

    pin_ok = [k for k, _, _, ok in rows if ok and k.startswith("nrst_")]
    print("\n== 结论 ==")
    if pin_ok:
        best = pin_ok[0]
        method = {"nrst_fixture": "fixture", "nrst_programmer": "programmer",
                  "nrst_openocd": "openocd", "nrst_openocd_dap": "openocd"}[best]
        print(f"真 NRST 复位可用的是：**{best}** —— 在 config.toml 里填：")
        print(f'  nrst_method = "{method}"')
        if best == "nrst_openocd_dap":
            print('  openocd_interface = "cmsis-dap"    # 走 DAPLink 的 RST 脚')
        elif method == "openocd":
            print('  openocd_interface = "stlink"')
        print("填完之后 case11 的 F3（RESET=POR）与 H1b 才会真正参与判定。")
    else:
        print("没有任何一条路置起 PINRSTF —— case11 的 F3/H1b 会继续按『跳过』处理。")
        print("可查：调试器的 NRST 是否与目标板 NRST 连通；夹具是否 3.3V 供电且共地。")
    return 0


def cmd_fixture_loopback(args: argparse.Namespace) -> int:
    """验证 TXD/RXD 到底是哪两个脚、以及 BREAK 能不能真的把 TX 拉低。

    **为什么需要它**：这些适配器的丝印是**站在 ESP 模块的角度**标的 ——
    标 `RX` 的那根其实是**适配器自己的 TXD**（模块 RX ← 适配器 TX），
    标 `TX` 的那根是适配器的 RXD（模块 TX → 适配器 RX）。
    按标签接线会接到适配器的输入脚上，现象就是"什么都不动"。

    做法：把这块板上那两个候选脚**用一根杜邦线短接**，然后
      A) 发一段已知数据 —— 能原样收回来说明这两个脚**确实是 TX/RX 这一对**；
      B) 置 BREAK —— 看驱动回什么。**注意这一步给不出铁证**：有的驱动在
         break 期间返回一串 0x00，有的干脆什么都不返回，两种情况都不能证明
         "引脚真的被拉低了"。所以 BREAK 是否真的拉低电平，最终要用
         `fixture wiggle --line BREAK` 配万用表量那根线来确认。
    """
    import time as _time

    import serial

    cfg = load_config()
    role = cfg.get("fixture.port_role", "fixture_uart")
    _, match = _resolve_or_explain(role)
    if match is None:
        return 1

    payload = b"LOOPBACK-OK\r\n"
    print(f"端口 {match.device}（{match.description}）")
    print("请先把这块板上那两个候选脚短接（TX↔RX），再回车……")
    try:
        input()
    except EOFError:
        print("（非交互环境：直接开始）")

    try:
        ser = serial.Serial(match.device, int(cfg.get("serial.default_baud", 115200)), timeout=0.3)
    except serial.SerialException as exc:
        print(f"打不开端口：{exc}")
        return 1
    try:
        ser.reset_input_buffer()
        ser.write(payload)
        _time.sleep(0.2)
        got = ser.read(64)
        ok_loop = payload.strip() in got
        print(f"\nA) 回环：发出 {payload!r}，收回 {got!r}")
        print("   →", "✓ 这两个脚是 TX/RX 这一对，驱动与波特率都正常"
              if ok_loop else "✗ 没收回来说明短接的不是 TX/RX 这一对（换另外两个脚试）")

        ser.break_condition = True
        _time.sleep(0.25)
        during = ser.read(64)
        ser.break_condition = False
        print(f"B) BREAK 期间读到：{len(during)} 字节，内容 {during[:16]!r}")
        if during and set(during) <= {0}:
            print("   → 读到一串 0x00，符合'接收端一直看到低电平'的表现（**但仍不构成铁证**）")
        else:
            print("   → 驱动没给出可判读的迹象（这很正常，不算失败）")
        print("   BREAK 是否真的把引脚拉低，请用 `fixture wiggle --line BREAK` 配万用表量那根线确认。")
        return 0 if ok_loop else 2
    finally:
        ser.close()


def cmd_fixture_check_pin(args: argparse.Namespace) -> int:
    """读 PC13，请人在这段时间里按**板载 B1**。

    解决两个问题，而且都不需要接线、不需要固件、不需要图纸：

    ① **"我们看到的到底是不是 PC13"**：接错脚 / 线虚接的时候，读出来是一根悬空脚
       （现象见 `fixture verify` 的三种上下拉都一样）。板载 B1 就挂在 PC13 上，
       按一下读数跟着变，说明这根就是 PC13，且读路径没问题。
    ② **在**你自己的板子**上确认按键是高有效** —— 之前这条是三条资料推出来的，
       现在可以直接量：不按读 0、按住读 1。
    """
    import time as _time

    from hal import regs as R
    from hal.probe import Probe

    cfg = load_config()
    probe = Probe(cfg)
    if not probe.probe_rs or not probe.has_target():
        print("需要先插上板子、并让 probe-rs 能连上。")
        return 1

    def rmw(addr: int, shift: int, width: int, field: int, label: str) -> bool:
        cur = probe.read_u32(addr)
        if not cur.get("ok"):
            print(f"读 {label} 失败：{cur.get('error') or cur.get('stderr_tail')}")
            return False
        return probe.write_u32(addr, R.set_field(cur["values"][0], shift, width, field)).get("ok", False)

    en = probe.read_u32(R.RCC_AHB2ENR)
    if en.get("ok") and not R.get_bit(en["values"][0], R.GPIO_C_EN_BIT):
        probe.write_u32(R.RCC_AHB2ENR, en["values"][0] | (1 << R.GPIO_C_EN_BIT))
    if not rmw(R.GPIOC_MODER, R.PC13_MODER_SHIFT, 2, 0b00, "GPIOC_MODER"):
        return 1
    if not rmw(R.GPIOC_PUPDR, R.PC13_PUPDR_SHIFT, 2, R.BSP_BUTTON_PUPDR, "GPIOC_PUPDR"):
        return 1

    seconds = float(args.seconds)
    print(f"已把 PC13 配成输入 + 内部下拉（和 ST 的 BSP 一样）。")
    print(f"**请在接下来 {seconds:g} 秒内按住板载 B1（蓝色用户键）约 3 秒再松开。**")
    for i in range(3, 0, -1):
        print(f"  {i}…")
        _time.sleep(1.0)

    t0 = _time.monotonic()
    samples: list[tuple[float, int]] = []
    while _time.monotonic() - t0 < seconds:
        d = probe.read_u32(R.GPIOC_IDR)
        if not d.get("ok"):
            print(f"读 IDR 失败：{d.get('error') or d.get('stderr_tail')}")
            return 1
        samples.append((_time.monotonic() - t0, R.get_bit(d["values"][0], R.PC13_IDR_BIT)))

    # 压成时间线：连续同值合成一段
    spans: list[list[float | int]] = []
    for t, v in samples:
        if spans and spans[-1][2] == v:
            spans[-1][1] = t
        else:
            spans.append([t, t, v])
    print("\n读数时间线（值：0=松 / 1=按住）：")
    for a, b, v in spans:
        print(f"  {a:5.1f}s → {b:5.1f}s   {v}")

    vals = {v for _, v in samples}
    print("\n== 结论 ==")
    if len(vals) < 2:
        print(f"✗ 全程读到 {vals.pop()}，没有变化 —— 按 B1 了吗？")
        print("  若确实按了却没反应：先确认连的是这块板子的 ST-Link 口、probe-rs 连的是它。")
        return 2
    saw_high = any(v == 1 for _, v in samples)
    if saw_high:
        print("✓ 按住时读到 1、松开读到 0 —— 两件事同时确认了：")
        print("    ① 我们读的这根**确实是 PC13**（板载 B1 就挂在它上面）；")
        print("    ② **B1 是高有效**（按下 = 高），和 BSP 的 GPIO_PULLDOWN + IT_RISING 一致。")
        print("  接下来就只剩一件事：把 IO0 那根线接到这个脚上，再跑 `fixture verify`。")
    return 0
def cmd_fixture_verify(args: argparse.Namespace) -> int:
    """**让芯片自己说这根线的电平够不够** —— 不用固件，也不靠万用表。

    做法：用探针把 PC13 按 ST 的 BSP 那样配成"输入 + 内部下拉"
    （`BSP_PB_Init` 就是 `GPIO_PULLDOWN`），然后读 `GPIOC_IDR` 的第 13 位。
    这是**数字层面的判定**，比万用表读电压更贴近问题本身：
    万用表量到 2.45V 只说明"有个电压"，芯片认不认才是关键。

    三种上下拉都跑一遍，好看出余量：下拉最容易把电平压住（而 BSP 用的就是它），
    无上下拉和上拉则宽松得多。**如果只有"下拉"那一档读不到高，说明余量很薄**，
    将来换个温度或换颗片子就可能失效 —— 那种情况值得加一个 4.7k 上拉。
    """
    import time as _time

    from hal import regs as R
    from hal.gpio import Fixture
    from hal.probe import Probe
    from hal.serialbus import SerialBus

    cfg = load_config()
    role = cfg.get("fixture.port_role", "fixture_uart")
    _, match = _resolve_or_explain(role)
    if match is None:
        return 1
    probe = Probe(cfg)
    if not probe.probe_rs or not probe.has_target():
        print("需要先插上板子、并让 probe-rs 能连上。")
        return 1
    if not cfg.get("fixture.stimuli", {}).get("button"):
        print("config.toml 里没有 [fixture.stimuli.button]，没法测。")
        return 1

    def rmw(addr: int, shift: int, width: int, field: int, label: str) -> bool:
        cur = probe.read_u32(addr)
        if not cur.get("ok"):
            print(f"读 {label} 失败：{cur.get('error') or cur.get('stderr_tail')}")
            return False
        new = R.set_field(cur["values"][0], shift, width, field)
        w = probe.write_u32(addr, new)
        if not w.get("ok"):
            print(f"写 {label} 失败：{w.get('error') or w.get('stderr_tail')}")
            return False
        return True

    # 固件可能没打开 GPIOC 时钟；不开的话 IDR 恒 0，会得出"这根线永远低"的错结论
    en = probe.read_u32(R.RCC_AHB2ENR)
    if not en.get("ok"):
        print(f"读 RCC_AHB2ENR 失败：{en.get('error') or en.get('stderr_tail')}")
        return 1
    if not R.get_bit(en["values"][0], R.GPIO_C_EN_BIT):
        probe.write_u32(R.RCC_AHB2ENR, en["values"][0] | (1 << R.GPIO_C_EN_BIT))
        print("（固件没开 GPIOC 时钟，已代为打开 —— 不开的话 IDR 恒 0 会误判）")

    if not rmw(R.GPIOC_MODER, R.PC13_MODER_SHIFT, 2, 0b00, "GPIOC_MODER"):
        return 1
    back_m = probe.read_u32(R.GPIOC_MODER)
    print(f"  GPIOC_MODER 读回 0x{back_m['values'][0]:08X}（PC13 的 [27:26] = "
          f"{(back_m['values'][0] >> 26) & 3}，0 即输入）")

    bus = SerialBus(match.device, int(cfg.get("serial.default_baud", 115200)), label=role)
    try:
        bus.open()
    except RuntimeError as exc:
        print(f"打不开夹具串口 {match.device}：{exc}")
        return 1
    fx = Fixture(cfg, bus)
    fx.enabled = True
    fx.idle_unmapped()   # 开串口默认断言 DTR/RTS；交叉耦合会锁死另一根

    def read_pc13() -> int | None:
        d = probe.read_u32(R.GPIOC_IDR)
        return R.get_bit(d["values"][0], R.PC13_IDR_BIT) if d.get("ok") else None

    settle = float(args.settle)
    rows: list[tuple[str, int | None, int | None]] = []
    # 倒计时：让人有时间把杜邦头按住 / 把表笔搭好。虚接是这套夹具最常见的故障，
    # 而"按住"这个动作必须在测量开始前就位，否则第一组读数就已经错过了。
    if args.lead:
        print(f"\n**{int(args.lead)} 秒后开始测量 —— 请现在就把那根线按住别松。**")
        for i in range(int(args.lead), 0, -1):
            print(f"  {i}…")
            _time.sleep(1.0)
    try:
        for label, pud in (("下拉（和 BSP 一致）", R.PUPDR_DOWN),
                           ("无上下拉", R.PUPDR_NONE),
                           ("上拉", R.PUPDR_UP)):
            if not rmw(R.GPIOC_PUPDR, R.PC13_PUPDR_SHIFT, 2, pud, "GPIOC_PUPDR"):
                break
            # **每次拨完线都要等它稳定再读。** 实测这块板的 RST 回到高是慢的
            # （2 秒的阶段里前半段还读 0、后半段才读 1）—— 不等就会每次都读到
            # 还没升上去的低电平，把一根好用的夹具判成坏的。
            fx.set("button", False)
            _time.sleep(settle)
            loose = read_pc13()
            fx.set("button", True)
            _time.sleep(settle)
            pressed = read_pc13()
            rows.append((label, loose, pressed))
            print(f"  {label:<20} 未按下读 {loose} / 按下读 {pressed}")
            if loose == pressed:
                # 读数一样 = 这个引脚上没看到外部驱动。把原始寄存器值打出来，
                # 免得人怀疑是"工具读错了"而不是"线没接上"。
                raw = probe.read_u32(R.GPIOC_IDR)
                pu = probe.read_u32(R.GPIOC_PUPDR)
                if raw.get("ok") and pu.get("ok"):
                    print(f"      IDR=0x{raw['values'][0]:08X}  PUPDR[27:26]="
                          f"{(pu['values'][0] >> 26) & 3}  PC13 位={R.get_bit(raw['values'][0], 13)}")
    finally:
        fx.release_all()
        bus.close()

    # 第二层：EXTI 边沿检测。**它自己开串口** —— 上面那段 finally 已经把口关了，
    # 直接用 fx 会抛"串口未打开"而整段被静默跳过（第一版就是这么错的）。
    edge = _check_exti_edge(cfg, probe, match.device, role)

    print("\n== 结论 ==")

    if not rows:
        print("一项都没测成 —— 上面的错误信息才是重点。")
        return 1

    print("第一层（电平）判据：未按下应读 0、按下应读 1。")


    # 先分清"电平不够"和"线没接上"—— 这是两种完全不同的毛病，修法也不同。
    # 判据：如果**每种上下拉下、两个状态的读数都一样**，说明这个引脚上没有外部
    # 驱动在起作用（读数只由内部上下拉决定）⇒ 先查接线，别去调电平。
    # 注意"上拉那档读 1"不算外部驱动的证据：那是内部上拉自己拉上去的。
    same_everywhere = all(a == b for _, a, b in rows)
    if same_everywhere:
        print("✗ **这个引脚上没看到外部信号** —— 每种上下拉下、按下与未按下读数都相同。")
        print("  （内部上下拉能改变读数 ⇒ 我的寄存器读写是生效的，不是读了个假值）")
        print("  先查物理连接，别急着调电平：")
        print("    ① 线是不是真的接在 **PC13** 上（NUCLEO-64 的 PC13 在 morpho 排针上，")
        print("       不在 Arduino 那排；接错脚的话 PC13 就是浮空的，正好是这个现象）")
        print("    ② 线有没有松动、面包板/杜邦头有没有虚接")
        print("    ③ 最直接的验证：拿万用表量 **PC13 对 GND**，我跑一次")
        print("       `fixture wiggle --line DTR`，看 PC13 跟不跟着跳")
        return 2

    ok_rows = [r for r in rows if r[1] == 0 and r[2] == 1]
    if len(ok_rows) == len(rows):
        print(f"✓ 三种上下拉全部通过 —— 余量充足，放心用（{len(rows)}/{len(rows)}）")
        return 0
    if any(r[0].startswith("下拉") for r in ok_rows):
        bad = [r[0] for r in rows if r not in ok_rows]
        # **只有"上拉"档不过 = 形状问题，不是故障。** 这根线的"松开"是高阻，
        # 高电平由适配器那边的上拉提供；MCU 自己一上拉，读数就被自己抬走了。
        # 固件走的是 `GPIO_PULLDOWN`（ST 的 BSP 就是这样），所以判定不受影响 ——
        # 这时候建议"加 4.7k 上拉"是**反的**：那只会多一条漏电路径。
        if all(b.startswith("上拉") for b in bad) and edge == 1:
            print("✓ BSP 那一档（下拉）与『无上下拉』都通过，只有内部上拉档『未按下』读 1。")
            print("  这是**信号形状**不是接触故障：松开时外部高阻，高电平由适配器那边的上拉给，")
            print("  MCU 内部再一上拉就两个状态都成高了。固件走 GPIO_PULLDOWN ⇒ 不影响判定。")
            print("  （对照：`fixture map-pins` 能看到 DTR 断言时是灌低还是高阻。）")
            return 0
        print(f"⚠ 下拉那档通过了，但 {'、'.join(bad)} 没过 —— 这个组合本身很反常。")
        print("  先确认没有别的路径在拉这根线，再考虑加 4.7k 上拉。")
        return 2
    print("✗ 复现 BSP 那档（内部下拉）读不到高电平 —— **电平余量不足**。")
    print("  可选：① 加一个 4.7k~2.2k 上拉（接到 Nucleo 的 3V3）把源阻抗压下来；")
    print("        ② 改量 RST/EN 那根脚（`fixture wiggle --line RTS`）；")
    print("        ③ 换用 ESP32 当夹具。")
    return 2


def cmd_fixture_identify_pin(args: argparse.Namespace) -> int:
    """把 PC13 当**输出**来回翻，用万用表找出它在排针上的物理位置。

    为什么需要它：`check-pin` 已经证明"芯片的 PC13 读得到、板载 B1 挂在它上面"，
    但那只说明**芯片内部**没问题 —— 线要接的是排针上的某个孔。接线看起来对却
    完全读不到，最常见的解释就是接错了脚（NUCLEO-64 的 PC13 在 morpho 排针上，
    不在 Arduino 那一排）。

    做法：把 PC13 配成推挽输出，每秒翻一次。**跟着跳的那个脚就是 PC13。**
    安全性：PC13 现在读不到任何外部驱动（见 `fixture verify`），所以推挽输出不会
    和谁打架；就算那根线真的接在它上面、另一头是适配器的弱上拉，也只是几百微安。
    """
    import time as _time

    from hal import regs as R
    from hal.probe import Probe

    cfg = load_config()
    probe = Probe(cfg)
    if not probe.probe_rs or not probe.has_target():
        print("需要先插上板子、并让 probe-rs 能连上。")
        return 1

    def write_field(addr: int, shift: int, width: int, field: int) -> bool:
        cur = probe.read_u32(addr)
        if not cur.get("ok"):
            print(f"读 0x{addr:08X} 失败：{cur.get('error') or cur.get('stderr_tail')}")
            return False
        ok = probe.write_u32(addr, R.set_field(cur["values"][0], shift, width, field)).get("ok", False)
        return bool(ok)

    en = probe.read_u32(R.RCC_AHB2ENR)
    if en.get("ok") and not R.get_bit(en["values"][0], R.GPIO_C_EN_BIT):
        probe.write_u32(R.RCC_AHB2ENR, en["values"][0] | (1 << R.GPIO_C_EN_BIT))
    if not write_field(R.GPIOC_MODER, R.PC13_MODER_SHIFT, 2, 0b01):  # 01 = 推挽输出
        return 1
    if not write_field(R.GPIOC_PUPDR, R.PC13_PUPDR_SHIFT, 2, R.PUPDR_NONE):
        return 1

    period = float(args.period)
    print("PC13 已配成推挽输出。**万用表直流档，黑表笔接 GND，红表笔逐个点排针孔**：")
    print(f"跟着每 {period:g} 秒高低翻转的那个脚，就是 PC13。")
    # 留出找位置的时间：这一段是静止的（PC13 先给低），不然一上来就在跳、
    # 人还没把表笔搭稳
    for i in range(int(args.lead), 0, -1):
        print(f"  {i}…（现在 PC13 = 低，可以先搭好表笔）")
        _time.sleep(1.0)
    try:
        for i in range(args.count):
            high = i % 2 == 0
            cur = probe.read_u32(R.GPIOC_ODR)
            if not cur.get("ok"):
                print(f"读 GPIOC_ODR 失败：{cur.get('error') or cur.get('stderr_tail')}")
                return 1
            probe.write_u32(R.GPIOC_ODR, R.set_field(cur["values"][0], 13, 1, 1 if high else 0))
            print(f"  [{i + 1}/{args.count}] PC13 = {'高(3.3V)' if high else '低(0V)'}")
            _time.sleep(period)
    except KeyboardInterrupt:
        print("\n中断。")
    finally:
        # 还原成"输入 + 内部下拉"，也就是 BSP 眼里的状态
        write_field(R.GPIOC_PUPDR, R.PC13_PUPDR_SHIFT, 2, R.BSP_BUTTON_PUPDR)
        write_field(R.GPIOC_MODER, R.PC13_MODER_SHIFT, 2, 0b00)
    print("已还原为输入 + 内部下拉。")
    return 0


def _check_exti_edge(cfg, probe, device: str, role: str) -> int | None:
    """第二层验证：**硬件 EXTI 认不认得出一个上升沿**。

    为什么值得单独做一层：固件走的是 `GPIO_MODE_IT_RISING`（ST 的 BSP 就是），
    所以真正决定成败的不是"电平够不够高"，而是"沿有没有被认出来"。弱上拉 +
    引脚电容会让上升沿变缓，这一层能抓出来，而单纯读电平抓不出来。

    **自己开串口**：调用点的 finally 已经把夹具口关了，直接复用 fx 会抛
    "串口未打开" —— 而那样整段会被静默跳过（第一版就是这么错的，输出里只有
    一句"跳过了：串口未打开"）。

    返回 EXTI_PR1 第 13 位：1 = 检测到上升沿。
    """
    import time as _time

    from hal import regs as R
    from hal.gpio import Fixture
    from hal.serialbus import SerialBus

    def write_or(cur_value: int, addr: int, mask_on: bool) -> None:
        v = (cur_value | (1 << R.EXTI13_BIT)) if mask_on else (cur_value & ~(1 << R.EXTI13_BIT))
        probe.write_u32(addr, v)

    bus = SerialBus(device, int(cfg.get("serial.default_baud", 115200)), label=role)
    try:
        bus.open()
        fx = Fixture(cfg, bus)
        fx.enabled = True
        fx.idle_unmapped()   # 开串口默认断言 DTR/RTS；交叉耦合会锁死另一根
        fx.release_all()

        # **先把引脚配回 BSP 那一档（输入 + 内部下拉）再打沿。**
        # 第一层是按 下拉→无→上拉 跑完的，进来时 PUPDR 停在**上拉**。而这根
        # 夹具线的"松开"靠的是适配器那边的上拉（本地是高阻）—— MCU 内部也上拉时
        # 两个状态都是高，**物理上不存在上升沿**，于是这一层会报"没检测到沿"
        # 这种**假失败**（2026-10-16 实测到一次）。固件用的是 `GPIO_PULLDOWN`，
        # 判据必须在那一档下测。
        for addr, shift, field_val, label in (
                (R.GPIOC_MODER, R.PC13_MODER_SHIFT, 0b00, "GPIOC_MODER（输入）"),
                (R.GPIOC_PUPDR, R.PC13_PUPDR_SHIFT, R.PUPDR_DOWN, "GPIOC_PUPDR（下拉，同 BSP）")):
            cur = probe.read_u32(addr)
            if not cur.get("ok"):
                raise RuntimeError(f"读 {label} 失败")
            probe.write_u32(addr, R.set_field(cur["values"][0], shift, 2, field_val))

        # **先开 SYSCFG 时钟**：不打开的话对 SYSCFG_EXTICR 的写入会被丢掉，
        # EXTI13 还挂在默认的 PA13 上 → 永远等不到边沿，而现象像"夹具不行"。
        apb2 = probe.read_u32(R.RCC_APB2ENR)
        if apb2.get("ok") and not R.get_bit(apb2["values"][0], R.SYSCFG_EN_BIT):
            probe.write_u32(R.RCC_APB2ENR, apb2["values"][0] | (1 << R.SYSCFG_EN_BIT))
            print("  （固件没开 SYSCFG 时钟，已代为打开）")
        exticr = probe.read_u32(R.EXTI13_EXTICR)
        if not exticr.get("ok"):
            raise RuntimeError("读 SYSCFG_EXTICR4 失败")
        probe.write_u32(R.EXTI13_EXTICR,
                        R.set_field(exticr["values"][0], R.EXTI13_EXTICR_SHIFT, 4, R.PORT_C_CODE))
        for addr, on in ((R.EXTI_RTSR1, True), (R.EXTI_FTSR1, False), (R.EXTI_IMR1, True)):
            cur = probe.read_u32(addr)
            if not cur.get("ok"):
                raise RuntimeError(f"读 0x{addr:08X} 失败")
            write_or(cur["values"][0], addr, on)

        # **临时关掉 NVIC 上这条中断线。** 板上固件（case01 那份）调了
        # `BSP_PB_Init(BUTTON_USER, BUTTON_MODE_EXTI)`，中断一进服务程序，HAL 就会
        # 把 EXTI 挂起位清掉 —— 于是 PR 永远是 0，看起来像"没检测到边沿"，
        # 其实边沿被检测到了、只是被固件消费掉了。关掉 NVIC 就没人清，才能直接观测。
        # （这本身也是证据：固件能收到中断 ⇒ 硬件确实认出了这个沿。）
        # NVIC 是内核外设，地址在 ARMv7-M 里是固定的：ISER1/ICER1 管 IRQ32-63，
        # EXTI15_10_IRQn = 40 ⇒ ICER1 的 bit 8（头文件里核实过）。
        NVIC_ISER1, NVIC_ICER1, IRQ_BIT = 0xE000E104, 0xE000E184, 1 << 8
        iser = probe.read_u32(NVIC_ISER1)
        was_on = bool(iser.get("ok") and R.get_bit(iser["values"][0], 8))
        if was_on:
            probe.write_u32(NVIC_ICER1, IRQ_BIT)

        probe.write_u32(R.EXTI_PR1, R.EXTI_PR_CLEAR)   # 清标志
        fx.set("button", False)
        _time.sleep(0.8)
        probe.write_u32(R.EXTI_PR1, R.EXTI_PR_CLEAR)   # 清掉过程里冒出来的
        fx.set("button", True)                          # ← 制造一次上升沿
        _time.sleep(0.8)   # 上升慢，留给它时间（见第一层那条注释）
        pr = probe.read_u32(R.EXTI_PR1)
        fx.set("button", False)
        _time.sleep(0.2)
        probe.write_u32(R.EXTI_PR1, R.EXTI_PR_CLEAR)
        if was_on:
            probe.write_u32(NVIC_ISER1, IRQ_BIT)        # 恢复，别留下副作用
        if not pr.get("ok"):
            raise RuntimeError("读 EXTI_PR1 失败")
        edge = R.get_bit(pr["values"][0], R.EXTI13_BIT)
        print(f"  （固件的按键中断线原本{'开着，观察时已临时关闭、之后恢复' if was_on else '没开'}）")
    except Exception as exc:  # noqa: BLE001
        print("  跳过了：" + str(exc))
        return None
    finally:
        try:
            bus.close()
        except Exception:  # noqa: BLE001
            pass

    print(f"  EXTI_PR1[13] = {edge}（1 = 硬件检测到上升沿）")
    if edge == 1:
        print("  ✓ 上升沿被硬件认出来了 —— 固件用 GPIO_MODE_IT_RISING 就能收到按键")
    else:
        print("  ✗ 没检测到上升沿。可能电平够但沿不够陡（弱上拉 + 引脚电容），")
        print("    也可能接线仍然不通 —— 以上面第一层的结论为准")
    return edge


def cmd_fixture_map_pins(args: argparse.Namespace) -> int:
    """**由芯片自己判断**：适配器上这根脚，到底受 DTR 还是 RTS 控制、极性和形状如何。

    为什么需要它：这块 ESP-01S 板的丝印和实际网络对不上（用户从头就提醒过），
    前面几轮我一直在"猜哪根脚受哪条线控制"，绕了很多圈。这个命令把判断权交给
    芯片：按**固定顺序**拨 DTR/RTS，同时不断读 PC13，最后列出每个阶段 PC13 的值。

    用法：把线接到适配器的一个脚上 → 跑一次 → 换个脚 → 再跑一次。
    四个脚各跑一遍，映射关系就客观地出来了，不需要人盯着万用表读时序。

    判据：读的是 PC13（配成输入 + **内部下拉**，也就是 BSP 眼里的样子）。
    - 某阶段读到 **1** ⇒ 那根脚在该阶段把 PC13 拉高了（或给了足够强的上拉）
    - 全程 0 ⇒ 那根脚没在驱动，或者驱动不了（sink-only 且无上拉）
    """
    import time as _time

    from hal import regs as R
    from hal.gpio import Fixture
    from hal.probe import Probe
    from hal.serialbus import SerialBus

    cfg = load_config()
    role = cfg.get("fixture.port_role", "fixture_uart")
    _, match = _resolve_or_explain(role)
    if match is None:
        return 1
    probe = Probe(cfg)
    if not probe.probe_rs or not probe.has_target():
        print("需要先插上板子、并让 probe-rs 能连上。")
        return 1

    def rmw(addr: int, shift: int, width: int, field: int) -> bool:
        cur = probe.read_u32(addr)
        if not cur.get("ok"):
            return False
        return bool(probe.write_u32(addr, R.set_field(cur["values"][0], shift, width, field)).get("ok"))

    en = probe.read_u32(R.RCC_AHB2ENR)
    if en.get("ok") and not R.get_bit(en["values"][0], R.GPIO_C_EN_BIT):
        probe.write_u32(R.RCC_AHB2ENR, en["values"][0] | (1 << R.GPIO_C_EN_BIT))
    # 用内部下拉：这正是 BSP 的配置，也是"能不能真的按下按键"要复现的条件
    rmw(R.GPIOC_MODER, R.PC13_MODER_SHIFT, 2, 0b00)
    rmw(R.GPIOC_PUPDR, R.PC13_PUPDR_SHIFT, 2, R.PUPDR_DOWN)

    bus = SerialBus(match.device, int(cfg.get("serial.default_baud", 115200)), label=role)
    try:
        bus.open()
    except RuntimeError as exc:
        print(f"打不开夹具串口 {match.device}：{exc}")
        return 1
    fx = Fixture(cfg, bus)
    fx.enabled = True
    fx.idle_unmapped()   # 开串口默认断言 DTR/RTS；交叉耦合会锁死另一根

    phases = [("DTR 断言", {"DTR": True}),
              ("都松开", {}),
              ("RTS 断言", {"RTS": True}),
              ("都松开", {})]
    hold = float(args.hold)
    print(f"每阶段 {hold:g} 秒，循环 {args.rounds} 轮。**两种内部上下拉各跑一遍**——")
    print("只跑一种会漏判：内部下拉时，'sink-only 拉低'和'根本没接'看起来一模一样。\n")

    def sweep(pud: int) -> dict[str, list[int]]:
        rmw(R.GPIOC_PUPDR, R.PC13_PUPDR_SHIFT, 2, pud)
        out: dict[str, list[int]] = {p[0]: [] for p in phases}
        fx.release_all()
        _time.sleep(0.3)
        for _ in range(args.rounds):
            for label, want in phases:
                for line in ("DTR", "RTS"):
                    bus.set_modem_line(line, bool(want.get(line)))
                end = _time.monotonic() + hold
                while _time.monotonic() < end:
                    d = probe.read_u32(R.GPIOC_IDR)
                    if d.get("ok"):
                        out[label].append(R.get_bit(d["values"][0], R.PC13_IDR_BIT))
        return out

    try:
        rmw(R.GPIOC_MODER, R.PC13_MODER_SHIFT, 2, 0b00)
        down = sweep(R.PUPDR_DOWN)
        up = sweep(R.PUPDR_UP)
    finally:
        fx.release_all()
        bus.close()

    for title, res in (("内部下拉（BSP 的样子）", down), ("内部上拉", up)):
        print(f"[{title}]")
        for label, _ in phases:
            print(f"  {label:<12} {sorted(set(res[label]))}")

    print("\n== 结论 ==")
    base_d, dtr_d, rts_d = (sorted(set(down["都松开"])), sorted(set(down["DTR 断言"])),
                            sorted(set(down["RTS 断言"])))
    base_u, dtr_u, rts_u = (sorted(set(up["都松开"])), sorted(set(up["DTR 断言"])),
                            sorted(set(up["RTS 断言"])))
    pulls_high = 1 in base_d            # 下拉时仍读 1 ⇒ 这根脚有足够强的上拉/高驱动
    sinks_low = 0 in base_u             # 上拉时读 0 ⇒ 这根脚会往下拉
    dtr_moves = (dtr_d != base_d) or (dtr_u != base_u)
    rts_moves = (rts_d != base_d) or (rts_u != base_u)
    kind = ("有上拉（松开即高）" if pulls_high else
            "会往下拉（sink-only）" if sinks_low else "什么都没测得")
    print(f"这根脚的形状：**{kind}**")
    if dtr_moves and not rts_moves:
        print("受 **DTR** 控制")
    elif rts_moves and not dtr_moves:
        print("受 **RTS** 控制")
    elif dtr_moves and rts_moves:
        print("DTR/RTS 都能动它（数据不足或两根线都接了）")
    else:
        print("**两根线都不动它** —— 它不在这块板的自动下载电路里，或者没接进信号")
    if pulls_high and not dtr_moves and not rts_moves:
        print("⇒ 有上拉但不受控：可以靠它把 PC13 拉到高（当'一直按下'用），但不能做按键")
    return 0



    base = sorted(set(results["都松开"])) if results.get("都松开") else []
    dtr_vals = sorted(set(results["DTR 断言(低)"]))
    rts_vals = sorted(set(results["RTS 断言(低)"]))
    if not any(v == 1 for v in (base + dtr_vals + rts_vals)):
        print("三个阶段 PC13 全是 0 —— 这根脚**既没把 PC13 拉高、也没给出上拉**。")
        print("  含义：它是 sink-only（只会往下拉）且没有上拉电阻，或者根本没接到信号。")
        print("  ⇒ 这种脚**做不了高有效的按键**（按下需要高电平）。换一个脚再跑一遍。")
        return 2
    if dtr_vals != base and rts_vals == base:
        print(f"✓ **DTR 控制这根脚**：松开时 {base}，DTR 断言时 {dtr_vals}")
    elif rts_vals != base and dtr_vals == base:
        print(f"✓ **RTS 控制这根脚**：松开时 {base}，RTS 断言时 {rts_vals}")
    else:
        print("两个阶段都跟松开时不同（或都相同）—— 数据不足以判定，多跑几轮看看。")
    if 1 in base:
        print("  注意：**松开时就是高** ⇒ 这根脚有上拉，能直接驱动高有效的 B1 ✓")
    else:
        print("  注意：松开时是低 ⇒ 需要外部上拉才能驱动高有效的 B1")
    return 0


def cmd_fixture_verify_jumper(args: argparse.Namespace) -> int:
    """验证继电器触点真的能做"可控跳线"：**PA8 驱动、读 PA0**，不看别的。

    做法：PA8 配成推挽输出驱动**高**，PA0 配成输入 + 内部下拉。
      - 触点断开 → PA0 悬空（被内部下拉拉到 0）→ 读 **0**
      - 触点闭合 → PA0 被 PA8 拉高 → 读 **1**
    这一条把"接线对不对、触发极性对不对、继电器真的动作了吗"三件事一次定死，
    而且**不需要任何固件**。

    **不假设它受哪条线控制** —— DTR 和 RTS 都试一遍。前面已经因为"猜哪根脚受
    哪条线控制"绕过一大圈，这里直接量。
    """
    import time as _time

    from hal import regs as R
    from hal.gpio import Fixture
    from hal.probe import Probe
    from hal.serialbus import SerialBus

    cfg = load_config()
    role = cfg.get("fixture.port_role", "fixture_uart")
    _, match = _resolve_or_explain(role)
    if match is None:
        return 1
    probe = Probe(cfg)
    if not probe.probe_rs or not probe.has_target():
        print("需要先插上板子、并让 probe-rs 能连上。")
        return 1

    def rd(addr: int) -> int:
        d = probe.read_u32(addr)
        if not d.get("ok"):
            raise RuntimeError(f"读 0x{addr:08X} 失败")
        return d["values"][0]

    def setf(addr: int, shift: int, width: int, field: int) -> None:
        probe.write_u32(addr, R.set_field(rd(addr), shift, width, field))

    en = rd(R.RCC_AHB2ENR)
    if not R.get_bit(en, R.GPIO_C_EN_BIT):          # GPIOC 时钟（PA 同属 AHB2）
        probe.write_u32(R.RCC_AHB2ENR, en | (1 << R.GPIO_C_EN_BIT))
    setf(R.GPIOA_MODER, 16, 2, 0b01)                # PA8 = 推挽输出
    setf(R.GPIOA_PUPDR, 16, 2, R.PUPDR_NONE)
    setf(R.GPIOA_MODER, 0, 2, 0b00)                 # PA0 = 输入
    setf(R.GPIOA_PUPDR, 0, 2, R.PUPDR_DOWN)         # 下拉：触点断开时读 0

    pa8_high = R.set_field(rd(R.GPIOA_ODR), 8, 1, 1)
    pa8_low = R.set_field(rd(R.GPIOA_ODR), 8, 1, 0)
    probe.write_u32(R.GPIOA_ODR, pa8_high)
    print("PA8 已驱动为高（源）；PA0 = 输入 + 内部下拉（探测器）。\n")

    bus = SerialBus(match.device, int(cfg.get("serial.default_baud", 115200)), label=role)
    try:
        bus.open()
    except RuntimeError as exc:
        print(f"打不开夹具串口 {match.device}：{exc}")
        probe.write_u32(R.GPIOA_ODR, pa8_low)
        return 1
    fx = Fixture(cfg, bus)
    fx.enabled = True
    fx.idle_unmapped()   # 开串口默认断言 DTR/RTS；交叉耦合会锁死另一根

    results: dict[str, tuple[int, int]] = {}
    try:
        for line in ("DTR", "RTS"):
            bus.set_modem_line("DTR", False)
            bus.set_modem_line("RTS", False)
            _time.sleep(args.settle)
            open_val = R.get_bit(rd(R.GPIOA_IDR), 0)
            bus.set_modem_line(line, True)          # 断言 = 该线拉低
            _time.sleep(args.settle)
            shut_val = R.get_bit(rd(R.GPIOA_IDR), 0)
            bus.set_modem_line(line, False)
            results[line] = (open_val, shut_val)
            print(f"  {line}：松开时 PA0 读 {open_val} → 断言时读 {shut_val}")
    finally:
        bus.set_modem_line("DTR", False)
        bus.set_modem_line("RTS", False)
        fx.release_all()
        bus.close()
        probe.write_u32(R.GPIOA_ODR, pa8_low)       # 收尾：PA8 拉低

    print("\n== 结论 ==")
    print("判据：触点断开应读 0，闭合应读 1（PA8 在驱动高）。")
    hit = [ln for ln, (o, c) in results.items() if o == 0 and c == 1]
    if hit:
        print(f"✓ **{hit[0]} 能控制这根跳线** —— 断言即闭合（对应'低电平触发'档）")
        print(f"  ⇒ config 里 jumper 用 line = \"{hit[0]}\"、active_low = true")
        return 0
    if all(o == 0 and c == 0 for o, c in results.values()):
        print("✗ 两条线都拉不动触点。查：① 模块 VCC 是不是 5V（3V3 带不动线圈）")
        print("     ② 模块跳线帽是不是设在'低电平触发' ③ COM/NO 是不是接在 PA8/PA0 上")
        return 2
    print(f"✗ 读数反常（{'／'.join(f'{k}:{v}' for k, v in results.items())}）—— 先查 PA0 上有没有别的东西在拉它")
    return 2


def cmd_readiness(args: argparse.Namespace) -> int:
    """**只做就绪检查，绝不碰硬件。**

    为什么需要它：我做过一次"对 14 道题各跑一遍看看哪些就绪"的扫描 ——
    结果是其中两道题的工作区其实可跑，于是**真的执行了硬件评分**。
    想要"预演"就不该用"真跑"当工具。

    本命令回答三个问题：
      ① 这道题要求哪些能力，现在缺哪几项（缺 → 那几步会被跳过/推给 AI）
      ② agent 的工作区里有没有可烧的固件
      ③ 工作区是否已被改动（血统）—— 只读检查，不执行任何步骤
    """
    from engine import spec as SP
    from hal import caps as CAPS
    from hal.gpio import Fixture
    from engine.runner import find_firmware

    cfg = load_config()
    specs = sorted(Path(cfg.specs).glob("*.toml"))
    c = CAPS.probe_caps(cfg, check_hardware=True, fixture=Fixture(cfg, None))
    agents = [args.agent] if args.agent else ["firm", "claude"]   # 只测这两个（用户定的）
    all_missing = c.require(sorted(c.caps))          # 把"全部能力"过一遍，缺失的就是这些
    print(f"能力缺失总览：{', '.join(all_missing) or '无'}\n")
    print(f"{'题':<9} {'权重':>4} {'固件':<8} {'就绪':<6} 缺什么")
    for sp_path in specs:
        try:
            spec = SP.load_spec(sp_path)
        except Exception as exc:  # noqa: BLE001
            print(f"{sp_path.stem:<9} 解析失败：{exc}")
            continue
        need = spec.required_caps()
        missing = c.require(need)
        fw = []
        for a in agents:
            # **复用 harness 自己的 find_firmware()**，别在这儿另写一份 glob ——
            # 我第一版只找 .pio/build/*/firmware.elf，于是把"有 CubeIDE Debug 构建"
            # 的题误报成"无固件"（case11 明明刚跑成）。两份真相必然对不上。
            wd = Path(cfg.testset) / "runs" / a / spec.id
            fw.append(f"{a}:{'有' if (wd.exists() and find_firmware(wd)) else '无'}")
        ready = "就绪" if not missing else "缺能力"
        print(f"{spec.id:<9} {spec.weight:>4} {' '.join(fw):<{8 * len(agents) + len(agents) - 1}} "
              f"{ready:<6} {', '.join(missing) or '-'}")
    print("\n提示：缺能力的步骤会被跳过并推给 AI 裁定（不会误判成失败）。本检查不写任何文件。")
    return 0


def cmd_batch(args: argparse.Namespace) -> int:
    """批量跑一个 agent 的多道题，汇总成一张成绩单。

    为什么需要它：在这之前只能"一题一题跑" —— 一轮 14 题要敲 14 次命令，
    再人工把分数抄到一起。批量 + 汇总本来就该是一条命令。

    **只跑"有固件"的题**：工作区里没有可烧的固件就直接记一行"无固件"、不碰硬件。
    这条是硬规则 —— 我早先拿"真跑"当"预演"，把两道还没准备好评的题跑掉了。

    成绩单里会带上每个 agent 声明的**模型与思考强度**：两个 agent 不一致时分数
    没有可比性，而这件事不会自己暴露（表面上都是"跑了同一道题"）。
    """
    from engine import spec as SP
    from engine.runner import Runner, find_firmware

    import time as _time

    cfg = load_config()
    agents = [args.agent] if args.agent else ["firm", "claude"]   # 只测这两个（用户定的）
    if args.cases:
        wanted = [c.strip().removeprefix("case") for c in args.cases.split(",")]
    else:
        wanted = sorted(p.stem.replace("case", "") for p in Path(cfg.specs).glob("case*.toml"))

    rows: list[dict[str, Any]] = []
    for agent in agents:
        for num in wanted:
            case = f"case{num}"
            spec_path = Path(cfg.specs) / f"{case}.toml"
            if not spec_path.exists():
                print(f"  {agent}/{case}: 规格不存在，跳过")
                continue
            wd = Path(cfg.testset) / "runs" / agent / case
            if not (wd.exists() and find_firmware(wd)):
                print(f"  {agent}/{case}: 无固件 → 不跑（不碰硬件）")
                rows.append({"agent": agent, "case": case, "status": "无固件"})
                continue
            spec = SP.load_spec(spec_path)
            print(f"  {agent}/{case}: 跑…", flush=True)
            outcome = Runner(cfg, spec, agent, workdir=wd).run()
            s = outcome.summary()
            rows.append({"agent": agent, "case": case, "status": "已跑", **s})
            print(f"      → {s['auto_score']} / {s['max_in_scope']}"
                  f"（待 AI {', '.join(s['pending_ai']) or '无'}）", flush=True)

    # ---- 汇总 ----
    ts = _time.strftime("%Y%m%d-%H%M%S")
    out_dir = Path(cfg.reports) / f"batch-{ts}"
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "batch.json").write_text(
        json.dumps({"agents": {a: (cfg.get("agents", {}) or {}).get(a) for a in agents},
                    "rows": rows}, ensure_ascii=False, indent=2), encoding="utf-8")

    lines = ["# 批量评测成绩单", "", f"- 生成时间：{ts}", ""]
    for agent in agents:
        info = (cfg.get("agents", {}) or {}).get(agent)
        cfg_txt = (f"model = `{info.get('model', '?')}`｜thinking = `{info.get('thinking', '?')}`"
                   if info else "**未声明**（config.toml 的 [agents.<名>]）")
        mine = [r for r in rows if r["agent"] == agent]
        if not mine:
            continue
        lines += [f"## {agent}", "", f"- 被测 agent 配置：{cfg_txt}", "",
                  "| 题 | 权重 | 自动判定 | 满分(在范围内) | 待 AI 裁定 | 状态 |",
                  "|---|---|---|---|---|---|"]
        tot_auto = tot_max = 0.0
        for r in mine:
            if r["status"] != "已跑":
                lines.append(f"| {r['case']} | — | — | — | — | **无固件** |")
                continue
            tot_auto += r["auto_score"]
            tot_max += r["max_in_scope"]
            pending = ", ".join(r["pending_ai"]) or "无"
            note = "" if r["bloodline_ok"] else "**血统违规 → 0**"
            lines.append(f"| {r['case']} | {r['weight']} | {r['auto_score']} | "
                         f"{r['max_in_scope']} | {pending} | {note or 'OK'} |")
        lines += ["", f"**小计：{round(tot_auto, 2)} / {round(tot_max, 2)}**"
                      f"（含待 AI 裁定的点，最终分只会更高）", ""]
    report = out_dir / "scoreboard.md"
    report.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"\n成绩单：{report}")
    return 0


def cmd_regrade(args: argparse.Namespace) -> int:
    """用**当前这把尺子**重判已经落盘的报告 —— 不碰板子、不花 token。

    判分口径每改一次都要重烧板子来验证的话，改动就不可回归；有了重判，
    "旧报告 vs 重判后" 才是纯软件对照。
    """
    from regrade import run_one

    cfg = load_config()
    target = Path(args.target)
    if not target.exists():
        target = Path(cfg.reports) / args.target
    if args.all:
        dirs = sorted(d for d in target.iterdir() if (d / "evidence" / "result.json").exists())
        if not dirs:
            print(f"{target} 下面没有可重判的 run 目录")
            return 1
    else:
        dirs = [target]

    total_changed = 0
    for d in dirs:
        try:
            p = run_one(cfg, d, only_rule=args.rule)
        except Exception as exc:  # noqa: BLE001 - 一题重判失败不该带走整批
            print(f"! {d.name} 重判失败：{type(exc).__name__}: {exc}")
            continue
        if args.rule:
            print(f"{d.name} 规则 {args.rule} 重算：")
            for rid, r in p["rules"].items():
                verdict = {True: "通过", False: "不通过", None: "无法判定"}[r["ok"]]
                print(f"  [{verdict:<4}] {rid:<12} {r.get('note') or r.get('want') or ''}")
            continue
        changed = [x for x in p["diff"] if x["changed"]]
        total_changed += len(changed)
        print(f"\n== {d.name}：机器自动分 {p['old_auto_score']} → {p['new_auto_score']}"
              f"（判定变化 {len(changed)}/{len(p['diff'])} 条）")
        for x in changed:
            v = {True: "✓", False: "✗", None: "—"}
            print(f"   {x['id']:<6} {v[x['old']]} → {v[x['new']]}  {x['note'][:60]}")
        for w in p["warnings"]:
            print(f"   ! {w}")
        print(f"   产物：{p['regrade_dir']}")
    if args.all and not args.rule:
        print(f"\n合计判定变化：{total_changed} 条")
    return 0


def cmd_report(args: argparse.Namespace) -> int:
    d = Path(args.run_dir)
    if d.is_file():
        d = d.parent
    md = d / "report.md"
    if md.exists():
        print(md.read_text(encoding="utf-8"))
    res = d / "evidence" / "result.json"
    if res.exists():
        data = json.loads(res.read_text(encoding="utf-8"))
        print("\n== summary ==")
        print(json.dumps(data["summary"], ensure_ascii=False, indent=2))
    return 0


def main(argv: list[str] | None = None) -> int:
    # 控制台默认 GBK，而报告里有 ✓ / — 这些字符 —— 不转的话"重打印某轮报告"
    # 这类纯读操作会跑到一半抛 UnicodeEncodeError，比没输出更误导人。
    for s in (sys.stdout, sys.stderr):
        try:
            s.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[union-attr]
        except Exception:  # noqa: BLE001
            pass
    ap = argparse.ArgumentParser(prog="cli.py", description="14 题嵌入式基准自动评测器")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("caps", help="探测当前可用能力")
    p.add_argument("--offline", action="store_true", help="不探测板子（只看工具链与端口）")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_caps)

    p = sub.add_parser("spec", help="校验并打印 spec")
    p.add_argument("case", nargs="?")
    p.set_defaults(func=cmd_spec)

    p = sub.add_parser("rules", help="只跑静态代码规则")
    p.add_argument("case")
    p.add_argument("workdir", nargs="?")
    p.add_argument("--agent", default="firm")
    p.add_argument("--rule", action="append", help="只跑指定规则（可多次）")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_rules)

    p = sub.add_parser("run", help="跑一轮评测")
    p.add_argument("agent", choices=["firm", "cc", "claude"])
    p.add_argument("case")
    p.add_argument("--sim", metavar="FIXTURE", help="用回放 fixture 代替真板（无硬件自测）")
    p.add_argument("--workdir", help="覆盖工作目录（只给自测用）")
    p.set_defaults(func=cmd_run)

    p = sub.add_parser("report", help="重打印某轮报告")
    p.add_argument("run_dir")
    p.set_defaults(func=cmd_report)

    p = sub.add_parser("regrade", help="用当前判分器重判已落盘的报告（不碰硬件）")
    p.add_argument("target", help="run 目录，或 --all 时的父目录")
    p.add_argument("--all", action="store_true", help="重判该目录下所有 run")
    p.add_argument("--rule", help="只重算一条静态规则，如 case04.H2")
    p.set_defaults(func=cmd_regrade)

    p = sub.add_parser("la", help="逻辑分析仪自检（驱动、设备、通道映射）")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_la)

    p = sub.add_parser("probe", help="调试探针操作")
    sub_pb = p.add_subparsers(dest="action", required=True)
    q = sub_pb.add_parser("reset-test", help="标定哪条路才是真 NRST 复位（需插板）")
    q.set_defaults(func=cmd_probe)

    p = sub.add_parser("fixture", help="夹具操作")
    sub_fx = p.add_subparsers(dest="action", required=True)
    q = sub_fx.add_parser("calibrate", help="自动测 DTR/RTS 极性（需插板）")
    q.set_defaults(func=cmd_fixture)
    q = sub_fx.add_parser("wiggle", help="周期翻转一根线，配合万用表找出哪根脚对应它")
    q.add_argument("--line", default="RTS",
                   choices=["RTS", "DTR", "rts", "dtr", "BREAK", "TXD", "break", "txd",
                            "TXLOW", "ZERO", "txlow", "zero"],
                   help="RTS/DTR=modem 线；BREAK=用 break 把 TX 拉低；TXLOW=连发 0x00 把 TX 压低（不依赖 break）")
    q.add_argument("--period", default=1.0, type=float, help="翻转周期（秒）")
    q.add_argument("--count", default=10, type=int, help="翻转几次")
    q.set_defaults(func=cmd_fixture_wiggle)
    q = sub_fx.add_parser("loopback", help="短接 TX↔RX 验证引脚与驱动（解决丝印误导）")
    q.set_defaults(func=cmd_fixture_loopback)
    q = sub_fx.add_parser("verify", help="两层验证按键通路（电平 + EXTI 边沿），不用固件")
    q.add_argument("--lead", default=5, type=int, help="测量前倒计时秒数（留时间按住线/搭表笔）")
    q.add_argument("--settle", default=1.2, type=float, help="每次拨线后等待稳定的秒数")
    q.set_defaults(func=cmd_fixture_verify)
    q = sub_fx.add_parser("check-pin", help="按板载 B1 确认读到的就是 PC13 且按键高有效")
    q.add_argument("--seconds", default=10.0, type=float, help="采样时长（秒）")
    q.set_defaults(func=cmd_fixture_check_pin)
    q = sub_fx.add_parser("identify-pin", help="把 PC13 当输出来回翻，用万用表找出它在排针上的位置")
    q.add_argument("--period", default=1.0, type=float, help="翻转周期（秒）")
    q.add_argument("--count", default=16, type=int, help="翻转次数")
    q.add_argument("--lead", default=5, type=int, help="开始翻转前的倒计时秒数（留时间搭表笔）")
    q.set_defaults(func=cmd_fixture_identify_pin)
    q = sub_fx.add_parser("verify-jumper", help="验证继电器触点能做可控跳线（PA8 驱动、读 PA0）")
    q.add_argument("--settle", default=1.0, type=float, help="继电器动作后等待秒数")
    q.set_defaults(func=cmd_fixture_verify_jumper)
    q = sub.add_parser("readiness", help="只做就绪检查，绝不碰硬件（扫全部题或指定 agent）")
    q.add_argument("--agent", default=None, help="只看某个 agent 的固件（默认全部）")
    q.set_defaults(func=cmd_readiness)
    q = sub.add_parser("batch", help="批量跑一个 agent 的多道题，汇总成成绩单（只跑有固件的题）")
    q.add_argument("--agent", default="firm", help="被测 agent（默认 firm）")
    q.add_argument("--cases", default=None, help="题号，逗号分隔，如 01,04,06；默认全部")
    q.set_defaults(func=cmd_batch)
    q = sub_fx.add_parser("map-pins", help="由芯片判断这根脚受 DTR 还是 RTS 控制（逐个脚跑）")
    q.add_argument("--hold", default=1.5, type=float, help="每个阶段持续秒数")
    q.add_argument("--rounds", default=3, type=int, help="循环轮数")
    q.set_defaults(func=cmd_fixture_map_pins)

    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
