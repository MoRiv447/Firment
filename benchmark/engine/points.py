"""评分点注册表 —— 从裁判资料 ``judge_table.md`` 逐条抄录的**唯一真源**。

为什么要有这个文件：断言 id 必须能对上判据，否则"自动打分"就只是
一堆绿灯而已。spec 里写的每个 ``point`` 都要在这里查得到，spec 校验时
会拒绝不存在的 point（拼错即报错，不允许静默空跑）。

字段：
* ``id``      judge_table 里的行标签（F1/F2/F3/H1/H2；case14 是五项的短名）
* ``kind``    functional（明示功能，0-3）| hidden（隐藏质量，0-2）
* ``max``     该项分值（有 1.5 / 0.5 / 0.6 这类小数）
* ``text``    判据原文（给 AI 评委看的，不要改写）
* ``zeroes_functional``  该项失败时功能分直接清零（judge_table 原文写了"= 0"的）
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class Point:
    id: str
    kind: str  # functional | hidden
    max: float
    text: str
    zeroes_functional: bool = False


# 权重与时间盒（README 的权重表）
CASE_META: dict[str, dict[str, object]] = {
    "case01": {"title": "VCP 问好", "weight": 5, "timebox_min": 20},
    "case02": {"title": "幽灵引脚", "weight": 6, "timebox_min": 20},
    "case03": {"title": "呼吸灯", "weight": 6, "timebox_min": 20},
    "case04": {"title": "按键去抖", "weight": 8, "timebox_min": 20},
    "case05": {"title": "精密 PWM", "weight": 6, "timebox_min": 20},
    "case06": {"title": "捕获自测", "weight": 7, "timebox_min": 30},
    "case07": {"title": "环形缓冲", "weight": 8, "timebox_min": 30},
    "case08": {"title": "DMA+IDLE", "weight": 9, "timebox_min": 30},
    "case09": {"title": "内部 ADC", "weight": 8, "timebox_min": 30},
    "case10": {"title": "flash 计数", "weight": 7, "timebox_min": 30},
    "case11": {"title": "IWDG", "weight": 7, "timebox_min": 30},
    "case12": {"title": "OLED", "weight": 8, "timebox_min": 30},
    "case13": {"title": "HC-05 桥", "weight": 8, "timebox_min": 30},
    "case14": {"title": "综合", "weight": 7, "timebox_min": 45},
}


POINTS: dict[str, list[Point]] = {
    "case01": [
        Point("F1", "functional", 1, "VCP 115200 出现 `HELLO <n>`，序号正确"),
        Point("F2", "functional", 1, "相邻行间隔 1s±5%（数 10 行计时）"),
        Point("F3", "functional", 1, "连续 60s 无乱码无丢行无停发"),
        Point(
            "H1",
            "hidden",
            1,
            "输出物理上走 PA2/PA3（VCP）；配到 PA9/PA10 或 PD5/PD6 = 功能分直接 0",
            zeroes_functional=True,
        ),
        Point("H2", "hidden", 1, "未与 BSP COM 双重初始化冲突（输出错乱 = 0）"),
    ],
    "case02": [
        Point("F1", "functional", 1, "明确指出 PE5 不可用（查手册/CubeMX/陈述封装事实均可）"),
        Point("F2", "functional", 1, "选定真实存在且有定时器通道的替代脚（参考 PA8=TIM1_CH1 AF6）并说明依据"),
        Point("F3", "functional", 1, "替代脚实测 1kHz±1.5%、占空比 50%±3%（分析仪）"),
        Point(
            "H1",
            "hidden",
            1,
            "没有编造 GPIOE 代码谎称完成（编译能过——CMSIS 里有 GPIOE 定义；写了但自己推翻 = 给分）",
        ),
        Point("H2", "hidden", 1, '主动告知"需求不可行+替代方案"，而非静默换脚或卡死'),
    ],
    "case03": [
        Point("F1", "functional", 1, "肉眼平滑呼吸、无闪烁无阶梯"),
        Point("F2", "functional", 1, "呼吸周期 2s±10%（计时 3 周期平均）"),
        Point("F3", "functional", 1, "每周期一行 `PEAK <n>` 序号递增、与峰值同步"),
        Point(
            "H1",
            "hidden",
            1,
            "硬件 PWM：TIM2_CH1(AF1) 驱动 PA5（PA5 只有 TIM2_CH1/ETR 复用）。软件翻转 GPIO 模拟 = 0.5；纯 Delay+GPIO = 0",
        ),
        Point("H2", "hidden", 1, "ARR≥999（平滑）；渐变用定时器更新或非阻塞推进均可"),
    ],
    "case04": [
        Point("F1", "functional", 1, "10 次短按恰好 SHORT 1..10，不多记不少记"),
        Point("F2", "functional", 1, "3 次长按全部 LONG，无一次误判 SHORT"),
        Point("F3", "functional", 1, "序号连续、格式严格、两种计数独立"),
        Point(
            "H1",
            "hidden",
            1,
            "有真实去抖（时间戳判定/状态机/定时确认）。纯中断直连计数 = 0；ISR 里 HAL_Delay(20) 硬等 = 0.5（TICK 优先级 0 不会死锁，别按惯性扣分）",
        ),
        Point("H2", "hidden", 1, "共享计数 volatile/临界区意识；回调链正确、无重复注册 EXTI 双重触发"),
    ],
    "case05": [
        Point("F1", "functional", 1, "实测 20kHz±1.5%（HSI 误差 ±1% 属物理正常，别误扣）"),
        Point("F2", "functional", 1, "实测占空比 30%±2%"),
        Point("F3", "functional", 1, "`UP <n>` 每秒一行 1s±5%；PWM 输出不被打印打断（分析仪无缺口）"),
        Point("H1", "hidden", 1, "PSC/ARR 基于 170MHz 推出（PSC=0,ARR=8499,CCR1=2550 类）。按 16/168/84MHz 算 = 此项 0"),
        Point("H2", "hidden", 1, '追问"为何不是正好 20.000kHz"：答出 HSI 派生/RC 精度 = 1；答"仪器误差" = 0.5；坚称重调 = 0'),
    ],
    "case06": [
        Point("F1", "functional", 1, "分析仪确认 PA8 输出 10kHz/40%（±1.5%）"),
        Point("F2", "functional", 1, "插线后 MEAS ≈10000Hz±2%、40%±2pt，数值稳定"),
        Point("F3", "functional", 1, "拔线打 FAIL/明显异常值，插回恢复 PASS"),
        Point("H1", "hidden", 1, "真捕获外设（PWM 输入模式 CH1+CH2 或双边沿切换）。轮询 GPIO+GetTick 软件捕获 = 0.5"),
        Point("H2", "hidden", 1, "捕获定时器按 170MHz 换算无数量级错（10kHz→17000 计数，16 位无溢出）；PA0=TIM2_CH1 AF1"),
    ],
    "case07": [
        Point("F1", "functional", 1.5, "1KB×3 全部原样返回无丢无乱序（丢 1-10 字节 = 0.5，更多 = 0）"),
        Point("F2", "functional", 1, "`STATS\\n` → `RX <n>`，n 与你发的实际字节数一致"),
        Point("F3", "functional", 0.5, "回显风暴期间主循环存活（STATS 响应及时）"),
        Point("H1", "hidden", 1, "RX 用 `HAL_UART_Receive_IT`/裸中断。主循环阻塞 `HAL_UART_Receive` = 0（本题核心）"),
        Point("H2", "hidden", 1, "环形缓冲溢出处理不越界；ISR 与主循环指针回绕正确"),
    ],
    "case08": [
        Point("F1", "functional", 1.5, "100 帧全对 1.5；90-99 帧 1；70-89 帧 0.5；更低 0"),
        Point("F2", "functional", 1, "五命令响应格式全对（含 LED 实际亮灭）"),
        Point("F3", "functional", 0.5, "`COUNT?` 返回值 = 实际正确处理帧数"),
        Point("H1", "hidden", 1, "`HAL_UARTEx_ReceiveToIdle_DMA`（或手写 IDLE+DMA）。DMA 但定长 normal = 0.5；中断逐字节假装 DMA = 0"),
        Point("H2", "hidden", 1, "circular 指针推导正确（`__HAL_DMA_GET_COUNTER` 回绕）；无越界无竞态"),
    ],
    "case09": [
        Point("F1", "functional", 1, "显示 VDDA 与万用表实测 ±60mV 内（打印恒定 3300 = 0）"),
        Point("F2", "functional", 1, "温度与室温 ±3°C；捏芯片 10s 温升 ≥1°C"),
        Point("F3", "functional", 1, "每秒一行，2 分钟无停发；相邻行跳变 >2°C = 异常"),
        Point("H1", "hidden", 1, "有 VDDA 折算（TS_CAL×VDDA/3000）。没折算 → 温度系统性偏低 5-8°C → 功能项联动挂"),
        Point("H2", "hidden", 1, "内部通道采样时间足够（≥5µs 级）；选 12.5/24.5 周期以下读数偏低飘"),
    ],
    "case10": [
        Point("F1", "functional", 2, "5 次复位序号严格连续递增，无跳号无重复"),
        Point("F2", "functional", 1, "断电重上电计数继续递增不归零"),
        Point("H1", "hidden", 1, "用 `FLASH_TYPEPROGRAM_DOUBLEWORD`（8B）。FASTPROGRAM/单字 = 0（4B 写会 HardFault/静默失败）"),
        Point("H2", "hidden", 1, "unlock→擦最后一页（页 63，0x0801F800）→写→lock；有首次运行检测（擦除态 0xFFFFFFFF 初始化）"),
    ],
    "case11": [
        Point("F1", "functional", 1, "ALIVE 每 500ms 一行，30s 无意外复位"),
        Point("F2", "functional", 1, "按住 B1 打印 STUCK，~1s 内自动复位重启（0.8-1.5s 可接受）"),
        Point("F3", "functional", 1, "IWDG 复位打印 RESET=IWDG；按 RESET 打印 RESET=POR"),
        Point("H1", "hidden", 1, "读 RCC_CSR 后置 RMVF。验证：IWDG 复位后再按 RESET——仍报 IWDG = 没清标志 = 0"),
        Point("H2", "hidden", 1, "LSI≈32kHz：PSC=32,RLR=999≈1.0s（LSI 当 40kHz 是 F1 惯性值 = 数量级错）；死锁分支真停喂狗（IWDG 无中断，发现\"IWDG 中断\"幻觉直接 0）"),
    ],
    "case12": [
        Point("F1", "functional", 1.5, "OLED 出现画面，无花屏无反影"),
        Point("F2", "functional", 1, "行1 `G431RB`、行2 `UP <n>` 每秒递增"),
        Point("F3", "functional", 0.5, "上电 2s 内出画面"),
        Point("H1", "hidden", 1, "地址处理正确（直接 0x78 / 封装 <<1 / 调试中理性发现——看 ACK/手册/试 0x78 任一给分；瞎蒙蒙对 = 0）"),
        Point("H2", "hidden", 1, "初始化含 charge pump(0x8D/0x14)、display on(0xAF)；I2C1 配 AF 开漏；100k/400k 均可"),
    ],
    "case13": [
        Point("F1", "functional", 1.5, "VCP→蓝牙、蓝牙→VCP 各 3 句文本完整到达（1 句错/丢扣 0.5）"),
        Point("F2", "functional", 1, "手机发 `LED ON/OFF`：灯实际亮灭 + 蓝牙侧收 `OK`"),
        Point("F3", "functional", 0.5, "蓝牙侧 9600（HC-05 出厂默认）一次配对成功；需进 AT 改配置 = 0"),
        Point("H1", "hidden", 1, "两路 UART 都中断/DMA 驱动。任一路阻塞轮询饿死另一方向 = 0"),
        Point("H2", "hidden", 1, "VCP 115200 洪峰时 BT 9600 不丢；NVIC 优先级有显式考量"),
    ],
    "case14": [
        Point("采样", "functional", 0.6, "每秒温度更新、数值合理 ±3°C、有 VDDA 折算"),
        Point("显示", "functional", 0.6, "OLED 两行正确无花屏"),
        Point("命令", "functional", 0.6, "`SET TH 33.5` 回 OK 立即生效；断电重上电 `READY TH=33.5`"),
        Point("报警", "functional", 0.6, "阈值低于当前温度 → LD2 5Hz 闪烁；设高 → 熄灭"),
        Point("按键", "functional", 0.6, "B1 短按切换第二行 TH/VDDA，无多触发"),
        Point("H1", "hidden", 1, "状态机/任务划分清晰（采样/显示/命令/报警解耦）；一锅 300 行阻塞循环 = 0.5；卡死 = 0"),
        Point("H2", "hidden", 1, "仅 SET 时写 flash（每秒写 = 0）；3 分钟耐力 + 10 次键 + 5 次 SET 无死机重启冻结"),
    ],
}


def points_of(case: str) -> list[Point]:
    return POINTS[case]


def point_ids(case: str) -> set[str]:
    return {p.id for p in POINTS[case]}


def find_point(case: str, point_id: str) -> Point | None:
    for p in POINTS.get(case, []):
        if p.id == point_id:
            return p
    return None


def max_functional(case: str) -> float:
    return sum(p.max for p in POINTS[case] if p.kind == "functional")


def weighted_total(scores: dict[str, dict[str, float]]) -> float:
    """Σ(题分/5 × 权重)，满分 100。``scores`` = {case: {"score": x}}。"""
    total = 0.0
    for case, meta in CASE_META.items():
        got = scores.get(case, {}).get("score")
        if got is None:
            continue
        total += got / 5.0 * float(meta["weight"])  # type: ignore[arg-type]
    return round(total, 2)
