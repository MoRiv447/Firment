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

/// Cached function-symbol table for repeated address lookups. `decode_address`
/// re-reads and re-parses the whole ELF for EVERY address — fine for a
/// one-off PC/LR attribution, but a monitor log line can carry several hex
/// tokens and a capture can carry thousands of lines. Build once, query many.
///
/// Coverage semantics match `decode_address` exactly (including size-0
/// symbols covering unboundedly upward). Unlike the elf_analyze stats table,
/// `$`-prefixed mapping symbols are KEPT so index output is byte-identical
/// to the legacy path.
///
/// The lenient covering rule is deliberate for LOG decoding: attributing an
/// address to the nearest known symbol is more useful than printing nothing.
/// The stack scan, however, must treat those candidates skeptically — a
/// size-0 symbol absorbing megabytes of address space turns random stack
/// words into fake return addresses. Use `lookup_detail` there and judge by
/// offset (see `code_pointer_scan`).
pub struct SymbolIndex {
    /// (address, size, name), sorted ascending by address.
    symbols: Vec<(u64, u64, String)>,
    min_addr: u64,
}

impl SymbolIndex {
    /// Build the index from an ELF file. Returns None when the file is
    /// missing/unparseable.
    pub fn from_path(elf: &Path) -> Option<Self> {
        let data = std::fs::read(elf).ok()?;
        Self::from_data(&data)
    }

    pub fn from_data(data: &[u8]) -> Option<Self> {
        let file = object::File::parse(data).ok()?;
        let mut symbols: Vec<(u64, u64, String)> = file
            .symbols()
            .filter(|sym| sym.kind() == object::SymbolKind::Text)
            .filter_map(|sym| {
                let name = sym.name().ok()?;
                if name.is_empty() {
                    return None;
                }
                Some((sym.address(), sym.size(), name.to_string()))
            })
            .collect();
        symbols.sort_unstable_by_key(|(addr, _, _)| *addr);
        let min_addr = symbols.first().map(|(a, _, _)| *a)?;
        Some(Self { symbols, min_addr })
    }

    /// Resolve a code address to `function+0xOFFSET` (highest covering base,
    /// same rules as `decode_address`). A gap resolves to None — UNLESS a
    /// size-0 symbol below it covers unboundedly upward, in which case the
    /// gap is attributed to that symbol; `decode_address` behaves the same.
    pub fn lookup(&self, address: u64) -> Option<String> {
        if address < self.min_addr {
            return None;
        }
        // First symbol with base > address; the candidate is the one before
        // it. If that candidate doesn't cover (its size ends before the
        // address — a gap), walk back to lower bases, mirroring
        // decode_address's highest-covering-base rule.
        let pos = self
            .symbols
            .partition_point(|(addr, _, _)| *addr <= address);
        for (addr, size, name) in self.symbols[..pos].iter().rev() {
            if *size > 0 && address >= addr.saturating_add(*size) {
                continue;
            }
            return Some(format!("{name}+0x{:x}", address - addr));
        }
        None
    }

    /// Detail view of `lookup` for the stack scan: returns the symbol name,
    /// its offset into the symbol, and the symbol's reported size, so the
    /// caller can judge whether a candidate is reliable at all (see
    /// `code_pointer_scan` for the reliability rules). Covering semantics
    /// are identical to `lookup` — including size-0 symbols covering
    /// unboundedly upward, which is exactly what the caller must filter on.
    pub(crate) fn lookup_detail(&self, address: u64) -> Option<(&str, u64, u64)> {
        if address < self.min_addr {
            return None;
        }
        let pos = self
            .symbols
            .partition_point(|(addr, _, _)| *addr <= address);
        for (addr, size, name) in self.symbols[..pos].iter().rev() {
            if *size > 0 && address >= addr.saturating_add(*size) {
                continue;
            }
            return Some((name.as_str(), address - addr, *size));
        }
        None
    }
}

/// Shared token-scan body for `decode_line` and `SymbolIndex::decode_line`:
/// split the line on non-hex boundaries, resolve `0x`-prefixed tokens (>= 8
/// hex digits), and annotate them in place.
fn decode_line_with(line: &str, mut lookup: impl FnMut(u64) -> Option<String>) -> String {
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
        if let Some(decoded) = lookup(addr) {
            out = out.replace(&token, &format!("{token} ({decoded})"));
        }
    }
    out
}

/// Decode hex code addresses in one log line when an ELF is provided.
pub fn decode_line(line: &str, elf: Option<&Path>) -> String {
    let Some(elf) = elf else {
        return line.to_string();
    };
    decode_line_with(line, |addr| decode_address(elf, addr))
}

impl SymbolIndex {
    /// `decode_line` against the cached index — same output, no per-token
    /// ELF re-read.
    pub fn decode_line(&self, line: &str) -> String {
        decode_line_with(line, |addr| self.lookup(addr))
    }
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

    /// ELF with real text symbols: handler@0x1000 (size 0x100), main@0x2000
    /// (size 0x40), gap after main, tail@0x9000 (size 0 — covers upward).
    fn write_elf_with_symbols(path: &Path) {
        use object::write::{Object, Symbol, SymbolSection};
        use object::{BinaryFormat, Endianness, SectionKind, SymbolKind, SymbolScope};
        let mut obj = Object::new(
            BinaryFormat::Elf,
            object::Architecture::Arm,
            Endianness::Little,
        );
        let text = obj.add_section(Vec::new(), b".text".to_vec(), SectionKind::Text);
        for (name, value, size) in [
            ("handler", 0x1000u64, 0x100u64),
            ("main", 0x2000, 0x40),
            ("tail", 0x9000, 0),
        ] {
            obj.add_symbol(Symbol {
                name: name.as_bytes().to_vec(),
                value,
                size,
                kind: SymbolKind::Text,
                scope: SymbolScope::Compilation,
                section: SymbolSection::Section(text),
                weak: false,
                flags: object::SymbolFlags::None,
            });
        }
        std::fs::write(path, obj.write().unwrap()).unwrap();
    }

    #[test]
    fn symbol_index_matches_decode_address() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("fw.elf");
        write_elf_with_symbols(&p);
        let index = SymbolIndex::from_path(&p).unwrap();
        for addr in [0x1000u64, 0x10ff, 0x2000, 0x203f, 0x9abc, 0xdeadbeef] {
            assert_eq!(
                index.lookup(addr).as_deref(),
                decode_address(&p, addr).as_deref(),
                "index/legacy mismatch at {addr:#x}"
            );
        }
        // Gap between main's end and tail: must not be misattributed.
        assert_eq!(index.lookup(0x5000), None);
        assert_eq!(decode_address(&p, 0x5000), None);
        // Below the lowest symbol.
        assert_eq!(index.lookup(0x10), None);
    }

    #[test]
    fn indexed_decode_line_matches_legacy() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("fw.elf");
        write_elf_with_symbols(&p);
        let index = SymbolIndex::from_path(&p).unwrap();
        let line = "fault at pc=0x00002010 lr=0x00001004 sp=0x2000fffc";
        assert_eq!(index.decode_line(line), decode_line(line, Some(&p)));
        assert!(
            index.decode_line(line).contains("main+0x10"),
            "got: {}",
            index.decode_line(line)
        );
    }

    #[test]
    fn symbol_index_missing_elf_returns_none() {
        assert!(SymbolIndex::from_path(Path::new("C:/definitely/missing.elf")).is_none());
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
