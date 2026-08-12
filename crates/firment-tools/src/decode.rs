use object::{Object, ObjectSymbol};
use std::path::Path;

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
}
