use super::util::resolve_within;
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use object::{Object, ObjectSection, ObjectSymbol, SectionKind, SymbolKind};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct ElfAnalyze;

/// Classified section bytes: flash (text + rodata) vs RAM (data + bss).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct MemoryStats {
    text: u64,
    rodata: u64,
    data: u64,
    bss: u64,
}

impl MemoryStats {
    fn flash(&self) -> u64 {
        self.text + self.rodata
    }
    fn ram(&self) -> u64 {
        self.data + self.bss
    }
}

#[derive(Debug, Clone)]
struct FunctionInfo {
    name: String,
    size: u64,
    address: u64,
    /// Max static stack usage from the `.stack_usage` section, when present.
    stack: Option<u32>,
    /// True when the compiler flagged dynamic (alloca/VLA) stack usage.
    stack_dynamic: bool,
}

#[derive(Debug, Default)]
struct ElfReport {
    mem: MemoryStats,
    functions: Vec<FunctionInfo>,
    /// True when no stack info was available at all: no `.stack_usage`
    /// section and no parsed `.su` files.
    stack_section_missing: bool,
    /// Number of functions whose stack info was resolved from `.su` files.
    su_records: usize,
    /// Section names of non-code/non-data sections we skip (informative).
    skipped: Vec<String>,
}

impl ElfReport {
    fn maybe_gi_format(&self) -> String {
        format!(
            "Flash: {} KiB (text {}, rodata {}) | RAM: {} KiB (data {}, bss {})",
            kib(self.mem.flash()),
            kib(self.mem.text),
            kib(self.mem.rodata),
            kib(self.mem.ram()),
            kib(self.mem.data),
            kib(self.mem.bss),
        )
    }

    fn largest_functions(&self, n: usize) -> Vec<&FunctionInfo> {
        let mut list: Vec<&FunctionInfo> = self.functions.iter().collect();
        list.sort_by_key(|f| std::cmp::Reverse(f.size));
        list.truncate(n);
        list
    }

    fn deepest_stack(&self, n: usize) -> Vec<&FunctionInfo> {
        let mut list: Vec<&FunctionInfo> = self
            .functions
            .iter()
            .filter(|f| f.stack.is_some())
            .collect();
        list.sort_by_key(|f| std::cmp::Reverse(f.stack.unwrap_or(0)));
        list.truncate(n);
        list
    }
}

fn kib(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    }
}

/// Parse an ELF/PE/COFF file into an analysis report.
fn analyze(elf: &Path) -> Result<ElfReport, ToolError> {
    let data =
        std::fs::read(elf).map_err(|e| ToolError::new(format!("[Io] cannot read ELF: {e}")))?;
    let file = object::File::parse(&*data)
        .map_err(|e| ToolError::new(format!("[InvalidInput] not a parseable binary: {e}")))?;

    let mut report = ElfReport::default();
    let mut has_stack_section = false;

    // Function table first so the `.stack_usage` section below can attach
    // records to functions by name.
    for symbol in file.symbols() {
        if symbol.kind() != SymbolKind::Text {
            continue;
        }
        let Ok(name) = symbol.name() else {
            continue;
        };
        if name.is_empty() || name.starts_with('$') {
            continue;
        }
        let size = symbol.size();
        if size == 0 {
            continue;
        }
        report.functions.push(FunctionInfo {
            name: name.to_string(),
            size,
            address: symbol.address(),
            stack: None,
            stack_dynamic: false,
        });
    }
    report.functions.sort_by_key(|f| f.address);

    for section in file.sections() {
        let Ok(name) = section.name() else {
            continue;
        };
        let kind = section.kind();
        let size = section.size();
        // GCC/Clang emit `.stack_usage` as an allocated (or metadata) section;
        // identify it by name regardless of how the parser classifies it.
        if name == ".stack_usage" && kind != SectionKind::UninitializedData {
            has_stack_section = true;
            collect_stack_usage(&file, section.data().as_deref().unwrap_or(&[]), &mut report);
            continue;
        }
        match kind {
            SectionKind::Text => report.mem.text += size,
            SectionKind::ReadOnlyData | SectionKind::ReadOnlyString => report.mem.rodata += size,
            SectionKind::Data | SectionKind::ReadOnlyDataWithRel => report.mem.data += size,
            SectionKind::UninitializedData | SectionKind::UninitializedTls => {
                report.mem.bss += size
            }
            SectionKind::Tls => report.mem.data += size,
            _ => {
                if !name.is_empty() {
                    report.skipped.push(name.to_string());
                }
            }
        }
    }
    report.stack_section_missing = !has_stack_section;

    Ok(report)
}

