use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

pub struct PeriphInit;

/// Serializes seed-KB materialization so concurrent callers (e.g. parallel
/// tests, parallel tool waves) never read a half-written cheatsheet.
static KB_SEED_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

#[async_trait]
impl Tool for PeriphInit {
    fn name(&self) -> &'static str {
        "periph_init"
    }

    fn description(&self) -> &'static str {
        "Generate an MCU peripheral initialization skeleton (STM32 HAL style) plus the matching \
         knowledge-base cheatsheet, so you don't start from scratch on chip/peripheral config. \
         Inputs: part (e.g. stm32f103c8t6), peripheral (uart/gpio/i2c/spi/tim/adc get full \
         skeletons; dma falls back to a generic skeleton + cheatsheet), optional baudrate / pins \
         / dma / interrupt. Output is a compile-ready skeleton with TODO(fill) markers and key \
         config notes (clock domain, DMA channel mapping, common pitfalls) — adapt it to the \
         actual project (pins, clock tree, CubeMX) before use."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "part": {"type": "string", "description": "MCU part number, e.g. stm32f103c8t6 or esp32s3"},
                "peripheral": {"type": "string", "enum": ["uart", "gpio", "i2c", "spi", "tim", "adc", "dma"], "description": "Peripheral to initialize"},
                "baudrate": {"type": "integer", "description": "UART baud rate (default 115200)"},
                "pins": {"type": "string", "description": "Pin names, e.g. PA9/PA10"},
                "dma": {"type": "boolean", "default": false, "description": "Enable DMA on the peripheral (uart only for now)"},
                "interrupt": {"type": "boolean", "default": false, "description": "Enable the peripheral interrupt (uart only for now)"}
            },
            "required": ["part", "peripheral"]
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let part = args
            .get("part")
            .and_then(|p| p.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing 'part'"))?;
        let peripheral = args
            .get("peripheral")
            .and_then(|p| p.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing 'peripheral'"))?
            .to_lowercase();
        let valid = ["uart", "gpio", "i2c", "spi", "tim", "adc", "dma"];
        if !valid.contains(&peripheral.as_str()) {
            return Err(ToolError::new(format!(
                "[InvalidInput] unsupported peripheral '{peripheral}' (use one of: {})",
                valid.join(", ")
            )));
        }
        let baudrate = args
            .get("baudrate")
            .and_then(|b| b.as_u64())
            .unwrap_or(115_200);
        let pins = args
            .get("pins")
            .and_then(|p| p.as_str())
            .map(str::to_string);
        let dma = args.get("dma").and_then(|d| d.as_bool()).unwrap_or(false);
        let interrupt = args
            .get("interrupt")
            .and_then(|i| i.as_bool())
            .unwrap_or(false);

        // Materialize the bundled seed KB once (idempotent), serialized so
        // parallel callers never race the materialization; then locate a
        // matching cheatsheet: {family}-{peripheral}.toml under cheatsheets/.
        let _guard = KB_SEED_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = firment_core::kb::ensure_seed_kb();
        drop(_guard);
        let kb_dir = firment_core::kb::seed_kb_dir();
        let family = family_for(part);
        // Family-specific cheatsheet first ({family}-{periph}.toml), then a
        // family-generic one (stm32-{periph}.toml, e.g. stm32-dma / stm32-clock).
        let cheatsheet = family
            .and_then(|f| find_cheatsheet(&kb_dir, f, &peripheral))
            .or_else(|| find_cheatsheet(&kb_dir, "stm32", &peripheral));

        // The full HAL skeletons are STM32-specific. Non-STM32 parts (e.g.
        // ESP32, which uses its own SDK) must not get STM32 HAL code — emit a
        // generic skeleton + the cheatsheet instead.
        let is_stm32 = part.to_lowercase().starts_with("stm32");
        let skeleton = if is_stm32 {
            match peripheral.as_str() {
                "uart" => uart_skeleton(baudrate, dma, interrupt, pins.as_deref()),
                "gpio" => gpio_skeleton(pins.as_deref()),
                "i2c" => i2c_skeleton(),
                "spi" => spi_skeleton(),
                "tim" => tim_skeleton(),
                "adc" => adc_skeleton(),
                other => generic_skeleton(other, family),
            }
        } else {
            generic_skeleton(&peripheral, family)
        };

        let mut text = format!(
            "# periph_init: {part} {peripheral}\n## 初始化骨架（STM32 HAL 风格）\n```c\n{skeleton}\n```"
        );
        if let Some((path, content)) = &cheatsheet {
            text.push_str(&format!(
                "\n## 参考 cheatsheet ({})\n```toml\n{content}\n```",
                path.display()
            ));
        } else {
            text.push_str(&format!(
                "\n## 提示：没有 {} 的 cheatsheet，请对照官方参考手册确认时钟域、复用功能和 DMA 通道后再填写 TODO(fill)。",
                family.unwrap_or(part)
            ));
        }
        text.push_str(
            "\n## 用法\n\
             - 先检查工程是否已有 CubeMX/厂商工具生成的初始化（在 main.c / *_hal_msp.c 里 grep \
             MX_*_Init、HAL_*_Init、SystemClock_Config）：如果已有，直接调用现有函数，\
             **不要重复初始化、重新定义同名函数或句柄变量**；仅当工程是纯手写（无生成代码）时才落盘此骨架。\n\
             - 骨架里的 TODO(fill) 需要按项目实际（引脚、时钟树）填写；函数名/句柄名若与已有代码冲突请改名（如加后缀）。\n\
             - 生成后交给 edit_file / write_file 落盘，并结合 build 验证编译。",
        );
        if let Some(hal_note) = project_hal_note(&ctx.cwd) {
            text.push_str(&format!("\n## 工程注意\n{hal_note}"));
        }
        Ok(ToolOutput { text })
    }
}

