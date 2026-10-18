"""串口层：按行切分 + 打时间戳 + 原始证据落盘。

设计要点（每一条都是踩过的坑）：

* **按行解码，不按读块解码。** 一个多字节 UTF-8 字符可能被内核切成两次
  read 到达；用 ``codecs`` 的增量解码器跨调用保留半个序列，否则会得到
  两个 U+FFFD，正则再也匹配不上。同 `firment-tools` 的 `utf8::LineSplitter`
  教训。
* **时间戳记在"行完成"的那一刻。** 用来量 1s 周期、500ms 心跳这类时序，
  毫秒级抖动足够；不要用它量微秒级的东西（那该交给逻辑分析仪）。
* **两个后端同一接口**：``SerialBus``（真串口）与 ``SimBus``（回放 fixture，
  虚拟时间不睡）。断言引擎只认接口，所以整条管线可以在没有板子的情况下
  端到端自测。
"""

from __future__ import annotations

import codecs
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Iterable

try:  # pyserial 只在真串口路径需要；sim 路径不该因为它没装就跑不起来
    import serial
except ImportError:  # pragma: no cover
    serial = None  # type: ignore[assignment]


@dataclass
class Line:
    index: int
    t: float  # 相对采集起点的秒数
    text: str
    partial: bool = False

    def as_dict(self) -> dict[str, Any]:
        return {"i": self.index, "t": round(self.t, 4), "text": self.text, "partial": self.partial}


class Capture:
    """把字节流按行切开并打时间戳。线程安全，多个 reader 也可以喂。"""

    def __init__(self) -> None:
        self.lines: list[Line] = []
        self.bytes_in = 0
        self._dec = codecs.getincrementaldecoder("utf-8")("replace")
        self._pending = ""
        self._t0: float | None = None
        self._lock = threading.RLock()

    # -- 内部 -----------------------------------------------------------
    def _emit(self, text: str, now: float, partial: bool = False) -> None:
        if self._t0 is None:
            self._t0 = now
        self.lines.append(
            Line(index=len(self.lines), t=max(0.0, now - self._t0), text=text, partial=partial)
        )

    # -- 对外 -----------------------------------------------------------
    def feed(self, data: bytes, now: float | None = None) -> None:
        if not data:
            return
        ts = time.monotonic() if now is None else now
        with self._lock:
            self.bytes_in += len(data)
            self._pending += self._dec.decode(data)
            while True:
                idx = self._pending.find("\n")
                if idx < 0:
                    break
                raw_line = self._pending[:idx]
                self._pending = self._pending[idx + 1 :]
                self._emit(raw_line.rstrip("\r"), ts)

    def flush(self, now: float | None = None) -> None:
        """采集结束时把最后一段没有换行的内容也交出去（标 partial）。"""
        ts = time.monotonic() if now is None else now
        with self._lock:
            tail = self._pending + self._dec.decode(b"", True)
            self._pending = ""
            if tail.strip():
                self._emit(tail.rstrip("\r"), ts, partial=True)

    def text(self) -> str:
        with self._lock:
            return "\n".join(ln.text for ln in self.lines)

    def snapshot(self) -> list[Line]:
        with self._lock:
            return list(self.lines)

    def relative_now(self, now: float) -> float:
        """把外部时钟换算到采集时间轴（与 Line.t 同一基准）。

        断言要算"从发送到收到回显有多久"，就得把发送时刻放到和行时间戳
        同一个坐标系里，否则量出来的延迟里混着采集起点的偏移。
        """
        with self._lock:
            return max(0.0, now - self._t0) if self._t0 is not None else 0.0

    def since(self, index: int) -> list[Line]:
        with self._lock:
            return [ln for ln in self.lines if ln.index >= index]

    def write_log(self, path: Path, header: str = "") -> Path:
        """原始证据落盘。带相对时间戳，便于回看时复现时序。"""
        path.parent.mkdir(parents=True, exist_ok=True)
        with self._lock:
            lines = list(self.lines)
        with path.open("w", encoding="utf-8", newline="\n") as fh:
            if header:
                fh.write(f"# {header}\n")
            fh.write(f"# bytes_in={self.bytes_in} lines={len(lines)}\n")
            for ln in lines:
                mark = "~" if ln.partial else " "
                fh.write(f"[+{ln.t:9.4f}] {mark}{ln.text}\n")
        return path


# ---------------------------------------------------------------------------
# 真串口
# ---------------------------------------------------------------------------