/// Parse GCC/Clang `.stack_usage` records: each entry is
/// `[4B static size][4B address][1B dynamic]`. The address is resolved to the
/// nearest preceding text symbol. Malformed trailing bytes are ignored.
fn collect_stack_usage(file: &object::File<'_>, data: &[u8], report: &mut ElfReport) {
    let mut symbols: Vec<(u64, usize)> = Vec::new();
    for (idx, symbol) in file.symbols().enumerate() {
        if symbol.kind() == SymbolKind::Text && symbol.size() > 0 {
            symbols.push((symbol.address(), idx));
        }
    }
    symbols.sort_by_key(|(addr, _)| *addr);

    let mut i = 0;
    while i + 9 <= data.len() {
        let size = read_le32(data, i);
        let address = read_le32(data, i + 4) as u64;
        let dynamic = data[i + 8] != 0;
        // The address points at a function start; resolve the nearest
        // preceding text symbol and attach the record to that function.
        let owner_idx = match symbols.binary_search_by_key(&address, |(addr, _)| *addr) {
            Ok(pos) => Some(symbols[pos].1),
            Err(0) => None,
            Err(pos) => Some(symbols[pos - 1].1),
        };
        if let Some(idx) = owner_idx
            && let Some(sym) = file.symbols().nth(idx)
            && let Ok(name) = sym.name()
            && let Some(info) = report.functions.iter_mut().find(|f| f.name == name)
        {
            info.stack = Some(size);
            info.stack_dynamic |= dynamic;
        }
        // Records are 9 bytes; any alignment padding becomes malformed bytes
        // which the next iteration rejects (size+3 < 9 requires 5 bytes left).
        i += 9;
    }
}

fn read_le32(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

/// GCC/Clang `-fstack-usage` produces a `.su` text file per translation unit
/// (e.g. `build/obj/main.su`) with lines like:
/// `src/main.c:12:5:main 48 static`
/// `src/main.c:30:6:foo 24 dynamic`
/// This is the real-world source of per-function stack depth — the ELF
/// `.stack_usage` section only exists for exotic toolchains. Parse these files
/// and attach records to the report's functions by name (exact match first,
/// then a `.isra`/`.constprop`/`.part`-suffix prefix match, which -O2+ adds).
fn parse_su_files(paths: &[PathBuf], report: &mut ElfReport) -> usize {
    let mut matched = 0usize;
    for path in paths {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // `path:line:col:func size qualifier` — split on the last ':' to
            // isolate the `func size qualifier` tail.
            let Some((_, tail)) = line.rsplit_once(':') else {
                continue;
            };
            let mut words = tail.split_whitespace();
            let Some(func) = words.next() else {
                continue;
            };
            let Some(size) = words.next().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let dynamic = words.next().is_some_and(|q| q.contains("dynamic"));
            // Exact match first, then an -O2+ clone-suffix prefix match
            // (foo -> foo.isra.0 / foo.constprop.1 / foo.part.2).
            let idx = report
                .functions
                .iter()
                .position(|f| f.name == func)
                .or_else(|| {
                    report
                        .functions
                        .iter()
                        .position(|f| f.name.starts_with(&format!("{func}.")))
                });
            if let Some(idx) = idx {
                let info = &mut report.functions[idx];
                info.stack = Some(size);
                info.stack_dynamic |= dynamic;
                matched += 1;
            }
        }
    }
    matched
}

/// Discover `.su` files by scanning the ELF directory tree (depth-limited).
/// Covers common build layouts where the ELF and per-object `.su` files live
/// under the same `build/` tree.
fn discover_su_files(elf_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![elf_dir.to_path_buf()];
    let mut depth = 0;
    while !stack.is_empty() && depth < 4 {
        depth += 1;
        let mut next = Vec::new();
        for dir in stack.drain(..) {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in read.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    next.push(path);
                } else if path.extension().is_some_and(|e| e == "su") {
                    out.push(path);
                }
            }
        }
        stack = next;
    }
    out
}

