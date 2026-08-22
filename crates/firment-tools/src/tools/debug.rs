use super::util::{resolve_within, run_probe_rs, token_arg, truncate};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct Debug;

/// Address parser: accepts `0x...` (any hex length) or `symbol:name`
/// (resolved against the ELF's symbol table).
fn parse_address(s: &str, elf: Option<&Path>) -> Result<u64, String> {
    let s = s.trim();
    if let Some(name) = s.strip_prefix("symbol:") {
        let elf = elf.ok_or(
            "[InvalidInput] symbol:name addresses need an elf parameter (the firmware ELF \
             to resolve the symbol against)",
        )?;
        let name = name.trim();
        if name.is_empty() {
            return Err("[InvalidInput] empty symbol name after 'symbol:'".to_string());
        }
        return crate::decode::symbol_address(elf, name).ok_or_else(|| {
            format!("[NotFound] symbol '{name}' not found in the ELF symbol table")
        });
    }
    let hex = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .ok_or_else(|| "[InvalidInput] address must be '0x<hex>' or 'symbol:<name>'".to_string())?;
    if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("[InvalidInput] address must be '0x<hex>' or 'symbol:<name>'".to_string());
    }
    u64::from_str_radix(hex, 16).map_err(|_| "[InvalidInput] address is too large".to_string())
}

/// Width token validation for probe-rs read/write (`b8`/`b16`/`b32`/`b64`).
fn parse_width(s: &str) -> Result<String, String> {
    match s {
        "b8" | "b16" | "b32" | "b64" => Ok(s.to_string()),
        _ => Err("[InvalidInput] width must be b8, b16, b32 or b64".to_string()),
    }
}

/// Strip ANSI escape sequences (SGR colors, cursor moves) that some console
/// tools emit even when piped; they contain digits that would corrupt parsing.
fn strip_ansi(text: &str) -> String {
    let re = Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").unwrap();
    re.replace_all(text, "").into_owned()
}

/// Parse probe-rs `reg` output into a register-name -> value map. The exact
/// layout differs across probe-rs versions ("pc (r15) = 0x...", "sp: 0x...",
/// "R15/PC: 0x...", "xpsr 0x01000000", ...), so this is deliberately loose:
/// any line containing a register name and a hex value counts, later wins.
fn parse_regs(text: &str) -> HashMap<String, u64> {
    let re = Regex::new(r"(?i)\b(pc|sp|lr|r\d+|xpsr|msp|psp)\b[^0-9x]{0,40}?(?:0x)?([0-9a-f]{8})")
        .unwrap();
    let mut out: HashMap<String, u64> = HashMap::new();
    for cap in re.captures_iter(&strip_ansi(text)) {
        let name = cap.get(1).unwrap().as_str().to_lowercase();
        let Ok(value) = u64::from_str_radix(cap.get(2).unwrap().as_str(), 16) else {
            continue;
        };
        out.insert(name, value);
    }
    out
}

fn reg_value(map: &HashMap<String, u64>, names: &[&str]) -> Option<u64> {
    names.iter().find_map(|n| map.get(*n).copied())
}

/// Parse `probe-rs read -f simple-hex` output: whitespace-separated hex words.
fn parse_hex_words(text: &str) -> Vec<u64> {
    text.split_whitespace()
        .filter_map(|t| {
            let t = t.trim();
            let hex = t.strip_prefix("0x").unwrap_or(t);
            if !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                u64::from_str_radix(hex, 16).ok()
            } else {
                None
            }
        })
        .collect()
}

/// Cortex-M (M0/M3/M4/M33-compatible) CFSR/HFSR flag names and meanings.
/// CFSR = 0xE000ED28, HFSR = 0xE000ED2C, MMFAR = 0xE000ED34, BFAR = 0xE000ED38.
const CFSR_BITS: &[(u32, &str, &str)] = &[
    (
        1 << 0,
        "IACCVIOL",
        "instruction access violation (MPU fault)",
    ),
    (1 << 1, "DACCVIOL", "data access violation (MPU fault)"),
    (1 << 2, "MUNSTKERR", "unstacking error on exception entry"),
    (1 << 3, "MSTKERR", "stacking error on exception entry"),
    (
        1 << 4,
        "MLSPERR",
        "floating-point lazy stacking error (M4F)",
    ),
    (1 << 7, "MMARVALID", "MMFAR holds a valid fault address"),
    (1 << 8, "IBUSERR", "instruction bus error (precise)"),
    (1 << 9, "PRECISERR", "precise data bus error"),
    (
        1 << 10,
        "IMPRECISERR",
        "imprecise data bus error (write-buffer)",
    ),
    (1 << 11, "UNSTKERR", "unstacking error on exception return"),
    (1 << 12, "STKERR", "stacking error on exception entry"),
    (
        1 << 13,
        "LSPERR",
        "floating-point lazy state preservation error (M4F)",
    ),
    (1 << 14, "BUSFAULTVALID", "BFAR holds a valid fault address"),
    // UFSR occupies CFSR bits [31:16]: UNDEFINSTR=16, INVSTATE=17,
    // INVPC=18, NOCP=19, STKOF=20 (M33 only), UNALIGNED=24, DIVBYZERO=25.
    (1 << 16, "UNDEFINSTR", "executed an undefined instruction"),
    (
        1 << 17,
        "INVSTATE",
        "invalid execution state (e.g. EXC_RETURN misuse)",
    ),
    (
        1 << 18,
        "INVPC",
        "invalid PC load (EXC_RETURN to invalid state)",
    ),
    (
        1 << 19,
        "NOCP",
        "coprocessor disabled (e.g. FPU without CP10/CP11 enable)",
    ),
    (1 << 20, "STKOF", "stack overflow (M33)"),
    (
        1 << 24,
        "UNALIGNED",
        "unaligned access with UNALIGN_TRP set",
    ),
    (1 << 25, "DIVBYZERO", "divide by zero with DIV_0_TRP set"),
];