/// Map a part number to a seed-KB family prefix, e.g. stm32f103c8t6 -> stm32f1.
fn family_for(part: &str) -> Option<&'static str> {
    let p = part.to_lowercase();
    if p.starts_with("stm32f1") {
        Some("stm32f1")
    } else if p.starts_with("stm32f4") {
        Some("stm32f4")
    } else if p.starts_with("stm32g0") {
        Some("stm32g0")
    } else if p.starts_with("esp32s3") {
        Some("esp32s3")
    } else if p.starts_with("esp32") {
        Some("esp32")
    } else {
        None
    }
}

/// Locate a cheatsheet under `<kb>/cheatsheets/<name>-<periph>.toml` and read it.
fn find_cheatsheet(kb_dir: &Path, family: &str, periph: &str) -> Option<(PathBuf, String)> {
    let path = kb_dir
        .join("cheatsheets")
        .join(format!("{family}-{periph}.toml"));
    if path.is_file() {
        fs::read_to_string(&path).ok().map(|text| (path, text))
    } else {
        None
    }
}

/// Detect the project's build framework and warn about the classic HAL
/// duplication trap: PlatformIO ships its own STM32Cube HAL, and copying a
/// CubeMX-generated `Drivers/` into such a project redefines every HAL symbol
/// (the exact conflict that makes people abandon PlatformIO).
fn project_hal_note(cwd: &Path) -> Option<String> {
    let has_ioc = find_file_up(cwd, ".ioc");
    let has_pio = find_file_up(cwd, "platformio.ini");
    match (has_ioc, has_pio) {
        (true, _) => Some(
            "检测到 CubeMX 工程（.ioc）：HAL 驱动由 CubeMX 生成于 Drivers/。使用本骨架时\
             复用现有 MX_*_Init / HAL_*_MspInit，不要引入第二份 HAL 驱动。"
                .to_string(),
        ),
        (_, true) => Some(
            "检测到 PlatformIO 工程（platformio.ini）：PlatformIO 自带 STM32Cube HAL\
             （framework=stm32cube）。切勿把 CubeMX 生成的 Drivers/ 复制进工程——两套 HAL\
             驱动会重复定义所有 HAL 符号；直接在 platformio.ini 里配置外设或调用自带 HAL。"
                .to_string(),
        ),
        _ => None,
    }
}

