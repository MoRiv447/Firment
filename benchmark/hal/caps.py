"""能力探测：这一轮到底能测什么。

每个能力的 key 与 spec 的声明一一对应：

    serial:dut_vcp / serial:fixture_uart / serial:bt_peer
    flash / reset:swd / reset:nrst / reset:power
    la / camera / gpio:<激励名>

探测结果落盘到 run 目录的 caps.json。**缺能力不报错**：spec 的
``[caps.fallback]`` 决定是 skip（标注后不计分）还是 fail。
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from hal import ports, proc
from hal.gpio import Fixture
from hal.probe import Probe
from hal.la import LogicAnalyzer as LA


@dataclass
class Caps:
    caps: dict[str, bool] = field(default_factory=dict)
    detail: dict[str, Any] = field(default_factory=dict)

    def has(self, key: str) -> bool:
        return bool(self.caps.get(key, False))

    def require(self, keys: list[str]) -> list[str]:
        """返回缺失的 key 列表（空列表 = 全有）。"""
        return [k for k in keys if not self.has(k)]

    def to_dict(self) -> dict[str, Any]:
        return {"caps": self.caps, "detail": self.detail}


def nrst_capable(method: str, *, fixture_ok: bool, target_ok: bool,
                 programmer_ok: bool, openocd_ok: bool) -> bool:
    """"能不能做真 NRST 复位"的判定策略（纯函数，便于单测）。

    为什么值得单独抽出来：这条判定直接决定 case11 的 F3/H1b 会不会参与打分。
    它一旦放宽，那两项就会在"其实只是系统复位"的情况下拿到达标，而报告上
    看不出区别。所以策略要能被测，而测它不该需要一块真板子。

    规则：**未标定（"none"）一律不算具备**，哪怕工具链齐全 —— 工具在不在，
    和"这条路的复位到底是不是引脚复位"是两件事，后者只能靠实测 RCC_CSR 确认。
    """
    if method == "fixture":
        return fixture_ok and target_ok
    if method == "programmer":
        return target_ok and programmer_ok
    if method == "openocd":
        return target_ok and openocd_ok
    return False


def probe_caps(cfg: Any, *, check_hardware: bool = True, fixture: Fixture | None = None) -> Caps:
    caps: dict[str, bool] = {}
    detail: dict[str, Any] = {}

    # ---- 工具链（不需要板子在场）---------------------------------------
    probe_rs = proc.which(cfg.get("toolchain.probe_rs", "probe-rs"))
    caps["tool:probe-rs"] = probe_rs is not None
    detail["probe_rs"] = probe_rs

    oc_dir = cfg.get("toolchain.openocd_dir")
    oc_ok = False
    if oc_dir:
        root = Path(oc_dir)
        oc_ok = (root / "bin" / "openocd.exe").exists() or bool(
            list(root.parent.glob("tool-openocd*/bin/openocd.exe"))
        )
    caps["tool:openocd"] = oc_ok

    prog = cfg.get("toolchain.stm32_programmer_cli")
    caps["tool:stm32_programmer_cli"] = bool(prog and Path(prog).exists())

    la_bin = proc.which(cfg.get("la.bin", "sigrok-cli"))
    caps["tool:sigrok-cli"] = la_bin is not None
    detail["sigrok_cli"] = la_bin
    # **`la` 必须真的扫到"真仪器"才算具备。** 只检查"工具装着、驱动在清单里"是假阳性：
    # 分析仪没插也会报 OK，于是 `la_capture` 步骤跑起来失败，
    # 而报告上看起来像"夹具/固件的问题"。（和 `gpio:nrst` 那次同一类错误。）
    # 还有两个坑：
    #  ① sigrok 自带一个 **`demo` 假驱动**，它永远"扫得到设备"—— 必须排除，
    #     否则仪器没插也报 OK（第一版就是这么又假了一次）。
    #  ② 要**点名配置的那个驱动**：装了别的型号驱动不算这台机器具备能力。
    la_found = LA(cfg).detect_driver() if (la_bin and check_hardware) else []
    la_real = [d for d in la_found if d != "demo"]
    la_want = str(cfg.get("la.driver", "auto"))
    caps["la"] = (la_want in la_real) if la_want != "auto" else bool(la_real)
    detail["la_drivers_found"] = la_found
    detail["la_drivers_real"] = la_real
    detail["la_driver_configured"] = la_want

    ffmpeg = proc.which(cfg.get("camera.ffmpeg", "ffmpeg"))
    caps["tool:ffmpeg"] = ffmpeg is not None
    detail["ffmpeg"] = ffmpeg
    caps["camera"] = ffmpeg is not None

    # ---- 串口与目标 ----------------------------------------------------
    detail["ports"] = ports.describe_all()
    for role in ("dut_vcp", "fixture_uart", "bt_peer"):
        try:
            match = ports.resolve_role(cfg, role)
            ambiguous = ""
        except KeyError:
            match, ambiguous = None, "config 里没定义这个角色"
        except ports.PortAmbiguity as exc:
            # 匹配到多个同型号板：**不算具备这个能力**，并把人该填什么说清楚
            match, ambiguous = None, str(exc)
        caps[f"serial:{role}"] = match is not None
        detail[f"port:{role}"] = match.as_dict() if match else None
        if ambiguous:
            detail[f"port:{role}"] = {"error": ambiguous}
        if role == "bt_peer" and match is not None:
            # Windows 会给"配对过但当前不在线"的蓝牙设备保留出站端口，
            # 所以端口枚举存在 ≠ 对端在线。打开能成功、写进去也会被静默丢弃，
            # 只能靠真实收发验证 —— 别把这个 ✓ 当成"蓝牙通"。
            detail["port:bt_peer"]["note"] = (
                "端口已注册，但对端可能不在线（蓝牙设备关机后出站端口仍会保留）。"
                "跑 case13 之前先用串口工具发一句看有没有回显。"
            )

    if check_hardware:
        probe = Probe(cfg)
        caps["probe"] = probe.present()
        caps["target"] = probe.has_target() if probe.present() else False
        detail["probe_list"] = probe.list_probes().stdout.strip()[:400]
    else:
        caps["probe"] = probe_rs is not None
        caps["target"] = False
        detail["probe_list"] = "(skipped)"

    caps["flash"] = bool(caps["target"] and (caps["tool:probe-rs"] or caps["tool:openocd"]))

    # ---- 复位：SWD 只要 probe-rs 能连上；NRST 必须走**已标定过**的路径 -------
    caps["reset:swd"] = bool(caps["target"] and caps["tool:probe-rs"])
    nrst_method = str(cfg.get("probe.nrst_method", "none"))
    caps["reset:nrst"] = nrst_capable(
        nrst_method,
        fixture_ok=bool(fixture and fixture.available("nrst")),
        target_ok=bool(caps["target"]),
        programmer_ok=bool(caps["tool:stm32_programmer_cli"]),
        openocd_ok=bool(caps["tool:openocd"]),
    )
    detail["nrst_method"] = nrst_method
    caps["reset:power"] = False  # 受控电源需要外接继电器，别假装支持

    # ---- 夹具激励 -----------------------------------------------------
    if fixture is not None:
        for name in fixture.stimuli:
            caps[f"gpio:{name}"] = fixture.available(name)

    return Caps(caps=caps, detail=detail)