const HFSR_BITS: &[(u32, &str, &str)] = &[
    (1 << 1, "VECTTBL", "vector table read error"),
    (
        1 << 30,
        "FORCED",
        "fault escalated to HardFault (no active handler)",
    ),
    (1 << 31, "DEBUGEVT", "debug event (halt request)"),
];

/// Build a human-readable fault analysis from the raw Cortex-M fault
/// registers. Pure function (testable without hardware).
fn cfsr_analysis(cfsr: u64, hfsr: u64, mmfar: u64, bfar: u64) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("CFSR = 0x{cfsr:08x}"));
    if cfsr == 0 {
        lines.push("  (no configurable fault flags set)".to_string());
    } else {
        for (mask, name, meaning) in CFSR_BITS {
            if cfsr & u64::from(*mask) != 0 {
                lines.push(format!("  [{name}] {meaning}"));
            }
        }
    }
    lines.push(format!("HFSR = 0x{hfsr:08x}"));
    if hfsr == 0 {
        lines.push("  (no hard fault flags set)".to_string());
    } else {
        for (mask, name, meaning) in HFSR_BITS {
            if hfsr & u64::from(*mask) != 0 {
                lines.push(format!("  [{name}] {meaning}"));
            }
        }
    }
    if cfsr & (1 << 7) != 0 {
        lines.push(format!(
            "MMFAR = 0x{mmfar:08x} (valid — faulting data address)"
        ));
    } else {
        lines.push("MMFAR = 0x00000000 (not valid)".to_string());
    }
    if cfsr & (1 << 14) != 0 {
        lines.push(format!(
            "BFAR = 0x{bfar:08x} (valid — faulting bus address)"
        ));
    } else {
        lines.push("BFAR = 0x00000000 (not valid)".to_string());
    }
    lines
}

/// Probe-rs CLI argument arrays. `debug -c "<cmd>" -c "quit"` runs the console
/// command then disconnects (skipping the interactive console), keeping the
/// target halted on exit (disconnect with suspend_debuggee=true).
///
/// The DAP REPL command set (probe-rs 0.32), verified on a live ST-Link target:
/// - `break` with no argument HALTS the target (attach resumes the target, so
///   every read-only session must halt it first: `-c break -c "info reg"`).
/// - `info reg` prints the register table; `reg` alone is NOT a command.
/// - `c` continues, `step` single-steps, `reset` resets (and halts).
/// - `break *0x...` sets a breakpoint (the `*` prefix is required).
/// - `quit` disconnects. There is no `halt`/`regs`/`continue`/`mem`/`q`.
fn debug_args(chip: &str, probe: Option<&str>, commands: &[&str]) -> Vec<String> {
    let mut args = vec!["debug".to_string(), "--chip".to_string(), chip.to_string()];
    if let Some(probe) = probe {
        args.push("--probe".to_string());
        args.push(probe.to_string());
    }
    for cmd in commands {
        args.push("-c".to_string());
        args.push(cmd.to_string());
    }
    args.push("-c".to_string());
    args.push("quit".to_string());
    args
}

/// `probe-rs read <width> <addr> <words> -f simple-hex` — clean hex output for
/// machine parsing.
fn read_args(chip: &str, probe: Option<&str>, width: &str, addr: u64, words: usize) -> Vec<String> {
    let mut args = vec![
        "read".to_string(),
        "--chip".to_string(),
        chip.to_string(),
        width.to_string(),
        format!("{addr:#x}"),
        words.to_string(),
        "-f".to_string(),
        "simple-hex".to_string(),
    ];
    if let Some(probe) = probe {
        args.push("--probe".to_string());
        args.push(probe.to_string());
    }
    args
}

/// `probe-rs debug <elf> -c ...` — the REPL variant with an ELF, which loads
/// DWARF debug info so `bt` (backtrace) can resolve function names and
/// source locations. Without the ELF the REPL has no debug info and `bt`
/// prints nothing.
fn debug_elf_args(chip: &str, probe: Option<&str>, elf: &str, commands: &[&str]) -> Vec<String> {
    let mut args = vec![
        "debug".to_string(),
        elf.to_string(),
        "--chip".to_string(),
        chip.to_string(),
    ];
    if let Some(probe) = probe {
        args.push("--probe".to_string());
        args.push(probe.to_string());
    }
    for cmd in commands {
        args.push("-c".to_string());
        args.push(cmd.to_string());
    }
    args.push("-c".to_string());
    args.push("quit".to_string());
    args
}

/// `probe-rs itm swo <duration_ms> <clk_hz> <baud>` — streams ITM packets
/// out the TRACESWO pin for `<duration_ms>` milliseconds. probe-rs
/// configures the target's CoreSight TPIU/ITM itself (no firmware changes
/// needed to enable tracing), but the firmware must actually write ITM ports
/// (e.g. ITM_SendChar) for data to appear.
///
/// NOTE: `probe-rs itm swo` (0.32) takes NO `--chip`/`--probe`/`--non-interactive`
/// options (they are rejected with "unexpected argument"). The chip and probe
/// are passed via the `PROBE_RS_CHIP` / `PROBE_RS_PROBE` environment
/// variables instead, which the subcommand does honour.
fn itm_swo_args(duration_ms: u64, clk_hz: u64, baud: u64) -> Vec<String> {
    vec![
        "itm".to_string(),
        "swo".to_string(),
        duration_ms.to_string(),
        clk_hz.to_string(),
        baud.to_string(),
    ]
}

