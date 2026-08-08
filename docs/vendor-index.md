# 硬件知识库（Hardware Knowledge Base）

在固件/嵌入式项目里建议放一个轻量知识库，并通过提示词让 agent 优先查询（自动发现为规划中能力，当前请按下方检索顺序手动查询）：

```text
docs/
├── vendor-index.toml      # 机器可读索引（agent 直接 read_file / grep）
├── cheatsheets/            # 原创高频速查表（TOML，可选）
└── vendor-index.md         # 给人看的说明（本文件）
```

## vendor-index.toml

每个芯片系列一个条目，给出官方资料定位 + 「该查哪里」提示：

```toml
[stm32.f4]
family = "STM32F4xx"
reference_manual = { name = "RM0090", url = "https://www.st.com/...", pages = 1120 }
hal_repo = "https://github.com/STMicroelectronics/STM32CubeF4"
hal_header_path = "Drivers/STM32F4xx_HAL_Driver/Inc/stm32f4xx_hal_dma.h"

[[stm32.f4.quickref]]
topic = "DMA 配置"
doc_section = "RM0090 Section 10.3"
hal_functions = ["HAL_DMA_Init", "HAL_DMA_Start_IT"]
key_registers = ["DMA_SxCR", "DMA_SxNDTR", "DMA_SxPAR"]
notes = "DMA1 只能访问 AHB1 外设，DMA2 才能访问 AHB2（包括 USB）"
```

完整模板见 [vendor-index.toml](vendor-index.toml)。

## Agent 怎么用（检索顺序）

知识库不入 context，按需三步走，避免把整个 `docs/` 一把读空：

1. `read_file("docs/vendor-index.toml")` —— 拿到地图，定位到 `family / topic`；
2. `read_file("docs/cheatsheets/<file>.toml")` —— 取原创高频速查（寄存器、常见坑、配置套路）；
3. 仅当要查具体位域或更深层细节，再按 `reference_manual.url` 拉官方手册做 RAG。

## cheatsheets/

每张速查表一个 TOML 文件，只写**原创总结**（寄存器、常见坑、配置套路），**不要搬运厂商文档原文**（版权）。

## 规则

- 知识库属于项目工作区，不入 Firment 主仓库（除模板与示例）；
- 只放你能确认的事实；涉及具体芯片行为以官方手册为准，速查表里可注明 `doc_section` 便于核对；
- 本地有 STM32Cube 包时，可在 config.toml 配置额外路径让 agent 直接读 HAL 头文件（后续版本支持）。