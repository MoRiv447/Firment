//! Fault forensics core: pure functions that turn a captured Cortex-M scene
//! (registers + fault registers + a stack window) into a structured report.
//! No hardware access here — the debug tool drives probe-rs and feeds the
//! captured pieces in; everything in this module is unit-testable with
//! synthetic data (a synthetic ELF plus hand-built register/stack words).

use crate::decode::SymbolIndex;
use firment_core::journal::LedgerChange;

/// Cortex-M basic exception frame, pushed by the hardware on entry to the
/// fault handler: R0, R1, R2, R3, R12, LR, PC, xPSR (in that stack order).
#[derive(Debug, Clone, Copy)]
pub struct ExceptionFrame {
    pub r0: u64,
    pub r1: u64,
    pub r2: u64,
    pub r3: u64,
    pub r12: u64,
    pub lr: u64,
    pub pc: u64,
    pub xpsr: u64,
}

/// Interpret a stack window as an exception frame. Returns None when fewer
/// than 8 words were captured (the hardware always pushes at least 8).
pub fn exception_frame(words: &[u64]) -> Option<ExceptionFrame> {
    if words.len() < 8 {
        return None;
    }
    Some(ExceptionFrame {
        r0: words[0],
        r1: words[1],
        r2: words[2],
        r3: words[3],
        r12: words[4],
        lr: words[5],
        pc: words[6],
        xpsr: words[7],
    })
}

/// Candidate call-chain entries from the stack window: values that resolve
/// to a known function. The Cortex-M Thumb bit (LSB) is stripped before
/// lookup; consecutive duplicates collapse. Order is stack order — on a
/// fault frame that is most-recent-first.
/// Stack-scan verdict: the surviving candidates plus how many were rejected
/// as unreliable. The count matters — an empty candidate list backed by a
/// pile of suppressed words reads very differently from "nothing on the
/// stack looked like code at all".
pub struct ScanResult {
    pub candidates: Vec<String>,
    pub suppressed: usize,
}

/// A size-0 symbol covers every address above it (see `SymbolIndex::lookup`),
/// so a stack word landing on one is only plausibly a return address if it
/// sits close behind the symbol's base. Beyond this, treat it as noise.
const MAX_UNSIZED_OFFSET: u64 = 0x1000;

pub fn code_pointer_scan(words: &[u64], index: &SymbolIndex) -> ScanResult {
    let mut candidates: Vec<String> = Vec::new();
    let mut suppressed = 0usize;
    for &w in words {
        if w < 2 {
            continue;
        }
        let addr = w & !1;
        let Some((name, offset, size)) = index.lookup_detail(addr) else {
            continue;
        };
        // ARM mapping symbols ($a/$d/$t) mark instruction/data switches; they
        // are never call sites.
        if name.starts_with('$') {
            suppressed += 1;
            continue;
        }
        // Size-0 symbols cover unboundedly upward, so a random stack word far
        // above the base resolves to them by construction. A return address
        // never sits base + megabytes away.
        if size == 0 && offset > MAX_UNSIZED_OFFSET {
            suppressed += 1;
            continue;
        }
        let decoded = format!("{name}+0x{offset:x}");
        if candidates
            .last()
            .is_some_and(|last: &String| last == &decoded)
        {
            continue;
        }
        candidates.push(decoded);
    }
    ScanResult {
        candidates,
        suppressed,
    }
}

fn age_str(now: u64, created: u64) -> String {
    let d = now.saturating_sub(created);
    if d >= 86_400 {
        format!("{}d ago", d / 86_400)
    } else if d >= 3600 {
        format!("{}h ago", d / 3600)
    } else {
        format!("{}m ago", d / 60)
    }
}