fn find_file_up(cwd: &Path, name: &str) -> bool {
    cwd.ancestors().any(|dir| {
        let Ok(read) = fs::read_dir(dir) else {
            return false;
        };
        read.flatten().any(|entry| {
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            file_name == name || file_name.ends_with(name)
        })
    })
}

fn uart_skeleton(baudrate: u64, dma: bool, interrupt: bool, pins: Option<&str>) -> String {
    let pin_note = pins.unwrap_or("TODO(fill): e.g. USART1 TX=PA9 RX=PA10");
    let dma_block = if dma {
        "    // DMA (TODO(fill): verify the channel mapping from the cheatsheet)\n\
         \x20   __HAL_RCC_DMA1_CLK_ENABLE();\n\
         \x20   // e.g. USART1_TX = DMA1_Channel4, USART1_RX = DMA1_Channel5\n\
         \x20   // HAL_UART_Transmit_DMA / HAL_UART_Receive_DMA + idle-line handling for variable length"
    } else {
        "    // DMA 未启用"
    };
    let irq_block = if interrupt {
        "    HAL_NVIC_SetPriority(USART1_IRQn, 0, 0);\n    HAL_NVIC_EnableIRQ(USART1_IRQn);"
    } else {
        "    // 中断未启用"
    };
    format!(
        "// {pin_note}\n\
         // NOTE: 若工程已有 CubeMX 生成的初始化（MX_*_Init 在 main.c，HAL_UART_MspInit 在 \
         *_hal_msp.c），直接调用现有函数；句柄 huart1 也已在 main.c 定义，切勿重复定义。\
         仅纯手写工程才使用本函数。\n\
         static UART_HandleTypeDef huart1;\n\
         \n\
         void MX_USART1_UART_Init(void) {{\n\
         \x20 huart1.Instance = USART1;\n\
         \x20 huart1.Init.BaudRate = {baudrate};\n\
         \x20 huart1.Init.WordLength = UART_WORDLENGTH_8B;\n\
         \x20 huart1.Init.StopBits = UART_STOPBITS_1;\n\
         \x20 huart1.Init.Parity = UART_PARITY_NONE;\n\
         \x20 huart1.Init.Mode = UART_MODE_TX_RX;\n\
         \x20 huart1.Init.HwFlowCtl = UART_HWCONTROL_NONE;\n\
         \x20 huart1.Init.OverSampling = UART_OVERSAMPLING_16;\n\
         \x20 if (HAL_UART_Init(&huart1) != HAL_OK) {{ Error_Handler(); /* TODO(fill) */ }}\n\
         \x20 // TODO(fill): GPIO 复用 (HAL_GPIO_Init + GPIO_AF) 与时钟树\n\
         {dma_block}\n\
         {irq_block}\n\
         }}"
    )
}

fn gpio_skeleton(pins: Option<&str>) -> String {
    let pin_note = pins
        .map(str::to_string)
        .unwrap_or_else(|| "TODO(fill): e.g. PA5 (LED) or PB0 (button)".to_string());
    format!(
        "// {pin_note}\n\
         static GPIO_InitTypeDef GPIO_InitStruct = {{0}};\n\
         \n\
         void MX_GPIO_Init(void) {{\n\
         \x20 __HAL_RCC_GPIOA_CLK_ENABLE(); /* TODO(fill): 按端口使能 */\n\
         \x20 GPIO_InitStruct.Pin = GPIO_PIN_0; /* TODO(fill): 引脚掩码 */\n\
         \x20 GPIO_InitStruct.Mode = GPIO_MODE_OUTPUT_PP; /* OUTPUT_PP / INPUT / ANALOG / AF_PP ... */\n\
         \x20 GPIO_InitStruct.Pull = GPIO_NOPULL;\n\
         \x20 GPIO_InitStruct.Speed = GPIO_SPEED_FREQ_LOW;\n\
         \x20 HAL_GPIO_Init(GPIOA, &GPIO_InitStruct); /* TODO(fill): 端口 */\n\
         }}"
    )
}

