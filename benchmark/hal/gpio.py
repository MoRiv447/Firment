"""夹具激励。

三种后端，**方向性完全不同，接错就是"什么都没发生"或"一直按住"**：

``backend = "break"``（推荐用这个做按键）
    拿 USB-TTL 的 **TXD** 当一根推挽输出：UART 空闲 = 高，置 BREAK = 持续低。
    这是"只有 VCC/GND/TX/RX"的裸 TTL 板上**唯一**能主动驱动高电平的线。
    要求：实测 TXD 的空闲高电平是 3.3V（5V 不许直接接 PC13）。

``backend = "modem"``（CH340 的 DTR/RTS）
    pyserial 的 True/False 是"断言/取消断言"，不是电平；对低有效的 DTR#/RTS#
    断言 = 引脚拉低。**推挽输出，能高能低**，所以两根方向都能做。
    但注意：如果中间隔了一级三极管（ESP-01S 烧录板的自动下载电路就是），
    三极管只会往 GND 拉 —— 那就**只能做低有效信号（如 NRST）**，
    做不了高有效的按键。

``backend = "esp32"``（夹具固件的 GPIO）
    推挽，方向随便，还能做"可控跳线"和测频。P1 阶段实现固件与协议。

**NUCLEO-G431RB 的 B1 是高有效（按下 = PC13 到 3.3V）**，不是低有效 —— 见 config.toml。
所以"只会拉低"的方案对按键无效，而且它的空闲高电平还会把按键一直按住。
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Any

from hal.serialbus import SerialBus


class FixtureUnavailable(RuntimeError):
    """夹具能力缺失。runner 会把它翻成 caps 缺失，按 spec 的 fallback 处理。"""


@dataclass
class Stimulus:
    name: str
    backend: str
    line: str | None = None
    gpio: int | None = None
    # **目标引脚是不是低有效**（名字取自设备树的 GPIO_ACTIVE_LOW）：
    #   active_low = true  → "按下" = 把引脚**拉低**。例：NRST（复位低有效）
    #   active_low = false → "按下" = 把引脚**拉高**。例：**NUCLEO-G431RB 的 B1**
    # 这个方向由**目标板**决定，跟用哪块夹具板、哪种后端无关。
    # 搞反的表现是"按下去等于松开"，而且**看起来像是在正常运行**。
    # 拿不准就跑 `cli.py fixture wiggle` 配万用表看引脚怎么跳。
    active_low: bool = True
    # **信号路径本身有没有反相一级**（最典型的就是自动下载电路里那个三极管：
    # 集电极输出，基极被 DTR# 驱动 → 整体反相一次）。
    # active_low 说的是"目标引脚要什么"，line_inverted 说的是"我的线怎么传"，
    # 两件事分开写，换接线时只改一个。真正进到寄存器的是它们的异或。
    line_inverted: bool = False
    # **再借一根线把"松开"变成主动动作**（值 = 要断言的那根线，如 "RTS"）。
    # 为什么需要：这块 ESP-01S 板上有一类脚是 **sink-only** —— 断言时灌低，
    # 取消断言后**悬空**，节点上的电荷没处泄，于是"松开"读起来还是低。
    # 继电器的 IN 就挂在这种脚上：闭合（灌低）做得到，**拔线（松开）做不成**，
    # 而 case06 的判据正好要"拔线后固件报 FAIL"。
    # 交叉耦合真值表里"RTS 断言 ⇒ 触点一定断开"是**无条件**的，所以拿它当
    # 强制断开的手：按下 = 取消断言 force_line，松开 = 断言 force_line。
    # 只在 modem 后端有意义；不填就是原来的单线驱动。
    force_line: str | None = None
    pulse_ms: int = 100
    state: bool = False
    history: list[dict[str, Any]] = field(default_factory=list)

    def as_dict(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "backend": self.backend,
            "line": self.line,
            "gpio": self.gpio,
            "active_low": self.active_low,
            "line_inverted": self.line_inverted,
            "force_line": self.force_line,
            "state": self.state,
            "history": self.history,
        }


class Fixture:
    def __init__(self, cfg: Any, bus: SerialBus | None) -> None:
        self.cfg = cfg
        self.bus = bus
        self.enabled: bool = bool(cfg.get("fixture.enabled", False))
        # **这道题真的会用到的激励名**（runner 从 spec 的 caps.gpio 填进来）。
        # 交叉耦合下两根激励可能抢同一根线（按键"未按下"与跳线"闭合"在 DTR 上
        # 正好相反）⇒ 用不到的那根就不能"顺手放回空闲"，否则会把在用的那根踩掉。
        # None = 未指定（命令行自检），此时全部释放。
        self.active_names: set[str] | None = None
        self.stimuli: dict[str, Stimulus] = {}
        for name, spec in (cfg.get("fixture.stimuli", {}) or {}).items():
            self.stimuli[name] = Stimulus(
                name=name,
                backend=spec.get("backend", "modem"),
                line=spec.get("line"),
                gpio=spec.get("gpio"),
                active_low=bool(spec.get("active_low", True)),
                line_inverted=bool(spec.get("line_inverted", False)),
                force_line=spec.get("force_line"),
                pulse_ms=int(spec.get("pulse_ms", 100)),
            )

    # -- 能力 -----------------------------------------------------------
    def available(self, name: str) -> bool:
        """**只看配置与代码支不支持，不看总线是否已打开。**

        能力探测发生在开串口之前，若把"总线已打开"也算进条件，caps 就会
        永远报"不具备" —— 那样 case04/11 的按键步骤会被静默跳过，而报告上
        看不出是配置问题还是硬件问题。
        """
        if not self.enabled:
            return False
        st = self.stimuli.get(name)
        if st is None:
            return False
        if st.backend == "esp32":
            return False  # 固件未实现前一律视为不可用，别假装能跑
        if st.backend == "break":
            return True
        if st.backend != "modem" or st.line not in ("DTR", "RTS"):
            return False
        # force_line 填错（拼错/填了个不存在的线）会让"拔线"又退回成"靠悬空"，
        # 而这正是它要被引入的原因 —— 宁可在这里判"不具备"，让 readiness 喊出来。
        # 和 line 同一根更要紧：那样两次写落在同一根线上，后者覆盖前者，
        # 等于把方向**反**了过来，而现象只是"有时不灵"。
        if st.force_line is not None and (
                st.force_line not in ("DTR", "RTS") or st.force_line == st.line):
            return False
        return True

    def missing(self, names: list[str]) -> list[str]:
        return [n for n in names if not self.available(n)]

    # -- 动作 -----------------------------------------------------------
    def _require(self, name: str) -> Stimulus:
        if name not in self.stimuli:
            raise FixtureUnavailable(f"config.toml 未定义激励：{name}")
        if not self.available(name):
            raise FixtureUnavailable(f"激励不可用：{name}（夹具未启用或后端未实现）")
        if self.bus is None:
            raise FixtureUnavailable(f"激励 {name} 的串口尚未打开（步骤里要先 serial_open）")
        return self.stimuli[name]

    def _idle_lines_except(self, keep: Stimulus | None) -> None:
        """把"除了 keep 之外"的所有线放回空闲。

        - **其它激励**：按各自的"未按下"释放（低有效的会被取消断言，
          高有效的会被断言 —— 那正是它"未按下"的样子）。
        - **没有任何激励映射到的 modem 线：也要显式取消断言。**

        第二条是踩出来的：这块 ESP-01S 板的 DTR/RTS **交叉耦合**，只要有一根留在
        断言态，另一根就拉不起来。而"没映射到激励"的线以前从来没人管 ——
        它一直是 **pyserial 打开串口时的默认状态**。2026-10-16 把 `nrst` 激励从
        配置里删掉之后，RTS 就再也没人取消断言了，于是按键悄悄失效，
        而 `verify` 之前能过、之后不能过，看起来像随机故障。
        """
        assert self.bus is not None
        mapped = {st.line.upper() for st in self.stimuli.values()
                  if st.backend == "modem" and st.line}
        for name, st in self.stimuli.items():
            if st is not keep and self.available(name):
                self._apply_one(st, False)
        self._deassert_unmapped_lines(mapped)

    def _deassert_unmapped_lines(self, mapped: set[str] | None = None) -> None:
        """把**没有任何激励映射到**的 modem 线取消断言。

        为什么必须有：这些线平时"没人管"，于是它们一直是 **pyserial 打开串口时的
        默认状态**（断言）。而这块板 DTR/RTS 交叉耦合 —— 只要有一根留在断言态，
        另一根的功能就被锁死：
          - RTS 断言 ⇒ 继电器永远合不上（case06 实测）
          - RTS 断言 ⇒ 按键也按不下去（"按下"要求两根都取消）
        这个坑 2026-10-16 从**两个不同入口**各踩了一次（先是从 `_apply` 走，
        后是从 `release_all` 的 active_names 分支走）。所以清理逻辑抽到这里，
        两条路径共用。
        """
        assert self.bus is not None
        if mapped is None:
            mapped = {st.line.upper() for st in self.stimuli.values()
                      if st.backend == "modem" and st.line}
        for line in ("DTR", "RTS"):
            if line not in mapped:
                try:
                    self.bus.set_modem_line(line, False)   # False = 取消断言
                except Exception:  # noqa: BLE001 - 收尾/清理不该再抛
                    pass

    def _apply(self, st: Stimulus, press: bool) -> None:
        """拨动一根线。**只动这一根，不碰别的线。**

        2026-10-16 实测的两张真值表（这块 ESP-01S 板的 DTR/RTS 交叉耦合）：

            按键（PC13 读到 1 = 固件认为按下）      跳线（继电器触点）
              RTS    DTR    按键                    RTS    DTR    触点
              取消   取消    **按下**                取消   取消    断开
              取消   断言    未按下                  取消   **断言** **闭合**
              断言   取消    未按下                  断言   取消    断开
              断言   断言    未按下                  断言   断言    断开

        ⇒ 按键"未按下"= **两根里任意一根被断言**（或）；
          触点闭合 = **DTR 断言且 RTS 取消** —— 这个状态同时满足"按键未按下"。

        **所以两根线本来可以共存，坏就坏在"动一根前先把别的线放回空闲"**：
        按键的"未按下"实现成"断言 RTS"，而 RTS 一断言，继电器就永远合不上。
        case06 就是这么挂的（case04 只用按键，没有第二根线来打架，所以没暴露）。

        隐含状态要管，但该由 `release_all()` 统一管（开题前、收尾时各一次），
        而不是在每次拨线时顺手把别人踩掉。
        """
        self._apply_one(st, press)

    def _apply_one(self, st: Stimulus, press: bool) -> None:
        assert self.bus is not None
        # 把"按下"翻译成物理动作：目标引脚低有效就拉低，高有效就拉高。
        pull_low = press if st.active_low else (not press)
        if st.backend == "break":
            # TXD 推挽：BREAK = 持续低；空闲 = 高。
            # 一根线直连，不存在反相的那一级。
            self.bus.set_break(pull_low)
        else:
            # modem 线：pyserial 断言(True) 对 DTR#/RTS# 就是引脚拉低；
            # 若中间过了一级三极管，整体再反一次。
            self.bus.set_modem_line(st.line or "", pull_low != st.line_inverted)
            # 后动 force_line，让它成为最终状态：按下时取消断言，松开时断言。
            if st.force_line:
                self.bus.set_modem_line(st.force_line, not press)
        st.state = press
        st.history.append({"t": time.time(), "press": press, "pull_low": pull_low,
                           "backend": st.backend, "line": st.line,
                           "line_inverted": st.line_inverted,
                           "force_line": st.force_line})

    def set(self, name: str, press: bool) -> None:
        st = self._require(name)
        self._apply(st, press)

    def pulse(self, name: str, ms: int | None = None) -> None:
        st = self._require(name)
        dur = (ms if ms is not None else st.pulse_ms) / 1000.0
        self._apply(st, True)
        time.sleep(dur)
        self._apply(st, False)

    def idle_unmapped(self) -> None:
        """**端口刚打开时调用**：把没被激励映射的 modem 线放开。

        为什么：pyserial 打开 CH340 串口时，DTR/RTS 的默认态是**断言**。交叉耦合下
        一根断言的线会锁死另一根的功能（实测：RTS 断言 ⇒ 按键"按下"永远出不来，
        `fixture verify` 就这么挂过，而同样的 fx.set 隔离测试是过的 —— 差别只在
        隔离测试先调了 release_all）。所以"隐含状态"有三个时机要清：
        **开串口时、每次拨线时（不该动别人的）、收尾时** —— 缺一个都会随机翻车。
        """
        if self.bus is None:
            return
        self._deassert_unmapped_lines()

    def release_all(self) -> None:
        """收尾时把每根线放回**未按下**的状态。

        这一步不是可选的礼貌：如果按键是高有效而某根线的空闲电平是"高"，
        不放回去就等于一直按着 —— 下一题的表现会被上一题的残留污染。
        """
        if self.bus is None:
            return
        try:
            if self.active_names is None:
                # 没指定（命令行自检）：所有激励 + 未映射的 modem 线都放回空闲
                self._idle_lines_except(None)
            else:
                # 只释放**这道题真的会用到的那几根**（见 active_names 的注释），
                # **但未映射的线必须照样清掉** —— 它们没人管，会一直停在
                # pyserial 打开时的默认断言态，把交叉耦合的另一根锁死。
                for name, st in self.stimuli.items():
                    if name in self.active_names and self.available(name):
                        self._apply_one(st, False)
                self._deassert_unmapped_lines()
        except Exception:  # noqa: BLE001 - 收尾不该再抛
            pass

    def evidence(self) -> dict[str, Any]:
        return {"enabled": self.enabled, "stimuli": {k: v.as_dict() for k, v in self.stimuli.items()}}
