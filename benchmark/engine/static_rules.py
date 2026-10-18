"""静态代码规则包：隐藏分里能确定性判定的那一半。

四条自我约束：

1. **宁可不判，不许错判。** 判定不出来就返回 ``None``（inconclusive），
   由报告转给 AI；把"看不懂"说成"没做到"会冤枉人。
2. **判红必须指着行说话。** 一条 False 结论如果拿不出任何证据行，`run_rules`
   的闸门会把它降成未观测 —— "扫遍了但没有"要写成证据（`Workspace.absent`），
   不能只是返回 False。
3. **只搜应用代码，但别少搜。** 语料是整个工程减去 ``EXCLUDE_PARTS``
   （``Drivers/`` HAL/CMSIS、``.pio/``、``lib/`` 等）—— 否则库里的
   ``HAL_TIM_PWM_Start`` 会让 case03 全绿；而只按目录名白名单又会漏掉
   ``cubemx/Core/``，让两个 agent 的考卷厚度不一样。
4. **规则 id 与评分点绑定。** 一条规则只回答一个问题，理由写在源码里。
"""

from __future__ import annotations

import hashlib
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Iterable

from engine.rev import REV

# 排除名单：库与构建产物一律不进语料
EXCLUDE_PARTS = {".pio", "Drivers", ".git", "lib", "test", "build", "Debug", ".metadata"}
SRC_EXTS = {".c", ".h", ".cpp", ".hpp"}


def _strip_comments(text: str) -> list[str]:
    """去掉 `//` 与 `/* */`，保留行号；字符串字面量里的 `//` 不算注释。

    为什么要这一步：规则问的是"**代码**里有没有做这件事"。注释里写
    `/* TODO: also set RMVF */` 会让 `case11.H1` 判通过（实测），
    那是把"它知道该怎么做"当成了"它做了"。
    """
    out: list[str] = []
    in_block = False
    for line in text.splitlines():
        buf: list[str] = []
        i = 0
        n = len(line)
        while i < n:
            if in_block:
                j = line.find("*/", i)
                if j < 0:
                    i = n
                    break
                in_block = False
                i = j + 2
                continue
            two = line[i:i + 2]
            if two == "//":
                break
            if two == "/*":
                in_block = True
                i += 2
                continue
            ch = line[i]
            if ch in ('"', "'"):
                q = ch
                j = i + 1
                while j < n:
                    if line[j] == "\\":
                        j += 2
                        continue
                    if line[j] == q:
                        j += 1
                        break
                    j += 1
                buf.append(line[i:j])
                i = j
                continue
            buf.append(ch)
            i += 1
        out.append("".join(buf))
    return out


_ALLOWED_BINOPS = {
    "Add": lambda a, b: a + b,
    "Sub": lambda a, b: a - b,
    "Mult": lambda a, b: a * b,
    "Div": lambda a, b: a // b if b else None,
    "FloorDiv": lambda a, b: a // b if b else None,
    "Mod": lambda a, b: a % b if b else None,
    "LShift": lambda a, b: a << b,
    "RShift": lambda a, b: a >> b,
    "BitOr": lambda a, b: a | b,
    "BitAnd": lambda a, b: a & b,
}
_IDENT = re.compile(r"(?<![\w.])[A-Za-z_]\w*")
_INT_SUFFIX = re.compile(r"\b(0[xX][0-9A-Fa-f]+|\d+)[uUlL]+\b")


def _eval_int(expr: str, macros: dict[str, str], depth: int = 0) -> int | None:
    """整数表达式的白名单求值。未知符号 / 除法为零 / 非整数 ⇒ None。

    不做 `eval`：spec 与工作区内容都不该有机会执行任意表达式。
    """
    import ast

    if depth > 8:
        return None
    raw = expr.strip().rstrip(";,").rstrip("uUlL").strip()
    # 去掉 C 的强制类型转换：`(uint32_t)PWM_ARR_VALUE`、`(unsigned)` 这类前缀
    raw = re.sub(r"\(\s*(?:u?int(?:8|16|32|64)_t|unsigned(?:\s+\w+)?|signed(?:\s+\w+)?|volatile|const)\s*\)\s*",
                 "", raw)
    if not raw:
        return None

    def resolve(name: str) -> int | None:
        if name in macros:
            return _eval_int(macros[name], macros, depth + 1)
        return None

    def sub_names(s: str) -> str | None:
        def rep(m: re.Match[str]) -> str:
            name = m.group(0)
            v = resolve(name)
            if v is None:
                raise KeyError(name)
            return f"({v})"
        try:
            return _IDENT.sub(rep, s)
        except KeyError:
            return None

    text = sub_names(raw)
    if text is None:
        return None
    # C 的整型后缀：`1U` / `0x3CUL` 在 Python 里都是语法错误，得先去掉。
    # 注意 `0x3C` 里的 `x3C` 会被 `_IDENT` 当成标识符 —— 上面的 lookbehind 挡的就是它。
    text = _INT_SUFFIX.sub(r"\1", text)
    try:
        node = ast.parse(text, mode="eval").body
    except SyntaxError:
        return None

    def ev(nd: Any) -> int | None:
        if isinstance(nd, ast.Constant) and isinstance(nd.value, bool):
            return None
        if isinstance(nd, ast.Constant) and isinstance(nd.value, int):
            return int(nd.value)
        if isinstance(nd, ast.Constant) and isinstance(nd.value, float):
            # `1.5` 不是寄存器值：宁可算不出来，也不许悄悄截断
            return int(nd.value) if float(nd.value).is_integer() else None
        if isinstance(nd, ast.UnaryOp) and isinstance(nd.op, (ast.USub, ast.UAdd)):
            v = ev(nd.operand)
            if v is None:
                return None
            return -v if isinstance(nd.op, ast.USub) else v
        if isinstance(nd, ast.BinOp):
            fn = _ALLOWED_BINOPS.get(type(nd.op).__name__)
            if fn is None:
                return None
            a, b = ev(nd.left), ev(nd.right)
            if a is None or b is None:
                return None
            return fn(a, b)
        return None

    val = ev(node)
    return None if val is None else int(val)



@dataclass
class Hit:
    path: str
    line: int
    text: str

    def ref(self) -> str:
        return f"{self.path}:{self.line}"


@dataclass
class RuleResult:
    rule: str
    point: str
    ok: bool | None
    want: str
    got: dict[str, Any] = field(default_factory=dict)
    evidence: list[str] = field(default_factory=list)
    note: str = ""
    confidence: str = "high"

    def as_dict(self) -> dict[str, Any]:
        return {
            "rule": self.rule,
            "point": self.point,
            "ok": self.ok,
            "want": self.want,
            "got": self.got,
            "evidence": self.evidence[:4],
            "note": self.note,
            "confidence": self.confidence,
        }