fn i2c_skeleton() -> String {
    "static I2C_HandleTypeDef hi2c1;\n\
     \n\
     void MX_I2C1_Init(void) {\n\
     \x20 hi2c1.Instance = I2C1;\n\
     \x20 hi2c1.Init.ClockSpeed = 100000; /* TODO(fill): 100k 或 400k */\n\
     \x20 hi2c1.Init.DutyCycle = I2C_DUTYCYCLE_2;\n\
     \x20 hi2c1.Init.OwnAddress1 = 0;\n\
     \x20 hi2c1.Init.AddressingMode = I2C_ADDRESSINGMODE_7BIT;\n\
     \x20 hi2c1.Init.DualAddressMode = I2C_DUALADDRESS_DISABLE;\n\
     \x20 hi2c1.Init.OwnAddress2 = 0;\n\
     \x20 hi2c1.Init.GeneralCallMode = I2C_GENERALCALL_DISABLE;\n\
     \x20 hi2c1.Init.NoStretchMode = I2C_NOSTRETCH_DISABLE;\n\
     \x20 if (HAL_I2C_Init(&hi2c1) != HAL_OK) { Error_Handler(); /* TODO(fill) */ }\n\
     \x20 // TODO(fill): GPIO 复用 (SCL/SDA) 与时钟使能\n\
     }"
    .to_string()
}

fn spi_skeleton() -> String {
    "static SPI_HandleTypeDef hspi1;\n\
     \n\
     void MX_SPI1_Init(void) {\n\
     \x20 hspi1.Instance = SPI1;\n\
     \x20 hspi1.Init.Mode = SPI_MODE_MASTER; /* MASTER / SLAVE */\n\
     \x20 hspi1.Init.Direction = SPI_DIRECTION_2LINES;\n\
     \x20 hspi1.Init.DataSize = SPI_DATASIZE_8BIT;\n\
     \x20 hspi1.Init.CLKPolarity = SPI_POLARITY_LOW;\n\
     \x20 hspi1.Init.CLKPhase = SPI_PHASE_1EDGE;\n\
     \x20 hspi1.Init.NSS = SPI_NSS_SOFT;\n\
     \x20 hspi1.Init.BaudRatePrescaler = SPI_BAUDRATEPRESCALER_16; /* TODO(fill): 按时钟算 */\n\
     \x20 hspi1.Init.FirstBit = SPI_FIRSTBIT_MSB;\n\
     \x20 hspi1.Init.TIMode = SPI_TIMODE_DISABLE;\n\
     \x20 hspi1.Init.CRCCalculation = SPI_CRCCALCULATION_DISABLE;\n\
     \x20 if (HAL_SPI_Init(&hspi1) != HAL_OK) { Error_Handler(); /* TODO(fill) */ }\n\
     \x20 // TODO(fill): GPIO 复用 (SCK/MISO/MOSI + AF) 与时钟使能\n\
     }"
    .to_string()
}

fn tim_skeleton() -> String {
    "static TIM_HandleTypeDef htim2;\n\
     \n\
     void MX_TIM2_Init(void) {\n\
     \x20 TIM_ClockConfigTypeDef sClockSourceConfig = {0};\n\
     \x20 TIM_MasterConfigTypeDef sMasterConfig = {0};\n\
     \x20 htim2.Instance = TIM2;\n\
     \x20 htim2.Init.Prescaler = 71; /* TODO(fill): 预分频，决定时基 */\n\
     \x20 htim2.Init.CounterMode = TIM_COUNTERMODE_UP;\n\
     \x20 htim2.Init.Period = 999; /* TODO(fill): 自动重载 */\n\
     \x20 htim2.Init.ClockDivision = TIM_CLOCKDIVISION_DIV1;\n\
     \x20 htim2.Init.AutoReloadPreload = TIM_AUTORELOAD_PRELOAD_DISABLE;\n\
     \x20 if (HAL_TIM_Base_Init(&htim2) != HAL_OK) { Error_Handler(); /* TODO(fill) */ }\n\
     \x20 sClockSourceConfig.ClockSource = TIM_CLOCKSOURCE_INTERNAL;\n\
     \x20 if (HAL_TIM_ConfigClockSource(&htim2, &sClockSourceConfig) != HAL_OK) { Error_Handler(); }\n\
     \x20 sMasterConfig.MasterOutputTrigger = TIM_TRGO_RESET;\n\
     \x20 sMasterConfig.MasterSlaveMode = TIM_MASTERSLAVEMODE_DISABLE;\n\
     \x20 if (HAL_TIMEx_MasterConfigSynchronization(&htim2, &sMasterConfig) != HAL_OK) { Error_Handler(); }\n\
     \x20 // TODO(fill): PWM 输出用 HAL_TIM_PWM_Start；输入捕获/编码器模式参考 cheatsheet\n\
     }"
        .to_string()
}