class SerialBus:
    """一条真串口。reader 线程持续读，全部进 Capture。"""

    def __init__(
        self,
        port: str,
        baud: int = 115200,
        *,
        label: str = "",
        read_timeout_s: float = 0.05,
        open_retries: int = 5,
        retry_delay_s: float = 0.4,
    ) -> None:
        if serial is None:
            raise RuntimeError("没有安装 pyserial；真串口路径不可用")
        self.port = port
        self.baud = baud
        self.label = label or port
        self.capture = Capture()
        self._read_timeout = read_timeout_s
        self._open_retries = open_retries
        self._retry_delay = retry_delay_s
        self._ser: Any = None
        self._thread: threading.Thread | None = None
        self._stop = threading.Event()
        self.opened_at: float | None = None
        self.open_error: str | None = None

    # -- 生命周期 -------------------------------------------------------
    def open(self) -> None:
        last_err: Exception | None = None
        for attempt in range(1, self._open_retries + 1):
            try:
                self._ser = serial.Serial(
                    self.port,
                    self.baud,
                    timeout=self._read_timeout,
                    write_timeout=2.0,
                )
                self.opened_at = time.monotonic()
                break
            except Exception as exc:  # noqa: BLE001 - 端口被占用要重试，不是错误类型问题
                last_err = exc
                # agent 自己也占着 VCP；裁判接管时它可能还没松手
                if attempt < self._open_retries:
                    time.sleep(self._retry_delay)
        else:
            self.open_error = f"{type(last_err).__name__}: {last_err}"
            raise RuntimeError(f"打开 {self.port} 失败（{self._open_retries} 次）：{self.open_error}")

        self._stop.clear()
        self._thread = threading.Thread(target=self._pump, name=f"rx-{self.label}", daemon=True)
        self._thread.start()

    def _pump(self) -> None:
        assert self._ser is not None
        while not self._stop.is_set():
            try:
                n = self._ser.in_waiting or 1
                data = self._ser.read(n)
            except Exception:  # noqa: BLE001 - 拔线/复位会让读抛错，收尾即可
                break
            if data:
                self.capture.feed(data)

    def close(self) -> None:
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=1.5)
            self._thread = None
        if self._ser is not None:
            try:
                self._ser.close()
            except Exception:  # noqa: BLE001
                pass
            self._ser = None
        self.capture.flush()

    def __enter__(self) -> "SerialBus":
        self.open()
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()

    # -- 收发 -----------------------------------------------------------
    def write(self, data: bytes | str) -> None:
        if self._ser is None:
            raise RuntimeError("串口未打开")
        payload = data.encode("utf-8") if isinstance(data, str) else data
        self._ser.write(payload)
        self._ser.flush()

    def send_line(self, text: str, eol: str = "\r\n") -> None:
        self.write(text + eol)

    def wait(self, seconds: float) -> None:
        time.sleep(seconds)

    def now(self) -> float:
        return time.monotonic()

    def flush_evidence(self) -> None:
        """收尾：把最后一段没有换行的内容也交出去。"""
        self.capture.flush()

    # -- 夹具 modem 线（只有 CH340 夹具口才有意义）-----------------------
    def set_modem_line(self, name: str, value: bool) -> None:
        if self._ser is None:
            raise RuntimeError("串口未打开")
        if name.upper() == "DTR":
            self._ser.dtr = value
        elif name.upper() == "RTS":
            self._ser.rts = value
        else:
            raise ValueError(f"未知 modem 线：{name}")

    def set_break(self, hold_low: bool) -> None:
        """把 TXD 拉成**持续低电平**（BREAK），或恢复空闲（高）。

        这是"用 TXD 当一根推挽输出"的办法：UART 空闲时 TX 是**推挽高**，
        置 BREAK 则持续拉低。对那些只有 VCC/GND/TX/RX 的裸 USB-TTL 来说，
        这是唯一一根**能主动驱动高电平**的线 —— 而 NUCLEO-G431RB 的 B1 恰好
        需要高电平才算按下（见 config.toml 的说明）。

        ⚠️ **有些 CH340 驱动不实现 BREAK**（软件不报错，引脚就是不动）。
        怀疑时用 `cli.py fixture wiggle --line TXLOW` 复核：那个模式不依赖 BREAK。
        """
        if self._ser is None:
            raise RuntimeError("串口未打开")
        self._ser.break_condition = bool(hold_low)

    def send_bytes(self, data: bytes) -> None:
        """直接写裸字节（`TXLOW` 模式靠连发 0x00 把 TX 压到低位）。"""
        if self._ser is None:
            raise RuntimeError("串口未打开")
        self._ser.write(data)


# ---------------------------------------------------------------------------
# 回放后端（无硬件自测）
# ---------------------------------------------------------------------------