/// Human-readable analysis output.
fn format_analyze(elf: &Path, report: &ElfReport) -> String {
    let mut out = format!(
        "ELF analysis: {}\n{}",
        elf.display(),
        report.maybe_gi_format()
    );
    let largest = report.largest_functions(10);
    if !largest.is_empty() {
        out.push_str("\nLargest functions:");
        for (i, f) in largest.iter().enumerate() {
            out.push_str(&format!("\n  {}. {} {}", i + 1, f.name, kib(f.size)));
        }
    }
    if report.stack_section_missing {
        out.push_str(
            "\nStack usage: not available. Rebuild with `-fstack-usage` (GCC/Clang) to get \
             per-function stack depth — Firment auto-reads the resulting .su files next to \
             the ELF; keep the ELF and .su files under the same build directory.",
        );
    } else {
        let deepest = report.deepest_stack(10);
        if deepest.is_empty() {
            out.push_str("\nStack usage: section present but no records matched functions.");
        } else {
            out.push_str("\nStack usage (max static depth):");
            for f in deepest {
                let dyn_mark = if f.stack_dynamic { " ⚠ dynamic" } else { "" };
                out.push_str(&format!(
                    "\n  {} -> {} bytes{dyn_mark}",
                    f.name,
                    f.stack.unwrap_or(0)
                ));
            }
        }
    }
    out
}

/// Structural diff between two reports, used both for the human-readable
/// summary and for gate-threshold judgement.
#[derive(Debug, Default)]
struct ElfDiff {
    /// Flash delta in bytes (+ growth / - shrink). 0 when unchanged.
    flash_delta: i64,
    /// RAM delta in bytes.
    ram_delta: i64,
    /// Function-size deltas (name, bytes, signed).
    size_deltas: Vec<(String, i64)>,
    /// Stack-depth deltas: (name, old, new).
    stack_deltas: Vec<(String, u32, u32)>,
}

impl ElfDiff {
    fn has_changes(&self) -> bool {
        self.flash_delta != 0
            || self.ram_delta != 0
            || !self.size_deltas.is_empty()
            || !self.stack_deltas.is_empty()
    }
}

fn diff_reports(old: &ElfReport, new: &ElfReport) -> ElfDiff {
    let old_map = by_name(&old.functions);
    let new_map = by_name(&new.functions);

    let mut size_deltas: Vec<(String, i64)> = Vec::new();
    for (name, f) in &new_map {
        match old_map.get(name) {
            Some(old_f) => {
                let d = f.size as i64 - old_f.size as i64;
                if d != 0 {
                    size_deltas.push((name.to_string(), d));
                }
            }
            None => size_deltas.push((name.to_string(), f.size as i64)),
        }
    }
    for (name, f) in &old_map {
        if !new_map.contains_key(name) {
            size_deltas.push((name.to_string(), -(f.size as i64)));
        }
    }

    let mut stack_deltas: Vec<(String, u32, u32)> = Vec::new();
    for (name, f) in &new_map {
        if let Some(stack) = f.stack
            && let Some(old_f) = old_map.get(name).and_then(|o| o.stack)
            && stack != old_f
        {
            stack_deltas.push((name.to_string(), old_f, stack));
        }
    }

    ElfDiff {
        flash_delta: new.mem.flash() as i64 - old.mem.flash() as i64,
        ram_delta: new.mem.ram() as i64 - old.mem.ram() as i64,
        size_deltas,
        stack_deltas,
    }
}