class Workspace:
    """被测工程的可搜索视图。"""

    def __init__(self, root: Path, dirs: Iterable[str] | None = None) -> None:
        self.root = Path(root)
        # 语料 = 整个工程 **减去** EXCLUDE_PARTS。
        # 原来写死只扫根目录下的 Core/ src/ include/ 三个目录，但两个 agent 的转换布局不一样：
        #   firm    ：<根>/src/  +  <根>/cubemx/Core/Src/   ← CubeMX 那半不在名单里
        #   claude  ：<根>/src/  +  <根>/Core/Src/          ← 在名单里
        # 实测同一道考题，firm 侧有 6 个工程**只扫到 1 个源文件**，claude/case04 扫到 14 个 ——
        # 两个 agent 被看的代码量差一个数量级，这已经不是"判得准不准"而是"判的份数不一样"。
        # HAL/CMSIS 由 `Drivers`/`.pio` 排除项挡住，不会因此让 case03 的 PWM 规则全绿。
        self.dirs = list(dirs) if dirs is not None else []
        self.files: list[Path] = []
        for base in ([self.root / d for d in self.dirs] or [self.root]):
            if not base.exists():
                continue
            for p in base.rglob("*"):
                if p.suffix.lower() not in SRC_EXTS or not p.is_file():
                    continue
                if EXCLUDE_PARTS & set(p.relative_to(self.root).parts):
                    continue
                self.files.append(p)
        self._cache: dict[Path, str] = {}
        self._lines: list[tuple[Path, int, str]] = []
        # 去掉注释后的同一批行，行号与 `_lines` 严格对齐。存在的理由：
        # `grep` 会把注释里的名字一起命中，"在 TODO 里写了 RMVF"于是被当成"清了 RMVF"。
        # 判"有没有做过某件事"只能看代码，不能看它会不会说。
        self._code_lines: list[tuple[Path, int, str]] = []
        self.baseline = self._baseline_digests()
        self.origin: dict[str, str] = {}      # 相对路径 → "母本" | "agent"
        self._macros: dict[str, str] = {}
        self._load()

    @staticmethod
    def _digest(p: Path) -> str:
        try:
            return hashlib.sha256(p.read_bytes()).hexdigest()
        except OSError:
            return ""

    def _baseline_digests(self) -> set[str] | None:
        """找到 CubeMX 母本（`<测试集>/start/cubemx`），把它的文件指纹收起来。

        判分器最容易犯的不是漏判，是**把母本里本来就有的代码当成 agent 的功劳**：
        CubeMX 生成的 `stm32g4xx_it.c` 里天然有 `HAL_NVIC_SetPriority`，规则只问
        "有没有这一行"就会给一个从没考虑过优先级的实现打勾（case13.H2 原样就是这样）。
        有母本可比，才问得出"这一行是不是他写的"。
        """
        for up in (self.root, *self.root.parents):
            cand = up / "start" / "cubemx"
            if cand.is_dir():
                return {self._digest(p) for p in cand.rglob("*")
                        if p.suffix.lower() in SRC_EXTS and p.is_file()}
        return None

    def _load(self) -> None:
        for p in sorted(self.files):
            try:
                # 固件源码里有 GBK 注释的可能性不为零；不让编码异常吃掉整个规则
                text = p.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                text = p.read_text(encoding="gbk", errors="replace")
            except OSError:
                continue
            self._cache[p] = text
            rel = str(p.relative_to(self.root)).replace("\\", "/")
            if self.baseline is None:
                self.origin[rel] = "未知"
            else:
                self.origin[rel] = "母本" if self._digest(p) in self.baseline else "agent"
            code = _strip_comments(text)
            for i, line in enumerate(text.splitlines(), start=1):
                self._lines.append((p, i, line))
            for i, line in enumerate(code, start=1):
                self._code_lines.append((p, i, line))
            self._collect_macros(code)

    def _collect_macros(self, lines: list[str]) -> None:
        """收集对象式 `#define`（`NAME <expr>`），给"符号写法的数值配置"用。

        case05 的两个 agent 都把 `PWM_ARR` 写成 `(PWM_PERIOD - 1)`，规则只吃纯数字，
        于是这一分永远交给 AI —— 不是判错，是**同一道题在两侧都要人来看**。

        它是实例方法而不是 `@staticmethod`：`self._macros` 就是它要写的表。
        写成 staticmethod 时 `self` 未定义，语料里只要有一个 `#define 名字 值`
        就当场 NameError —— 整个静态步全灭（`_load` 每读一个文件都调它）。
        """
        for line in lines:
            m = re.match(r"^\s*#\s*define\s+([A-Za-z_]\w*)\s+(.+?)\s*$", line)
            if m and "(" not in m.group(1):
                self._macros[m.group(1)] = m.group(2)

    # -- 查询 -----------------------------------------------------------
    @property
    def empty(self) -> bool:
        return not self._cache

    def grep(self, pattern: str, flags: int = re.IGNORECASE) -> list[Hit]:
        rx = re.compile(pattern, flags)
        out = []
        for p, i, line in self._lines:
            if rx.search(line):
                out.append(Hit(str(p.relative_to(self.root)).replace("\\", "/"), i, line.strip()))
        return out

    def grep_code(self, pattern: str, flags: int = re.IGNORECASE) -> list[Hit]:
        """只搜代码（注释已剔除）。判"做过某事"一律走这里。"""
        rx = re.compile(pattern, flags)
        out = []
        for p, i, line in self._code_lines:
            if rx.search(line):
                out.append(Hit(str(p.relative_to(self.root)).replace("\\", "/"), i, line.strip()))
        return out

    def has(self, pattern: str, flags: int = re.IGNORECASE) -> bool:
        return bool(self.grep(pattern, flags))

    def eval_int(self, expr: str, depth: int = 0) -> int | None:
        """把 `(PWM_PERIOD - 1)` 这类**只含数字与已定义宏**的表达式算成整数。

        算不出来就返回 None（调用方据此记未观测）—— 这个函数只负责"能算就算"，
        不许因为算不出来就假设一个值。只接受整数四则与移位，没有函数调用、
        没有下标、没有字符串；解析用 `ast` 白名单而不是 `eval`。
        """
        return _eval_int(expr, self._macros, depth)

    def code_spans(self, open_rx: str, close: str = "}") -> set[tuple[str, int]]:
        """返回"落在一段初始化块里"的行号集合。

        用途：SSD1306 的命令表通常写成
        `static const uint8_t init_seq[] = {` 换行，然后每行两三个字节。
        这些行本身没有 `cmd`/`init` 之类的词，只有在块的**开头**才有 ——
        按行找上下文的规则会把整张命令表判成"没写初始化"。
        """
        rx = re.compile(open_rx, re.IGNORECASE)
        out: set[tuple[str, int]] = set()
        for p, i, line in self._code_lines:
            if not rx.search(line):
                continue
            rel = str(p.relative_to(self.root)).replace("\\", "/")
            depth = line.count("{") - line.count("}")
            for p2, i2, line2 in self._code_lines:
                if p2 != p or i2 <= i:
                    continue
                out.add((rel, i2))
                depth += line2.count("{") - line2.count("}")
                if depth <= 0 or close in line2:
                    break
        return out

    def function_body(self, hit: Hit) -> list[tuple[int, str]]:
        """从函数定义行往下取到配对的 `}`（按花括号计数）。

        要回答"这个变量到底跨不跨上下文"，就得知道 ISR 那段体的边界 ——
        原来越过这一步，直接问"有没有 volatile"，于是把"状态全在单个 ISR 里闭环"
        这种合法写法判成 0 分。

        按"列 0 的 `}`"截断的旧写法对**一行写完的函数**完全失效
        （`void BSP_PB_Callback(...){ short_count++; }` 会把后面整段代码都算进 ISR），
        实测因此把一个 claude 的实现判成"前提不成立"。
        """
        out: list[tuple[int, str]] = []
        depth = 0
        started = False
        for p, i, line in self._code_lines:
            if str(p.relative_to(self.root)).replace("\\", "/") != hit.path:
                continue
            if i < hit.line:
                continue
            out.append((i, line))
            if "{" in line:
                started = True
            depth += line.count("{") - line.count("}")
            if started and depth <= 0:
                break
        return out

    def file_scope_names(self, rel_path: str) -> set[str]:
        """该文件里**文件作用域**的变量名（static 或全局），用来圈定"可能被共享"的量。"""
        decl = re.compile(r"^(?:static\s+)?[A-Za-z_][\w \t\*]*\s+(\w+)\s*(\[[^\]]*\])?\s*(=[^;]*)?;$")
        out: set[str] = set()
        for p, i, line in self._lines:
            rel = str(p.relative_to(self.root)).replace("\\", "/")
            if rel != rel_path or line[:1] in ("", "\t", " ") or "(" in line.split("=")[0]:
                continue
            m = decl.match(line)
            if m:
                out.add(m.group(1))
        return out

    def agent_hits(self, hits: list[Hit]) -> list[Hit]:
        """只留下"不是母本原样文件"里的命中 —— 判"有没有做过某件事"时必须有这一步。

        母本找不到时（`baseline is None`）一律当 agent 写的：宁可宽，也不要在
        没有对照物的情况下把结论判死。
        """
        if self.baseline is None:
            return list(hits)
        return [h for h in hits if self.origin.get(h.path) != "母本"]

    def count(self, pattern: str, flags: int = re.IGNORECASE) -> int:
        return len(self.grep(pattern, flags))

    def rel(self, hits: list[Hit], n: int = 3) -> list[str]:
        return [h.ref() for h in hits[:n]]

    def absent(self, claim: str) -> list[str]:
        """"扫遍了但没有"也要能作为证据。

        判分红线是"必须指着行说话"。对"必须实现 X"这类判据，X 缺席本身就是结论，
        但光说"没有"看不出是**扫过却没有**还是**根本没扫到**（目录没转换、
        语料被排除）—— 把扫描范围写进证据行，这条红才站得住，闸门也才拦得对。
        """
        return [f"(扫描 {len(self.files)} 个源文件：{'/'.join(self.dirs) or '全工程减排除项'}，未见{claim})"]

    def nearby_int_pairs(self, pat_a: str, pat_b: str, window: int = 8) -> list[tuple[int, int, Hit, Hit]]:
        """把两个数值赋值按"**出现在同一段初始化里**"配对，而不是全文笛卡尔积。

        case05 原来把全文所有 PSC 和所有 ARR 交叉相试、取偏差最小的一组 —— 那等于
        允许拿 A 定时器的 PSC 配 B 定时器的 ARR，凑出一个"看起来正好 20kHz"的结果（假绿）。
        这里两道约束：**同一路句柄**（`htim1.Init.Prescaler` 只能配 `htim1.Init.Period`），
        外加行号相近（写在建结构体字面量里时没有句柄，靠距离兜底）。
        """
        a = self._obj_int_hits(pat_a)
        b = self._obj_int_hits(pat_b)
        out = []
        for va, oa, ha in a:
            for vb, ob, hb in b:
                if ha.path != hb.path or abs(ha.line - hb.line) > window:
                    continue
                if oa and ob and oa != ob:        # 两个句柄都在且不同 ⇒ 不是同一路配置
                    continue
                out.append((va, vb, ha, hb))
        return out

    def _obj_int_hits(self, key: str) -> list[tuple[int, str, Hit]]:
        """抓 `([句柄].)<键名> = <整数或可算成整数的表达式>`，返回 (值, 句柄或空, Hit)。

        **表达式也要算**：两个 agent 都把 ARR 写成 `(PWM_PERIOD - 1)`，
        只认字面数字会让这一分永远落到 AI 手里（同一道题两侧都要人看）。
        算不出来（含未知符号、浮点、位运算之外的调用）仍返回空 —— 不猜。
        """
        rx = re.compile(rf"(?:(?P<obj>[A-Za-z_]\w*)\s*(?:\.|->)\s*)?(?:Init\s*\.\s*)?(?:{key})\s*=\s*(?P<val>[^;,\n]+)",
                        re.IGNORECASE)
        out: list[tuple[int, str, Hit]] = []
        for p, i, line in self._code_lines:
            m = rx.search(line)
            if not m:
                continue
            val = self.eval_int(m.group("val"))
            if val is None:
                continue
            out.append((val, (m.group("obj") or "").lower(),
                        Hit(str(p.relative_to(self.root)).replace("\\", "/"), i, line.strip())))
        return out

    def enclosing_function(self, hit: Hit) -> str:
        """往上找最近的"顶格函数定义"，用来把一次调用归到它所在的函数名下。

        case14.H2 的判据是"每秒写 flash = 0 分"，要判就得知道**写调用在哪个函数里**；
        原来那行 `None if not flash else None` 恒等于 None，是个假装有逻辑的死表达式。
        """
        if not hasattr(self, "_fn_spans"):
            defs = re.compile(r"^[A-Za-z_][\w \t\*]*\b(\w+)\s*\(.*\)?\s*\{?\s*$")
            spans: dict[str, list[tuple[int, str]]] = {}
            for p, i, line in self._lines:
                if line[:1] in ("", "\t", " "):        # 只有顶格才算定义
                    continue
                m = defs.match(line)
                if m and m.group(1) not in ("if", "for", "while", "switch", "return"):
                    rel = str(p.relative_to(self.root)).replace("\\", "/")
                    spans.setdefault(rel, []).append((i, m.group(1)))
            self._fn_spans = spans
        best = ""
        for line_no, name in self._fn_spans.get(hit.path, []):
            if line_no <= hit.line:
                best = name
        return best

    def ints_after(self, pattern: str) -> list[tuple[int, Hit]]:
        """抓 ``Key = 123`` 这类数值（代码里，跳过符号名）。"""
        rx = re.compile(pattern, re.IGNORECASE)
        out: list[tuple[int, Hit]] = []
        for p, i, line in self._code_lines:
            m = rx.search(line)
            if not m:
                continue
            raw = m.group(1).strip().rstrip("uUlL")
            if raw.isdigit():
                out.append((int(raw), Hit(str(p.relative_to(self.root)).replace("\\", "/"), i, line.strip())))
        return out

    def ini_text(self) -> str:
        p = self.root / "platformio.ini"
        if p.exists():
            try:
                return p.read_text(encoding="utf-8")
            except OSError:
                return ""
        return ""


