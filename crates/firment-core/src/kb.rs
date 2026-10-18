use crate::config::config_dir;
use std::fs;
use std::path::Path;

/// Bump when the bundled seed knowledge base changes (forces re-materialization).
///
/// 8：把知识库里**替读者把考题答完**的那几句改写成方法。`vendor-index.toml` 的 G4
/// pinout 条目曾直接列出 "NO Port E … (phantom-pin trap); NUCLEO: LED=PA5, button=PC13,
/// ST-Link VCP=LPUART1 PA2/PA3"，`stm32g4-tim.toml` 里还留着 "(case01 lesson: …)" 这种
/// 带测试用例编号的句子 —— 那是把某一道题的答案写进了每个用户都会读到的产品资料。
/// 引脚表、AF 表、"先核封装再写代码"这类通用知识全部保留。
/// 9：8 里漏改的一条 —— `stm32g4-gpio.toml` 的 common_mistake 仍写着 "Picking PE* pins on a
/// G431RB (LQFP64) — no Port E on that package"。它没有 case 编号，所以当时那条按 case 编号
/// 扫的合同测试抓不到；改写成"照别块板子的例程写引脚名、先对封装"的通用教训。判据从"有没有
/// 提 case 号"收紧成"坑列表里不许出现被测板的名字"，见 `prompt_contract.rs`。
/// 10：新增 `cheatsheets/gpio-layouts.toml`（GPIO 字段解码的机器可读布局，
/// 见 `firment-tools/src/tools/gpio_decode.rs`）。版本号必须跟着 bump，
/// 否则已有用户的 KB 副本里没有这张表，`debug mem` 会退化成只回原始字。
pub const SEED_VERSION: &str = "10";

const SEED_FILES: &[(&str, &str)] = &[
    (
        "vendor-index.toml",
        include_str!("../../../docs/vendor-index.toml"),
    ),
    (
        "cheatsheets/esp32-gpio.toml",
        include_str!("../../../docs/cheatsheets/esp32-gpio.toml"),
    ),
    (
        "cheatsheets/esp32-uart.toml",
        include_str!("../../../docs/cheatsheets/esp32-uart.toml"),
    ),
    (
        "cheatsheets/esp32-adc.toml",
        include_str!("../../../docs/cheatsheets/esp32-adc.toml"),
    ),
    (
        "cheatsheets/esp32-spi.toml",
        include_str!("../../../docs/cheatsheets/esp32-spi.toml"),
    ),
    (
        "cheatsheets/esp32c3-gpio.toml",
        include_str!("../../../docs/cheatsheets/esp32c3-gpio.toml"),
    ),
    (
        "cheatsheets/esp32c3-uart.toml",
        include_str!("../../../docs/cheatsheets/esp32c3-uart.toml"),
    ),
    (
        "cheatsheets/esp32c6-gpio.toml",
        include_str!("../../../docs/cheatsheets/esp32c6-gpio.toml"),
    ),
    (
        "cheatsheets/esp32c6-uart.toml",
        include_str!("../../../docs/cheatsheets/esp32c6-uart.toml"),
    ),
    (
        "cheatsheets/esp32s31-gpio.toml",
        include_str!("../../../docs/cheatsheets/esp32s31-gpio.toml"),
    ),
    (
        "cheatsheets/esp32s31-uart.toml",
        include_str!("../../../docs/cheatsheets/esp32s31-uart.toml"),
    ),
    (
        "cheatsheets/esp32s3-gpio.toml",
        include_str!("../../../docs/cheatsheets/esp32s3-gpio.toml"),
    ),
    (
        "cheatsheets/rp2040-gpio.toml",
        include_str!("../../../docs/cheatsheets/rp2040-gpio.toml"),
    ),
    (
        "cheatsheets/rp2040-uart.toml",
        include_str!("../../../docs/cheatsheets/rp2040-uart.toml"),
    ),
    (
        "cheatsheets/esp32s3-uart.toml",
        include_str!("../../../docs/cheatsheets/esp32s3-uart.toml"),
    ),
    (
        "cheatsheets/stm32-clock.toml",
        include_str!("../../../docs/cheatsheets/stm32-clock.toml"),
    ),
    (
        "cheatsheets/stm32-dma.toml",
        include_str!("../../../docs/cheatsheets/stm32-dma.toml"),
    ),
    (
        "cheatsheets/stm32f1-clock.toml",
        include_str!("../../../docs/cheatsheets/stm32f1-clock.toml"),
    ),
    (
        "cheatsheets/stm32f1-tim.toml",
        include_str!("../../../docs/cheatsheets/stm32f1-tim.toml"),
    ),
    (
        "cheatsheets/stm32f1-uart.toml",
        include_str!("../../../docs/cheatsheets/stm32f1-uart.toml"),
    ),
    (
        "cheatsheets/stm32f4-gpio.toml",
        include_str!("../../../docs/cheatsheets/stm32f4-gpio.toml"),
    ),
    (
        "cheatsheets/stm32f4-tim.toml",
        include_str!("../../../docs/cheatsheets/stm32f4-tim.toml"),
    ),
    (
        "cheatsheets/stm32g0-gpio.toml",
        include_str!("../../../docs/cheatsheets/stm32g0-gpio.toml"),
    ),
    (
        "cheatsheets/stm32g0-uart.toml",
        include_str!("../../../docs/cheatsheets/stm32g0-uart.toml"),
    ),
    (
        "cheatsheets/stm32g4-uart.toml",
        include_str!("../../../docs/cheatsheets/stm32g4-uart.toml"),
    ),
    (
        "cheatsheets/stm32g4-pinout.toml",
        include_str!("../../../docs/cheatsheets/stm32g4-pinout.toml"),
    ),
    (
        "cheatsheets/stm32g4-tim.toml",
        include_str!("../../../docs/cheatsheets/stm32g4-tim.toml"),
    ),
    (
        "cheatsheets/gpio-layouts.toml",
        include_str!("../../../docs/cheatsheets/gpio-layouts.toml"),
    ),
    (
        "cheatsheets/stm32g4-gpio.toml",
        include_str!("../../../docs/cheatsheets/stm32g4-gpio.toml"),
    ),
    (
        "cheatsheets/stm32g4-adc.toml",
        include_str!("../../../docs/cheatsheets/stm32g4-adc.toml"),
    ),
    (
        "cheatsheets/stm32g4-flash.toml",
        include_str!("../../../docs/cheatsheets/stm32g4-flash.toml"),
    ),
    (
        "cheatsheets/stm32g4-iwdg.toml",
        include_str!("../../../docs/cheatsheets/stm32g4-iwdg.toml"),
    ),
    (
        "cheatsheets/stm32h7-uart.toml",
        include_str!("../../../docs/cheatsheets/stm32h7-uart.toml"),
    ),
];

