"""端口解析。

铁律：不写死 COM 号。复位 / 断电 / 重新枚举之后 COM 会漂，本机现在
注册表里躺着 4 个 ST-Link、6 个 CH340 的历史记录（全是 ghost），
只有按 VID/PID + 描述匹配才靠得住。
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

import serial.tools.list_ports as list_ports


@dataclass
class PortMatch:
    role: str
    device: str
    description: str
    hwid: str
    vid: int | None
    pid: int | None
    serial_number: str | None

    def as_dict(self) -> dict[str, Any]:
        return {
            "role": self.role,
            "device": self.device,
            "description": self.description,
            "vid": f"0x{self.vid:04X}" if self.vid is not None else None,
            "pid": f"0x{self.pid:04X}" if self.pid is not None else None,
            "serial": self.serial_number,
        }


def all_ports() -> list[Any]:
    return sorted(list_ports.comports(), key=lambda p: p.device)


def _location_base(loc: str) -> str:
    """location 的**物理口前缀**。

    Windows 给复合设备的 location 长这样：``1-1.4:x.2``；而同一个口上的普通设备
    可能只报 ``1-1.4`` —— 那个 ``:x.N`` 后缀会随枚举情况出现/消失
    （2026-10-15 实测：同一块板、同一个口，一次报 ``1-1.4:x.2``，另一次报 ``1-1.4``）。
    所以比较一律取冒号前的部分，否则"插回原来的口"也会匹配失败，
    而报错看起来像"板子没插"，把人往错方向带。
    冒号后的部分**不参与区分**：不同物理口差在 ``1-1.4`` / ``1-1.3`` 上。
    """
    return str(loc).split(":", 1)[0].strip()


def _matches(role_cfg: dict[str, Any], port: Any) -> bool:
    # 显式点名 COM 号：最直接，但换个 USB 口 COM 号可能会变
    want_dev = role_cfg.get("device")
    if want_dev and str(want_dev).upper() != str(port.device).upper():
        return False
    # 按**物理 USB 口**点名：location 是集线器/端口路径，换口才变，重启不变。
    # 两块都是 CH340 的板子并存时，这是唯一稳的区分方式。
    want_loc = role_cfg.get("location")
    if want_loc:
        loc = getattr(port, "location", None) or _location_from_hwid(port.hwid or "")
        if _location_base(want_loc) != _location_base(loc):
            return False
    vid = role_cfg.get("vid")
    pid = role_cfg.get("pid")
    # vid/pid 为 0 表示"不校验这一项"——蓝牙 SPP 这类虚拟口没有 VID/PID。
    if vid:
        if port.vid is None or port.vid != int(vid):
            return False
    if pid:
        if port.pid is None or port.pid != int(pid):
            return False
    needle = role_cfg.get("description_contains")
    if needle and needle not in (port.description or ""):
        return False
    # hwid 里的匹配：蓝牙有一堆"本机虚拟"端口（LOCALMFG&0000）与真实配对的
    # 远端设备混在一起，描述字符串一模一样，只能靠实例 ID 区分。
    hwid_needle = role_cfg.get("hwid_contains")
    if hwid_needle and hwid_needle not in (port.hwid or ""):
        return False
    return True


def _location_from_hwid(hwid: str) -> str:
    """老版本 pyserial 没有 ``port.location``，从 hwid 里抠 LOCATION=。"""
    for tok in hwid.split():
        if tok.startswith("LOCATION="):
            return tok.split("=", 1)[1]
    return ""


class PortAmbiguity(RuntimeError):
    """同一个角色匹配到多个端口。

    为什么要炸而不是"取第一个"：这个端口是用来**拨动物理电平**的。
    两块都是 CH340 的板子并存时，描述和 VID/PID 完全一样，随便挑一个
    可能把命令发到另一块板子上 —— 表现是"按键没反应"，而且完全看不出原因。
    宁可在这里停住，让人指名道姓（device= 或 location=）。
    """


def _role_cfg(cfg: Any, role: str) -> dict[str, Any]:
    """取某个角色的匹配条件。没定义就直接报出来 —— 静默返回空字典会让
    "配置写错角色名"表现为"板子没插"，排查方向完全是错的。"""
    for item in cfg.get("serial.roles", []) or []:
        if item.get("role") == role:
            return item
    raise KeyError(f"config.toml 里没有定义串口角色：{role}")


def resolve_role(cfg: Any, role: str) -> PortMatch | None:
    """按角色解析端口。

    多个候选时**不猜**：抛 PortAmbiguity 并把候选列出来，由人用
    ``device = "COM9"`` 或 ``location = "1-2:x.1"`` 指定。
    一个都没匹配到就返回 None —— 想知道**为什么**，调 ``diagnose()``。
    """
    role_cfg = _role_cfg(cfg, role)

    hits = [p for p in all_ports() if _matches(role_cfg, p)]
    if not hits:
        return None
    if len(hits) > 1:
        detail = ", ".join(
            f"{p.device}({p.description or '?'} @ {getattr(p, 'location', '') or _location_from_hwid(p.hwid or '') or '?'})"
            for p in hits
        )
        raise PortAmbiguity(
            f"角色 {role} 匹配到 {len(hits)} 个端口：{detail}。"
            "两块同型号 CH340 板并存时无法自动区分，请在 config.toml 的该角色下"
            "用 device = \"COMx\"（点名口）或 location = \"...\"（点名物理 USB 口）指定。"
        )
    port = hits[0]
    return PortMatch(
        role=role,
        device=port.device,
        description=port.description or "",
        hwid=port.hwid or "",
        vid=port.vid,
        pid=port.pid,
        serial_number=port.serial_number,
    )


def diagnose(cfg: Any, role: str) -> str:
    """没匹配到端口时，告诉人**为什么**，而不是只说"没找到"。

    最容易被这个坑住的是"锁了物理口"的情形：`location` 一旦写进配置，
    板子拔走或换到另一个 USB 口就再也匹配不上，而报错只有一句"未找到"，
    看起来像驱动坏了或配置写错了。
    """
    role_cfg = _role_cfg(cfg, role)
    conds = {k: role_cfg.get(k) for k in
             ("device", "location", "vid", "pid", "description_contains", "hwid_contains")
             if role_cfg.get(k)}
    out = [f"角色 {role} 的匹配条件：{conds}"]
    if not (role_cfg.get("device") or role_cfg.get("location")):
        out.append("当前没有任何串口满足这些条件 —— 板子可能没插。")
        return "\n".join(out)

    vid = role_cfg.get("vid")
    near = [p for p in all_ports() if not vid or p.vid == int(vid)]
    if not near:
        out.append("当前没有任何 VID 对得上的口 —— **板子多半没插**。")
        return "\n".join(out)
    out.append("这些口 VID/PID 对得上，但 device/location 没对上：")
    for p in near:
        loc = getattr(p, "location", None) or _location_from_hwid(p.hwid or "")
        out.append(f"  {p.device:<8} loc={loc or '-'}")
    out.append("→ 要么把它插回配置里写的那个 USB 口；要么把上面某行的 location "
               "抄进 config.toml 该角色的 location = \"...\"")
    return "\n".join(out)


def describe_all() -> list[dict[str, Any]]:
    out = []
    for p in all_ports():
        out.append(
            {
                "device": p.device,
                "description": p.description,
                "vid": f"0x{p.vid:04X}" if p.vid is not None else None,
                "pid": f"0x{p.pid:04X}" if p.pid is not None else None,
                "serial": p.serial_number,
                # 物理 USB 口路径：两块同型号板并存时靠它区分（可直接抄进 config）
                "location": getattr(p, "location", None) or _location_from_hwid(p.hwid or ""),
            }
        )
    return out