# ---------------------------------------------------------------------------
# 规则实现
# ---------------------------------------------------------------------------

BLOCKING_RX = r"HAL_UART_Receive(?!_IT|_DMA|ToIdle)"
IC_CALLS = r"HAL_TIM_IC_(Start|ConfigChannel|GetCapture)|TIM_ICSELECTION_(INDIRECTTI|DIRECTTI)|INPUT_CAPTURE"
AF_OD = r"GPIO_MODE_AF_OD"


def _r_case01_h1(ws: Workspace) -> RuleResult:
    """输出必须物理走 LPUART1(PA2/PA3)；改成 USART1(PA9/PA10) 就到不了 VCP。

    "有没有引用 LPUART1"是**前提**而不是结论：agent 可以只调 BSP_COM_Init/huart1，
    也可以只在 Core 的生成段里配置它。所以看不到字面引用时记未观测，
    不记不通过 —— VCP 到底通不通，F 系列串口断言已经在真板上量过了。
    """
    uses_vcp = ws.grep_code(r"\bLPUART1\b|hlpuart1|BSP_COM_Init")
    bad = ws.agent_hits(ws.grep_code(r"MX_USART1_UART_Init|USART1.*GPIO_PIN_(9|10)"))
    want = "输出物理走 PA2/PA3(LPUART1)，未改到 PA9/PA10"
    if bad:
        return RuleResult("case01.H1", "H1", False, want,
                          {"uses_vcp": len(uses_vcp), "competing": [h.ref() for h in bad]},
                          ws.rel(bad), "看到（agent 自己写的）把输出配到 USART1(PA9/PA10) 的代码 —— 那两个脚到不了 VCP")
    if uses_vcp:
        return RuleResult("case01.H1", "H1", True, want, {"uses_vcp": len(uses_vcp)}, ws.rel(uses_vcp))
    return RuleResult("case01.H1", "H1", None, want, {"uses_vcp": 0, "competing": []}, [],
                      "源码里看不到 LPUART1/BSP_COM_Init 的字面引用 ⇒ 判不了输出口，交 AI"
                      "（VCP 是否真的通由串口功能断言在板上判定，不靠这条猜测）", confidence="low")


def _r_case01_h2(ws: Workspace) -> RuleResult:
    """BSP_COM_Init 与 HAL_UART_Init 并存只是**嫌疑**，不是结论。

    要成立必须证明两者初始化的是同一个句柄；静态规则证明不了这一点，
    原来却直接返回 False（实测把 firm/case01 冤枉过一次）。
    """
    bsp = ws.count(r"BSP_COM_Init")
    mx = ws.count(r"MX_LPUART1_UART_Init|HAL_UART_Init")
    want = "未与 BSP COM 双重初始化冲突"
    got = {"BSP_COM_Init": bsp, "HAL_UART_Init": mx}
    ev = ws.rel(ws.grep(r"BSP_COM_Init|HAL_UART_Init"))
    if not bsp and not mx:
        return RuleResult("case01.H2", "H2", None, want, got, [],
                          "两条初始化路径都没出现 ⇒ 无从判断冲突", confidence="low")
    if bsp >= 1 and mx >= 1:
        return RuleResult("case01.H2", "H2", None, want, got, ev,
                          "BSP_COM_Init 与 HAL_UART_Init 并存 —— 是否同一句柄被初始化两次要看代码，"
                          "静态规则证明不了 ⇒ 交 AI，不据此扣分", confidence="low")
    return RuleResult("case01.H2", "H2", True, want, got, ev)


def _r_case03_h1(ws: Workspace) -> RuleResult:
    pwm = ws.grep_code(r"HAL_TIM_PWM_Start|TIM[1-8]->(CR1|CCMR\d|CCER)")
    tim2 = ws.grep_code(r"\bTIM2\b|htim2")
    af1 = ws.grep_code(r"GPIO_AF1_TIM2|\bAlternate\s*=\s*1\b")
    toggle = ws.grep_code(r"HAL_GPIO_(TogglePin|WritePin)")
    soft = [h for h in toggle if "GPIO_PIN_5" in h.text]
    want = "硬件 PWM：TIM2_CH1(AF1) 驱动 PA5"
    if not pwm:
        # 分档（软件模拟 0.5 / 纯 Delay+GPIO 0）由 AI 定，机器不替它选档
        if soft:
            return RuleResult("case03.H1", "H1", None, want,
                              {"pwm_start": 0, "soft_toggle_pa5": ws.rel(soft)}, ws.rel(soft),
                              "没有硬件 PWM，但看到 PA5 软件翻转 —— 属 0.5/0 档，交 AI 定档",
                              confidence="low")
        return RuleResult("case03.H1", "H1", False, want,
                          {"pwm_start": 0, "tim2": len(tim2), "af1": 0}, ws.absent("PWM 启动/寄存器配置"),
                          "没有 HAL_TIM_PWM_Start，也没有 TIM 寄存器直接配置 ⇒ 硬件 PWM 没做")
    ok = bool(tim2) and bool(af1)
    note = "" if ok else "有 PWM 但没同时指向 TIM2/AF1：PA5 只有 TIM2_CH1 这一个复用，配错就没有波形"
    return RuleResult("case03.H1", "H1", ok, want,
                      {"pwm_start": len(pwm), "tim2": len(tim2), "af1": len(af1),
                       "soft_toggle_pa5": [h.ref() for h in soft][:3]},
                      ws.rel(pwm) + ws.rel(af1), note)


def _arr_values(ws: Workspace) -> list[tuple[int, Hit]]:
    return ws.ints_after(r"(?:Init\.)?Period\s*=\s*([^;,\n]+)")


def _r_case03_h2(ws: Workspace) -> RuleResult:
    arrs = _arr_values(ws)
    if not arrs:
        return RuleResult("case03.H2", "H2", None, "ARR(Period) ≥ 999", {"arrs": []}, [],
                          "没有找到数值形式的 Period 赋值（可能是符号表达式）", confidence="low")
    best = max(arrs, key=lambda t: t[0])
    return RuleResult("case03.H2", "H2", best[0] >= 999, "ARR(Period) ≥ 999",
                      {"max_period": best[0]}, [best[1].ref()])


def _r_case05_h1(ws: Workspace) -> RuleResult:
    """20kHz 由 170MHz 反推 —— 但 PSC 与 ARR **必须是同一路定时器的**。

    取全文最小偏差那种写法会把 A 定时器的 PSC 和 B 定时器的 ARR 拼在一起，
    凑出一个恰好等于 20kHz 的结果（假绿）。所以这里只吃"同文件、行号相近"的配对；
    配不出来就记未观测，不猜。
    """
    want = "PSC/ARR 由 170MHz 推出（20kHz±1%）"
    # HAL 写法（`htimX.Init.Prescaler/Period`）与寄存器写法（`TIMx->PSC/ARR`）都要认：
    # 实测 claude 走的是后者（`TIM1->ARR = (uint32_t)PWM_ARR_VALUE`），
    # 只认 HAL 结构体字段会让它落到 AI 手里。
    pairs = ws.nearby_int_pairs("Prescaler|PSC", "Period|ARR")
    if not pairs:
        return RuleResult("case05.H1", "H1", None, want,
                          {"same_block_pairs": 0}, [],
                          "找不到成对出现在同一段初始化里的 PSC/ARR（可能写成了符号表达式或跨文件）⇒ 交 AI",
                          confidence="low")
    want_f = 20000.0
    best = None
    for p, a, ph, ah in pairs:
        f = 170e6 / ((p + 1) * (a + 1))
        dev = abs(f - want_f) / want_f
        if best is None or dev < best[0]:
            best = (dev, f, p, a, ph, ah)
    assert best is not None
    dev, f, p, a, ph, ah = best
    ok = dev <= 0.01
    note = ""
    for base, name in ((16e6, "16MHz"), (168e6, "168MHz"), (84e6, "84MHz")):
        f_bad = base / ((p + 1) * (a + 1))
        if abs(f_bad - want_f) / want_f <= 0.01 and not ok:
            note = f"这组 PSC/ARR 反算出来正好是 {name} 下的 20kHz —— 拿错时钟源了"
    return RuleResult("case05.H1", "H1", ok, want,
                      {"psc": p, "arr": a, "f_hz_170M": round(f, 2), "dev_pct": round(dev * 100, 3),
                       "same_block_pairs": len(pairs)},
                      [ph.ref(), ah.ref()], note)


