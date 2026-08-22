use object::{Object, ObjectSymbol};
use std::path::Path;

/// Target CPU family of an ELF, from the e_machine field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfArch {
    /// ARM Cortex-M/A/R (EM_ARM). The only family with CoreSight fault
    /// registers (CFSR/...) and SWO/ITM trace.
    Arm,
    RiscV,
    Xtensa,
    Other,
}

impl ElfArch {
    pub fn name(self) -> &'static str {
        match self {
            ElfArch::Arm => "ARM",
            ElfArch::RiscV => "RISC-V",
            ElfArch::Xtensa => "Xtensa",
            ElfArch::Other => "non-ARM",
        }
    }
}

/// Decide whether the debug target is definitely not an ARM Cortex-M part:
/// from the firmware ELF's architecture when that file is readable, otherwise
/// from the probe-rs chip name (`esp32*` are all Xtensa or RISC-V).
///
/// Returns a human-readable reason when the target is definitely non-ARM
/// (Cortex-M-only features like CFSR fault registers or SWO/ITM trace do not
/// exist there); None when the target may be ARM. The ELF wins when both
/// sources are available — a wrong chip label must not block a real ARM ELF.
pub fn non_arm_reason(chip: &str, elf: Option<&Path>) -> Option<String> {
    if let Some(elf) = elf
        && let Some(arch) = elf_arch(elf)
    {
        return match arch {
            ElfArch::Arm => None,
            other => Some(format!("the firmware ELF is a {} build", other.name())),
        };
    }
    if chip.to_ascii_lowercase().starts_with("esp32") {
        return Some("the chip is an ESP32 (Xtensa/RISC-V family)".to_string());
    }
    None
}

/// Read the ELF header's e_machine. Returns None when the file is
/// missing/unparseable.
pub fn elf_arch(elf: &Path) -> Option<ElfArch> {
    let data = std::fs::read(elf).ok()?;
    let file = object::File::parse(&*data).ok()?;
    Some(match file.architecture() {
        object::Architecture::Arm => ElfArch::Arm,
        object::Architecture::Riscv32 | object::Architecture::Riscv64 => ElfArch::RiscV,
        object::Architecture::Xtensa => ElfArch::Xtensa,
        _ => ElfArch::Other,
    })
}

/// Resolve a symbol name (function or global variable) to its link address in
/// an ELF. Returns None when the file is missing/unparseable or the symbol
/// does not exist. Used by the debug tool so callers can read/write a named
/// variable (`symbol:name`) instead of guessing addresses.
pub fn symbol_address(elf: &Path, name: &str) -> Option<u64> {
    let data = std::fs::read(elf).ok()?;
    let file = object::File::parse(&*data).ok()?;
    for sym in file.symbols() {
        if sym.name().ok().is_some_and(|n| n == name) {
            return Some(sym.address());
        }
    }
    None
}

/// Resolve a code address to `function+0xOFFSET` using the ELF symbol table.
/// Returns None when the file is missing/unparseable or no symbol covers it.
pub fn decode_address(elf: &Path, address: u64) -> Option<String> {
    let data = std::fs::read(elf).ok()?;
    let file = object::File::parse(&*data).ok()?;
    let mut best: Option<(u64, String)> = None;
    for sym in file.symbols() {
        if sym.kind() != object::SymbolKind::Text {
            continue;
        }
        let addr = sym.address();
        // The symbol must actually cover the address (or be the last symbol
        // with an unknown size, which old tools may report as 0): an address
        // in a gap between functions must not be attributed to the previous
        // one as a huge +0x offset.
        let size = sym.size();
        if addr > address || (size > 0 && address >= addr.saturating_add(size)) {
            continue;
        }
        let Ok(name) = sym.name() else { continue };
        if name.is_empty() {
            continue;
        }
        if best.as_ref().map(|(a, _)| addr >= *a).unwrap_or(true) {
            best = Some((addr, name.to_string()));
        }
    }
    let (base, name) = best?;
    Some(format!("{name}+0x{:x}", address - base))
}

/// Decode hex code addresses in one log line when an ELF is provided.
pub fn decode_line(line: &str, elf: Option<&Path>) -> String {
    let Some(elf) = elf else {
        return line.to_string();
    };
    let mut out = line.to_string();
    let tokens: Vec<String> = out
        .split(|c: char| !c.is_ascii_hexdigit() && c != 'x' && c != 'X')
        .map(|s| s.to_string())
        .collect();
    for token in tokens {
        let lower = token.to_lowercase();
        if !lower.starts_with("0x") || lower.len() < 8 {
            continue;
        }
        let Ok(addr) = u64::from_str_radix(&lower[2..], 16) else {
            continue;
        };
        if let Some(decoded) = decode_address(elf, addr) {
            out = out.replace(&token, &format!("{token} ({decoded})"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_elf_returns_none() {
        assert!(decode_address(Path::new("C:/definitely/missing.elf"), 0x100).is_none());
    }

    #[test]
    fn decode_line_passthrough_without_elf() {
        let line = "panic at 0x08001234";
        assert_eq!(decode_line(line, None), line);
    }

    #[test]
    fn symbol_address_missing_elf_returns_none() {
        assert!(symbol_address(Path::new("C:/definitely/missing.elf"), "main").is_none());
    }

    fn write_minimal_elf(path: &Path, arch: object::Architecture) {
        use object::write::Object;
        use object::{BinaryFormat, Endianness};
        let obj = Object::new(BinaryFormat::Elf, arch, Endianness::Little);
        std::fs::write(path, obj.write().unwrap()).unwrap();
    }

    #[test]
    fn elf_arch_detects_target_family() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("fw.elf");
        write_minimal_elf(&p, object::Architecture::Arm);
        assert_eq!(elf_arch(&p), Some(ElfArch::Arm));
        write_minimal_elf(&p, object::Architecture::Riscv32);
        assert_eq!(elf_arch(&p), Some(ElfArch::RiscV));
        write_minimal_elf(&p, object::Architecture::Xtensa);
        assert_eq!(elf_arch(&p), Some(ElfArch::Xtensa));
        assert_eq!(elf_arch(Path::new("C:/definitely/missing.elf")), None);
    }

    #[test]
    fn non_arm_reason_prefers_elf_then_chip_name() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("fw.elf");
        // Chip-name fallback when no ELF is available.
        assert!(non_arm_reason("esp32s31", None).is_some());
        assert!(non_arm_reason("ESP32-C6", None).is_some());
        assert!(non_arm_reason("stm32g431rb", None).is_none());
        assert!(non_arm_reason("rp2040", None).is_none());
        // A readable ARM ELF wins even if the chip label says ESP32.
        write_minimal_elf(&p, object::Architecture::Arm);
        assert!(non_arm_reason("esp32s3", Some(&p)).is_none());
        // A readable RISC-V ELF blocks regardless of the label.
        write_minimal_elf(&p, object::Architecture::Riscv32);
        assert!(non_arm_reason("stm32f103", Some(&p)).is_some());
        // Unparseable ELF falls through to the chip name.
        assert!(non_arm_reason("esp32s31", Some(Path::new("C:/nope.elf"))).is_some());
    }
}