/// The `bt` REPL command needs DWARF debug info in the ELF; PlatformIO
/// defaults and plain `arm-none-eabi-gcc -O2` builds often have none, in
/// which case probe-rs prints nothing at all. Detect that and point the
/// agent at a debug build instead of leaving it with a silent empty result.
fn backtrace_hint(text: &str) -> String {
    if text.contains("Frame ") {
        text.to_string()
    } else {
        format!(
            "{text}\n(no stack frames resolved — the firmware ELF has no DWARF \
             debug info; rebuild with debug symbols, e.g. add `build_flags = -g -Og` \
             to platformio.ini, then pass the rebuilt ELF)"
        )
    }
}

fn write_args(chip: &str, probe: Option<&str>, width: &str, addr: u64, value: u64) -> Vec<String> {
    let mut args = vec![
        "write".to_string(),
        "--chip".to_string(),
        chip.to_string(),
        width.to_string(),
        format!("{addr:#x}"),
        format!("{value:#x}"),
    ];
    if let Some(probe) = probe {
        args.push("--probe".to_string());
        args.push(probe.to_string());
    }
    args
}

/// Combine the raw probe-rs outputs into the one-shot fault report.
fn analysis_report(
    chip: &str,
    regs_text: &str,
    cfsr_words: &[u64],
    stack_words: &[u64],
    stack_addr: u64,
    elf: Option<&Path>,
) -> String {
    let mut out = format!("=== debug analyze (chip {chip}) ===\n");
    let regs = parse_regs(regs_text);
    let pc = reg_value(&regs, &["pc", "r15"]);
    let lr = reg_value(&regs, &["lr", "r14"]);
    let sp = reg_value(&regs, &["sp", "r13"]);
    let xpsr = reg_value(&regs, &["xpsr"]);
    out.push_str("target: halted\n");
    let decode = |addr: u64| -> String {
        if let Some(elf) = elf {
            // Cortex-M PC/LR carry the Thumb bit in bit 0.
            let plain = addr & !1;
            crate::decode::decode_address(elf, plain).unwrap_or_else(|| format!("{plain:#010x}"))
        } else {
            format!("{addr:#010x}")
        }
    };
    match pc {
        Some(pc) => out.push_str(&format!("pc  : {pc:#010x} -> {}\n", decode(pc))),
        None => out.push_str("pc  : (not parsed from probe-rs output)\n"),
    }
    match lr {
        Some(lr) => out.push_str(&format!("lr  : {lr:#010x} -> {}\n", decode(lr))),
        None => out.push_str("lr  : (not parsed)\n"),
    }
    match sp {
        Some(sp) => out.push_str(&format!("sp  : {sp:#010x}\n")),
        None => out.push_str("sp  : (not parsed)\n"),
    }
    match xpsr {
        Some(xpsr) => out.push_str(&format!("xpsr: {xpsr:#010x}\n")),
        None => out.push_str("xpsr: (not parsed)\n"),
    }

    if cfsr_words.len() >= 5 {
        let cfsr = cfsr_words[0];
        let hfsr = cfsr_words[1];
        let mmfar = cfsr_words[3];
        let bfar = cfsr_words[4];
        for line in cfsr_analysis(cfsr, hfsr, mmfar, bfar) {
            out.push_str(&line);
            out.push('\n');
        }
    } else {
        out.push_str(
            "fault registers: could not read CFSR/HFSR (not enough words from probe-rs)\n",
        );
    }

    if !stack_words.is_empty() {
        out.push_str(&format!(
            "stack (sp={stack_addr:#010x}, {} words):\n",
            stack_words.len()
        ));
        for (i, word) in stack_words.iter().enumerate() {
            out.push_str(&format!(
                "  {:#010x}: {word:#010x}\n",
                stack_addr + i as u64 * 4
            ));
        }
    }
    out
}