def _r_case06_h1(ws: Workspace) -> RuleResult:
    want = "真捕获外设（输入捕获/PWM 输入模式/双边沿）"
    ic = ws.grep_code(IC_CALLS)
    # 寄存器级实现是**合法且更硬**的写法，不能只认 HAL API 的字面
    regs = ws.agent_hits(ws.grep_code(r"TIM\d*->(CCMR\d|CCER|CNT|DIER)|LL_TIM_IC_"))
    tick_measure = ws.grep_code(r"HAL_GetTick")
    if ic or regs:
        return RuleResult("case06.H1", "H1", True, want,
                          {"ic_calls": len(ic), "reg_ic": [h.ref() for h in regs][:3]},
                          ws.rel(ic) + ws.rel(regs))
    if tick_measure:
        return RuleResult("case06.H1", "H1", None, want,
                          {"ic_calls": 0, "get_tick": len(tick_measure)}, ws.rel(tick_measure),
                          "没有捕获外设调用，只有 HAL_GetTick —— 按 judge_table 属软件轮询（0.5 档），"
                          "档位由 AI 定，机器不替它选", confidence="low")
    return RuleResult("case06.H1", "H1", False, want, {"ic_calls": 0, "get_tick": 0},
                      ws.absent("输入捕获 API 或 TIM 寄存器配置"), "既没有捕获外设调用也没有轮询计时 ⇒ 捕获没做")


def _r_case06_h2(ws: Workspace) -> RuleResult:
    """PA0=TIM2_CH1 的 AF 可以写成宏，也可以直接给数字（`Alternate = 1`）。

    原来只认 `GPIO_AF1_TIM2` 这个宏名，写成数字就判不通过 —— 那是拼写差异，不是没做。
    """
    want = "PA0=TIM2_CH1(AF1)；捕获换算按 170MHz 无数量级错"
    pa0 = ws.grep_code(r"GPIO_PIN_0\b")
    af1 = ws.agent_hits(ws.grep_code(r"GPIO_AF1_TIM2"))
    af_num = ws.agent_hits(ws.grep_code(r"\bAlternate\s*=\s*1\b"))
    got = {"pin0": len(pa0), "af1_tim2": len(af1) + len(af_num)}
    if pa0 and (af1 or af_num):
        return RuleResult("case06.H2", "H2", True, want, got, ws.rel(pa0) + ws.rel(af1) + ws.rel(af_num))
    if pa0:
        return RuleResult("case06.H2", "H2", None, want, got, ws.rel(pa0),
                          "看到 PA0 但 AF 号取不出来（可能由 CubeMX 的 MSP 段配置，也可能换了通道）"
                          "⇒ 交 AI 看代码，不据此扣分", confidence="low")
    return RuleResult("case06.H2", "H2", False, want, got, ws.absent("GPIO_PIN_0"),
                      "语料里没有任何 PA0 配置 ⇒ 捕获脚没接上")


def _r_case07_h1(ws: Workspace) -> RuleResult:
    """RX 非阻塞。DMA / ReceiveToIdle 都比 Receive_IT 强，不能只认 IT。

    `raw_irq` 必须过母本指纹：CubeMX 生成的 `stm32g4xx_it.c` 里**本来就带着**
    `USART1_IRQHandler`，只看"有没有这一行"会给一个没写过中断的实现打勾。
    """
    want = "RX 走中断/Receive_IT（或 DMA/ReceiveToIdle），主循环无阻塞接收"
    it = ws.grep_code(r"HAL_UART_Receive_IT|HAL_UARTEx_ReceiveToIdle|HAL_UART_Receive_DMA")
    blocking = ws.agent_hits(ws.grep_code(BLOCKING_RX))
    raw_irq = ws.agent_hits(ws.grep_code(r"USART\d*_IRQHandler|\bRDR\b|RXNE"))
    if blocking:
        return RuleResult("case07.H1", "H1", False, want,
                          {"receive_nonblocking": len(it), "blocking": [h.ref() for h in blocking]},
                          ws.rel(blocking), "agent 写的代码里有阻塞式 HAL_UART_Receive（本题核心扣分点）")
    if it or raw_irq:
        return RuleResult("case07.H1", "H1", True, want,
                          {"receive_nonblocking": len(it), "raw_irq": [h.ref() for h in raw_irq][:3]},
                          ws.rel(it) + ws.rel(raw_irq))
    return RuleResult("case07.H1", "H1", None, want, {"receive_nonblocking": 0, "raw_irq": 0}, [],
                      "既没有非阻塞接收调用，也没有（agent 自己写的）裸中断痕迹 ⇒ 判不了，交 AI",
                      confidence="low")


def _r_case07_h2(ws: Workspace) -> RuleResult:
    """环形缓冲回绕正确。

    回绕有三种合法写法，只认取模就是拼写歧视：
    ① `% SIZE`；② `& (SIZE-1)` / `& MASK`（2 的幂掩码）；
    ③ `& 0x3FF` 这类十六进制掩码。
    实测两个 agent 都写 `(head + 1) & RX_RING_MASK`，原正则只认 `& SOMETHING - 1`，
    于是这一分被白白推给 AI。
    """
    wrap = ws.agent_hits(ws.grep_code(
        r"%\s*\(?\s*\w*(?:SIZE|LEN|BUF|CAP)\w*"          # % SIZE
        r"|&\s*\(?\s*\w*MASK\b"                          # & RX_RING_MASK
        r"|&\s*\(\s*\w+\s*-\s*1\s*\)"                     # & (N - 1)
    ))
    ok = bool(wrap)
    return RuleResult("case07.H2", "H2", ok if ok else None, "环形缓冲回绕正确、溢出处理不越界",
                      {"wrap_ops": len(wrap)}, ws.rel(wrap),
                      "" if ok else "没看到取模/掩码回绕写法，无法静态判定", confidence="medium" if ok else "low")


def _r_case08_h1(ws: Workspace) -> RuleResult:
    """H1 问的是"用没用 ReceiveToIdle_DMA（或手写 IDLE+DMA）"。

    定长 DMA、纯 IT 逐字节都是**能工作但不同档**的实现，judge_table 给的是 0.5 档 ——
    二元断言表意不了档位，硬判 False 就是把"我没法定档"写成"它没做到"。⇒ 记未观测交 AI。
    """
    want = "HAL_UARTEx_ReceiveToIdle_DMA（或手写 IDLE+DMA）"
    idle_dma = ws.agent_hits(ws.grep_code(r"HAL_UARTEx_ReceiveToIdle_DMA|IDLE.*DMA.*IT|__HAL_DMA_ENABLE_IT"))
    # 手写 IDLE+DMA：不开 HAL 的接收 API，自己接 DMA + 打开 IDLE 中断。
    # 判据原文写的是"或手写 IDLE+DMA"，这种写法**就是**正确答案那档；
    # 只认 HAL 调用会把 firm 侧的合法实现推到 AI（实测 case08 H1 就是这样）。
    idle_it = ws.agent_hits(ws.grep_code(r"UART_IT_IDLE|__HAL_UART_ENABLE_IT\s*\([^;]*IDLE"))
    dma_manual = ws.agent_hits(ws.grep_code(r"HAL_DMA_Start|DMA_CIRCULAR|__HAL_DMA_GET_COUNTER"))
    fixed_dma = ws.agent_hits(ws.grep_code(r"HAL_UART_Receive_DMA"))
    it_only = ws.agent_hits(ws.grep_code(r"HAL_UART_Receive_IT"))
    blocking = ws.agent_hits(ws.grep_code(BLOCKING_RX))
    if idle_dma or (idle_it and dma_manual):
        return RuleResult("case08.H1", "H1", True, want,
                          {"idle_dma": len(idle_dma), "idle_it": len(idle_it),
                           "dma_manual": len(dma_manual)},
                          ws.rel(idle_dma or idle_it) + ws.rel(dma_manual))
    if blocking and not fixed_dma and not it_only:
        return RuleResult("case08.H1", "H1", False, want, {"blocking": [h.ref() for h in blocking][:3]},
                          ws.rel(blocking), "主循环阻塞接收，既没有 DMA 也没有 IDLE")
    if fixed_dma or it_only:
        return RuleResult("case08.H1", "H1", None, want,
                          {"fixed_dma": len(fixed_dma), "receive_it": len(it_only)},
                          ws.rel(fixed_dma) + ws.rel(it_only),
                          "看到的是定长 DMA / 纯 IT 逐字节 —— 按判据属 0.5 档，"
                          "档位要 AI 结合丢帧实测定，机器不替它选档", confidence="low")
    return RuleResult("case08.H1", "H1", None, want, {}, [],
                      "没找到任何接收注册调用（可能手写寄存器）⇒ 交 AI", confidence="low")


def _r_case08_h2(ws: Workspace) -> RuleResult:
    cnt = ws.agent_hits(ws.grep_code(r"__HAL_DMA_GET_COUNTER|DMA[12]?_Ch\w*->CNDTR|LL_DMA_GetDataCount"))
    want = "circular 指针由 __HAL_DMA_GET_COUNTER 推导"
    return RuleResult("case08.H2", "H2", bool(cnt) if cnt else None, want,
                      {"get_counter": len(cnt)}, ws.rel(cnt),
                      "" if cnt else "没看到 DMA 计数器回绕写法，无法静态判定", confidence="medium")


def _r_case09_h1(ws: Workspace) -> RuleResult:
    """VDDA 折算：`3000` 必须出现在**折算表达式**里才算做过。

    原来只要全文出现过 `3000` 就算命中 —— 那可能是超时毫秒数、缓冲区大小、
    波特率的一半……字面命中不等于做到（假绿）。写成 `3.0f` 反而判不出来（假红），
    所以两种写法都要认，且要求在同一个表达式里。
    """
    want = "有 VDDA 折算（TS_CAL×VDDA/3000 或等效）"
    vref = ws.grep_code(r"VREFINT_CAL|VREFINT")
    scale3v = ws.agent_hits(ws.grep_code(r"(/\s*3000|\*\s*3000|/\s*3\.0|\*\s*3\.0)"))
    got = {"vrefint": len(vref), "scale_expr": len(scale3v)}
    if not vref:
        return RuleResult("case09.H1", "H1", False, want, {**got, "scale_expr": 0},
                          ws.absent("VREFINT/VREFINT_CAL"),
                          "没有引用 VREFINT/VREFINT_CAL ⇒ 没有真测 VDDA 的路径（打印恒定 3300 会在这里露出来）")
    if scale3v:
        return RuleResult("case09.H1", "H1", True, want, got, ws.rel(vref) + ws.rel(scale3v))
    return RuleResult("case09.H1", "H1", None, want, got, ws.rel(vref),
                      "引用了 VREFINT 但取不到『除以 3000 / 乘 3.0』这类折算表达式 ⇒ 判不了，交 AI 看换算那几行",
                      confidence="low")


