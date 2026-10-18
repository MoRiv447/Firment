"""从 CMSIS 头文件推导出来的外设寄存器地址。

**为什么不直接背地址**：起始地址、结构体字段偏移一变，写死的那份就会静默指向
别的地方 —— 读回来一堆看起来合法的废话。这里每条都写清推导过程，并且由
`tests/test_eval.py::TestResetSemantics` 里的测试重新推一遍来钉住。

来源：`start/cubemx/Drivers/CMSIS/Device/ST/STM32G4xx/Include/stm32g431xx.h`
（以及 PlatformIO 框架包里那份同源副本）。换固件包后测试会报警。
"""

from __future__ import annotations

# ---- 总线基址（头文件里的 #define 链）--------------------------------
# PERIPH_BASE      = 0x40000000
# AHB1PERIPH_BASE  = PERIPH_BASE + 0x00020000 = 0x40020000
# AHB2PERIPH_BASE  = PERIPH_BASE + 0x08000000 = 0x48000000
PERIPH_BASE = 0x40000000
AHB1PERIPH_BASE = PERIPH_BASE + 0x00020000
AHB2PERIPH_BASE = PERIPH_BASE + 0x08000000

# RCC  = AHB1PERIPH_BASE + 0x1000
RCC_BASE = AHB1PERIPH_BASE + 0x1000
# RCC_TypeDef 里 AHB2ENR 是第 19 个字段（偏移 0x4C，由字段累加得出）
RCC_AHB2ENR = RCC_BASE + 0x4C
# RCC_AHB2ENR_GPIOAEN_Pos=0 / B=1 / **C=2**
GPIO_C_EN_BIT = 2
# RCC_TypeDef 里 APB2ENR 偏移 0x60（由字段累加得出）
RCC_APB2ENR = RCC_BASE + 0x60
# SYSCFG 挂在 APB2 上，**不打开它的时钟，SYSCFG_EXTICR 的写入会被丢掉** ——
# 表现是"配了 EXTI 却永远不触发"，看起来像夹具坏了。
SYSCFG_EN_BIT = 0

# GPIOC = AHB2PERIPH_BASE + 0x0800
GPIOA_BASE = AHB2PERIPH_BASE + 0x0000    # 用于继电器跳线测试（PA8 驱动 / PA0 读回）
GPIOB_BASE = AHB2PERIPH_BASE + 0x0400    # PB8/PB9 = OLED 的 I2C，标定分析仪时用
GPIOC_BASE = AHB2PERIPH_BASE + 0x0800
# GPIO_TypeDef：MODER 0x00 / OTYPER 0x04 / OSPEEDR 0x08 / PUPDR 0x0C / IDR 0x10
GPIOC_MODER = GPIOC_BASE + 0x00
GPIOC_OTYPER = GPIOC_BASE + 0x04
GPIOC_OSPEEDR = GPIOC_BASE + 0x08
GPIOC_PUPDR = GPIOC_BASE + 0x0C
GPIOA_MODER = GPIOA_BASE + 0x00
GPIOA_PUPDR = GPIOA_BASE + 0x0C
GPIOA_IDR = GPIOA_BASE + 0x10
GPIOA_ODR = GPIOA_BASE + 0x14

GPIOB_MODER = GPIOB_BASE + 0x00
GPIOB_PUPDR = GPIOB_BASE + 0x0C
GPIOB_IDR = GPIOB_BASE + 0x10
GPIOB_ODR = GPIOB_BASE + 0x14

GPIOC_IDR = GPIOC_BASE + 0x10
# IDR 之后是 ODR（输出数据寄存器）—— 把引脚配成输出后，翻这一位就是翻引脚电平
GPIOC_ODR = GPIOC_BASE + 0x14

# PC13：MODER/PUPDR 都占 [27:26]，IDR 占 bit13
PC13_MODER_SHIFT = 26
PC13_PUPDR_SHIFT = 26
PC13_IDR_BIT = 13

# ---- EXTI：固件（BSP）用的就是这条路，所以这是比"读电平"更贴近真相的判据 ----
# APB2PERIPH_BASE = PERIPH_BASE + 0x00010000 = 0x40010000
APB2PERIPH_BASE = PERIPH_BASE + 0x00010000
SYSCFG_BASE = APB2PERIPH_BASE + 0x0000       # SYSCFG_BASE = APB2PERIPH_BASE + 0x0000UL
EXTI_BASE = APB2PERIPH_BASE + 0x0400         # EXTI_BASE   = APB2PERIPH_BASE + 0x0400UL
# SYSCFG_TypeDef：EXTICR 是 4 个字的数组，起点偏移 0x08
#   EXTICR[0]→pin0-3  [1]→4-7  [2]→8-11  [3]→**12-15**（PC13 在这组）
SYSCFG_EXTICR1 = SYSCFG_BASE + 0x08
EXTI13_EXTICR = SYSCFG_EXTICR1 + 0x0C         # EXTICR[3]，覆盖 PC13
EXTI13_EXTICR_SHIFT = 4                       # 13 = 12 + 1 → 组内第 1 个 → [7:4]
PORT_C_CODE = 0b0010                          # SYSCFG_EXTICR 里 C 口的编码
# EXTI_TypeDef：IMR1 0x00 / RTSR1 0x08 / FTSR1 0x0C / SWIER1 0x10 / PR1 0x14
EXTI_IMR1 = EXTI_BASE + 0x00
EXTI_RTSR1 = EXTI_BASE + 0x08
EXTI_FTSR1 = EXTI_BASE + 0x0C
EXTI_PR1 = EXTI_BASE + 0x14
EXTI13_BIT = 13
# PR1 是**写 1 清标志**
EXTI_PR_CLEAR = 1 << EXTI13_BIT

# PUPDR 的两种设定（00=无上下拉，01=上拉，10=下拉）
PUPDR_NONE = 0b00
PUPDR_UP = 0b01
PUPDR_DOWN = 0b10

# B1 在 NUCLEO-G431RB 上接的是 VDD（高有效），所以 ST 的 BSP_PB_Init 用下拉 ——
# 要复现"固件眼里的电路"，就得用这个设定。
BSP_BUTTON_PUPDR = PUPDR_DOWN


def set_field(value: int, shift: int, width: int, field: int) -> int:
    mask = (1 << width) - 1
    return (value & ~(mask << shift)) | ((field & mask) << shift)


def get_bit(value: int, bit: int) -> int:
    return (value >> bit) & 1