@dataclass
class SimScript:
    """一个可回放的脚本化设备。

    ``rx``：设备会吐出来的行，``(offset_ms, text)``，按时间排序。
    ``reply``：收到某条指令后追加的行，``{指令: [(delay_ms, text), ...]}``。
      文本里可以写 ``{sent_bytes}`` / ``{sent_bytes_excluding_last}``，
      用来伪造"把收到的字节数报回来"这类需要实时计算的外设（case07 的 STATS）。
    ``echo``：设备原样回显收到的内容（case07 的回显设备）。落在 ``reply``
      键上的指令不参与回显 —— 否则 STATS 会被回显成 "STATS" 而不是 "RX n"。
    ``truncate_chars``：只对回显内容截尾，用来伪造"接收计数正常、但回显
      路径丢字节"的故障（这样 F1 挂而 F2 仍能对上，正好检验故障隔离）。
    """

    rx: list[tuple[float, str]] = field(default_factory=list)
    reply: dict[str, list[tuple[float, str]]] = field(default_factory=dict)
    echo: bool = False
    echo_delay_ms: float = 20.0
    truncate_chars: int = 0


class SimBus:
    """与 ``SerialBus`` 同接口的虚拟设备，虚拟时间不睡觉。

    存在的理由：整条管线（spec → runner → asserts → report）必须在接板之前
    就能端到端验证；断言引擎的回归测试也不该依赖硬件在场。
    """

    def __init__(self, script: SimScript, *, label: str = "sim", baud: int = 115200) -> None:
        self.script = script
        self.port = "sim"
        self.baud = baud
        self.label = label
        self.capture = Capture()
        self.sent: list[tuple[float, str]] = []
        self._vnow = 0.0  # 虚拟时钟（秒）
        self._cursor = 0
        self._pending_reply: list[tuple[float, str]] = []
        self._opened = False

    # -- 生命周期 -------------------------------------------------------
    def open(self) -> None:
        self._opened = True
        self._drain_until(0.0)

    def close(self) -> None:
        self._drain_until(self._vnow)
        self.capture.flush(self._vnow)

    def __enter__(self) -> "SimBus":
        self.open()
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()

    # -- 内部 -----------------------------------------------------------
    def _drain_until(self, t_end: float) -> None:
        for when, text in self._pending_reply:
            if when <= t_end:
                self.capture.feed((text + "\r\n").encode("utf-8"), when)
        self._pending_reply = [(w, t) for w, t in self._pending_reply if w > t_end]
        while self._cursor < len(self.script.rx):
            off_s, text = self.script.rx[self._cursor]
            if off_s > t_end:
                break
            self.capture.feed((text + "\r\n").encode("utf-8"), off_s)
            self._cursor += 1

    # -- 收发 -----------------------------------------------------------
    def write(self, data: bytes | str) -> None:
        text = data.decode("utf-8") if isinstance(data, bytes) else data
        self.sent.append((self._vnow, text))
        key = text.strip()
        if key in self.script.reply:
            for delay_ms, resp in self.script.reply[key]:
                self._pending_reply.append((self._vnow + delay_ms / 1000.0, self._fill(resp)))
        elif self.script.echo:
            # 回显：去掉行尾后原样吐回去（内部换行保留，正是回显设备的行为）
            body = text.rstrip("\r\n")
            if self.script.truncate_chars:
                body = body[: max(0, len(body) - self.script.truncate_chars)]
            if body:
                self._pending_reply.append(
                    (self._vnow + self.script.echo_delay_ms / 1000.0, body)
                )

    def _fill(self, template: str) -> str:
        """把 {sent_bytes} 这类占位符换成实时值。"""
        if "{" not in template:
            return template
        total = sum(len(t.encode("utf-8")) for _, t in self.sent)
        last = len(self.sent[-1][1].encode("utf-8")) if self.sent else 0
        return template.format(
            sent_bytes=total,
            sent_bytes_excluding_last=total - last,
            sent_chars=sum(len(t) for _, t in self.sent),
            n_sends=len(self.sent),
        )

    def send_line(self, text: str, eol: str = "\r\n") -> None:
        self.write(text + eol)

    def wait(self, seconds: float) -> None:
        self._vnow += seconds
        self._drain_until(self._vnow)

    def now(self) -> float:
        return self._vnow

    def flush_evidence(self) -> None:
        # 用虚拟时钟，别用真实时钟 —— 否则回放里最后一段不完整行会拿到一个
        # 大得离谱的时间戳，时序断言就会被它带跑
        self.capture.flush(self._vnow)

    def set_modem_line(self, name: str, value: bool) -> None:
        self.sent.append((self._vnow, f"<{name}={'1' if value else '0'}>"))

    def set_break(self, hold_low: bool) -> None:
        self.sent.append((self._vnow, f"<BREAK={'1' if hold_low else '0'}>"))

    def send_bytes(self, data: bytes) -> None:
        self.sent.append((self._vnow, data.decode("latin-1")))