/// Config-dir location of the materialized seed knowledge base.
pub fn seed_kb_dir() -> std::path::PathBuf {
    config_dir().join("kb")
}

/// Materialize the bundled seed KB into `dir` unless the version stamp matches.
pub fn ensure_seed_kb_in(dir: &Path) -> Result<(), String> {
    let stamp = dir.join("VERSION");
    if let Ok(text) = fs::read_to_string(&stamp)
        && text.trim() == SEED_VERSION
    {
        return Ok(());
    }
    for (rel, content) in SEED_FILES {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // Atomic write (tmp + rename) so a concurrent reader never sees a
        // half-written file while the seed KB is being materialized.
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, content).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    }
    let stamp_tmp = dir.join("VERSION.tmp");
    fs::write(&stamp_tmp, SEED_VERSION).map_err(|e| e.to_string())?;
    fs::rename(&stamp_tmp, &stamp).map_err(|e| e.to_string())?;
    Ok(())
}

/// Materialize into the default config-dir location.
pub fn ensure_seed_kb() -> Result<(), String> {
    ensure_seed_kb_in(&seed_kb_dir())
}

/// Bundled seed index text for system-prompt injection.
pub fn seed_index_text() -> &'static str {
    SEED_FILES[0].1
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn seed_kb_materializes_and_parses() {
        let dir = tempdir().unwrap();
        ensure_seed_kb_in(dir.path()).unwrap();
        let index = dir.path().join("vendor-index.toml");
        assert!(index.is_file(), "missing index");
        let text = fs::read_to_string(&index).unwrap();
        let value: toml::Value = toml::from_str(&text).unwrap();
        let stm32 = value.get("stm32").expect("stm32 table");
        assert!(stm32.get("f4").is_some());
        assert!(stm32.get("f1").is_some());
        assert!(stm32.get("g0").is_some());
        let esp32 = value.get("esp32").expect("esp32 table");
        assert!(esp32.get("s31").is_some());
        assert!(esp32.get("s3").is_some());
        assert!(esp32.get("c6").is_some());
        assert!(esp32.get("c3").is_some());
        let rp2040 = value.get("rp2040").expect("rp2040 table");
        assert!(rp2040.get("family").is_some());
        assert!(
            dir.path().join("cheatsheets/stm32f1-uart.toml").is_file(),
            "cheatsheet missing"
        );
        assert!(
            dir.path().join("cheatsheets/stm32g4-pinout.toml").is_file(),
            "g4 pinout cheatsheet missing"
        );
        assert!(
            dir.path().join("cheatsheets/esp32c6-gpio.toml").is_file(),
            "esp32c6 cheatsheet missing"
        );
        assert!(
            dir.path().join("cheatsheets/esp32s31-gpio.toml").is_file(),
            "esp32s31 cheatsheet missing"
        );
        assert!(
            dir.path().join("cheatsheets/rp2040-gpio.toml").is_file(),
            "rp2040 cheatsheet missing"
        );
        ensure_seed_kb_in(dir.path()).unwrap(); // idempotent
    }
}
