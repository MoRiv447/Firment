"""烧录与复位。

三条路，按顺序退让：

1. ``probe-rs`` —— agent 自己也在用，能通就用它（最快，行为一致）
2. ``OpenOCD``（PlatformIO 自带包）
3. ``STM32_Programmer_CLI`` —— 最后手段，兼砖恢复（mass erase）

**复位语义的坑（别忘）**：``probe-rs reset`` 走 SYSRESETREQ，是"系统复位"；
板载 RESET 按键走 NRST 引脚。case11 要打印 ``RESET=POR``，那必须真拉 NRST
引脚（由夹具做），不能拿系统复位冒充 —— `RCC_CSR` 里留下的标志位不同。
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

from hal import proc


@dataclass
class FlashResult:
    ok: bool
    method: str
    detail: dict[str, Any]

    def as_dict(self) -> dict[str, Any]:
        return {"ok": self.ok, "method": self.method, **self.detail}


class ProbeError(RuntimeError):
    pass


# RCC_CSR：复位来源标志位。
#
# 地址不是背下来的，是从**两份独立的 CMSIS 头文件**（测试集里的
# start/cubemx/Drivers/... 与 ~/.platformio/packages/framework-stm32cubeg4/）
# 各自把 RCC_TypeDef 的字段偏移加出来的，两边都得到 0x94。
# 换芯片/换固件包之后要重新核一遍（本模块的 selftest 会做这件事）。
#
# **STM32G4 没有 PORRSTF。** 判据里说的 "RESET=POR" 是固件的分类桶，
# 不是寄存器位名 —— G4 上电/掉电复位对应的是 LPWRRSTF(bit31)。
# 谁要是按 F1 的习惯去找 PORRSTF 会找不到，然后"顺手"改错代码。
RCC_BASE = 0x40021000
RCC_CSR_OFFSET = 0x94
RCC_CSR = RCC_BASE + RCC_CSR_OFFSET
RCC_CSR_BITS = {
    "LPWRRSTF": 31,
    "WWDGRSTF": 30,
    "IWDGRSTF": 29,
    "SFTRSTF": 28,
    "BORRSTF": 27,
    "PINRSTF": 26,
    "OBLRSTF": 25,
    "RMVF": 23,
}
# 期望：哪种复位方式应当置起哪个位
RESET_KIND_EXPECTS = {
    "power": "LPWRRSTF",      # 真上电/掉电（LPWRRSTF 是 G4 的"POR"）
    "nrst": "PINRSTF",        # 拉 NRST 引脚（板载 RESET 键走这条）
    "swd": "SFTRSTF",         # probe-rs / SYSRESETREQ 的系统复位
    "iwdg": "IWDGRSTF",
}


def decode_csr(value: int) -> dict[str, bool]:
    return {name: bool(value >> bit & 1) for name, bit in RCC_CSR_BITS.items()}


def describe_csr(value: int) -> str:
    d = decode_csr(value)
    on = [k for k, v in d.items() if v and k != "RMVF"]
    return f"0x{value:08X} → " + (", ".join(on) if on else "（无标志位）")


class Probe:
    def __init__(self, cfg: Any) -> None:
        self.cfg = cfg
        self.chip: str = cfg.get("probe.chip", "STM32G431RBTx")
        self.timeout_s: float = float(cfg.get("probe.flash_timeout_s", 180))
        self.probe_rs: str | None = proc.which(cfg.get("toolchain.probe_rs", "probe-rs"))
        # 同时插着 ST-Link 和 DAPLink 时必须点名用哪个，否则 probe-rs 可能挑错
        self.probe_selector: str | None = cfg.get("probe.probe_selector") or None

    def _argv(self, *parts: str) -> list[str]:
        """拼 probe-rs 命令行。

        **``--probe`` 是子命令的选项，不是全局选项** —— 放错了会直接报
        "unexpected argument '--probe' found"，而且这个错误看起来像是版本不兼容，
        很容易被误判成"probe-rs 不支持选探针"。所以统一走这个函数拼，别在别处手写。
        """
        argv = [str(self.probe_rs), *parts]
        if self.probe_selector:
            # 插在子命令之后、位置参数之前：probe-rs read [OPTIONS] <WIDTH> <ADDRESS> <WORDS>
            argv = [str(self.probe_rs), parts[0], "--probe", str(self.probe_selector), *parts[1:]]
        return argv

    # ---- 寄存器 -------------------------------------------------------
    def read_u32(self, addr: int, words: int = 1) -> dict[str, Any]:
        """直接读目标内存/寄存器。

        **注意副作用**：SWD 读会先 attach，可能短暂停核。若被测固件开着 IWDG，
        停核超过看门狗超时就可能把它喂死（IWDG 在调试停核时默认继续计数）。
        所以这个能力只用于**标定**，不要塞进常规断言里。
        """
        if not self.probe_rs:
            return {"ok": False, "error": "probe-rs 不可用"}
        if not self.has_target():
            return {"ok": False, "error": "没有可用的调试探针"}
        res = proc.run(
            self._argv("read", "b32", hex(addr), str(words), "--chip", self.chip),
            timeout_s=30,
        )
        if not res.ok:
            return {"ok": False, **res.as_dict()}
        vals: list[int] = []
        for tok in res.stdout.split():
            try:
                vals.append(int(tok, 16))
            except ValueError:
                continue
        return {"ok": bool(vals), "values": vals, "addr": hex(addr), **res.as_dict()}

    def write_u32(self, addr: int, value: int) -> dict[str, Any]:
        if not self.probe_rs:
            return {"ok": False, "error": "probe-rs 不可用"}
        res = proc.run(
            self._argv("write", "b32", hex(addr), hex(value), "--chip", self.chip),
            timeout_s=30,
        )
        return {"ok": res.ok, "addr": hex(addr), "value": hex(value), **res.as_dict()}

    def reset_flags(self) -> dict[str, Any]:
        """读 RCC_CSR 并解码。

        顺手做一个**地址自检**：RMVF 为 0 且所有 RSTF 全为 0，说明这个地址
        要么读错了、要么标志刚被清过 —— 前者必须报出来，否则标定表会是一张
        合法的废话。
        """
        out = self.read_u32(RCC_CSR)
        if not out.get("ok"):
            return out
        value = out["values"][0]
        flags = decode_csr(value)
        plausible = flags["RMVF"] or any(v for k, v in flags.items() if k != "RMVF")
        return {**out, "raw": value, "flags": flags, "plausible": plausible,
                "summary": describe_csr(value)}

    def clear_reset_flags(self) -> dict[str, Any]:
        """写 RMVF=1 清掉复位标志（写 1 清零）。"""
        return self.write_u32(RCC_CSR, 1 << RCC_CSR_BITS["RMVF"])

    # ---- 探测 ---------------------------------------------------------
    def present(self) -> bool:
        return self.probe_rs is not None

    def list_probes(self) -> proc.ProcResult:
        if not self.probe_rs:
            return proc.ProcResult([], None, "", "probe-rs 不在 PATH 上", 0.0)
        return proc.run([self.probe_rs, "list"], timeout_s=30)

    def has_target(self) -> bool:
        """``probe-rs list`` 有一行真探针才算有目标；"No debug probes" 不算。"""
        res = self.list_probes()
        if not res.ok:
            return False
        text = res.stdout.strip()
        return bool(text) and "No debug probes" not in text

    # ---- 烧录 ---------------------------------------------------------
    def flash(self, elf: Path, *, reset_after: bool = True) -> FlashResult:
        elf = Path(elf)
        if not elf.exists():
            return FlashResult(False, "none", {"error": f"固件不存在：{elf}"})

        if self.probe_rs:
            res = proc.run(
                self._argv("download", "--chip", self.chip, str(elf)),
                timeout_s=self.timeout_s,
            )
            if res.ok:
                detail: dict[str, Any] = {"elf": str(elf), "download": res.as_dict()}
                if reset_after:
                    rst = self.reset("swd")
                    detail["reset"] = rst
                return FlashResult(True, "probe-rs", detail)
            last = res
        else:
            last = proc.ProcResult([], None, "", "probe-rs 不可用", 0.0)

        oc = self._openocd_argv(elf)
        if oc:
            res = proc.run(oc, timeout_s=self.timeout_s)
            if res.ok:
                return FlashResult(True, "openocd", {"elf": str(elf), "run": res.as_dict()})

        programmer = self.cfg.get("toolchain.stm32_programmer_cli")
        if programmer and Path(programmer).exists():
            res = proc.run(
                [programmer, "-c", "port=SWD", "mode=UR", "-w", str(elf), "-rst"],
                timeout_s=self.timeout_s,
            )
            if res.ok:
                return FlashResult(True, "stm32_programmer_cli", {"elf": str(elf), "run": res.as_dict()})
            last = res

        return FlashResult(False, "none", {"elf": str(elf), "last_error": last.as_dict()})

    def _openocd_common(self, interface: str | None = None) -> list[str] | None:
        """OpenOCD 的公共参数。``interface`` 决定用哪个探针：stlink 或 cmsis-dap。

        DAPLink 走 `interface/cmsis-dap.cfg` —— 它的 RST 脚同样能由 OpenOCD 驱动，
        所以它也是"真 NRST"的一条候选路径，不是只能当备用烧录器。
        """
        pkg = self.cfg.get("toolchain.openocd_dir")
        if not pkg:
            return None
        root = Path(pkg)
        exe = root / "bin" / "openocd.exe"
        if not exe.exists():
            # PlatformIO 的包目录有时带版本后缀
            cands = sorted(root.parent.glob("tool-openocd*/bin/openocd.exe"))
            if not cands:
                return None
            exe = cands[-1]
        iface = interface or str(self.cfg.get("probe.openocd_interface", "stlink"))
        cfg_name = "cmsis-dap.cfg" if iface in ("cmsis-dap", "daplink", "dap") else "stlink.cfg"
        return [
            str(exe),
            "-s", str(exe.parent.parent / "openocd" / "scripts"),
            "-f", f"interface/{cfg_name}",
            "-f", "target/stm32g4x.cfg",
        ]

    def _openocd_argv(self, elf: Path) -> list[str] | None:
        base = self._openocd_common()
        if base is None:
            return None
        return base + ["-c", f"program {elf} verify reset exit"]

    # ---- 复位 ---------------------------------------------------------
    def reset(self, kind: str = "swd") -> dict[str, Any]:
        if kind == "swd":
            if not self.probe_rs:
                return {"ok": False, "kind": kind, "error": "probe-rs 不可用"}
            res = proc.run(self._argv("reset", "--chip", self.chip), timeout_s=45)
            return {"ok": res.ok, "kind": kind, "method": "probe-rs", **res.as_dict()}
        if kind == "nrst":
            return self.reset_nrst()
        return {"ok": False, "kind": kind, "error": f"不支持的复位类型：{kind}"}

    def reset_nrst(self) -> dict[str, Any]:
        """真 NRST 复位。

        **哪条路真的产生引脚复位，必须先标定**（`cli.py probe reset-test`）：
        拉 NRST 应当置 PINRSTF，而 SWD 系统复位置的是 SFTRSTF —— 两者在
        `RCC_CSR` 里留下的痕迹不同。没标定过就当"不具备这个能力"
        （`[probe].nrst_method = "none"`），让 case11 的相关步骤跳过，
        而不是拿一次系统复位冒充按键复位骗过一个绿灯。
        """
        method = str(self.cfg.get("probe.nrst_method", "none"))
        if method == "programmer":
            return self.reset_programmer_nrst()
        if method == "openocd":
            return self.reset_openocd_srst()
        return {
            "ok": False,
            "kind": "nrst",
            "method": method,
            "error": "尚未标定真 NRST 复位路径（[probe].nrst_method = \"none\"）。"
                     "插上板子跑 `cli.py probe reset-test`，把它告诉你那条填进 config.toml",
        }

    def reset_programmer_nrst(self) -> dict[str, Any]:
        """STM32_Programmer_CLI 的 `-rst`：它走的是 SWD 连接下的硬件复位。"""
        programmer = self.cfg.get("toolchain.stm32_programmer_cli")
        if not (programmer and Path(programmer).exists()):
            return {"ok": False, "kind": "nrst", "method": "programmer",
                    "error": "STM32_Programmer_CLI 不存在"}
        res = proc.run([programmer, "-c", "port=SWD", "mode=UR", "-rst"], timeout_s=60)
        return {"ok": res.ok, "kind": "nrst", "method": "programmer", **res.as_dict()}

    def reset_openocd_srst(self, interface: str | None = None) -> dict[str, Any]:
        """OpenOCD 的 `reset` + `srst_only`：显式走 SRST 引脚（真 NRST 的候选路径之一）。"""
        base = self._openocd_common(interface)
        if base is None:
            return {"ok": False, "kind": "nrst", "method": "openocd",
                    "error": "找不到 OpenOCD（toolchain.openocd_dir）"}
        argv = base + [
            "-c", "reset_config srst_only srst_push_pull",
            "-c", "init",
            "-c", "reset",
            "-c", "shutdown",
        ]
        res = proc.run(argv, timeout_s=60)
        return {"ok": res.ok, "kind": "nrst", "method": "openocd",
                "interface": interface or self.cfg.get("probe.openocd_interface", "stlink"),
                **res.as_dict()}

    def reset_with_programmer(self) -> dict[str, Any]:
        return self.reset_programmer_nrst()

    # ---- 砖恢复 -------------------------------------------------------
    def mass_erase_recover(self) -> dict[str, Any]:
        """整片擦除 + 回写母本。写完本题记 0（对应 README 的"烧砖"流程）。"""
        programmer = self.cfg.get("toolchain.stm32_programmer_cli")
        if not (programmer and Path(programmer).exists()):
            return {"ok": False, "error": "STM32_Programmer_CLI 不存在"}
        elf = self.cfg.get("toolchain.recover_elf")
        argv = [programmer, "-c", "port=SWD", "mode=UR", "-e", "all"]
        if elf and Path(elf).exists():
            argv += ["-w", elf, "-rst"]
        res = proc.run(argv, timeout_s=self.timeout_s)
        return {"ok": res.ok, "recover_elf": elf, **res.as_dict()}