fn adc_skeleton() -> String {
    "static ADC_HandleTypeDef hadc1;\n\
     \n\
     void MX_ADC1_Init(void) {\n\
     \x20 ADC_ChannelConfTypeDef sConfig = {0};\n\
     \x20 hadc1.Instance = ADC1;\n\
     \x20 hadc1.Init.ScanConvMode = DISABLE; /* ENABLE 多通道扫描 */\n\
     \x20 hadc1.Init.ContinuousConvMode = DISABLE; /* ENABLE 连续转换 */\n\
     \x20 hadc1.Init.DiscontinuousConvMode = DISABLE;\n\
     \x20 hadc1.Init.ExternalTrigConv = ADC_SOFTWARE_START;\n\
     \x20 hadc1.Init.DataAlign = ADC_DATAALIGN_RIGHT;\n\
     \x20 hadc1.Init.NbrOfConversion = 1; /* TODO(fill): 转换通道数 */\n\
     \x20 if (HAL_ADC_Init(&hadc1) != HAL_OK) { Error_Handler(); /* TODO(fill) */ }\n\
     \x20 /* TODO(fill): 若用了 DMA/定时器触发，在此配置 __HAL_RCC_*_CLK_ENABLE 与 DMA */\n\
     \x20 sConfig.Channel = ADC_CHANNEL_0; /* TODO(fill): 引脚对应通道 */\n\
     \x20 sConfig.Rank = 1;\n\
     \x20 sConfig.SamplingTime = ADC_SAMPLETIME_55CYCLES_5; /* TODO(fill): 按源阻抗 */\n\
     \x20 if (HAL_ADC_ConfigChannel(&hadc1, &sConfig) != HAL_OK) { Error_Handler(); }\n\
     }"
    .to_string()
}