#[async_trait]
impl Tool for Debug {
    fn name(&self) -> &'static str {
        "debug"
    }

    fn description(&self) -> &'static str {
        "Inspect and control the target over the debug probe via probe-rs (ST-Link / J-Link / \
         CMSIS-DAP). Actions: halt (pause the target, stays paused after the call), regs (read \
         all core registers), mem (read memory: width b8/b16/b32/b64, address 0x... or \
         symbol:name resolved from the ELF), write (write memory — changes target state), \
         analyze (one-shot fault diagnosis: halts, reads PC/LR/SP plus the Cortex-M fault \
         registers CFSR/HFSR/MMFAR/BFAR, decodes PC/LR against the ELF and explains the fault \
         cause), break (set a breakpoint, run, report registers when it hits), step (single \
         step from the current PC), continue (resume execution; note the target is re-halted \
         when the debug session disconnects), backtrace (halt and unwind \
         the call stack — needs an elf with DWARF debug info, i.e. built with -g), trace \
         (stream SWO/ITM trace packets for duration_ms — probe-rs configures CoreSight itself, \
         but the firmware must write ITM ports for data to appear). Use after flash/monitor \
         when the target misbehaves, hangs, or produces no serial output."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["halt", "regs", "mem", "write", "analyze", "break", "step", "continue", "backtrace", "trace"], "description": "What to do on the target"},
                "chip": {"type": "string", "description": "probe-rs chip id, e.g. stm32g431rb (defaults to [tools] default_chip)"},
                "probe": {"type": "string", "description": "Optional probe serial/id when multiple probes are attached"},
                "elf": {"type": "string", "description": "Path to the firmware ELF (inside the workspace); REQUIRED for backtrace (needs DWARF debug info) and needed for symbol:name addresses / analyze PC decoding"},
                "address": {"type": "string", "description": "For mem/write/break: '0x<hex>' or 'symbol:<name>' (resolved from the elf)"},
                "width": {"type": "string", "enum": ["b8", "b16", "b32", "b64"], "description": "Access width for mem/write (default b32)"},
                "words": {"type": "integer", "minimum": 1, "maximum": 1024, "description": "Number of words to read for mem (default 1)"},
                "value": {"type": "integer", "minimum": 0, "description": "Value to write for write (up to 64-bit)"},
                "duration_ms": {"type": "integer", "minimum": 1, "maximum": 60000, "default": 3000, "description": "For trace: how long to capture SWO/ITM data (default 3000)"},
                "clk_hz": {"type": "integer", "minimum": 1000, "maximum": 500000000, "default": 170000000, "description": "For trace: the clock feeding the TPIU/SWO module in Hz (default 170 MHz, e.g. STM32G4 HCLK); wrong clk makes the decoded stream garbage"},
                "baud": {"type": "integer", "minimum": 1000, "maximum": 25000000, "default": 2000000, "description": "For trace: SWO output baud rate (default 2 Mbps)"},
                "timeout_ms": {"type": "integer", "minimum": 1, "default": 30000}
            },
            "required": ["action"]
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        let action = args.get("action").and_then(|a| a.as_str()).unwrap_or("?");
        if action == "write" {
            let addr = args.get("address").and_then(|a| a.as_str()).unwrap_or("?");
            let value = args
                .get("value")
                .and_then(|v| v.as_u64())
                .map(|v| format!("{v:#x}"))
                .unwrap_or("?".to_string());
            Some(format!(
                "⚠ debug write: modify target memory at {addr} = {value}"
            ))
        } else {
            Some(format!(
                "⚠ debug {action}: pause/control the target via the debug probe"
            ))
        }
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = args
            .get("action")
            .and_then(|a| a.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing 'action'"))?;
        let chip = args
            .get("chip")
            .and_then(|c| c.as_str())
            .map(|s| token_arg(s, "chip"))
            .transpose()
            .map_err(ToolError::new)?
            .or_else(|| ctx.default_chip.clone())
            .ok_or_else(|| {
                ToolError::new(
                    "[InvalidInput] missing chip: pass a chip parameter (e.g. stm32g431rb) or \
                     set default_chip in [tools] of config.toml",
                )
            })?;
        let probe = args
            .get("probe")
            .and_then(|p| p.as_str())
            .map(|s| token_arg(s, "probe"))
            .transpose()
            .map_err(ToolError::new)?;
        let elf: Option<PathBuf> = args
            .get("elf")
            .and_then(|e| e.as_str())
            .map(|e| resolve_within(&ctx.cwd, e, &ctx.allowed_roots))
            .transpose()
            .map_err(ToolError::new)?;
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|t| t.as_u64())
            .unwrap_or(30_000);

        // Local-only validation happens before any probe-rs contact, so
        // parameter mistakes are reported consistently even without probe-rs
        // installed (and before the [NotFound] hint).
        let address: Option<u64> = match action {
            "mem" | "write" | "break" => {
                let s = args
                    .get("address")
                    .and_then(|a| a.as_str())
                    .ok_or_else(|| {
                        ToolError::new(format!(
                            "[InvalidInput] {action} needs an address (0x... or symbol:name)"
                        ))
                    })?;
                Some(parse_address(s, elf.as_deref()).map_err(ToolError::new)?)
            }
            _ => None,
        };
        let width = args
            .get("width")
            .and_then(|w| w.as_str())
            .map(parse_width)
            .transpose()
            .map_err(ToolError::new)?
            .unwrap_or_else(|| "b32".to_string());
        let words = args.get("words").and_then(|w| w.as_u64()).unwrap_or(1);
        if words == 0 || words > 1024 {
            return Err(ToolError::new("[InvalidInput] words must be 1..=1024"));
        }
        if action == "write" {
            let value = args
                .get("value")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| ToolError::new("[InvalidInput] write needs a value"))?;
            let max = match width.as_str() {
                "b8" => 0xff,
                "b16" => 0xffff,
                "b64" => u64::MAX,
                _ => 0xffff_ffff,
            };
            if value > max {
                return Err(ToolError::new(format!(
                    "[InvalidInput] value {value:#x} does not fit in {width}"
                )));
            }
        }
        if !matches!(
            action,
            "halt"
                | "regs"
                | "mem"
                | "write"
                | "analyze"
                | "break"
                | "step"
                | "continue"
                | "backtrace"
                | "trace"
        ) {
            return Err(ToolError::new(format!(
                "[InvalidInput] unknown action '{action}'"
            )));
        }
        if action == "backtrace" && elf.is_none() {
            return Err(ToolError::new(
                "[InvalidInput] backtrace needs an elf parameter (the firmware ELF with DWARF \
                 debug info, i.e. built with -g)",
            ));
        }
        let trace_duration = args
            .get("duration_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(3_000)
            .clamp(1, 60_000);
        let trace_clk = args
            .get("clk_hz")
            .and_then(|v| v.as_u64())
            .unwrap_or(170_000_000)
            .clamp(1_000, 500_000_000);
        let trace_baud = args
            .get("baud")
            .and_then(|v| v.as_u64())
            .unwrap_or(2_000_000)
            .clamp(1_000, 25_000_000);
        // Cortex-M symbols may carry the Thumb bit (bit 0) in their address;
        // breakpoints are halfword-aligned, so mask the LSB instead of
        // rejecting the address (a Thumb symbol's LSB is never an address bit).
        let break_addr = if action == "break" {
            Some(address.unwrap() & !1)
        } else {
            None
        };

        let probe_rs_ok = std::process::Command::new("probe-rs")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !probe_rs_ok {
            return Err(ToolError::new(
                "[NotFound] probe-rs is not installed or not on PATH: install it with \
                 `cargo install probe-rs-tools` or download from the probe-rs GitHub Releases",
            ));
        }

        let cwd = ctx.cwd.clone();
        let cancel = Some(ctx.cancel.clone());
        match action {
            "halt" => {
                // Attach resumes the target, so halt it explicitly with the
                // no-argument `break` REPL command.
                let (text, code) = run_probe_rs_retry(
                    debug_args(&chip, probe.as_deref(), &["break"]),
                    &cwd,
                    timeout_ms,
                    cancel.clone(),
                    &[],
                )
                .await
                .map_err(probe_err)?;
                finish(code, "target halted", &text)
            }
            "regs" => {
                let (text, code) = run_probe_rs_retry(
                    debug_args(&chip, probe.as_deref(), &["break", "info reg"]),
                    &cwd,
                    timeout_ms,
                    cancel.clone(),
                    &[],
                )
                .await
                .map_err(probe_err)?;
                finish(code, "registers", &text)
            }
            "mem" => {
                let addr = address.unwrap();
                let (text, code) = run_probe_rs_retry(
                    read_args(&chip, probe.as_deref(), &width, addr, words as usize),
                    &cwd,
                    timeout_ms,
                    cancel.clone(),
                    &[],
                )
                .await
                .map_err(probe_err)?;
                finish(
                    code,
                    &format!("memory {addr:#010x} ({width}, {words} words)"),
                    &text,
                )
            }
            "write" => {
                let addr = address.unwrap();
                let value = args
                    .get("value")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| ToolError::new("[InvalidInput] write needs a value"))?;
                let (text, code) = run_probe_rs_retry(
                    write_args(&chip, probe.as_deref(), &width, addr, value),
                    &cwd,
                    timeout_ms,
                    cancel.clone(),
                    &[],
                )
                .await
                .map_err(probe_err)?;
                finish(
                    code,
                    &format!("wrote {value:#x} to {addr:#010x} ({width})"),
                    &text,
                )
            }
            "analyze" => {
                // The fault-register decode below is Cortex-M specific
                // (CFSR/HFSR/MMFAR/BFAR are CoreSight system addresses at
                // 0xE000EDxx). On Xtensa/RISC-V targets (ESP32*, ...) those
                // reads return garbage — say so instead of analyzing noise.
                if let Some(reason) = crate::decode::non_arm_reason(&chip, elf.as_deref()) {
                    return Err(ToolError::new(format!(
                        "[InvalidInput] debug analyze decodes Cortex-M fault registers \
                         (CFSR/HFSR/MMFAR/BFAR), but {reason} — this target has no such \
                         registers. Use `action: halt` + `regs` + `backtrace` instead, and \
                         read the target's own panic/fault report over the console (e.g. \
                         ESP-IDF panic backtrace via `monitor`)."
                    )));
                }
                let (regs_text, regs_code) = run_probe_rs_retry(
                    debug_args(&chip, probe.as_deref(), &["break", "info reg"]),
                    &cwd,
                    timeout_ms,
                    cancel.clone(),
                    &[],
                )
                .await
                .map_err(probe_err)?;
                if regs_code != Some(0) {
                    return Err(ToolError::new(format!(
                        "[Io] debug attach/halt/regs failed (exit {:?})\n{regs_text}",
                        regs_code
                    )));
                }
                let (fault_text, fault_code) = run_probe_rs_retry(
                    read_args(&chip, probe.as_deref(), "b32", 0xE000_ED28, 5),
                    &cwd,
                    timeout_ms,
                    cancel.clone(),
                    &[],
                )
                .await
                .map_err(probe_err)?;
                let cfsr_words = if fault_code == Some(0) {
                    parse_hex_words(&fault_text)
                } else {
                    Vec::new()
                };
                let sp = parse_regs(&regs_text)
                    .get("sp")
                    .copied()
                    .or_else(|| parse_regs(&regs_text).get("r13").copied());
                let (stack_words, stack_addr) = if let Some(sp) = sp {
                    match run_probe_rs_retry(
                        read_args(&chip, probe.as_deref(), "b32", sp, 8),
                        &cwd,
                        timeout_ms,
                        cancel.clone(),
                        &[],
                    )
                    .await
                    {
                        Ok((t, Some(0))) => (parse_hex_words(&t), sp),
                        _ => (Vec::new(), sp),
                    }
                } else {
                    (Vec::new(), 0)
                };
                let report = analysis_report(
                    &chip,
                    &regs_text,
                    &cfsr_words,
                    &stack_words,
                    stack_addr,
                    elf.as_deref(),
                );
                Ok(ToolOutput {
                    text: truncate(&report, 32_000),
                })
            }
            "break" => {
                let addr = break_addr.unwrap();
                let break_cmd = format!("break *{addr:#x}");
                // Halt first: `c` (continue) is rejected by probe-rs while the
                // target is running ("The target is running. Only the 'break',
                // 'help' or 'quit' commands are allowed"), and an attach
                // session resumes the target. `break` with no argument halts;
                // then set the breakpoint, run until it hits, and read regs.
                let cmds: [&str; 4] = ["break", break_cmd.as_str(), "c", "info reg"];
                let (text, code) = run_probe_rs_retry(
                    debug_args(&chip, probe.as_deref(), &cmds),
                    &cwd,
                    timeout_ms,
                    cancel.clone(),
                    &[],
                )
                .await
                .map_err(probe_err)?;
                finish(
                    code,
                    &format!("breakpoint at {addr:#010x} (reported registers after hit)"),
                    &text,
                )
            }
            "step" => {
                let (text, code) = run_probe_rs_retry(
                    debug_args(&chip, probe.as_deref(), &["break", "step", "info reg"]),
                    &cwd,
                    timeout_ms,
                    cancel.clone(),
                    &[],
                )
                .await
                .map_err(probe_err)?;
                finish(code, "single step (registers after step)", &text)
            }
            "continue" => {
                let (text, code) = run_probe_rs_retry(
                    debug_args(&chip, probe.as_deref(), &["c"]),
                    &cwd,
                    timeout_ms,
                    cancel.clone(),
                    &[],
                )
                .await
                .map_err(probe_err)?;
                // probe-rs `quit` disconnects by suspending the target, so a
                // resumed target is halted again as soon as this session ends;
                // say so instead of claiming it keeps running.
                finish(
                    code,
                    "resumed execution (the target is re-halted when this debug session disconnects)",
                    &text,
                )
            }
            "backtrace" => {
                let elf_path = elf.unwrap();
                let (text, code) = run_probe_rs_retry(
                    debug_elf_args(
                        &chip,
                        probe.as_deref(),
                        &elf_path.display().to_string(),
                        &["break", "bt"],
                    ),
                    &cwd,
                    timeout_ms,
                    cancel.clone(),
                    &[],
                )
                .await
                .map_err(probe_err)?;
                match code {
                    Some(0) => Ok(ToolOutput {
                        text: truncate(
                            &format!("debug backtrace (exit 0)\n{}", backtrace_hint(&text)),
                            32_000,
                        ),
                    }),
                    Some(c) => Err(ToolError::new(format!(
                        "[Io] probe-rs failed (exit {c})\n{text}"
                    ))),
                    None => Err(ToolError::new(format!(
                        "[Timeout] probe-rs was killed\n{text}"
                    ))),
                }
            }
            "trace" => {
                // probe-rs 0.32 checks the capture duration inside the packet
                // loop, so with no ITM traffic the command never exits on its
                // own — the outer timeout ending the session is therefore the
                // NORMAL completion path, not an error. No retry either: a
                // probe that cannot receive SWO will just fail again.
                //
                // `itm swo` has no --chip/--probe options; pass them via the
                // PROBE_RS_CHIP / PROBE_RS_PROBE env vars (honoured by the
                // subcommand, verified against probe-rs 0.32).
                let outer = timeout_ms.max(trace_duration + 5_000);
                let mut envs = vec![("PROBE_RS_CHIP".to_string(), chip.clone())];
                if let Some(probe) = probe.as_deref() {
                    envs.push(("PROBE_RS_PROBE".to_string(), probe.to_string()));
                }
                let result = run_probe_rs(
                    itm_swo_args(trace_duration, trace_clk, trace_baud),
                    &cwd,
                    outer,
                    Some(ctx.cancel.clone()),
                    &envs,
                )
                .await;
                let (text, code) = match result {
                    Ok(v) => v,
                    // Timeout on an idle SWO stream = the capture window
                    // ended; treat it as a normal (empty) capture.
                    Err(e) if e.contains("[Timeout]") => (String::new(), None),
                    Err(e) => return Err(probe_err(e)),
                };
                match code {
                    Some(0) => {
                        let summary = if text.trim().is_empty() {
                            "no ITM packets captured (firmware must write ITM ports, e.g. \
                             ITM_SendChar, for data to appear)\n"
                        } else {
                            ""
                        };
                        Ok(ToolOutput {
                            text: truncate(
                                &format!(
                                    "debug trace captured {trace_duration} ms of SWO/ITM \
                                     (clk {trace_clk} Hz, baud {trace_baud}) (exit 0)\n{summary}{text}"
                                ),
                                32_000,
                            ),
                        })
                    }
                    None => Ok(ToolOutput {
                        text: truncate(
                            &format!(
                                "debug trace: capture window closed after {outer} ms on an idle \
                                 SWO stream (no ITM packets — firmware must write ITM ports, \
                                 e.g. ITM_SendChar, for data to appear)\n{text}"
                            ),
                            32_000,
                        ),
                    }),
                    Some(c) => Err(ToolError::new(format!(
                        "[Io] probe-rs trace failed (exit {c}) — the probe may not support SWO \
                         reception, or the chip/clk/baud combination is wrong\n{text}"
                    ))),
                }
            }
            _ => Err(ToolError::new(format!(
                "[InvalidInput] unknown action '{action}'"
            ))),
        }
    }
}

