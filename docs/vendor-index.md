# Hardware Knowledge Base

Put a lightweight knowledge base in your firmware/embedded project and Firment will
auto-discover it and instruct the agent to consult it first:

```text
docs/
├── vendor-index.toml      # machine-readable index (agent reads it with read_file / grep)
├── cheatsheets/           # original high-frequency cheatsheets (TOML, optional)
└── vendor-index.md        # human-readable guide (this file)
```

## vendor-index.toml

One entry per chip family, pointing to official documentation and telling the agent
"where to look":

```toml
[stm32.f4]
family = "STM32F4xx"
reference_manual = { name = "RM0090", url = "https://www.st.com/...", pages = 1120 }
hal_repo = "https://github.com/STMicroelectronics/STM32CubeF4"
hal_header_path = "Drivers/STM32F4xx_HAL_Driver/Inc/stm32f4xx_hal_dma.h"

[[stm32.f4.quickref]]
topic = "DMA configuration"
doc_section = "RM0090 Section 10.3"
hal_functions = ["HAL_DMA_Init", "HAL_DMA_Start_IT"]
key_registers = ["DMA_SxCR", "DMA_SxNDTR", "DMA_SxPAR"]
notes = "DMA1 can only reach AHB1 peripherals; DMA2 is required for AHB2 (including USB)"
```

See [vendor-index.toml](vendor-index.toml) for the full template.

## How the agent uses it (lookup order)

The knowledge base is not loaded into context; use three on-demand steps instead of
reading the whole `docs/` at once:

1. `read_file("docs/vendor-index.toml")` — get the map and locate the `family / topic`;
2. `read_file("docs/cheatsheets/<file>.toml")` — read the original high-frequency
   cheatsheet (registers, common mistakes, config patterns);
3. only for specific bitfields or deeper detail, fetch the official manual via
   `reference_manual.url` for RAG.

## cheatsheets/

Each cheatsheet is one TOML file containing only **original summaries** (registers,
common mistakes, config patterns). **Do not copy vendor documentation verbatim**
(copyright).

## Rules

- The knowledge base belongs to the project workspace, not the Firment repo (except
  templates and examples);
- Only include facts you can verify; chip-specific behavior follows the official manual,
  and cheatsheets should note `doc_section` for cross-checking;
- If you have a local STM32Cube package, config.toml can add extra paths so the agent can
  read HAL headers directly (supported in a later release).