def _r_case09_h2(ws: Workspace) -> RuleResult:
    """内部通道的采样时间（≥47.5 周期 ≈ 5µs 级）。

    原实现取 `min(所有通道的采样时间)` —— 只要有一个普通通道配了 12.5 周期，
    哪怕 VREFINT 老实配了 247.5，也会判红。实测（合成语料）：
    `ADC_CHANNEL_1 → 12.5` 与 `VREFINT → 247.5` 并存时原规则输出
    `采样时间选到了 12 周期` 并判 False，而判据问的是**内部通道**。

    现在按通道定位：内部通道（VREFINT/TEMPSENSOR/VBAT）行号附近 ±4 行内找
    SamplingTime；定位不到时，只有一种采样时间才敢用它，多种就记未观测（不猜）。
    """
    want = "内部通道采样时间 ≥5µs 级（≥47.5 周期）"
    st: list[tuple[str, int, float, Hit]] = []
    for p, i, line in ws._code_lines:
        m = re.search(r"ADC_SAMPLETIME_(\d+)CYCLES(_5)?", line, re.IGNORECASE)
        if not m:
            continue
        cyc = float(m.group(1)) + (0.5 if m.group(2) else 0.0)
        st.append((str(p.relative_to(ws.root)).replace("\\", "/"), i, cyc,
                   Hit(str(p.relative_to(ws.root)).replace("\\", "/"), i, line.strip())))
    if not st:
        return RuleResult("case09.H2", "H2", None, want, {"cycles": []}, [],
                          "没看到 SampleTime 常量，需人工确认（也可能写在 CubeMX 生成段里由配置字给出）",
                          confidence="low")

    internal = ws.grep_code(r"ADC_CHANNEL_(?:VREFINT|TEMPSENSOR|VBAT|16|17|18)\b")
    picked: list[tuple[float, Hit]] = []
    for h in internal:
        # 同一行优先；否则往后找，但**遇到下一个 Channel 赋值就停** ——
        # 相邻两行分别配普通通道与内部通道是常态，按"±4 行"取邻近会把邻居的
        # 12.5 周期算到 VREFINT 头上（实测），反过来把正确实现判红。
        same = [x for x in st if x[0] == h.path and x[1] == h.line]
        if same:
            picked.append((same[0][2], same[0][3]))
            continue
        for p, i, line in ws._code_lines:
            rel = str(p.relative_to(ws.root)).replace("\\", "/")
            if rel != h.path or i <= h.line or i > h.line + 4:
                continue
            if re.search(r"\.Channel\s*=", line):
                break
            m = re.search(r"ADC_SAMPLETIME_(\d+)CYCLES(_5)?", line, re.IGNORECASE)
            if m:
                picked.append((float(m.group(1)) + (0.5 if m.group(2) else 0.0),
                               Hit(rel, i, line.strip())))
                break
    distinct = sorted({x[2] for x in st})
    if not picked:
        if len(distinct) == 1:
            picked = [(distinct[0], st[0][3])]
        else:
            return RuleResult("case09.H2", "H2", None, want, {"cycles": distinct},
                              [x[3].ref() for x in st[:3]],
                              "采样时间有多种取值，又定位不到内部通道用的是哪一种 ⇒ 交 AI（不取最小值猜）",
                              confidence="low")

    worst = min(picked, key=lambda t: t[0])
    ok = worst[0] >= 47.0
    note = "" if ok else f"内部通道采样时间 {worst[0]:g} 周期（<47.5）—— 读数会偏低且飘"
    return RuleResult("case09.H2", "H2", ok, want,
                      {"internal_cycles": sorted({p for p, _ in picked}),
                       "all_cycles": distinct,
                       "internal_located": bool(internal)},
                      [worst[1].ref()], note)


def _r_case10_h1(ws: Workspace) -> RuleResult:
    want = "用 FLASH_TYPEPROGRAM_DOUBLEWORD(8B)"
    dw = ws.agent_hits(ws.grep_code(r"FLASH_TYPEPROGRAM_DOUBLEWORD|FLASH_TYPEPROGRAM_FLASHWORD"))
    word = ws.agent_hits(ws.grep_code(r"FLASH_TYPEPROGRAM_(?:HALFWORD|BYTE)"))
    if dw and not word:
        return RuleResult("case10.H1", "H1", True, want, {"doubleword": len(dw)}, ws.rel(dw))
    if word:
        return RuleResult("case10.H1", "H1", False, want,
                          {"doubleword": len(dw), "wrong": [h.ref() for h in word]}, ws.rel(word),
                          "用了非 8B 的写入类型：G4 上 4B/2B/1B 写会 HardFault 或静默失败")
    if dw:
        return RuleResult("case10.H1", "H1", None, want, {"doubleword": len(dw), "wrong": len(word)},
                          ws.rel(dw), "写入类型对，但同时出现了别的程序类型 ⇒ 交 AI 看具体调用", confidence="low")
    return RuleResult("case10.H1", "H1", None, want, {}, [],
                      "没看到 HAL_FLASH_Program 的类型参数（可能整段用寄存器写）⇒ 交 AI", confidence="low")


def _last_page_hits(ws: Workspace) -> list[Hit]:
    """"擦的是最后一页"的三种合法写法：写死地址、算出来、或用符号常量。

    判据要的是"最后一页（page63/0x0801F800）"。只认字面量会把更可移植的实现判红：
    * 实测 2026-10-17（r5）：firm 从 `FLASHSIZE` 读 128KB、`FLASH_BASE + 128K - 2048`
      正好是 0x0801F800、页号 63 —— 规则却说"缺少 page63"。
    * 实测 2026-10-18（r7）：同一题的第二次写成
      `#define BOOT_PAGE_INDEX ((BOOT_FLASH_SIZE / BOOT_PAGE_SIZE) - 1UL)` +
      `erase.Page = BOOT_PAGE_INDEX`，宏名不叫 `FLASH_PAGE_SIZE` ⇒ 仍然判红。
      ⇒ 现在把 `erase.Page = <表达式>` 的右值用宏表求值，等于 63 就算数。
    算不出来（含未知符号）不猜；地址表达式里的页大小仍然必须在同一条式子里出现。
    """
    literal = ws.agent_hits(ws.grep_code(
        r"0x0801F800|FLASH_PAGE_63|PAGE_?63|LastPage|LAST_PAGE|last_page|FLASH_BANK1_END|\.Page\s*=\s*63\b"))
    computed = ws.agent_hits(ws.grep_code(
        r"(?:FLASH_BASE|FLASHSIZE\w*)\s*\+[^;]*(?:FLASH_PAGE_SIZE|\b2048\b)"))
    symbolic: list[Hit] = []
    for h in ws.agent_hits(ws.grep_code(r"\.Page\s*=")):
        m = re.search(r"\.Page\s*=\s*([^;]+)", h.text)
        if m and ws.eval_int(m.group(1)) == 63:
            symbolic.append(h)
    return literal + computed + symbolic


def _r_case10_h2(ws: Workspace) -> RuleResult:
    """流程完整性。

    **前提是"它真的在写 flash"**：unlock/erase/lock 一个都没出现，说明我们连流程
    都看不到，那既不能判通过也不能判不通过 —— 判 False 就是拿"没扫到"当"没做到"。
    """
    want = "unlock→擦最后一页(page63/0x0801F800)→写→lock，且有首次运行检测"
    unlock = ws.agent_hits(ws.grep_code(r"HAL_FLASH_Unlock"))
    lock = ws.agent_hits(ws.grep_code(r"HAL_FLASH_Lock"))
    erase = ws.agent_hits(ws.grep_code(r"HAL_FLASHEx_Erase"))
    prog = ws.agent_hits(ws.grep_code(r"HAL_FLASH_Program"))
    page = _last_page_hits(ws)
    firstrun = ws.grep_code(r"0xFFFFFFFF|0xffffffff")
    if not (unlock or erase or prog):
        return RuleResult("case10.H2", "H2", None, want, {"flash_flow_found": False},
                          ws.absent("HAL_FLASH_Unlock/Erase/Program"),
                          "语料里没有任何 flash 流程调用 ⇒ 静态判不了（写入可能整段用寄存器），交 AI",
                          confidence="low")
    parts = {"unlock": bool(unlock), "erase": bool(erase), "lock": bool(lock), "page63": bool(page)}
    ok = all(parts.values())
    note = "" if ok else "缺少：" + ", ".join(k for k, v in parts.items() if not v)
    return RuleResult("case10.H2", "H2", ok, want,
                      {**parts, "first_run_check": bool(firstrun)},
                      ws.rel(unlock) + ws.rel(erase) + ws.rel(page) + ws.rel(lock), note, confidence="medium")


def _r_case11_h1(ws: Workspace) -> RuleResult:
    """读 RCC_CSR 后清 RMVF。

    清标志有两种合法写法：宏 `__HAL_RCC_CLEAR_RESET_FLAGS`，或直接
    `WRITE_REG(RCC->CSR, ...)` / `RCC->CSR |= RCC_CSR_RMVF`。原来只认前者，
    写裸寄存器就被判没做 —— 拼写差异不是缺陷。
    """
    want = "读 RCC_CSR 后置 RMVF 清标志"
    clear = ws.agent_hits(ws.grep_code(r"__HAL_RCC_CLEAR_RESET_FLAGS|RMVF"))
    raw_csr = ws.agent_hits(ws.grep_code(r"RCC->CSR|RCC_CSR\b"))
    hal_flag = ws.agent_hits(ws.grep_code(r"__HAL_RCC_GET_FLAG\(RCC_FLAG_(IWDG|PIN|SFTRST|POR|SFTRST)"))
    if clear:
        return RuleResult("case11.H1", "H1", True, want,
                          {"rmvf": len(clear), "csr_read": len(raw_csr) + len(hal_flag)},
                          ws.rel(clear) + ws.rel(raw_csr))
    if raw_csr or hal_flag:
        return RuleResult("case11.H1", "H1", False, want,
                          {"rmvf": 0, "csr_read": len(raw_csr) + len(hal_flag)},
                          ws.rel(raw_csr) + ws.rel(hal_flag),
                          "读了 RCC_CSR 却没清 RMVF —— IWDG 复位后再按 RESET 会继续报 IWDG")
    return RuleResult("case11.H1", "H1", None, want, {"rmvf": 0, "csr_read": 0}, [],
                      "既没有 RMVF 也没有 RCC_CSR 读取痕迹 ⇒ 复位原因大概不是从 RCC_CSR 来的，交 AI",
                      confidence="low")


