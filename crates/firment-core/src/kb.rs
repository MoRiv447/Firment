use crate::config::config_dir;
use std::fs;
use std::path::Path;

/// Bump when the bundled seed knowledge base changes (forces re-materialization).
pub const SEED_VERSION: &str = "2";

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
        "cheatsheets/esp32s3-gpio.toml",
        include_str!("../../../docs/cheatsheets/esp32s3-gpio.toml"),
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
        "cheatsheets/stm32g0-gpio.toml",
        include_str!("../../../docs/cheatsheets/stm32g0-gpio.toml"),
    ),
    (
        "cheatsheets/stm32g0-uart.toml",
        include_str!("../../../docs/cheatsheets/stm32g0-uart.toml"),
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
        assert!(esp32.get("s3").is_some());
        assert!(
            dir.path().join("cheatsheets/stm32f1-uart.toml").is_file(),
            "cheatsheet missing"
        );
        ensure_seed_kb_in(dir.path()).unwrap(); // idempotent
    }
}