/// Correlate the fault with the session ledger: change batches committed in
/// the 7 days before the fault, newest first, capped at 5. When the faulting
/// function name is known, batches touching a file whose stem matches it are
/// flagged — a light heuristic, clearly labeled as such in the output.
pub fn correlate_changes(
    entries: &[(u64, u64, Vec<LedgerChange>)],
    fault_time: u64,
    faulting_fn: Option<&str>,
) -> Vec<String> {
    let window_start = fault_time.saturating_sub(7 * 86_400);
    let mut recent: Vec<&(u64, u64, Vec<LedgerChange>)> = entries
        .iter()
        .filter(|(_, created, _)| (window_start..=fault_time.saturating_add(60)).contains(created))
        .collect();
    recent.sort_by_key(|e| std::cmp::Reverse(e.1));
    recent.truncate(5);
    let stem = faulting_fn.map(|f| f.split("::").next().unwrap_or(f).to_ascii_lowercase());
    let mut out = Vec::new();
    if recent.is_empty() {
        out.push("  (no changes committed in the 7 days before the fault)".to_string());
        return out;
    }
    for (seq, created, changes) in recent {
        let relevant = stem.as_ref().is_some_and(|stem| {
            changes.iter().any(|c| {
                c.path
                    .file_stem()
                    .map(|s| {
                        s.to_string_lossy()
                            .to_ascii_lowercase()
                            .contains(stem.as_str())
                    })
                    .unwrap_or(false)
            })
        });
        let files: Vec<String> = changes
            .iter()
            .map(|c| c.path.to_string_lossy().into_owned())
            .collect();
        out.push(format!(
            "  seq {} ({}): {}{}",
            seq,
            age_str(fault_time, *created),
            files.join(", "),
            if relevant {
                " — touches a file matching the faulting function"
            } else {
                ""
            }
        ));
    }
    out
}

/// Everything the report needs, captured by the debug tool.
pub struct Scene<'a> {
    pub chip: &'a str,
    pub pc: u64,
    pub lr: u64,
    pub sp: u64,
    /// CFSR, HFSR, DFSR, MMFAR, BFAR (0xE000ED28..).
    pub fault_regs: [u64; 5],
    /// Pre-rendered fault-flag explanations from the debug tool.
    pub cfsr_lines: Vec<String>,
    pub stack_words: &'a [u64],
    pub stack_addr: u64,
    pub index: &'a SymbolIndex,
    /// Pre-correlated ledger lines (see `correlate_changes`).
    pub ledger_lines: Vec<String>,
    /// PC re-read after the capture differed from the first read.
    pub pc_drifted: bool,
    pub snapshot_path: Option<String>,
}