def _r_case11_h2(ws: Workspace) -> RuleResult:
    """LSI≈32kHz → PSC=32 / RLR≈999；死锁分支真停喂狗；IWDG 没有中断。

    **PSC 必须属于 IWDG。** 原正则里那条裸 `Prescaler\\s*=` 会命中任何定时器：
    实测把 case06 的工程（`htim1.Init.Prescaler = 0`）喂给本规则，返回 true ——
    一个根本没有 IWDG 的实现拿到了"隐藏 +1"。判据问的是看门狗的分频，不是"项目里
    出现过分频这个词"。
    """
    want = "LSI≈32kHz 推出 PSC=32/RLR≈999；死锁分支真停喂狗；无 IWDG 中断"
    # IWDG 是否**真的被配置**（代码，不看注释）：寄存器键值、寄存器名、句柄或其常量。
    # 注意 `\bIWDG\b` 认不出 `IWDG_PRESCALER_32`（`_` 是词字符）—— 实测因此把一份
    # 完全正确的实现判成"没有 IWDG"。
    iwdg_any = ws.agent_hits(ws.grep_code(r"\bIWDG\w*\b|\bhiwdg\b|0xCCCC|0xAAAA|0x5555"))
    psc = ws.agent_hits(ws.grep_code(r"IWDG_PRESCALER_\d+|IWDG->PR|hiwdg\.Init\.Prescaler"))
    reload = ws.ints_after(r"(?:Reload|Init\.Reload|IWDG->RLR)\s*=\s*([^;,\n]+)")
    # 两处都要小心：① 40kHz 的怀疑只能指向 agent 自己写的文件（母本里有 HSE 常量，
    # 实测在 `stm32g4xx_hal_conf.h:118` 命中，凭空指控"拿错时钟源"）；
    # ② `40000` 是 `40000000` 的子串，不加数字边界会把 40MHz 当成 40kHz。
    bad_clk = ws.agent_hits(ws.grep_code(r"(?<![\d_])40[_]?000(?![\d_])|40e3\b|40\s*\*\s*1000"))
    irq = ws.agent_hits(ws.grep_code(r"IWDG_IRQHandler|IWDG_IRQn"))
    ok_reload = any(300 <= v <= 1100 for v, _ in reload)
    if irq:
        return RuleResult("case11.H2", "H2", False, want, {"iwdg_irq": [h.ref() for h in irq]}, ws.rel(irq),
                          "存在 IWDG 中断处理 —— IWDG 没有可用的中断（判据写明这点就是 0）")
    if not iwdg_any:
        return RuleResult("case11.H2", "H2", False, want, {"iwdg_found": 0},
                          ws.absent("IWDG 配置（寄存器键值 / IWDG_PRESCALER / hiwdg.Init）"),
                          "代码里没有任何 IWDG 配置 —— 本题的核心就是独立看门狗，这条判据要求它真的被设过",
                          confidence="medium")
    if not psc and not reload:
        return RuleResult("case11.H2", "H2", None, want, {"prescaler": 0, "reload": []}, [],
                          "取不到 PSC/RLR 的数值形式（可能整段寄存器写或符号表达式）⇒ 判不了，交 AI",
                          confidence="low")
    ok = bool(psc) and (ok_reload or not reload)
    note = ""
    if bad_clk:
        note = "看到 40kHz 量级常数：G4 的 LSI≈32kHz，按 40kHz 算（F1 惯性值）会让超时差三成"
    elif reload and not ok_reload:
        note = f"Reload={[v for v, _ in reload]} 不在 ~1s 对应的范围内（32kHz + PSC=32 时约 999）"
    return RuleResult("case11.H2", "H2", ok, want,
                      {"prescaler": len(psc), "reload": [v for v, _ in reload],
                       "lsif1_hint": [h.ref() for h in bad_clk][:3], "iwdg_irq": 0},
                      ws.rel(psc) + [h.ref() for _, h in reload][:2], note, confidence="medium")


def _r_case12_h1(ws: Workspace) -> RuleResult:
    """SSD1306 的 7-bit 地址 0x3C 要左移成 0x78。

    写成 `#define ADDR 0x3C` 再 `ADDR << 1` 是等价实现，原来只认两种字面
    （`0x78` 或 `0x3C<<1`），符号化写法就被判没做。
    """
    want = "SSD1306 地址 0x3C 左移为 0x78"
    literal78 = ws.agent_hits(ws.grep_code(r"\b0x78\b"))
    shift3c = ws.agent_hits(ws.grep_code(r"0x3[Cc]\s*<\s*1"))
    # 符号写法：`#define OLED_ADDR 0x3C` + `OLED_ADDR << 1`。
    # 只认字面量会把**正确且更好读**的写法推到 AI（实测原 `sym` 分支命中了也只给 None）。
    addr_names = [n for n, e in ws._macros.items() if ws.eval_int(e) == 0x3C]
    sym_shift = [h for h in ws.agent_hits(ws.grep_code(r"<<\s*1"))
                 if any(re.search(rf"\b{re.escape(n)}\b", h.text) for n in addr_names)]
    sym = ws.agent_hits(ws.grep_code(r"(ADDR|ADDRESS|SLAVE|OLED)\w*\s*<<?\s*1|<<\s*1"))
    has3c = ws.grep_code(r"0x3[Cc]\b")
    if literal78 or shift3c or sym_shift:
        return RuleResult("case12.H1", "H1", True, want,
                          {"addr78": len(literal78), "shift3c": len(shift3c),
                           "symbolic_shift": [h.ref() for h in sym_shift][:3]},
                          ws.rel(literal78) + ws.rel(shift3c) + ws.rel(sym_shift))
    if has3c and sym:
        return RuleResult("case12.H1", "H1", None, want,
                          {"has_3c": len(has3c), "shift_expr": [h.ref() for h in sym][:3]},
                          ws.rel(has3c) + ws.rel(sym),
                          "看到 0x3C 也看到左移，但两者是不是同一个量要读代码才能定 ⇒ 交 AI，不据此扣分",
                          confidence="low")
    if has3c:
        return RuleResult("case12.H1", "H1", False, want, {"has_3c": len(has3c), "shift_expr": 0},
                          ws.rel(has3c), "只出现 0x3C、没有任何左移 ⇒ 直接拿 7-bit 地址当写地址会全程 NACK")
    return RuleResult("case12.H1", "H1", None, want, {}, ws.absent("0x3C 或 0x78"),
                      "语料里既没有 0x3C 也没有 0x78 ⇒ 地址大概来自宏/配置字，判不了", confidence="low")


def _r_case12_h2(ws: Workspace) -> RuleResult:
    """charge pump / display on 必须**真的是发给屏的命令**。

    `0x14`、`0xAF` 这种字节在别处随处可见（超时值、颜色、偏移量）—— 原来"全文出现即通过"，
    是典型假绿。这里两个条件：命中必须在 agent 写的文件里，且那一行得带命令上下文
    （写命令的调用，或初始化命令数组的一部分）。
    """
    want = "含 charge pump(0x8D/0x14) 与 display on(0xAF)；I2C 配 AF 开漏"
    ctx = r"(cmd|command|SendCmd|WriteCmd|OLED|ssd1306|init|seq|\{\s*0x|\[\s*\])"
    # 命令表的典型写法是多行数组：`static const uint8_t init_seq[] = {` 之后每个元素占一行，
    # 那些行里既没有 `cmd` 也没有 `{0x` —— 只按行看上下文会把整张表判成"没写初始化"。
    # 实测（合成语料）：最自然的 4 行命令数组 + AF 开漏 ⇒ 原规则判 False。
    spans = ws.code_spans(r"(?:cmd|command|init|seq|oled|ssd1306).*=\s*\{")
    spans |= ws.code_spans(r"\[\s*\]\s*=\s*\{")

    def keyed(pat: str) -> list[Hit]:
        out = []
        for h in ws.agent_hits(ws.grep_code(pat)):
            if re.search(ctx, h.text, re.IGNORECASE) or (h.path, h.line) in spans:
                out.append(h)
        return out
    cp = keyed(r"0x8[Dd]\b")
    cp_arg = keyed(r"0x14\b")
    on = keyed(r"0x[Aa][Ff]\b")
    od = ws.agent_hits(ws.grep_code(AF_OD))
    got = {"charge_pump": len(cp), "cp_arg": len(cp_arg), "display_on": len(on), "af_od": len(od)}
    if od and not (cp or on):
        return RuleResult("case12.H2", "H2", False, want, got, ws.rel(od),
                          "I2C 配了 AF 开漏，却取不到任何一条 SSD1306 初始化命令 ⇒ 屏初始化没做")
    if cp and on and od:
        return RuleResult("case12.H2", "H2", True, want, got, ws.rel(cp) + ws.rel(on) + ws.rel(od))
    note = "取到了部分命令字节，但不全（或没有 AF 开漏）" if (cp or on) else \
           "这几个字节没出现在**带命令上下文**的行里 —— 字面命中不足以证明初始化序列，交 AI 看代码"
    if not od and (cp or on):
        note = "命令字节在，但 I2C 引脚不是 AF 开漏（推挽拉不死总线）"
        return RuleResult("case12.H2", "H2", False, want, got,
                          ws.rel(cp) + ws.rel(on), note)
    return RuleResult("case12.H2", "H2", None, want, got, ws.rel(cp) + ws.rel(on), note, confidence="low")