/// Human-readable diff summary.
fn format_diff(elf: &Path, old: &ElfReport, new: &ElfReport) -> String {
    let diff = diff_reports(old, new);
    let mut out = format!("ELF diff: {} (previous build)\n", elf.display());
    out.push_str(&format!(
        "Flash: {} -> {} ({} {})\nRAM: {} -> {} ({} {})\n",
        kib(old.mem.flash()),
        kib(new.mem.flash()),
        kib(delta(old.mem.flash(), new.mem.flash())),
        updown(old.mem.flash(), new.mem.flash()),
        kib(old.mem.ram()),
        kib(new.mem.ram()),
        kib(delta(old.mem.ram(), new.mem.ram())),
        updown(old.mem.ram(), new.mem.ram()),
    ));

    if !diff.size_deltas.is_empty() {
        let mut size_deltas = diff.size_deltas.clone();
        size_deltas.sort_by_key(|(_, d)| std::cmp::Reverse(d.abs()));
        out.push_str("Function size changes (by |delta|):");
        for (name, d) in size_deltas.into_iter().take(8) {
            out.push_str(&format!(
                "\n  {} {} {}",
                if d >= 0 { "+" } else { "-" },
                kib(d.unsigned_abs()),
                name
            ));
        }
    }

    if !diff.stack_deltas.is_empty() {
        out.push_str("\nStack depth changes:");
        for (name, old, new) in diff.stack_deltas.into_iter().take(8) {
            out.push_str(&format!(
                "\n  {name}: {old} -> {new} bytes ({}{})",
                if new > old { "+" } else { "-" },
                new.abs_diff(old)
            ));
        }
    }
    out
}

fn delta(old: u64, new: u64) -> u64 {
    new.abs_diff(old)
}

fn by_name(list: &[FunctionInfo]) -> BTreeMap<&str, &FunctionInfo> {
    list.iter().map(|f| (f.name.as_str(), f)).collect()
}

fn updown(old: u64, new: u64) -> &'static str {
    match new.cmp(&old) {
        std::cmp::Ordering::Greater => "+",
        std::cmp::Ordering::Less => "-",
        std::cmp::Ordering::Equal => "±0",
    }
}

/// Baseline ELFs are cached per file path inside the session work dir so a
/// later `elf_analyze` can diff without the agent passing both paths.
fn baseline_path(ctx: &ToolContext, elf: &Path) -> Result<Option<PathBuf>, ToolError> {
    let Some(dir) = &ctx.session_dir else {
        return Ok(None);
    };
    let key = elf
        .canonicalize()
        .unwrap_or_else(|_| elf.to_path_buf())
        .to_string_lossy()
        .replace(['\\', '/', ':', '*', '?', '"', '<', '>', '|'], "_");
    Ok(Some(dir.join("elf-baseline").join(format!("{key}.elf"))))
}

/// Gate verdict against thresholds: `Some(true)` when the diff exceeds a
/// threshold (blocking), `Some(false)` when benign, `None` when unchanged or
/// when no baseline was available.
fn gate_blocks(
    diff: Option<&ElfDiff>,
    stack_threshold: u32,
    flash_threshold_kib: u64,
) -> Option<bool> {
    let diff = diff?;
    if !diff.has_changes() {
        return None;
    }
    let flash_bytes = flash_threshold_kib.saturating_mul(1024) as i64;
    let stack_grew = diff
        .stack_deltas
        .iter()
        .any(|(_, old, new)| *new > *old && (*new - *old) as u64 > stack_threshold as u64);
    let flash_grew = diff.flash_delta > flash_bytes;
    Some(stack_grew || flash_grew)
}

/// `[GATE:BLOCK]` / `[GATE:OK]` / `[GATE:CLEAN]` marker for the agent's
/// binary-analysis gate; empty when no thresholds were requested.
fn gate_marker(
    diff: Option<&ElfDiff>,
    stack_threshold: Option<u32>,
    flash_threshold_kib: Option<u64>,
) -> String {
    let (Some(stack_t), Some(flash_t)) = (stack_threshold, flash_threshold_kib) else {
        return String::new();
    };
    match gate_blocks(diff, stack_t, flash_t) {
        Some(true) => "[GATE:BLOCK]".to_string(),
        Some(false) => "[GATE:OK]".to_string(),
        None => "[GATE:CLEAN]".to_string(),
    }
}