/// Map probe-rs invocation errors (missing binary, stuck ST-Link session on
/// Windows/WinUSB, generic) to tagged tool errors with actionable guidance.
fn probe_err(e: String) -> ToolError {
    super::util::probe_rs_err(e)
}

/// Wrap a probe-rs exit code into the tool result text.
fn finish(code: Option<i32>, what: &str, text: &str) -> Result<ToolOutput, ToolError> {
    match code {
        Some(0) => Ok(ToolOutput {
            text: format!("debug {what} (exit 0)\n{text}"),
        }),
        Some(c) => Err(ToolError::new(format!(
            "[Io] probe-rs failed (exit {c})\n{text}"
        ))),
        None => Err(ToolError::new(format!(
            "[Timeout] probe-rs was killed\n{text}"
        ))),
    }
}

/// Run a probe-rs session, retrying once after a short pause on a non-zero
/// exit. Rapid consecutive debug calls can hit the probe while it is still
/// releasing the previous session ("probe busy / failed to open"), and a
/// single retry resolves most of these without masking genuine errors.
/// Timeouts (code `None`) are NOT retried — a hung session (e.g. a breakpoint
/// that never hits) would just burn the timeout twice.
async fn run_probe_rs_retry(
    args: Vec<String>,
    cwd: &Path,
    timeout_ms: u64,
    cancel: Option<firment_core::Cancellable>,
    envs: &[(String, String)],
) -> Result<(String, Option<i32>), String> {
    let (text, code) = run_probe_rs(args.clone(), cwd, timeout_ms, cancel.clone(), envs).await?;
    match code {
        Some(0) => Ok((text, Some(0))),
        Some(_) => {
            tokio::time::sleep(Duration::from_millis(2000)).await;
            let (text2, code2) = run_probe_rs(args, cwd, timeout_ms, cancel, envs).await?;
            let note = "\n(probe-rs exited non-zero; retried once after 2 s)";
            Ok((format!("{text}{note}\n{text2}"), code2))
        }
        None => Ok((text, None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn parse_address_hex_and_symbol_forms() {
        let elf = Path::new("C:/definitely/missing.elf");
        assert_eq!(parse_address("0x08001234", None).unwrap(), 0x0800_1234);
        assert_eq!(parse_address("0X2000ABCD", None).unwrap(), 0x2000_ABCD);
        assert!(parse_address("0x", None).is_err());
        assert!(parse_address("2000", None).is_err());
        assert!(parse_address("0xZZ", None).is_err());
        let err = parse_address("symbol:main", Some(elf)).unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
        assert!(parse_address("symbol:main", None).is_err());
    }

    #[test]
    fn break_address_masks_thumb_lsb() {
        // A Thumb symbol value may carry bit 0; the breakpoint is placed at
        // the halfword-aligned address, not rejected.
        assert_eq!(0x0800_1234 & !1, 0x0800_1234);
        assert_eq!(0x0800_1235 & !1, 0x0800_1234);
        assert_eq!(0x0800_1234 & !1, 0x0800_1234);
    }

    #[test]
    fn parse_width_accepts_valid_forms() {
        assert_eq!(parse_width("b8").unwrap(), "b8");
        assert_eq!(parse_width("b64").unwrap(), "b64");
        assert!(parse_width("b128").is_err());
        assert!(parse_width("word").is_err());
    }

    #[test]
    fn parse_regs_handles_common_probe_rs_layouts() {
        let text = "\
r0 = 0x00000001
r1 (a1) = 0x08004567
sp (r13) = 0x20001abc
lr (r14) = 0x08001234
pc (r15) = 0x08005678
xpsr = 0x01000000
";
        let regs = parse_regs(text);
        assert_eq!(reg_value(&regs, &["pc", "r15"]), Some(0x0800_5678));
        assert_eq!(reg_value(&regs, &["lr", "r14"]), Some(0x0800_1234));
        assert_eq!(reg_value(&regs, &["sp", "r13"]), Some(0x2000_1abc));
        assert_eq!(reg_value(&regs, &["xpsr"]), Some(0x0100_0000));

        let alt = parse_regs("sp: 0x20000000\npc  0x08000008\n");
        assert_eq!(reg_value(&alt, &["pc", "r15"]), Some(0x0800_0008));
        assert_eq!(reg_value(&alt, &["sp", "r13"]), Some(0x2000_0000));

        assert!(parse_regs("no registers here").is_empty());
    }

    #[test]
    fn parse_regs_handles_probe_rs_032_reg_table_format() {
        // probe-rs 0.32 `reg` prints a width-80 table:
        // "R0: 0x00000001  R1: 0x00000002  ...  R15/PC: 0x08005678  XPSR/PSR: 0x01000000"
        let text = "\
R0: 0x00000001  R1: 0x00000002  R2: 0x00000003  R3: 0x00000004
R4: 0x00000005  R5: 0x00000006  R6: 0x00000007  R7: 0x00000008
R8: 0x00000009  R9: 0x0000000a  R10: 0x0000000b  R11: 0x0000000c
R12: 0x0000000d  R13/SP: 0x20001abc  R14/RA: 0x08001234  R15/PC: 0x08005678
XPSR/PSR: 0x01000000
";
        let regs = parse_regs(text);
        assert_eq!(reg_value(&regs, &["pc", "r15"]), Some(0x0800_5678));
        assert_eq!(reg_value(&regs, &["lr", "r14"]), Some(0x0800_1234));
        assert_eq!(reg_value(&regs, &["sp", "r13"]), Some(0x2000_1abc));
        assert_eq!(reg_value(&regs, &["xpsr"]), Some(0x0100_0000));
        assert_eq!(reg_value(&regs, &["r0"]), Some(1));
    }

    #[test]
    fn parse_regs_survives_ansi_escapes() {
        // Some probe-rs console output is ANSI-coloured even when piped.
        let text = "\x1b[1m\x1b[32mR15/PC:\x1b[0m \x1b[33m0x08005678\x1b[0m\n\x1b[32mR13/SP:\x1b[0m \x1b[33m0x20001abc\x1b[0m\n";
        let regs = parse_regs(text);
        assert_eq!(reg_value(&regs, &["pc", "r15"]), Some(0x0800_5678));
        assert_eq!(reg_value(&regs, &["sp", "r13"]), Some(0x2000_1abc));
    }

    #[test]
    fn parse_hex_words_collects_simple_hex_output() {
        let words = parse_hex_words("00010000 40000000 00000000 00000000 00000000");
        assert_eq!(words.len(), 5);
        assert_eq!(words[0], 0x0001_0000);
        assert_eq!(words[1], 0x4000_0000);
        let with_prefix = parse_hex_words("0x00010000 0x40000000");
        assert_eq!(with_prefix.len(), 2);
    }

    #[test]
    fn cfsr_analysis_flags_and_valid_addresses() {
        // UNDEFINSTR (bit 16) + FORCED + valid BFAR.
        let lines = cfsr_analysis((1 << 16) | (1 << 14), 0x4000_0000, 0, 0x0800_AAAA);
        let joined = lines.join("\n");
        assert!(joined.contains("[UNDEFINSTR]"), "got: {joined}");
        assert!(joined.contains("[FORCED]"), "got: {joined}");
        assert!(joined.contains("BFAR = 0x0800aaaa (valid"), "got: {joined}");
        assert!(
            joined.contains("MMFAR = 0x00000000 (not valid)"),
            "got: {joined}"
        );

        // IBUSERR with BFAR not valid.
        let lines = cfsr_analysis(1 << 8, 0, 0, 0);
        let joined = lines.join("\n");
        assert!(joined.contains("[IBUSERR]"), "got: {joined}");
        assert!(
            joined.contains("BFAR = 0x00000000 (not valid)"),
            "got: {joined}"
        );

        // Zero flags.
        let lines = cfsr_analysis(0, 0, 0, 0);
        let joined = lines.join("\n");
        assert!(
            joined.contains("no configurable fault flags"),
            "got: {joined}"
        );
        assert!(joined.contains("no hard fault flags"), "got: {joined}");
    }

    #[test]
    fn debug_and_read_args_are_ordered() {
        // A session must halt the target first (attach resumes it), then read
        // the register table via `info reg` (there is no bare `reg` command).
        let args = debug_args("stm32g431rb", Some("P1"), &["break", "info reg"]);
        assert_eq!(
            args,
            vec![
                "debug",
                "--chip",
                "stm32g431rb",
                "--probe",
                "P1",
                "-c",
                "break",
                "-c",
                "info reg",
                "-c",
                "quit",
            ]
        );
        let break_args = debug_args("stm32g431rb", None, &["break *0x08001234", "c", "info reg"]);
        assert_eq!(
            break_args,
            vec![
                "debug",
                "--chip",
                "stm32g431rb",
                "-c",
                "break *0x08001234",
                "-c",
                "c",
                "-c",
                "info reg",
                "-c",
                "quit",
            ]
        );
        let args = read_args("stm32f407vetx", None, "b32", 0xE000_ED28, 5);
        assert_eq!(
            args,
            vec![
                "read",
                "--chip",
                "stm32f407vetx",
                "b32",
                "0xe000ed28",
                "5",
                "-f",
                "simple-hex",
            ]
        );
    }

    #[test]
    fn analysis_report_decodes_pc_and_faults() {
        let regs_text = "pc (r15) = 0x08005678\nlr (r14) = 0x08001234\nsp (r13) = 0x20001abc\nxpsr = 0x01000000\n";
        let cfsr = vec![1 << 16, 0x4000_0000, 0, 0, 0]; // CFSR, HFSR, DFSR, MMFAR, BFAR
        let stack = vec![0xdead_beef, 0x0800_1234, 0, 0, 0, 0, 0, 0];
        let report = analysis_report("stm32g431rb", regs_text, &cfsr, &stack, 0x2000_1abc, None);
        assert!(report.contains("chip stm32g431rb"), "got: {report}");
        assert!(report.contains("pc  : 0x08005678"), "got: {report}");
        assert!(report.contains("sp  : 0x20001abc"), "got: {report}");
        assert!(report.contains("[UNDEFINSTR]"), "got: {report}");
        assert!(
            report.contains("stack (sp=0x20001abc, 8 words)"),
            "got: {report}"
        );
        assert!(report.contains("0x20001abc: 0xdeadbeef"), "got: {report}");
    }

    fn ctx(dir: &Path) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            permission: Arc::new(firment_core::AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(std::sync::Mutex::new(firment_core::EditJournal::new(
                dir.join("undo"),
            ))),
            verify_command: None,
            symbols_backend: None,
            build_command: None,
            default_chip: Some("stm32g431rb".to_string()),
            monitor_port: None,
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        }
    }

    #[tokio::test]
    async fn missing_chip_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = ctx(dir.path());
        c.default_chip = None;
        let err = Debug.run(json!({"action": "regs"}), &c).await.unwrap_err();
        assert!(err.message.contains("default_chip"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn unknown_action_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let err = Debug
            .run(json!({"action": "frobnicate"}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("unknown action"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn bad_address_is_rejected_before_probe_contact() {
        let dir = tempfile::tempdir().unwrap();
        let err = Debug
            .run(
                json!({"action": "mem", "address": "not-an-address"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("0x"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn write_value_range_is_checked() {
        let dir = tempfile::tempdir().unwrap();
        let err = Debug
            .run(
                json!({"action": "write", "address": "0x20000000", "value": 4294967296u64, "width": "b32"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("does not fit"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn backtrace_requires_elf() {
        let dir = tempfile::tempdir().unwrap();
        let err = Debug
            .run(json!({"action": "backtrace"}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("backtrace needs an elf"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn debug_elf_args_orders_elf_before_chip_flags() {
        let args = debug_elf_args("stm32g431rb", None, "C:/fw/fw.elf", &["break", "bt"]);
        assert_eq!(&args[0..2], &["debug", "C:/fw/fw.elf"]);
        assert!(args.contains(&"--chip".to_string()));
        assert!(args.contains(&"stm32g431rb".to_string()));
        assert!(args.windows(2).any(|w| w == ["-c", "bt"]), "got: {args:?}");
        assert!(
            args.windows(2).any(|w| w == ["-c", "quit"]),
            "got: {args:?}"
        );
    }

    #[test]
    fn itm_swo_args_carry_duration_clock_and_baud() {
        // `itm swo` takes no --chip/--probe/--non-interactive (probe-rs 0.32
        // rejects them); the chip/probe travel via PROBE_RS_CHIP/PROBE_RS_PROBE.
        let args = itm_swo_args(2500, 170_000_000, 2_000_000);
        assert_eq!(&args[0..2], &["itm", "swo"]);
        assert!(args.contains(&"2500".to_string()), "got: {args:?}");
        assert!(args.contains(&"170000000".to_string()), "got: {args:?}");
        assert!(args.contains(&"2000000".to_string()), "got: {args:?}");
        assert_eq!(args.len(), 5, "no option flags may be passed: {args:?}");
    }

    #[test]
    fn backtrace_hint_detects_missing_dwarf() {
        assert!(backtrace_hint("    Frame 1: main @ 0x8001234").contains("Frame 1"));
        let empty = backtrace_hint("");
        assert!(empty.contains("no DWARF debug info"), "got: {empty}");
        let noisy = backtrace_hint("probe-rs: some warning\n");
        assert!(noisy.contains("no DWARF debug info"), "got: {noisy}");
    }
}