fn generic_skeleton(peripheral: &str, family: Option<&'static str>) -> String {
    format!(
        "// {peripheral} 初始化骨架（无内置模板；请结合 cheatsheet/参考手册填写）\n\
         // 1. 时钟使能: __HAL_RCC_*_CLK_ENABLE()\n\
         // 2. GPIO 复用（引脚与 AF 映射，{family_unset}）\n\
         // 3. 外设句柄 + HAL_*_Init + 返回值检查\n\
         // 4. 中断 / DMA 使能\n\
         // TODO(fill): 全部参数按项目实际填写",
        family_unset = match family {
            Some(f) => format!("参考 cheatsheets/{f}-{peripheral}.toml"),
            None => "无 cheatsheet，对照官方参考手册".to_string(),
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    fn ctx(dir: &Path) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(EditJournal::new(dir.join("undo")))),
            verify_command: None,
            symbols_backend: None,
            build_command: None,
            default_chip: None,
            monitor_port: None,
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        }
    }

    #[tokio::test]
    async fn uart_skeleton_includes_hal_and_dma_notes() {
        let dir = tempdir().unwrap();
        let out = PeriphInit
            .run(
                json!({"part": "stm32f103c8t6", "peripheral": "uart", "baudrate": 115200, "dma": true}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(out.text.contains("HAL_UART_Init"), "got: {}", out.text);
        assert!(out.text.contains("USART1"), "got: {}", out.text);
        assert!(out.text.contains("115200"), "got: {}", out.text);
        assert!(out.text.contains("TODO(fill)"), "got: {}", out.text);
        // Must warn against re-initializing when the project already has
        // generated MX_*_Init code (CubeMX-style projects).
        assert!(
            out.text.contains("不要重复初始化") || out.text.contains("CubeMX"),
            "must warn about existing generated init code, got: {}",
            out.text
        );
        // Seed KB cheatsheet must be injected for stm32f1 uart.
        assert!(
            out.text.contains("stm32f1-uart.toml") || out.text.contains("cheatsheets"),
            "got: {}",
            out.text
        );
    }

    #[tokio::test]
    async fn unknown_family_falls_back_with_hint() {
        let dir = tempdir().unwrap();
        let out = PeriphInit
            .run(
                json!({"part": "stm32h750", "peripheral": "uart"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(out.text.contains("HAL_UART_Init"), "got: {}", out.text);
        assert!(
            out.text.contains("没有") || out.text.contains("参考"),
            "must hint about missing cheatsheet: {}",
            out.text
        );
    }

    #[tokio::test]
    async fn generic_family_cheatsheet_is_used_when_no_family_specific_one() {
        let dir = tempdir().unwrap();
        // stm32f4 has no stm32f4-dma.toml; the family-generic stm32-dma.toml
        // must be injected instead.
        let out = PeriphInit
            .run(
                json!({"part": "stm32f407vgt6", "peripheral": "dma"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(
            out.text.contains("stm32-dma.toml"),
            "generic cheatsheet must be injected, got: {}",
            out.text
        );
    }

    #[tokio::test]
    async fn unsupported_peripheral_is_rejected() {
        let dir = tempdir().unwrap();
        let err = PeriphInit
            .run(
                json!({"part": "stm32f103c8t6", "peripheral": "can"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("[InvalidInput]"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn family_mapping() {
        assert_eq!(family_for("stm32f103c8t6"), Some("stm32f1"));
        assert_eq!(family_for("STM32F407VGT6"), Some("stm32f4"));
        assert_eq!(family_for("stm32g070"), Some("stm32g0"));
        assert_eq!(family_for("esp32s3"), Some("esp32s3"));
        assert_eq!(family_for("esp32"), Some("esp32"));
        assert_eq!(family_for("nrf52840"), None);
    }

    #[tokio::test]
    async fn spi_tim_adc_skeletons_are_generated() {
        let dir = tempdir().unwrap();
        let out = PeriphInit
            .run(
                json!({"part": "stm32f103c8t6", "peripheral": "spi"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(out.text.contains("HAL_SPI_Init"), "got: {}", out.text);
        assert!(out.text.contains("TODO(fill)"), "got: {}", out.text);

        let out = PeriphInit
            .run(
                json!({"part": "stm32f103c8t6", "peripheral": "tim"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(out.text.contains("HAL_TIM_Base_Init"), "got: {}", out.text);
        assert!(out.text.contains("TODO(fill)"), "got: {}", out.text);

        let out = PeriphInit
            .run(
                json!({"part": "stm32f103c8t6", "peripheral": "adc"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(out.text.contains("HAL_ADC_Init"), "got: {}", out.text);
        assert!(out.text.contains("TODO(fill)"), "got: {}", out.text);
    }

    #[test]
    fn project_framework_detection_warns_about_hal_duplication() {
        let dir = tempdir().unwrap();
        // CubeMX project: .ioc present.
        std::fs::write(dir.path().join("fw.ioc"), "Mcu.Family=STM32F1\n").unwrap();
        let note = project_hal_note(dir.path()).unwrap();
        assert!(note.contains("CubeMX"), "got: {note}");

        // PlatformIO project: platformio.ini present, and the note must warn
        // about the classic two-HAL conflict.
        let pio = tempdir().unwrap();
        std::fs::write(
            pio.path().join("platformio.ini"),
            "[env:nucleo_f103rb]\nplatform = ststm32\nframework = stm32cube\n",
        )
        .unwrap();
        let note = project_hal_note(pio.path()).unwrap();
        assert!(note.contains("PlatformIO"), "got: {note}");
        assert!(
            note.contains("重复定义") || note.contains("两套 HAL"),
            "must warn about HAL duplication, got: {note}"
        );
    }
}