/// Assemble the forensic report. The first 8 stack words are interpreted as
/// the exception frame; the whole window is scanned for return-address
/// candidates.
pub fn forensic_report(scene: &Scene) -> String {
    let mut out = String::new();
    if scene.pc_drifted {
        out.push_str(
            "WARNING: the PC changed between captures — the scene may have been \
             corrupted (watchdog reset race?). Treat the report with suspicion.\n",
        );
    }
    out.push_str(&format!("=== forensic report ({}) ===\n", scene.chip));

    let frame = exception_frame(scene.stack_words);
    match &frame {
        Some(f) => {
            let pc_site = scene.index.lookup(f.pc & !1);
            let lr_site = scene.index.lookup(f.lr & !1);
            out.push_str(&format!(
                "exception frame:\n  pc  = 0x{:08x}{}\n  lr  = 0x{:08x}{}\n  r0..r3 = {:#x} {:#x} {:#x} {:#x}\n  r12 = 0x{:08x}  xpsr = 0x{:08x}\n",
                f.pc,
                pc_site.as_deref().map(|d| format!(" ({d})")).unwrap_or_default(),
                f.lr,
                lr_site.as_deref().map(|d| format!(" ({d})")).unwrap_or_default(),
                f.r0, f.r1, f.r2, f.r3, f.r12, f.xpsr,
            ));
        }
        None => out.push_str("exception frame: unavailable (stack capture too short)\n"),
    }

    for line in &scene.cfsr_lines {
        out.push_str(&format!("  {line}\n"));
    }

    let scan = code_pointer_scan(scene.stack_words, scene.index);
    out.push_str("candidate call chain (stack scan, most recent first):\n");
    if scan.candidates.is_empty() {
        if scan.suppressed > 0 {
            // Field-access captures don't work across a line-continuation
            // backslash, so bind the count to a plain identifier first.
            let n = scan.suppressed;
            out.push_str(&format!(
                "  ({n} candidate(s) suppressed as unreliable — mapping symbols \
                 or unbounded offsets; rebuild with debug symbols for a real chain)\n"
            ));
        } else {
            out.push_str(
                "  (no code pointers resolved — rebuild with debug symbols or widen the capture)\n",
            );
        }
    } else {
        for (i, c) in scan.candidates.iter().enumerate() {
            out.push_str(&format!("  {}. {c}\n", i + 1));
        }
    }

    out.push_str("change ledger (7-day window before the fault, newest first):\n");
    for line in &scene.ledger_lines {
        out.push_str(line);
        out.push('\n');
    }

    if let Some(path) = &scene.snapshot_path {
        out.push_str(&format!("snapshot: {path}\n"));
    }
    out.push_str("[evidence: hardware — fault capture]");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn write_elf_with_symbols(path: &Path) {
        use object::write::{Object, Symbol, SymbolSection};
        use object::{BinaryFormat, Endianness, SectionKind, SymbolKind, SymbolScope};
        let mut obj = Object::new(
            BinaryFormat::Elf,
            object::Architecture::Arm,
            Endianness::Little,
        );
        let text = obj.add_section(Vec::new(), b".text".to_vec(), SectionKind::Text);
        for (name, value, size) in [("handler", 0x1000u64, 0x100u64), ("main", 0x2000, 0x40)] {
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

    /// Adds the two symbol shapes the stack scan must defend against: an ARM
    /// mapping symbol ($a — never a call site) and a size-0 linker-style
    /// symbol (covers unboundedly upward, so far-above stack words resolve to
    /// it by construction).
    fn write_elf_with_edge_symbols(path: &Path) {
        use object::write::{Object, Symbol, SymbolSection};
        use object::{BinaryFormat, Endianness, SectionKind, SymbolKind, SymbolScope};
        let mut obj = Object::new(
            BinaryFormat::Elf,
            object::Architecture::Arm,
            Endianness::Little,
        );
        let text = obj.add_section(Vec::new(), b".text".to_vec(), SectionKind::Text);
        let symbols: [(&str, u64, u64); 3] = [
            ("handler", 0x1000, 0x100),
            ("$a", 0x2000, 0),
            ("bare", 0x3000, 0),
        ];
        for (name, value, size) in symbols {
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
    fn exception_frame_interprets_stack_order() {
        let words = [1u64, 2, 3, 4, 5, 0x2011, 0x1005, 0x0100_0000];
        let f = exception_frame(&words).unwrap();
        assert_eq!(f.pc, 0x1005);
        assert_eq!(f.lr, 0x2011);
        assert_eq!(f.r0, 1);
        assert_eq!(f.xpsr, 0x0100_0000);
        assert!(exception_frame(&words[..7]).is_none());
    }

    #[test]
    fn code_pointer_scan_resolves_strips_thumb_dedupes() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("fw.elf");
        write_elf_with_symbols(&p);
        let index = SymbolIndex::from_path(&p).unwrap();
        // Thumb bits set, one duplicate, one non-pointer.
        let words = [0x1005, 0x1005, 0x1011, 0x50];
        let scan = code_pointer_scan(&words, &index);
        assert_eq!(
            scan.candidates,
            vec!["handler+0x4".to_string(), "handler+0x10".to_string()]
        );
        assert_eq!(scan.suppressed, 0);
    }

    #[test]
    fn code_pointer_scan_suppresses_unreliable_symbols() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("fw.elf");
        write_elf_with_edge_symbols(&p);
        let index = SymbolIndex::from_path(&p).unwrap();
        // $a mapping symbol (never a call site); size-0 symbol at a close
        // offset (plausible); the SAME size-0 symbol far above its base (a
        // random word that only resolves because the symbol covers
        // unboundedly); a real function.
        let words = [0x2001, 0x3005, 0x5001, 0x1005];
        let scan = code_pointer_scan(&words, &index);
        assert_eq!(
            scan.candidates,
            vec!["bare+0x4".to_string(), "handler+0x4".to_string()]
        );
        assert_eq!(scan.suppressed, 2);
    }

    #[test]
    fn correlate_windows_and_flags_relevance() {
        let now = 1_000_000u64;
        let change = |p: &str| {
            vec![LedgerChange {
                path: p.into(),
                old_lines: 1,
                new_lines: 2,
                hunks: String::new(),
                old_sha256: String::new(),
                new_sha256: String::new(),
            }]
        };
        let entries = vec![
            (1u64, now - 100u64, change("src/main.c")),
            (2u64, now - 10 * 86_400, change("src/old.c")), // outside window
            (3u64, now - 50, change("src/handler.c")),
        ];
        let lines = correlate_changes(&entries, now, Some("handler"));
        assert_eq!(lines.len(), 2, "outside-window entry excluded: {lines:?}");
        assert!(
            lines[0].contains("handler.c") && lines[0].contains("matching the faulting function"),
            "{:?}",
            lines[0]
        );
        assert!(lines[1].contains("main.c"));
        assert!(!lines[1].contains("matching"), "{:?}", lines[1]);

        let empty = correlate_changes(&[], now, None);
        assert!(empty[0].contains("no changes committed"));
    }

    #[test]
    fn forensic_report_assembles_sections() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("fw.elf");
        write_elf_with_symbols(&p);
        let index = SymbolIndex::from_path(&p).unwrap();
        // Exception frame: pc 0x1005 (handler+0x5), lr 0x2011 (main+0x11).
        let words = [1u64, 2, 3, 4, 5, 0x2011, 0x1005, 0x0100_0000];
        let scene = Scene {
            chip: "stm32g431rb",
            pc: 0x1005,
            lr: 0x2011,
            sp: 0x2000_fffc,
            fault_regs: [0x0000_8200, 0, 0, 0x1000_1234, 0],
            cfsr_lines: vec!["CFSR = 0x00008200".to_string()],
            stack_words: &words,
            stack_addr: 0x2000_fffc,
            index: &index,
            ledger_lines: vec!["  seq 1 (2h ago): src/main.c".to_string()],
            pc_drifted: true,
            snapshot_path: Some("snap/forensic.txt".to_string()),
        };
        let report = forensic_report(&scene);
        assert!(report.starts_with("WARNING"), "drift must lead: {report}");
        assert!(report.contains("forensic report (stm32g431rb)"));
        assert!(report.contains("handler+0x4"), "pc decoded: {report}");
        assert!(report.contains("candidate call chain"));
        assert!(report.contains("CFSR"));
        assert!(report.contains("change ledger"));
        assert!(report.contains("[evidence: hardware — fault capture]"));
        assert!(report.contains("snapshot: snap/forensic.txt"));
    }
}

/// Scan captured device output for fault signatures and return the marker
/// line the agent should see. Signature base is sbc-guard's rules.toml
/// (`panic|Guru Meditation|assert failed` — a Linux SBC has no Cortex-M
/// faults), EXTENDED with the Cortex-M fault names this crate's targets hit:
/// HardFault/faultISR/BusFault/UsageFault. Case-insensitive; a marker is a
/// hint to run `debug action=forensic`, not a verdict.
pub(crate) fn fault_detected_marker(text: &str) -> Option<&'static str> {
    const NEEDLES: [&str; 8] = [
        "hardfault",
        "hard fault",
        "panic",
        "guru meditation",
        "assert failed",
        "faultisr",
        "busfault",
        "usagefault",
    ];
    let lower = text.to_ascii_lowercase();
    NEEDLES
        .iter()
        .find(|n| lower.contains(*n))
        .map(|_| "[FAULT-DETECTED] — capture the scene now: debug action=forensic")
}