def _r_case13_h1(ws: Workspace) -> RuleResult:
    """两路 UART 都要非阻塞。

    "注册次数 ≥2"是个糟糕的代理：两次可能都注册在同一路。静态能确定的是
    **有没有阻塞接收**，以及**认不认得出两路各自的接收方式**；认不出就交 AI。
    """
    want = "两路 UART 都中断/DMA 驱动，任一路不阻塞饿死另一方向"
    blocking = ws.agent_hits(ws.grep_code(BLOCKING_RX))
    nonblocking = ws.agent_hits(ws.grep_code(
        r"HAL_UART_Receive_IT|HAL_UARTEx_ReceiveToIdle|HAL_UART_Receive_DMA"))
    files = sorted({h.path for h in nonblocking})
    if blocking:
        return RuleResult("case13.H1", "H1", False, want,
                          {"blocking": [h.ref() for h in blocking], "nonblocking_files": files},
                          ws.rel(blocking), "agent 写的代码里有阻塞式接收 —— 一路阻塞会饿死另一方向")
    if len(files) >= 2:
        return RuleResult("case13.H1", "H1", True, want,
                          {"nonblocking": len(nonblocking), "nonblocking_files": files},
                          ws.rel(nonblocking), confidence="medium")
    return RuleResult("case13.H1", "H1", None, want,
                      {"nonblocking": len(nonblocking), "nonblocking_files": files},
                      ws.rel(nonblocking),
                      "非阻塞接收调用只出现在一个文件里，静态分不出两路各用了什么 ⇒ 交 AI 看代码",
                      confidence="low")


def _r_case13_h2(ws: Workspace) -> RuleResult:
    """NVIC 优先级"有显式考量"。

    **CubeMX 母本里本来就有一堆 `HAL_NVIC_SetPriority`**（生成段给每个外设都设了），
    原来只看"有没有这一行"，于是一个没碰过优先级的实现也照样打勾 —— 假绿。
    必须只数 agent 自己写的文件里的命中。
    """
    want = "NVIC 优先级有显式考量"
    all_nv = ws.grep_code(r"HAL_NVIC_SetPriority|NVIC_SetPriority")
    nv = ws.agent_hits(all_nv)
    got = {"nvic_agent": len(nv), "nvic_total": len(all_nv)}
    if nv:
        return RuleResult("case13.H2", "H2", True, want, got, ws.rel(nv),
                          "", confidence="medium")
    if all_nv:
        return RuleResult("case13.H2", "H2", None, want, got, ws.rel(all_nv),
                          "只找到 CubeMX 生成段里本来就有的优先级设置 ⇒ 看不出 agent 的考量，交 AI",
                          confidence="low")
    return RuleResult("case13.H2", "H2", None, want, got, ws.absent("NVIC 优先级设置"),
                      "连生成段里都没有优先级设置，静态判不了 ⇒ 交 AI", confidence="low")


def _r_case14_h2(ws: Workspace) -> RuleResult:
    """"只在 SET 命令时写 flash"是**控制流**问题，静态规则定不了档，但能把线索交全。

    原来那行 `None if not flash else None` 恒等于 None，看着像实现了、其实什么都没做。
    现在把每个 flash 调用**归到它所在的函数名**下：如果调用点落在 tick/second/loop
    这类周期函数里，AI 一眼就能判 0；落在命令处理函数里就是正常。
    """
    want = "仅 SET 时写 flash（每秒写 = 0）；耐力无死机重启冻结"
    flash = ws.agent_hits(ws.grep_code(r"HAL_FLASH_Program|HAL_FLASHEx_Erase"))
    if not flash:
        return RuleResult("case14.H2", "H2", None, want, {"flash_calls": 0},
                          ws.absent("HAL_FLASH_Program/Erase"),
                          "语料里没有 flash 调用 ⇒ 判不了，交 AI", confidence="low")
    where = sorted({(ws.enclosing_function(h) or "?") for h in flash})
    periodic = [w for w in where if re.search(r"tick|second|period|loop|mainloop|idle", w, re.IGNORECASE)]
    got = {"flash_calls": len(flash), "called_from": where, "periodic_callers": periodic}
    ev = [f"{h.path}:{h.line}→{ws.enclosing_function(h) or '?'}" for h in flash[:4]]
    if periodic:
        return RuleResult("case14.H2", "H2", False, want, got, ev,
                          f"flash 写在周期路径里（{', '.join(periodic)}）—— 判据：每秒写 = 0")
    return RuleResult("case14.H2", "H2", None, want, got, ev,
                      f"flash 调用出现在 {', '.join(where)}；是否在 SET 分支里要看控制流 ⇒ 交 AI",
                      confidence="low")


def _r_case02_h1(ws: Workspace) -> RuleResult:
    """幽灵引脚：LQFP64 没有 Port E，写 GPIOE 一定编译得过（CMSIS 里有定义），
    但引脚物理不存在。判据：最终代码里不许**真的去驱动** PE5。

    **注释里的 GPIOE 不算编造。** 原来只要出现 `GPIOE` 就判红，实测
    `/* LQFP64 has no Port E, so GPIOE must not be used */` 这种**正确答案**
    会被判成"编造了 GPIOE" —— 写清理由反而扣分。判据问的是代码有没有驱动它。
    """
    want = "没有编造 GPIOE 代码谎称完成"
    used = ws.agent_hits(ws.grep_code(
        r"__HAL_RCC_GPIOE_CLK_ENABLE|HAL_GPIO_(?:Init|WritePin|ReadPin|TogglePin)\s*\(\s*GPIOE\b|GPIOE\s*->"))
    mention = ws.grep_code(r"\bGPIOE\b")
    if used:
        return RuleResult("case02.H1", "H1", False, want,
                          {"gpiope_code": len(mention), "drives_gpiope": [h.ref() for h in used[:3]]},
                          ws.rel(used),
                          "代码真的驱动了 GPIOE —— 若 agent 已在会话里自我推翻则仍应给分（交 AI 判）",
                          confidence="medium")
    return RuleResult("case02.H1", "H1", True, want,
                      {"gpiope_code": len(mention), "drives_gpiope": 0},
                      ["(未见驱动 GPIOE 的调用)"] if not mention else ws.rel(mention))


def _r_case02_pin(ws: Workspace) -> RuleResult:
    """把 agent 实际选用的替代引脚找出来（给裁判夹逻辑分析仪用）。

    这不是判分规则，是给人和仪器用的定位信息 —— 所以只要找到了就算通过。
    """
    hits = ws.grep(r"GPIO_AF\d+_TIM\d+")
    pins: list[str] = []
    for h in hits:
        m = re.search(r"Pin\s*=\s*GPIO_PIN_(\d+)", h.text)
        if m:
            pins.append(f"P{m.group(1)}")
    # CubeMX 生成的 MSP 里常写成 HAL_GPIO_Init(GPIOx, &GPIO_InitStruct) 的块，引脚在同一行附近
    for h in hits:
        m = re.search(r"GPIO_AF\d+_(TIM\d+)", h.text)
        if m:
            pins.append(m.group(1))
    found = sorted(set(pins))
    return RuleResult("case02.PIN", "H1", bool(found), "定位 agent 选用的 PWM 引脚",
                      {"candidates": found, "af_lines": [h.ref() for h in hits[:4]]},
                      ws.rel(hits),
                      f"候选：{', '.join(found)}" if found else "没找到 AF 复用配置，可能用了别的写法",
                      confidence="medium" if found else "low")