#[async_trait]
impl Tool for ElfAnalyze {
    fn name(&self) -> &'static str {
        "elf_analyze"
    }

    fn description(&self) -> &'static str {
        "Analyze a compiled firmware binary (ELF) without running it: flash/RAM usage per section, largest functions, and per-function stack depth when the build used -fstack-usage (GCC/Clang). Pass the same file again after rebuilding to get a diff against the previous build (baseline is cached per path in the session). Use after build/flash to catch flash growth, RAM growth, and stack depth increases that still compile fine. If stack usage is missing, the report says so and you should note it rather than guess."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file": {"type": "string", "description": "Path to the firmware ELF (inside the workspace), e.g. build/fw.elf"},
                "baseline": {"type": "string", "description": "Optional explicit previous ELF to diff against; when omitted, the last analyzed build of this exact path is used"},
                "stack_threshold": {"type": "integer", "description": "Internal (harness gate): per-function stack-depth increase in bytes; when set with flash_threshold_kib, the output starts with a [GATE:...] marker (BLOCK when a threshold is exceeded, OK when changes are benign, CLEAN when unchanged)"},
                "flash_threshold_kib": {"type": "integer", "description": "Internal (harness gate): flash growth in KiB that blocks"}
            },
            "required": ["file"]
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let file = args
            .get("file")
            .and_then(|f| f.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing 'file'"))?;
        let elf = resolve_within(&ctx.cwd, file, &ctx.allowed_roots).map_err(ToolError::new)?;
        if !elf.is_file() {
            return Err(ToolError::new(format!(
                "[NotFound] no ELF at {}: build first (firm build or your build command)",
                elf.display()
            )));
        }
        let mut current = analyze(&elf)?;
        // Per-function stack depth comes from `-fstack-usage` `.su` files in
        // the real world (GCC/Clang do not emit an ELF `.stack_usage`
        // section); scan the ELF tree and attach the records.
        let su_files = discover_su_files(elf.parent().unwrap_or_else(|| Path::new(".")));
        if !su_files.is_empty() {
            let matched = parse_su_files(&su_files, &mut current);
            current.su_records = matched;
            if matched > 0 {
                current.stack_section_missing = false;
            }
        }

        // Explicit baseline argument wins; otherwise use the cached baseline.
        // Threshold args are used by the harness's binary-analysis gate;
        // when present, the output starts with a machine-readable marker.
        let stack_threshold = args
            .get("stack_threshold")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let flash_threshold_kib = args.get("flash_threshold_kib").and_then(|v| v.as_u64());
        let baseline_arg = args.get("baseline").and_then(|b| b.as_str());
        if let Some(baseline) = baseline_arg {
            let old_path =
                resolve_within(&ctx.cwd, baseline, &ctx.allowed_roots).map_err(ToolError::new)?;
            let old = analyze(&old_path)?;
            let diff = diff_reports(&old, &current);
            let mut out = gate_marker(Some(&diff), stack_threshold, flash_threshold_kib);
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format_diff(&old_path, &old, &current));
            out.push('\n');
            out.push_str(&format_analyze(&elf, &current));
            return Ok(ToolOutput { text: out });
        }

        // Cache baseline for the next run.
        let mut baseline_text = String::new();
        let cached_diff = if let Some(cache) = baseline_path(ctx, &elf)? {
            if cache.exists() {
                let old = analyze(&cache)?;
                baseline_text.push_str(&format_diff(&cache, &old, &current));
                baseline_text.push('\n');
                Some(diff_reports(&old, &current))
            } else {
                None
            }
        } else {
            None
        };
        let marker = gate_marker(cached_diff.as_ref(), stack_threshold, flash_threshold_kib);
        if !marker.is_empty() {
            baseline_text.insert_str(0, &format!("{marker}\n"));
        }
        if let Some(cache) = baseline_path(ctx, &elf)? {
            if let Some(parent) = cache.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::copy(&elf, &cache) {
                baseline_text.push_str(&format!(
                    "\n(no baseline drawn: cannot cache {}: {e})",
                    cache.display()
                ));
            }
        }
        baseline_text.push_str(&format_analyze(&elf, &current));
        Ok(ToolOutput {
            text: baseline_text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal};
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    fn ctx(dir: &Path, session: bool) -> ToolContext {
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
            subagent: None,
            subagent_depth: 0,
            max_subagent_depth: 2,
            asker: None,
            web_search_provider: None,
            web_search_api_key: None,
            session_dir: if session {
                Some(dir.join("session"))
            } else {
                None
            },
            allowed_roots: Vec::new(),
            cancel: firment_core::Cancellable::new(),
        }
    }

    fn write_test_elf(path: &Path, text_size: u64, stack_section: Option<&[u8]>) {
        use object::write::Object as WObject;
        use object::{Architecture, BinaryFormat, Endianness, SectionKind};
        let mut obj = WObject::new(BinaryFormat::Elf, Architecture::Arm, Endianness::Little);
        let text = obj.add_section(Vec::new(), b".text".to_vec(), SectionKind::Text);
        let body: Vec<u8> = (0..text_size).map(|i| (i % 251) as u8).collect();
        obj.append_section_data(text, &body, 4);
        obj.add_symbol(object::write::Symbol {
            name: b"main".to_vec(),
            value: 0,
            size: text_size,
            kind: SymbolKind::Text,
            scope: object::SymbolScope::Linkage,
            weak: false,
            section: object::write::SymbolSection::Section(text),
            flags: object::SymbolFlags::None,
        });
        if let Some(stack) = stack_section {
            let data = obj.add_section(Vec::new(), b".stack_usage".to_vec(), SectionKind::Metadata);
            obj.append_section_data(data, stack, 1);
        }
        let bytes = obj.write().unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn flash_ram_totals_from_synthetic_elf() {
        let dir = tempdir().unwrap();
        let elf = dir.path().join("fw.elf");
        write_test_elf(&elf, 2048, None);
        let report = analyze(&elf).unwrap();
        assert_eq!(report.mem.text, 2048);
        assert_eq!(report.functions.len(), 1);
        assert_eq!(report.functions[0].name, "main");
        assert_eq!(report.functions[0].size, 2048);
        assert!(report.stack_section_missing);
    }

    #[test]
    fn stack_usage_section_is_parsed() {
        let dir = tempdir().unwrap();
        let elf = dir.path().join("fw.elf");
        // Record for address 0 (main): 128-byte static stack, not dynamic.
        let mut stack = vec![128u8, 0, 0, 0, 0, 0, 0, 0, 0];
        write_test_elf(&elf, 512, Some(&stack));
        let report = analyze(&elf).unwrap();
        assert!(!report.stack_section_missing);
        assert_eq!(report.functions[0].stack, Some(128));
        assert!(!report.functions[0].stack_dynamic);

        // Dynamic flag + a different size for a second build.
        let dir2 = tempdir().unwrap();
        let elf2 = dir2.path().join("fw.elf");
        stack = vec![64u8, 0, 0, 0, 0, 0, 0, 0, 1];
        write_test_elf(&elf2, 512, Some(&stack));
        let report2 = analyze(&elf2).unwrap();
        assert_eq!(report2.functions[0].stack, Some(64));
        assert!(report2.functions[0].stack_dynamic);
    }

    #[test]
    fn su_files_are_parsed_and_attached() {
        let dir = tempdir().unwrap();
        let elf = dir.path().join("fw.elf");
        write_test_elf(&elf, 512, None);
        let mut report = analyze(&elf).unwrap();
        assert!(report.stack_section_missing);

        // GCC-style .su text next to the ELF; the test ELF only defines a
        // `main` symbol, so `foo` records are ignored (must attach only to
        // functions that actually exist in the binary).
        std::fs::write(
            dir.path().join("main.su"),
            "src/main.c:12:5:main 48 static\nsrc/foo.c:3:6:foo 24 dynamic\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("util.su"),
            "src/util.c:7:6:nonexistent 96 static\n",
        )
        .unwrap();
        let files = discover_su_files(dir.path());
        assert_eq!(files.len(), 2, "both .su files found");
        let matched = parse_su_files(&files, &mut report);
        assert_eq!(matched, 1, "only functions present in the ELF attach");
        let main = report.functions.iter().find(|f| f.name == "main").unwrap();
        assert_eq!(main.stack, Some(48));
        assert!(!main.stack_dynamic);
    }

    #[test]
    fn su_clone_suffix_matches_optimized_symbols() {
        // -O2+ renames static helpers: the .su line says `foo`, the ELF
        // symbol is `foo.isra.0` — the prefix match must attach the record.
        let dir = tempdir().unwrap();
        let mut report = ElfReport {
            functions: vec![FunctionInfo {
                name: "foo.isra.0".to_string(),
                size: 16,
                address: 0,
                stack: None,
                stack_dynamic: false,
            }],
            ..ElfReport::default()
        };
        std::fs::write(dir.path().join("util.su"), "src/util.c:7:6:foo 96 static\n").unwrap();
        let files = discover_su_files(dir.path());
        let matched = parse_su_files(&files, &mut report);
        assert_eq!(matched, 1, "clone-suffix prefix match");
        assert_eq!(report.functions[0].stack, Some(96));
        assert!(!report.functions[0].stack_dynamic);
    }

    #[test]
    fn run_reads_su_files_next_to_the_elf() {
        let dir = tempdir().unwrap();
        let elf = dir.path().join("fw.elf");
        write_test_elf(&elf, 512, None);
        std::fs::write(
            dir.path().join("main.su"),
            "src/main.c:12:5:main 48 static\n",
        )
        .unwrap();
        let out = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(ElfAnalyze.run(json!({"file": "fw.elf"}), &ctx(dir.path(), false)))
            .unwrap();
        assert!(
            out.text.contains("Stack usage (max static depth)"),
            "run output should show stack usage from .su, got: {}",
            out.text
        );
        assert!(out.text.contains("main"), "got: {}", out.text);
    }

    #[test]
    fn baseline_is_cached_and_diffed() {
        let dir = tempdir().unwrap();
        let elf = dir.path().join("fw.elf");
        let c = ctx(dir.path(), true);
        write_test_elf(&elf, 1024, None);

        let out = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(ElfAnalyze.run(json!({"file": "fw.elf"}), &c))
            .unwrap();
        assert!(out.text.contains("1.0 KiB"), "got: {}", out.text);
        assert!(out.text.contains("main"), "got: {}", out.text);

        // Grow the binary; the second run should diff against the cached one.
        write_test_elf(&elf, 2048, None);
        let out = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(ElfAnalyze.run(json!({"file": "fw.elf"}), &c))
            .unwrap();
        assert!(out.text.contains("ELF diff"), "got: {}", out.text);
        assert!(out.text.contains("2.0 KiB"), "got: {}", out.text);
        assert!(out.text.contains("main"), "got: {}", out.text);
    }

    #[test]
    fn missing_file_is_reported() {
        let dir = tempdir().unwrap();
        let c = ctx(dir.path(), true);
        let err = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(ElfAnalyze.run(json!({"file": "nope.elf"}), &c))
            .unwrap_err();
        assert!(err.message.contains("[NotFound]"), "got: {err}");
    }

    fn diff(flash_delta: i64, stack_deltas: Vec<(String, u32, u32)>) -> ElfDiff {
        ElfDiff {
            flash_delta,
            stack_deltas,
            ..ElfDiff::default()
        }
    }

    #[test]
    fn gate_blocks_on_stack_or_flash_threshold() {
        assert_eq!(gate_blocks(Some(&diff(0, vec![])), 32, 1), None);
        assert_eq!(
            gate_blocks(Some(&diff(100, vec![])), 32, 1),
            Some(false),
            "sub-threshold flash growth is benign"
        );
        assert_eq!(
            gate_blocks(Some(&diff(2048, vec![])), 32, 1),
            Some(true),
            "flash growth over 1 KiB blocks"
        );
        assert_eq!(
            gate_blocks(Some(&diff(0, vec![("foo".into(), 16, 48)])), 32, 1),
            Some(false),
            "stack growth equal to the threshold is benign, not blocking"
        );
        assert_eq!(
            gate_blocks(Some(&diff(0, vec![("foo".into(), 16, 64)])), 32, 1),
            Some(true),
            "stack growth over 32 B blocks"
        );
        assert_eq!(
            gate_blocks(Some(&diff(0, vec![("foo".into(), 64, 16)])), 32, 1),
            Some(false),
            "stack shrink never blocks"
        );
        assert_eq!(gate_blocks(None, 32, 1), None);
    }

    #[test]
    fn gate_marker_reflects_verdict() {
        assert_eq!(
            gate_marker(Some(&diff(100, vec![])), Some(32), Some(1)),
            "[GATE:OK]"
        );
        assert_eq!(
            gate_marker(Some(&diff(2048, vec![])), Some(32), Some(1)),
            "[GATE:BLOCK]"
        );
        assert_eq!(
            gate_marker(Some(&diff(0, vec![])), Some(32), Some(1)),
            "[GATE:CLEAN]"
        );
        assert_eq!(
            gate_marker(Some(&diff(100, vec![])), None, None),
            "",
            "no thresholds, no marker"
        );
    }
}