/// Append the fault marker when the captured output contains a fault
/// signature. Shared by the monitor tool, the hil run step and the run tool.
pub(crate) fn append_fault_marker(text: String) -> String {
    match fault_detected_marker(&text) {
        Some(marker) => format!("{text}\n{marker}"),
        None => text,
    }
}

#[cfg(test)]
mod marker_tests {
    use super::*;

    #[test]
    fn marker_hits_fault_signatures() {
        for line in [
            "HardFault_Handler entered",
            "running hard fault recovery",
            "panicked at src/main.rs:10",
            "Guru Meditation Error: Core 0",
            "assert failed: x > 0",
            "faultISR: bad thing",
            "BusFault at 0xdead",
            "UsageFault: unaligned",
        ] {
            assert!(fault_detected_marker(line).is_some(), "must detect: {line}");
        }
    }

    #[test]
    fn marker_ignores_clean_output() {
        for line in [
            "LED blinking at 1 Hz",
            "sensor value 42",
            "build finished (exit 0)",
            "usage: firm hil --suite blink",
        ] {
            assert!(
                fault_detected_marker(line).is_none(),
                "false positive on: {line}"
            );
        }
    }

    #[test]
    fn append_fault_marker_appends_once() {
        let out = append_fault_marker("HardFault\n".to_string());
        assert!(out.contains("[FAULT-DETECTED]"));
        assert_eq!(out.matches("[FAULT-DETECTED]").count(), 1);
        assert_eq!(append_fault_marker("all good\n".to_string()), "all good\n");
    }
}