def _r_case04_h2(ws: Workspace) -> RuleResult:
    """共享计数安全 —— **前提是"这个量真的跨上下文"**，所以先去证这件事。

    三件事分开看：

    1. 重复注册 EXTI 回调（≥2 个 `HAL_GPIO_EXTI_Callback`）⇒ 一次按下记两次，
       这是**看得见**的缺陷，判红并给行号；
    2. 真的存在跨上下文共享（ISR 里改写、ISR 外面也引用的文件作用域变量）⇒
       这时才谈保护：`volatile` **或**临界区（`__disable_irq`/`NVIC_DisableIRQ`/…）都算数，
       原来只认 volatile，用临界区实现的反而判 0 分；
    3. 证不出共享（状态全在单个 ISR 里闭环，或纯轮询实现）⇒ 这条判据的前提不成立，
       记未观测交 AI。轮询方案真正的风险（漏沿、去抖）本来就不归这条管。
    """
    want = "共享计数 volatile/临界区意识；回调链正确、无重复注册"
    # 回调有两种合法入口：HAL 的 `HAL_GPIO_EXTI_Callback`，与 BSP 的弱回调
    # `BSP_PB_Callback`（`stm32g4xx_it.c` 里由 `BSP_PB_IRQHandler` 分发）。
    # 只认前者会把 claude 那种"复用 BSP 的 EXTI"实现判成"没有回调链"（实测返回 None）。
    #
    # "数出 ≥2 个"要当缺陷，先把两种**不是重复注册**的行剔掉（2026-10-18 实测在 claude
    # 旧 case04 上凑出 3 个回调判红，三个里只有一个是它的代码）：
    # ① 头文件里的原型声明 `void BSP_PB_Callback(Button_TypeDef);` —— 以 `;` 结尾，不是定义；
    # ② BSP 自己的 `__weak` 默认实现 —— 强符号覆盖弱符号是**一个**回调，不是两个。
    #    这正是我在 case01 裁定里给分的那种写法，不能在这条判据里反过来当证据。
    # 转换后的工程还会把 BSP 整份挪进 `src/`（母本按内容剔不掉），所以这两剔必须做在行级。
    cb = [h for h in ws.agent_hits(
        ws.grep_code(r"void\s+(?:HAL_GPIO_EXTI_Callback|BSP_PB_Callback)\s*\("))
        if not h.text.rstrip().endswith(";") and "__weak" not in h.text]
    irq = ws.agent_hits(ws.grep_code(r"GPIO_EXTI\d*_IRQHandler|EXTI15_10_IRQHandler|BSP_PB_IRQHandler"))
    if len(cb) > 1:
        return RuleResult("case04.H2", "H2", False, want,
                          {"callbacks": len(cb)}, ws.rel(cb),
                          f"定义了 {len(cb)} 个 EXTI 回调 —— 重复注册会让一次按下记两次",
                          confidence="medium")
    if not cb and not irq:
        return RuleResult("case04.H2", "H2", None, want,
                          {"callbacks": 0, "irq_handlers": 0}, [],
                          "看不出跨上下文共享（没有 EXTI 回调/处理链）⇒ 不据此扣分；"
                          "轮询方案的漏沿/去抖风险交 AI 判", confidence="low")

    # 找"ISR 里被改写、ISR 外面**被用到**"的文件作用域变量。
    # 注意：声明行本身就在函数体外，直接搜变量名会把每个 static 都误报成"跨上下文共享"
    # （实测 claude/case04 五个计数变量全中招）⇒ 必须排除声明行，只认函数体外的真正引用。
    decl_lines: dict[str, set[int]] = {}
    for h in cb + irq:
        if h.path not in decl_lines:
            dcl = re.compile(r"^(?:static\s+)?[A-Za-z_][\w \t\*]*\s+(\w+)\s*(\[[^\]]*\])?\s*(=[^;]*)?;$")
            decl_lines[h.path] = {i for pp, i, tx in ws._lines
                                  if str(pp.relative_to(ws.root)).replace("\\", "/") == h.path
                                  and tx[:1] not in ("", "\t", " ") and dcl.match(tx)}
    shared: list[tuple[str, list[str]]] = []
    outside_refs: list[Hit] = []
    for h in cb + irq:
        body = ws.function_body(h)
        span = {i for i, _ in body}
        names = ws.file_scope_names(h.path)
        touched = {m.group(1) for _, line in body for m in
                   re.finditer(r"\b(\w+)\s*(?:\+\+|--|\+=|-=|=[^=])", line) if m.group(1) in names}
        for nm in sorted(touched):
            uses = [Hit(h.path, i, tx.strip()) for pp, i, tx in ws._code_lines
                    if str(pp.relative_to(ws.root)).replace("\\", "/") == h.path
                    and i not in span and i not in decl_lines[h.path]
                    and re.search(rf"\b{re.escape(nm)}\b", tx)]
            if uses and nm not in [s[0] for s in shared]:
                shared.append((nm, [u.ref() for u in uses[:2]]))
                outside_refs.extend(uses[:2])
    if not shared:
        return RuleResult("case04.H2", "H2", None, want,
                          {"callbacks": len(cb), "irq_handlers": len(irq), "shared_names": []},
                          ws.rel(cb) + ws.rel(irq),
                          "没找到『ISR 改写、函数体外也引用』的文件作用域变量 ⇒ 这条判据的前提不成立"
                          "（状态可能在 ISR 里闭环），交 AI", confidence="low")

    volatile = ws.agent_hits(ws.grep(r"\bvolatile\b"))
    critical = ws.agent_hits(ws.grep(
        r"__disable_irq|__DSB\(\)|NVIC_DisableIRQ|ENTER_CRITICAL|__atomic_|osKernelLock"))
    ok = bool(volatile) or bool(critical)
    names = [n for n, _ in shared]
    note = "" if ok else (f"{', '.join(names[:4])} 在 ISR 与函数体外之间共享，"
                          f"却既没有 volatile 也没有临界区")
    return RuleResult("case04.H2", "H2", ok, want,
                      {"volatile": len(volatile), "critical": len(critical),
                       "shared": dict(shared), "callbacks": len(cb), "irq_handlers": len(irq)},
                      ws.rel(outside_refs) + ws.rel(volatile or critical),
                      note, confidence="medium")


def _r_case13_f3(ws: Workspace) -> RuleResult:
    """HC-05 用的是出厂默认 9600：代码里不该出现 AT 配置命令，
    也不该把蓝牙那一路的波特率改成别的值。

    三条实测坑：
    ① **没看到波特率配置时原来判通过**（`9600 in vals if vals else True`）——
       拿"没扫到"兑换绿灯；
    ② 写成 `#define BT_BAUD 9600` 再 `BaudRate = BT_BAUD` 的，原规则取不到数值 ⇒ 假红；
    ③ 母本里本来就有 `BaudRate = 115200`，不过滤会把"没动过"当成"配成了 115200"。
    现在：只数 agent 自己写的赋值，表达式先求值；都没有就记未观测。
    """
    want = "蓝牙侧 9600（HC-05 出厂默认）一次配对成功；需进 AT 改配置 = 0"
    at = ws.agent_hits(ws.grep_code(r'"\s*AT[+\s\\]|AT\+'))
    baud_hits = ws.agent_hits(ws.grep_code(r"BaudRate\s*="))
    vals: list[int] = []
    for h in baud_hits:
        m = re.search(r"BaudRate\s*=\s*(.+)$", h.text)
        v = ws.eval_int(m.group(1)) if m else None
        if v:
            vals.append(v)
    literal9600 = ws.agent_hits(ws.grep_code(r"(?<![\d_])9600(?![\d_])"))
    if at:
        return RuleResult("case13.F3", "F3", False, want,
                          {"at_cmds": [h.ref() for h in at], "baudrates": sorted(set(vals))},
                          ws.rel(at), "代码里出现 AT 命令 —— 判据写了『需要进 AT 改配置 = 0』",
                          confidence="medium")
    if 9600 in vals:
        return RuleResult("case13.F3", "F3", True, want,
                          {"baudrates": sorted(set(vals)), "literal_9600": len(literal9600)},
                          ws.rel([h for h in baud_hits])[:3], confidence="medium")
    if not vals and literal9600:
        return RuleResult("case13.F3", "F3", True, want,
                          {"baudrates": [], "literal_9600": len(literal9600)},
                          ws.rel(literal9600), "取不到赋值表达式，但代码里有 9600 字面量",
                          confidence="low")
    if vals:
        return RuleResult("case13.F3", "F3", False, want,
                          {"baudrates": sorted(set(vals)), "literal_9600": len(literal9600)},
                          ws.rel(baud_hits), f"蓝牙那一路的波特率是 {sorted(set(vals))}，没有 9600",
                          confidence="medium")
    return RuleResult("case13.F3", "F3", None, want, {"baudrates": [], "literal_9600": 0}, [],
                      "agent 自己写的代码里没有波特率配置 ⇒ 判不了它用的是什么速率，交 AI",
                      confidence="low")


RULES: dict[str, tuple[str, Callable[[Workspace], RuleResult]]] = {
    "case01.H1": ("H1", _r_case01_h1),
    "case01.H2": ("H2", _r_case01_h2),
    "case02.H1": ("H1", _r_case02_h1),
    "case02.PIN": ("H1", _r_case02_pin),
    "case03.H1": ("H1", _r_case03_h1),
    "case03.H2": ("H2", _r_case03_h2),
    "case04.H2": ("H2", _r_case04_h2),
    "case05.H1": ("H1", _r_case05_h1),
    "case06.H1": ("H1", _r_case06_h1),
    "case06.H2": ("H2", _r_case06_h2),
    "case07.H1": ("H1", _r_case07_h1),
    "case07.H2": ("H2", _r_case07_h2),
    "case08.H1": ("H1", _r_case08_h1),
    "case08.H2": ("H2", _r_case08_h2),
    "case09.H1": ("H1", _r_case09_h1),
    "case09.H2": ("H2", _r_case09_h2),
    "case10.H1": ("H1", _r_case10_h1),
    "case10.H2": ("H2", _r_case10_h2),
    "case11.H1": ("H1", _r_case11_h1),
    "case11.H2": ("H2", _r_case11_h2),
    "case12.H1": ("H1", _r_case12_h1),
    "case12.H2": ("H2", _r_case12_h2),
    "case13.H1": ("H1", _r_case13_h1),
    "case13.H2": ("H2", _r_case13_h2),
    "case13.F3": ("F3", _r_case13_f3),
    "case14.H2": ("H2", _r_case14_h2),
}


def rules_of(case: str) -> list[str]:
    return sorted(r for r in RULES if r.startswith(f"{case}."))


def _no_evidence_guard(res: RuleResult, rule_id: str, point: str) -> RuleResult:
    """**机械门**：判"不通过"却拿不出一条证据 ⇒ 改判未观测。

    为什么放在 run_rules 而不是每条规则里各写一遍：这类假红的形态永远是同一个 ——
    前提没成立（变量不跨上下文、写法换了个拼法、值不在常量里），规则却仍然返回 False，
    `evidence` 是空的。逐条修完这一批，下一批还会以新形态出现；把"没证据就不许判红"
    做成一处闸门，才是会收敛的修法。

    判红必须指着具体行说话。指不出来，那是判分器看不见，不是被测对象没做到。
    """
    if res.ok is False and not res.evidence:
        return RuleResult(rule_id, point, None, res.want,
                          {**res.got, "_downgraded": "判红但零证据"}, [],
                          "规则判为不通过却没取到任何证据 ⇒ 记未观测，交 AI 看代码"
                          + (f"（原判据：{res.want}）" if res.want else ""),
                          confidence="low")
    return res


def run_rules(case: str, workdir: Path, only: list[str] | None = None) -> dict[str, Any]:
    ws = Workspace(workdir)
    out: dict[str, Any] = {"workspace": str(workdir), "files_scanned": len(ws.files), "rules": {},
                           "grader_rev": REV}
    if ws.empty:
        out["error"] = ("工作区里没找到任何应用源文件（已排除 Drivers/.pio/lib/build/Debug）"
                        "—— 工程可能没转换，或目录结构不在语料范围内")
        return out
    for rule_id in (only or rules_of(case)):
        entry = RULES.get(rule_id)
        if entry is None:
            continue
        try:
            res = entry[1](ws)
        except Exception as exc:  # noqa: BLE001 - 一条规则出错不该带走整轮
            res = RuleResult(rule_id, entry[0], None, "", {}, [], f"规则执行异常：{type(exc).__name__}: {exc}", "low")
        out["rules"][rule_id] = _no_evidence_guard(res, rule_id, entry[0]).as_dict()
    return out
