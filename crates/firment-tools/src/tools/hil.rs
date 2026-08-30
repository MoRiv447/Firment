#![allow(clippy::collapsible_if)]
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Suite config structures (TOML + JSON inline steps)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
struct HilSuite {
    chip: Option<String>,
    port: Option<String>,
    probe: Option<String>,
    elf: Option<String>,
    baud: Option<u32>,
    #[serde(default)]
    steps: Vec<HilStep>,
}

// deny_unknown_fields: a typo'd expectation key (`expect_contins`) would
// otherwise be silently dropped and the step trivially PASS without
// verifying anything.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct HilStep {
    kind: String,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    elf: Option<String>,
    #[serde(default)]
    chip: Option<String>,
    #[serde(default)]
    probe: Option<String>,
    #[serde(default)]
    port: Option<String>,
    #[serde(default)]
    baud: Option<u32>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    reset: Option<bool>,
    #[serde(default)]
    expect_contains: Option<String>,
    #[serde(default)]
    expect_regex: Option<String>,
    #[serde(default)]
    expect_count: Option<usize>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    autodetect: Option<bool>,
    #[serde(default)]
    clk_hz: Option<u64>,
    // observe step: what to measure on the frame, where, and the verdict
    // to assert. See tools/observe.rs for the analysis.
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    roi: Option<[u32; 4]>,
    #[serde(default)]
    threshold: Option<u8>,
    #[serde(default)]
    expect_lit: Option<bool>,
    #[serde(default)]
    save: Option<bool>,
    // observe sequence modes (motion / blink): frames in capture order.
    #[serde(default)]
    paths: Option<Vec<String>>,
    // observe diff: the frame taken after the change.
    #[serde(default)]
    after: Option<String>,
    // observe blink: assumed gap between burst shots — the user knows their
    // burst rate, we do not. Required before any frequency can be claimed.
    #[serde(default)]
    interval_ms: Option<u64>,
    #[serde(default)]
    expect_blinking: Option<bool>,
    #[serde(default)]
    expect_blink_hz: Option<f64>,
    #[serde(default)]
    expect_motion: Option<bool>,
    #[serde(default)]
    expect_diff: Option<bool>,
    // allow `expect` object form: { contains, regex, count }
    #[serde(default)]
    expect: Option<HilExpect>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct HilExpect {
    #[serde(default)]
    contains: Option<String>,
    #[serde(default)]
    regex: Option<String>,
    #[serde(default)]
    count: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct HilFile {
    suite: HashMap<String, HilSuite>,
}

// ---------------------------------------------------------------------------
// Hil tool
// ---------------------------------------------------------------------------

pub struct Hil;

#[async_trait]
impl Tool for Hil {
    fn name(&self) -> &'static str {
        "hil"
    }

    fn description(&self) -> &'static str {
        "Hardware-in-the-loop suite: orchestrated build → flash → monitor/trace (with expectations) → elf_analyze, with replay. Prefer this over calling build/flash/monitor separately for firmware verification. Suites live in .firment/hil.toml; inline steps work too. Supports dry-run and replay. flash/run/elf steps auto-infer .pio/build/*/firmware.elf when file/elf is omitted."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "suite": {"type": "string", "description": "Suite name defined in .firment/hil.toml"},
                "steps": {"type": "array", "description": "Inline steps [{kind, file, elf, chip, probe, port, baud, clk_hz, timeout_ms, expect_contains, expect_regex, expect_count, duration_ms}] — kinds: build/flash/run/monitor/trace/observe/elf_analyze/delay; flash/run/elf auto-infer .pio/build/*/firmware.elf when elf omitted"},
                "chip": {"type": "string", "description": "Override chip for flash/run/trace steps"},
                "port": {"type": "string", "description": "Override serial port for monitor steps; 'auto' picks first detected port"},
                "probe": {"type": "string", "description": "Override probe id"},
                "elf": {"type": "string", "description": "Override ELF path"},
                "timeout_ms": {"type": "integer", "minimum": 1, "description": "Total suite timeout (default 180000)"},
                "dry_run": {"type": "boolean", "default": false, "description": "Simulate hardware steps without touching probe/serial"},
                "replay": {"type": "string", "description": "Replay a previous run by id, or 'list' to list replays"},
                "list_suites": {"type": "boolean", "default": false, "description": "List suites defined in .firment/hil.toml"}
            }
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        if args.get("replay").is_some()
            || args
                .get("list_suites")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        {
            return None;
        }
        let dry = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if dry {
            Some("run HIL suite (dry-run, no hardware)".to_string())
        } else {
            Some("⚠ run HIL suite: build/flash/monitor hardware".to_string())
        }
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        // list_suites shortcut
        if args
            .get("list_suites")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Ok(ToolOutput {
                text: list_suites(&ctx.cwd),
            });
        }
        // replay handling
        if let Some(replay) = args.get("replay").and_then(|v| v.as_str()) {
            return handle_replay(replay, ctx);
        }

        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let total_timeout = args
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(180_000);
        let suite_name = args
            .get("suite")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let chip_override = args
            .get("chip")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let port_override = args
            .get("port")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let probe_override = args
            .get("probe")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let elf_override = args
            .get("elf")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Resolve steps
        let (suite, steps) = resolve_steps(&args, &ctx.cwd, suite_name.as_deref())?;

        let suite_defaults = suite.unwrap_or_default();
        let mut resolved_steps: Vec<ResolvedStep> = Vec::new();
        for mut step in steps {
            // merge suite defaults into step
            if step.chip.is_none() {
                step.chip = suite_defaults.chip.clone().or(chip_override.clone());
            } else if chip_override.is_some() {
                step.chip = chip_override.clone();
            }
            if step.port.is_none() {
                step.port = suite_defaults.port.clone().or(port_override.clone());
            } else if port_override.is_some() {
                step.port = port_override.clone();
            }
            if step.probe.is_none() {
                step.probe = suite_defaults.probe.clone().or(probe_override.clone());
            }
            if step.elf.is_none() && step.file.is_none() {
                // flash/run/elf/trace steps may use suite elf or auto-infer .pio/build
                if matches!(
                    step.kind.as_str(),
                    "flash" | "run" | "elf_analyze" | "elf" | "trace" | "debug"
                ) {
                    if let Some(inferred) = suite_defaults.elf.clone().or(elf_override.clone()) {
                        step.elf = Some(inferred.clone());
                        step.file = Some(inferred);
                    } else if let Some(auto) = infer_firmware_elf(&ctx.cwd) {
                        let s = auto.to_string_lossy().to_string();
                        step.elf = Some(s.clone());
                        step.file = Some(s);
                    }
                }
            }
            if step.baud.is_none() {
                step.baud = suite_defaults.baud;
            }
            if step.clk_hz.is_none() {
                // default 170 MHz is handled in run_trace_step, no need to fill
            }
            // normalize expect object form into flat fields
            if let Some(exp) = step.expect.take() {
                if step.expect_contains.is_none() {
                    step.expect_contains = exp.contains;
                }
                if step.expect_regex.is_none() {
                    step.expect_regex = exp.regex;
                }
                if step.expect_count.is_none() {
                    step.expect_count = exp.count;
                }
            }
            // also accept generic `expect` string in inline JSON as contains shorthand
            resolved_steps.push(ResolvedStep { inner: step });
        }

        if resolved_steps.is_empty() {
            return Err(ToolError::new(
                "[InvalidInput] hil: no steps resolved; provide suite or steps",
            ));
        }

        // Prepare replay file
        let replay_id = uuid::Uuid::new_v4().to_string();
        let replay_path = replay_path_for(ctx, &replay_id);
        if let Some(parent) = replay_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let suite_label = suite_name.unwrap_or_else(|| "inline".to_string());
        let mut output_sections: Vec<String> = Vec::new();
        let mut overall_ok = true;
        let mut failed_expect = false;
        let overall_start = Instant::now();

        output_sections.push(format!("hil suite: {suite_label} (dry_run={dry_run})"));
        output_sections.push(format!("steps: {}", resolved_steps.len()));
        if dry_run {
            output_sections.push("mode: dry-run — hardware steps simulated".to_string());
        }

        for (idx, step) in resolved_steps.iter().enumerate() {
            if ctx.cancel.is_cancelled() {
                let msg = "(hil interrupted by turn cancellation)".to_string();
                write_replay_line(&replay_path, idx, &step.inner.kind, false, &msg, 0);
                output_sections.push(format!(
                    "\n── step {}/{}: {} ──\n{msg}",
                    idx + 1,
                    resolved_steps.len(),
                    step.inner.kind
                ));
                overall_ok = false;
                break;
            }
            if overall_start.elapsed().as_millis() as u64 > total_timeout {
                let msg = format!(
                    "[Timeout] hil suite timed out after {total_timeout} ms (total budget)"
                );
                write_replay_line(&replay_path, idx, &step.inner.kind, false, &msg, 0);
                output_sections.push(format!(
                    "\n── step {}/{}: {} ──\n{msg}",
                    idx + 1,
                    resolved_steps.len(),
                    step.inner.kind
                ));
                overall_ok = false;
                break;
            }

            let step_start = Instant::now();
            let kind = step.inner.kind.as_str();
            let remaining =
                total_timeout.saturating_sub(overall_start.elapsed().as_millis() as u64);

            let result: Result<String, String> = match kind {
                "build" => run_build_step(&step.inner, ctx, dry_run, remaining).await,
                "flash" => run_flash_step(&step.inner, ctx, dry_run, remaining).await,
                "run" => run_run_step(&step.inner, ctx, dry_run, remaining).await,
                "monitor" => run_monitor_step(&step.inner, ctx, dry_run, remaining).await,
                "trace" => run_trace_step(&step.inner, ctx, dry_run, remaining).await,
                "observe" => run_observe_step(&step.inner, ctx, dry_run, remaining).await,
                "elf_analyze" | "elf" => run_elf_step(&step.inner, ctx, remaining).await,
                "delay" | "sleep" => {
                    let ms = step
                        .inner
                        .duration_ms
                        .or(step.inner.timeout_ms)
                        .unwrap_or(1000)
                        // Respect the suite's total-time budget: a typo'd
                        // duration_ms must not hang past total_timeout.
                        .min(remaining.max(1));
                    if dry_run {
                        tokio::time::sleep(Duration::from_millis(ms.min(200))).await;
                    } else {
                        // Honor turn cancellation mid-sleep.
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_millis(ms)) => {}
                            _ = ctx.cancel.cancelled() => {
                                return Err(ToolError::new(
                                    "[Cancelled] hil suite interrupted during delay step",
                                ));
                            }
                        }
                    }
                    Ok(format!("delay {ms} ms"))
                }
                _ => Err(format!(
                    "[InvalidInput] unknown hil step kind: {kind} (expected build/flash/run/monitor/trace/observe/elf_analyze/delay)"
                )),
            };

            let elapsed = step_start.elapsed().as_millis() as u64;
            match result {
                Ok(text) => {
                    // Check monitor expectations: run_monitor_step returns text but also embeds
                    // [HIL_EXPECT: ...] marker when expectations fail; we detect it to mark overall fail
                    let expect_failed = text.contains("[HIL_EXPECT:FAIL]");
                    if expect_failed {
                        failed_expect = true;
                        overall_ok = false;
                    }
                    write_replay_line(&replay_path, idx, kind, !expect_failed, &text, elapsed);
                    output_sections.push(format!(
                        "\n── step {}/{}: {kind} ── ({} ms)\n{text}",
                        idx + 1,
                        resolved_steps.len(),
                        elapsed
                    ));
                    if expect_failed {
                        // continue to next steps (e.g. elf_analyze) even after expect failure, but mark suite fail
                    }
                }
                Err(e) => {
                    write_replay_line(&replay_path, idx, kind, false, &e, elapsed);
                    output_sections.push(format!(
                        "\n── step {}/{}: {kind} ── ({} ms)\n{e}",
                        idx + 1,
                        resolved_steps.len(),
                        elapsed
                    ));
                    overall_ok = false;
                    // Hard failures stop the suite (build/flash/run timeouts, etc.)
                    // Monitor expect failures already handled as Ok with marker, so this is hard Io
                    break;
                }
            }
        }

        let total_elapsed = overall_start.elapsed().as_millis() as u64;
        let status = if overall_ok { "PASS" } else { "FAIL" };
        // Evidence tag (verification ladder in the system prompt): the
        // highest level this suite ATTEMPTED, so a build-only suite is not
        // mistaken for hardware validation.
        let evidence = if dry_run {
            "dry-run — nothing was executed".to_string()
        } else {
            let mut level = 1u8;
            let mut name = "code";
            for s in &resolved_steps {
                let (l, n) = match s.inner.kind.as_str() {
                    "build" => (2, "build"),
                    "flash" => (3, "deploy"),
                    "run" | "monitor" => (4, "runtime"),
                    // SWO/ITM is RUNTIME observability, not physical
                    // behavior — level 5 belongs to the observe step.
                    "trace" => (4, "runtime"),
                    "observe" => (5, "physical"),
                    _ => continue, // elf_analyze / delay do not advance the ladder
                };
                if l > level {
                    level = l;
                    name = n;
                }
            }
            format!("reached level {level} ({name})")
        };
        let mut footer = format!(
            "\nhil: {status} suite={suite_label} in {total_elapsed} ms — replay: {replay_id}"
        );
        if failed_expect {
            footer.push_str(" (expectations not met)");
        }
        footer.push_str(&format!("\nevidence: {evidence}"));
        footer.push_str(&format!("\nreplay file: {}", replay_path.display()));
        footer.push_str("\nreplay: hil replay <id>  |  list: hil replay list");
        output_sections.push(footer);

        // Append suite footer to replay file as meta line
        let meta = json!({
            "meta": true,
            "suite": suite_label,
            "status": status,
            "dry_run": dry_run,
            "elapsed_ms": total_elapsed,
            "replay_id": replay_id,
        });
        if let Ok(line) = serde_json::to_string(&meta) {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&replay_path)
                .and_then(|mut f| {
                    use std::io::Write;
                    writeln!(f, "{line}")
                });
        }
        let text = output_sections.join("\n");
        let truncated = crate::tools::util::truncate(&text, 64_000);
        if overall_ok {
            Ok(ToolOutput { text: truncated })
        } else {
            // Return as error so the agent sees [Io]/[Timeout] semantics, but include full log
            Err(ToolError::new(truncated))
        }
    }
}

struct ResolvedStep {
    inner: HilStep,
}

fn replay_path_for(ctx: &ToolContext, id: &str) -> PathBuf {
    let base = ctx
        .session_dir
        .clone()
        .unwrap_or_else(|| ctx.cwd.join(".firment").join("work"));
    base.join("hil").join(format!("{id}.jsonl"))
}

fn write_replay_line(path: &Path, idx: usize, kind: &str, ok: bool, text: &str, elapsed_ms: u64) {
    let line = json!({
        "step": idx + 1,
        "kind": kind,
        "ok": ok,
        "elapsed_ms": elapsed_ms,
        "text": text,
    });
    if let Ok(s) = serde_json::to_string(&line) {
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut f| {
                use std::io::Write;
                writeln!(f, "{s}")
            });
    }
}

fn list_suites(cwd: &Path) -> String {
    match find_hil_file(cwd) {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<HilFile>(&text) {
                Ok(file) => {
                    if file.suite.is_empty() {
                        format!("hil: no suites in {}", path.display())
                    } else {
                        let mut out = format!("hil suites in {}:\n", path.display());
                        for (name, suite) in &file.suite {
                            out.push_str(&format!(
                                "  - {name}: {} steps, chip={}, port={}\n",
                                suite.steps.len(),
                                suite.chip.as_deref().unwrap_or("-"),
                                suite.port.as_deref().unwrap_or("-")
                            ));
                            for (i, s) in suite.steps.iter().enumerate() {
                                out.push_str(&format!("      {}. {}\n", i + 1, s.kind));
                            }
                        }
                        out.push_str("\nrun: hil suite=<name>  |  dry: hil suite=<name> dry_run=true");
                        out
                    }
                }
                Err(e) => format!("[InvalidInput] hil.toml parse error {}: {e}", path.display()),
            },
            Err(e) => format!("[Io] cannot read {}: {e}", path.display()),
        },
        None => "hil: no .firment/hil.toml found (searched cwd and ancestors); create one or pass steps inline".to_string(),
    }
}

fn handle_replay(arg: &str, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
    let base = ctx
        .session_dir
        .clone()
        .unwrap_or_else(|| ctx.cwd.join(".firment").join("work"))
        .join("hil");
    if arg == "list" {
        let mut out = format!("hil replays in {}:\n", base.display());
        let Ok(read) = std::fs::read_dir(&base) else {
            return Ok(ToolOutput {
                text: format!("{out}(no replays yet)"),
            });
        };
        let mut entries: Vec<_> = read.flatten().collect();
        entries.sort_by_key(|e| e.path());
        if entries.is_empty() {
            out.push_str("(no replays yet)");
        } else {
            for e in entries.iter().rev().take(20) {
                let name = e.file_name().to_string_lossy().into_owned();
                let id = name.trim_end_matches(".jsonl");
                // try read meta line
                let meta = std::fs::read_to_string(e.path())
                    .ok()
                    .and_then(|t| t.lines().last().map(|l| l.to_string()))
                    .unwrap_or_default();
                out.push_str(&format!("  {id}  {meta}\n"));
            }
        }
        return Ok(ToolOutput { text: out });
    }
    // Replay ids are UUIDs minted by this tool. Validating the charset is
    // not cosmetic: PathBuf::join with an absolute path REPLACES the base,
    // so an unchecked id would read arbitrary files — and the replay path
    // runs without the approval prompt sibling paths require.
    if !arg
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ToolError::new(format!(
            "[InvalidInput] replay id must be a UUID (alphanumerics and - _ only), got: {arg}"
        )));
    }
    let path = base.join(format!("{arg}.jsonl"));
    if !path.is_file() {
        // also try exact path if user passed full id with .jsonl
        let alt = base.join(arg);
        if alt.is_file() {
            let text = std::fs::read_to_string(&alt).map_err(|e| {
                ToolError::new(format!("[Io] cannot read replay {}: {e}", alt.display()))
            })?;
            return Ok(ToolOutput {
                text: crate::tools::util::truncate(&text, 64_000),
            });
        }
        return Err(ToolError::new(format!(
            "[NotFound] no hil replay {arg} in {}",
            base.display()
        )));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| ToolError::new(format!("[Io] cannot read replay {}: {e}", path.display())))?;
    // Pretty-print JSONL to human readable
    let mut out = format!("hil replay {arg} ({}):\n", path.display());
    for line in text.lines() {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if v.get("meta").is_some() {
                out.push_str(&format!(
                    "\n── meta ──\n{}\n",
                    serde_json::to_string_pretty(&v).unwrap_or(line.to_string())
                ));
            } else {
                let step = v.get("step").and_then(|s| s.as_u64()).unwrap_or(0);
                let kind = v.get("kind").and_then(|k| k.as_str()).unwrap_or("?");
                let ok = v.get("ok").and_then(|b| b.as_bool()).unwrap_or(false);
                let elapsed = v.get("elapsed_ms").and_then(|e| e.as_u64()).unwrap_or(0);
                let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("");
                out.push_str(&format!(
                    "\n── step {step}: {kind} {} ({} ms) ──\n{text}\n",
                    if ok { "✓" } else { "✗" },
                    elapsed
                ));
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    Ok(ToolOutput {
        text: crate::tools::util::truncate(&out, 64_000),
    })
}

fn find_hil_file(cwd: &Path) -> Option<PathBuf> {
    for dir in cwd.ancestors() {
        for name in [".firment/hil.toml", "hil.toml", ".firment/hil.toml"] {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
        // also support .firment/hil.toml via dir
        let p = dir.join(".firment").join("hil.toml");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Auto-infer the most recently built firmware ELF when a flash/run/elf step omits
/// its file. Covers PlatformIO (`.pio/build/<env>/firmware.elf`) and generic
/// `build/**/*.elf` layouts, picking the newest file by mtime.
fn infer_firmware_elf(cwd: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    // PlatformIO: .pio/build/<env>/firmware.elf
    let pio = cwd.join(".pio").join("build");
    if let Ok(read) = std::fs::read_dir(&pio) {
        for entry in read.flatten() {
            let elf = entry.path().join("firmware.elf");
            if elf.is_file() {
                if let Ok(meta) = std::fs::metadata(&elf) {
                    if let Ok(m) = meta.modified() {
                        candidates.push((elf, m));
                    }
                }
            }
        }
    }
    // Generic: build/**/*.elf (depth 2) and cwd/*.elf
    for base in [cwd.join("build"), cwd.to_path_buf()] {
        if let Ok(read) = std::fs::read_dir(&base) {
            for entry in read.flatten() {
                let p = entry.path();
                if p.is_file() && p.extension().is_some_and(|e| e == "elf") {
                    if let Ok(meta) = std::fs::metadata(&p) {
                        if let Ok(m) = meta.modified() {
                            candidates.push((p, m));
                        }
                    }
                } else if p.is_dir() && base != cwd.to_path_buf() {
                    // one level deeper under build/
                    if let Ok(inner) = std::fs::read_dir(&p) {
                        for e2 in inner.flatten() {
                            let p2 = e2.path();
                            if p2.is_file() && p2.extension().is_some_and(|e| e == "elf") {
                                if let Ok(meta) = std::fs::metadata(&p2) {
                                    if let Ok(m) = meta.modified() {
                                        candidates.push((p2, m));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    candidates.sort_by_key(|(_, t)| *t);
    candidates.pop().map(|(p, _)| p)
}

fn resolve_steps(
    args: &Value,
    cwd: &Path,
    suite_name: Option<&str>,
) -> Result<(Option<HilSuite>, Vec<HilStep>), ToolError> {
    // Priority: if suite name given -> load file; else if steps array given -> parse inline; else error
    if let Some(name) = suite_name {
        let path = find_hil_file(cwd).ok_or_else(|| {
            ToolError::new(
                "[NotFound] hil suite requested but no .firment/hil.toml found (searched cwd and ancestors); copy docs/hil-example.toml to .firment/hil.toml or pass inline steps=[{kind:\"build\"}, ...]",
            )
        })?;
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ToolError::new(format!("[Io] cannot read {}: {e}", path.display())))?;
        let file: HilFile = toml::from_str(&text).map_err(|e| {
            ToolError::new(format!(
                "[InvalidInput] hil.toml parse error {}: {e}",
                path.display()
            ))
        })?;
        let suite = file.suite.get(name).cloned().ok_or_else(|| {
            ToolError::new(format!(
                "[NotFound] hil suite '{name}' not in {} (available: {})",
                path.display(),
                file.suite.keys().cloned().collect::<Vec<_>>().join(", ")
            ))
        })?;
        let steps = suite.steps.clone();
        if steps.is_empty() {
            return Err(ToolError::new(format!(
                "[InvalidInput] hil suite '{name}' has no steps in {}",
                path.display()
            )));
        }
        return Ok((Some(suite), steps));
    }

    if let Some(arr) = args.get("steps").and_then(|v| v.as_array()) {
        let mut steps: Vec<HilStep> = Vec::new();
        for (i, v) in arr.iter().enumerate() {
            let step: HilStep = serde_json::from_value(v.clone()).map_err(|e| {
                ToolError::new(format!(
                    "[InvalidInput] hil steps[{i}] parse error: {e} (value: {v})"
                ))
            })?;
            steps.push(step);
        }
        return Ok((None, steps));
    }

    // Also accept inline steps as JSON string? no.

    // Fallback: try to load default suite named "default" or first suite?
    // Instead, error with guidance.
    Err(ToolError::new(
        "[InvalidInput] hil: provide suite (suite=\"blink\" with .firment/hil.toml) or steps ([{kind:\"build\"}, ...])",
    ))
}

// ---------------------------------------------------------------------------
// Step runners
// ---------------------------------------------------------------------------

async fn run_build_step(
    step: &HilStep,
    ctx: &ToolContext,
    dry_run: bool,
    remaining: u64,
) -> Result<String, String> {
    if dry_run {
        return Ok("[dry-run] build simulated (skipped)".to_string());
    }
    let (command, work_dir, note) = match ctx.build_command.clone() {
        Some(cmd) => (cmd, ctx.cwd.clone(), String::new()),
        None => match crate::tools::build::detect_build_command(&ctx.cwd) {
            Some(d) => (d.command, d.work_dir, d.note),
            None => {
                return Err(
                    "[InvalidInput] build not configured and no build system auto-detected (looked for platformio.ini / Makefile / CMakeLists.txt / *.uvprojx)".to_string(),
                )
            }
        },
    };
    if let Some(reason) = crate::tools::shell::dangerous_reason(&command)
        && !ctx.allow_dangerous
    {
        return Err(format!(
            "[Permission] build command blocked by dangerous-command guard ({reason}); refusing: {command}"
        ));
    }
    let timeout = step.timeout_ms.unwrap_or(600_000).min(remaining.max(1000));
    let (text, code) =
        crate::tools::util::run_command(&command, &work_dir, timeout, None, Some(&ctx.cancel))
            .await
            .map_err(|e| format!("[Io] build spawn failed: {e}"))?;
    match code {
        Some(0) => Ok(format!("{note}build passed (exit 0)\n{text}")),
        Some(c) => Err(format!(
            "[CompileError] build failed (exit {c})\n{note}{text}"
        )),
        None => Err(format!(
            "[Timeout] build timed out after {timeout} ms\n{note}{text}"
        )),
    }
}

async fn run_flash_step(
    step: &HilStep,
    ctx: &ToolContext,
    dry_run: bool,
    remaining: u64,
) -> Result<String, String> {
    if dry_run {
        let elf = step.elf.as_deref().or(step.file.as_deref()).unwrap_or("?");
        return Ok(format!("[dry-run] flash simulated: {elf}"));
    }
    let file = step.elf.as_deref().or(step.file.as_deref()).ok_or_else(|| {
        "[InvalidInput] flash step requires file/elf (e.g. elf=\".pio/build/nucleo_g431rb/firmware.elf\") — no ELF auto-inferred (looked in .pio/build/*/firmware.elf and build/*.elf); build first or pass elf explicitly".to_string()
    })?;
    let resolved = crate::tools::util::resolve_within(&ctx.cwd, file, &ctx.allowed_roots)
        .map_err(|e| format!("[Permission] {e}"))?;
    let chip = step
        .chip
        .clone()
        .or_else(|| ctx.default_chip.clone())
        .ok_or_else(|| {
            "[InvalidInput] missing chip: pass chip in step (e.g. chip=\"stm32g431rb\") or set default_chip in [tools]".to_string()
        })?;
    let chip =
        crate::tools::util::token_arg(&chip, "chip").map_err(|e| format!("[InvalidInput] {e}"))?;
    let probe = if let Some(p) = &step.probe {
        Some(crate::tools::util::token_arg(p, "probe").map_err(|e| format!("[InvalidInput] {e}"))?)
    } else {
        None
    };
    let timeout = step.timeout_ms.unwrap_or(180_000).min(remaining.max(1000));
    let reset = step.reset.unwrap_or(true);

    // quick probe-rs check
    let probe_ok = std::process::Command::new("probe-rs")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !probe_ok {
        return Err(
            "[NotFound] probe-rs is not installed or not on PATH: install with `cargo install probe-rs-tools`".to_string(),
        );
    }

    let mut dl_args = vec!["download".to_string(), "--chip".to_string(), chip.clone()];
    if let Some(p) = probe.as_ref() {
        dl_args.push("--probe".to_string());
        dl_args.push(p.clone());
    }
    dl_args.push(resolved.to_string_lossy().to_string());

    let result =
        crate::tools::util::run_probe_rs(dl_args, &ctx.cwd, timeout, Some(ctx.cancel.clone()), &[])
            .await;
    match result {
        Ok((text, Some(0))) if !reset => Ok(format!("flash passed (exit 0)\n{text}")),
        Ok((text, Some(0))) => {
            let mut reset_args = vec!["reset".to_string(), "--chip".to_string(), chip];
            if let Some(p) = probe {
                reset_args.push("--probe".to_string());
                reset_args.push(p);
            }
            match crate::tools::util::run_probe_rs(
                reset_args,
                &ctx.cwd,
                timeout,
                Some(ctx.cancel.clone()),
                &[],
            )
            .await
            {
                Ok((rtext, Some(0))) => Ok(format!(
                    "flash passed and target reset (exit 0)\n{text}\nreset: {rtext}"
                )),
                Ok((rtext, Some(c))) => {
                    Err(format!("[Io] reset after flash failed (exit {c})\n{rtext}"))
                }
                Ok((rtext, None)) => Err(format!("[Timeout] reset after flash timed out\n{rtext}")),
                Err(e) => Err(crate::tools::util::probe_rs_err(e).message),
            }
        }
        Ok((text, Some(c))) => Err(format!("[Io] flash failed (exit {c})\n{text}")),
        Ok((text, None)) => Err(format!("[Timeout] flash timed out\n{text}")),
        Err(e) => Err(crate::tools::util::probe_rs_err(e).message),
    }
}

async fn run_run_step(
    step: &HilStep,
    ctx: &ToolContext,
    dry_run: bool,
    remaining: u64,
) -> Result<String, String> {
    if dry_run {
        let elf = step.elf.as_deref().or(step.file.as_deref()).unwrap_or("?");
        return Ok(format!("[dry-run] run simulated: {elf}"));
    }
    let file = step.elf.as_deref().or(step.file.as_deref()).ok_or_else(|| {
        "[InvalidInput] run step requires file/elf (e.g. elf=\".pio/build/nucleo_g431rb/firmware.elf\") — no ELF auto-inferred; build first or pass elf explicitly".to_string()
    })?;
    let resolved = crate::tools::util::resolve_within(&ctx.cwd, file, &ctx.allowed_roots)
        .map_err(|e| format!("[Permission] {e}"))?;
    let chip = step
        .chip
        .clone()
        .or_else(|| ctx.default_chip.clone())
        .ok_or_else(|| {
            "[InvalidInput] missing chip for run step (e.g. chip=\"stm32g431rb\")".to_string()
        })?;
    let chip =
        crate::tools::util::token_arg(&chip, "chip").map_err(|e| format!("[InvalidInput] {e}"))?;
    let probe = if let Some(p) = &step.probe {
        Some(crate::tools::util::token_arg(p, "probe").map_err(|e| format!("[InvalidInput] {e}"))?)
    } else {
        None
    };
    let timeout = step.timeout_ms.unwrap_or(30_000).min(remaining.max(1000));
    // duplicate logic from run.rs but inline to avoid shell
    let probe_ok = std::process::Command::new("probe-rs")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !probe_ok {
        return Err("[NotFound] probe-rs not on PATH".to_string());
    }
    // Use a short-lived helper that mimics run.rs behaviour: spawn probe-rs run and kill after timeout
    // Reuse run_probe_rs for simplicity (though run needs streaming); we emulate via run_probe_rs with timeout
    // For now, call run_probe_rs with ["run", ...] (probe-rs run is supported)
    let mut args = vec!["run".to_string(), "--chip".to_string(), chip.clone()];
    if let Some(p) = probe {
        args.push("--probe".to_string());
        args.push(p);
    }
    args.push(resolved.to_string_lossy().to_string());
    match crate::tools::util::run_probe_rs(args, &ctx.cwd, timeout, Some(ctx.cancel.clone()), &[])
        .await
    {
        Ok((text, Some(0))) => Ok(format!(
            "run finished (exit 0)\n{}",
            crate::forensic::append_fault_marker(text)
        )),
        Ok((text, Some(c))) => Err(format!("[Io] run failed (exit {c})\n{text}")),
        Ok((text, None)) => Ok(format!(
            "run timed out after {timeout} ms; captured:\n{text}"
        )),
        Err(e) if e.contains("[Timeout]") => Ok(format!(
            "run captured {timeout} ms (window closed, probe-rs timed out)\n{e}"
        )),
        Err(e) => Err(crate::tools::util::probe_rs_err(e).message),
    }
}

async fn run_monitor_step(
    step: &HilStep,
    ctx: &ToolContext,
    dry_run: bool,
    remaining: u64,
) -> Result<String, String> {
    let timeout = step.timeout_ms.unwrap_or(10_000).min(remaining.max(500));
    let port_raw = step
        .port
        .clone()
        .or_else(|| ctx.monitor_port.clone())
        .ok_or_else(|| {
            format!(
                "[InvalidInput] monitor step missing port: pass port or set monitor_port; detected: {}",
                crate::tools::monitor::enumerate_ports()
            )
        })?;

    if dry_run {
        let port_display = if port_raw == "auto" {
            "auto (simulated)".to_string()
        } else {
            port_raw.clone()
        };
        let contains = step
            .expect_contains
            .clone()
            .or_else(|| step.expect.as_ref().and_then(|e| e.contains.clone()));
        if let Some(pat) = contains {
            return Ok(format!(
                "[dry-run] monitor {port_display} simulated ({timeout} ms) — would check expect_contains=\"{pat}\"\n[HIL_EXPECT:FAIL] dry-run cannot verify hardware output (no data)"
            ));
        }
        return Ok(format!(
            "[dry-run] monitor {port_display} simulated ({timeout} ms)"
        ));
    }

    // auto port (real run only)
    let port = if port_raw == "auto" {
        let ports = serialport::available_ports().unwrap_or_default();
        if ports.is_empty() {
            return Err("[InvalidInput] monitor auto: no serial ports detected".to_string());
        }
        ports[0].port_name.clone()
    } else {
        port_raw.clone()
    };

    let baud = step.baud.unwrap_or(ctx.monitor_baud);
    let autodetect = step.autodetect.unwrap_or(false);
    let elf: Option<PathBuf> = if let Some(e) = step.elf.as_deref().or(step.file.as_deref()) {
        Some(
            crate::tools::util::resolve_within(&ctx.cwd, e, &ctx.allowed_roots)
                .map_err(|e| format!("[Permission] {e}"))?,
        )
    } else {
        None
    };

    // Expect config
    let expect_contains = step
        .expect_contains
        .clone()
        .or_else(|| step.expect.as_ref().and_then(|e| e.contains.clone()));
    let expect_regex = step
        .expect_regex
        .clone()
        .or_else(|| step.expect.as_ref().and_then(|e| e.regex.clone()));
    let expect_count = step
        .expect_count
        .or_else(|| step.expect.as_ref().and_then(|e| e.count))
        .unwrap_or(1)
        // expect_count = 0 would pass vacuously before any byte arrives.
        .max(1);
    let regex_obj = if let Some(rx) = &expect_regex {
        Some(
            regex::Regex::new(rx)
                .map_err(|e| format!("[InvalidInput] expect_regex invalid: {e}"))?,
        )
    } else {
        None
    };

    // Run blocking serial read in spawn_blocking, but with expect-aware early exit
    let port_clone = port.clone();
    let cancel = ctx.cancel.clone();
    let expected_text = expect_contains.clone();
    let regex_for_thread = regex_obj.clone();
    let captured = tokio::task::spawn_blocking(move || {
        read_serial_with_expect(
            &port_clone,
            baud,
            timeout,
            elf.as_deref(),
            true,
            Some(cancel.clone()),
            autodetect,
            expected_text.as_deref(),
            regex_for_thread.as_ref(),
            expect_count,
        )
    })
    .await
    .map_err(|e| format!("[Io] monitor task failed: {e}"))?
    .map_err(|e| format!("[Io] {e}"))?;

    // Evaluate expectations
    if expect_contains.is_some() || expect_regex.is_some() {
        let (matched, _total) =
            evaluate_expect(&captured, expect_contains.as_deref(), regex_obj.as_ref());
        let ok = matched >= expect_count;
        let marker = if ok { "PASS" } else { "FAIL" };
        let detail = if ok {
            format!("[HIL_EXPECT:{marker}] monitor expect matched {matched}/{expect_count}")
        } else {
            format!(
                "[HIL_EXPECT:{marker}] monitor expect matched {matched}/{expect_count} — wanted contains={:?} regex={:?} in:\n{captured}",
                expect_contains, expect_regex
            )
        };
        // Append marker so the orchestrator can detect failure while still returning text
        return Ok(format!(
            "monitor {port} ({baud} baud, {timeout} ms)\n{captured}\n{detail}"
        ));
    }

    Ok(format!(
        "monitor {port} ({baud} baud, {timeout} ms)\n{captured}"
    ))
}

async fn run_trace_step(
    step: &HilStep,
    ctx: &ToolContext,
    dry_run: bool,
    remaining: u64,
) -> Result<String, String> {
    let duration = step
        .duration_ms
        .or(step.timeout_ms)
        .unwrap_or(3000)
        .clamp(100, 60_000)
        .min(remaining.max(500));
    let clk_hz = step.clk_hz.unwrap_or(170_000_000).clamp(1_000, 500_000_000);
    let baud = step.baud.unwrap_or(2_000_000).clamp(1_000, 25_000_000);
    let chip = step
        .chip
        .clone()
        .or_else(|| ctx.default_chip.clone())
        .ok_or_else(|| "[InvalidInput] trace step requires chip (or default_chip)".to_string())?;
    let chip =
        crate::tools::util::token_arg(&chip, "chip").map_err(|e| format!("[InvalidInput] {e}"))?;
    let probe = if let Some(p) = &step.probe {
        Some(crate::tools::util::token_arg(p, "probe").map_err(|e| format!("[InvalidInput] {e}"))?)
    } else {
        None
    };
    if dry_run {
        return Ok(format!(
            "[dry-run] trace simulated: {duration} ms clk {clk_hz} Hz baud {baud} chip {chip}"
        ));
    }
    // SWO/ITM is ARM CoreSight; ESP32* (Xtensa/RISC-V) and other non-ARM
    // targets have no TPIU/ITM to configure — fail with the alternative
    // instead of capturing an empty stream. The ELF decides when present,
    // the chip name covers trace steps that omit it.
    if let Some(reason) = crate::decode::non_arm_reason(&chip, step.elf.as_deref().map(Path::new)) {
        return Err(format!(
            "[InvalidInput] trace streams SWO/ITM packets (ARM CoreSight), but {reason} — \
             this target has no SWO/ITM. Use a `monitor` step (UART or USB-CDC console) \
             with expect_contains instead."
        ));
    }
    let probe_ok = std::process::Command::new("probe-rs")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !probe_ok {
        return Err("[NotFound] probe-rs not on PATH".to_string());
    }
    // probe-rs itm swo takes no --chip/--probe flags; use env vars
    let outer = duration.saturating_add(5_000).min(remaining.max(5_000));
    let mut envs: Vec<(String, String)> = vec![("PROBE_RS_CHIP".to_string(), chip.clone())];
    if let Some(p) = probe {
        envs.push(("PROBE_RS_PROBE".to_string(), p));
    }
    let args = vec![
        "itm".to_string(),
        "swo".to_string(),
        duration.to_string(),
        clk_hz.to_string(),
        baud.to_string(),
    ];
    let result =
        crate::tools::util::run_probe_rs(args, &ctx.cwd, outer, Some(ctx.cancel.clone()), &envs)
            .await;
    let (text, code) = match result {
        Ok(v) => v,
        Err(e) if e.contains("[Timeout]") => (String::new(), None),
        Err(e) => return Err(crate::tools::util::probe_rs_err(e).message),
    };
    match code {
        Some(0) => {
            let summary = if text.trim().is_empty() {
                "no ITM packets captured (firmware must write ITM ports, e.g. ITM_SendChar)\n"
            } else {
                ""
            };
            let mut out = format!(
                "trace captured {duration} ms (clk {clk_hz} Hz, baud {baud}) (exit 0)\n{summary}{text}"
            );
            // honor monitor-style expectations if provided
            let expect_contains = step
                .expect_contains
                .clone()
                .or_else(|| step.expect.as_ref().and_then(|e| e.contains.clone()));
            let expect_regex = step
                .expect_regex
                .clone()
                .or_else(|| step.expect.as_ref().and_then(|e| e.regex.clone()));
            let expect_count = step
                .expect_count
                .or_else(|| step.expect.as_ref().and_then(|e| e.count))
                .unwrap_or(1)
                // Same clamp as the monitor step: expect_count = 0 would pass
                // vacuously before any byte arrives.
                .max(1);
            if expect_contains.is_some() || expect_regex.is_some() {
                let regex_obj = if let Some(rx) = &expect_regex {
                    Some(
                        regex::Regex::new(rx)
                            .map_err(|e| format!("[InvalidInput] expect_regex invalid: {e}"))?,
                    )
                } else {
                    None
                };
                let (matched, _) =
                    evaluate_expect(&text, expect_contains.as_deref(), regex_obj.as_ref());
                let ok = matched >= expect_count;
                let marker = if ok { "PASS" } else { "FAIL" };
                out.push_str(&format!(
                    "\n[HIL_EXPECT:{marker}] trace expect matched {matched}/{expect_count} — wanted contains={expect_contains:?} regex={expect_regex:?}"
                ));
            }
            Ok(out)
        }
        None => Ok(format!(
            "trace: capture window closed after {outer} ms on an idle SWO stream (no ITM packets — firmware must write ITM ports, e.g. ITM_SendChar)\n{text}"
        )),
        Some(c) => Err(format!("[Io] probe-rs trace failed (exit {c})\n{text}")),
    }
}

/// Observe step: run the deterministic CV analysis (tools/observe.rs) on a
/// frame and assert the physical verdict. Follows the monitor convention —
/// expectation failures return Ok with an embedded [HIL_EXPECT:FAIL] marker
/// so the suite still records the capture; only infra errors are Err.
async fn run_observe_step(
    step: &HilStep,
    ctx: &ToolContext,
    dry_run: bool,
    remaining: u64,
) -> Result<String, String> {
    let mode = step.mode.as_deref().unwrap_or("brightness");
    if dry_run {
        // Mirror run_monitor_step: a dry run has no frame to look at, so any
        // expectation must FAIL instead of silently passing.
        let has_expect = step.expect_lit.is_some()
            || step.expect_blinking.is_some()
            || step.expect_blink_hz.is_some()
            || step.expect_motion.is_some()
            || step.expect_diff.is_some();
        return Ok(if has_expect {
            format!(
                "[dry-run] observe simulated (mode={mode}) — would check expectations\n\
                 [HIL_EXPECT:FAIL] dry-run cannot verify hardware output (no frame)"
            )
        } else {
            format!("[dry-run] observe simulated (mode={mode})")
        });
    }
    // Decoding and measuring frames is not free; if the suite budget is
    // already gone, say so rather than burning it.
    if remaining == 0 {
        return Err("[Timeout] hil suite budget exhausted before the observe step".to_string());
    }
    match mode {
        "brightness" => run_observe_brightness(step, ctx).await,
        "motion" => run_observe_motion(step, ctx),
        "blink" => run_observe_blink(step, ctx),
        "diff" => run_observe_diff(step, ctx),
        other => Err(format!(
            "[InvalidInput] observe mode={other} is not valid (brightness | motion | blink | diff)"
        )),
    }
}

/// Load a burst of frames from workspace paths, in capture order.
fn load_observe_frames(
    ctx: &ToolContext,
    paths: &[String],
) -> Result<Vec<image::RgbaImage>, String> {
    let mut frames = Vec::with_capacity(paths.len());
    for (i, p) in paths.iter().enumerate() {
        let resolved = crate::tools::util::resolve_within(&ctx.cwd, p, &ctx.allowed_roots)
            .map_err(|e| format!("[Permission] frames[{i}] ({p}): {e}"))?;
        let img = image::open(&resolved)
            .map_err(|e| {
                format!(
                    "[Io] cannot decode frames[{i}] ({}): {e}",
                    resolved.display()
                )
            })?
            .to_rgba8();
        frames.push(img);
    }
    Ok(frames)
}

/// brightness: is the target lit, and how bright. The only still-frame mode;
/// the verdict is asserted via expect_lit.
async fn run_observe_brightness(step: &HilStep, ctx: &ToolContext) -> Result<String, String> {
    let Some(path) = step.file.as_deref() else {
        return Err("[InvalidInput] observe step requires file (the frame to analyze)".to_string());
    };
    let resolved = crate::tools::util::resolve_within(&ctx.cwd, path, &ctx.allowed_roots)
        .map_err(|e| format!("[Permission] {e}"))?;
    let frame = image::open(&resolved)
        .map_err(|e| format!("[Io] cannot decode {}: {e}", resolved.display()))?
        .to_rgba8();
    let roi = step.roi.map(|r| crate::tools::observe::Rect {
        x: r[0],
        y: r[1],
        w: r[2],
        h: r[3],
    });
    let (b, suggested) = crate::tools::observe::analyze_brightness(
        &frame,
        &crate::tools::observe::Spec {
            roi,
            threshold: step.threshold,
        },
    );
    let mut out = format!(
        "observe {path} ({}x{})
  luma: min={} max={} mean={}
  lit: {} ({:.0}% of ROI above threshold {})
  confidence: {} — {}",
        frame.width(),
        frame.height(),
        b.min,
        b.max,
        b.mean,
        if b.lit { "yes" } else { "no" },
        b.lit_fraction * 100.0,
        b.threshold_used,
        b.confidence.name(),
        b.note,
    );
    if let Some(r) = suggested {
        out.push_str(&format!(
            "
  suggested roi: [{}, {}, {}, {}]",
            r.x, r.y, r.w, r.h
        ));
    }
    if step.save == Some(true) {
        let dir = ctx.cwd.join(".firment").join("observe");
        std::fs::create_dir_all(&dir).map_err(|e| format!("[Io] create {}: {e}", dir.display()))?;
        let dest = dir.join(format!(
            "{}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            resolved
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default()
        ));
        std::fs::copy(&resolved, &dest)
            .map_err(|e| format!("[Io] save {}: {e}", dest.display()))?;
        out.push_str(&format!(
            "
  saved: {}",
            dest.display()
        ));
    }
    if let Some(want) = step.expect_lit {
        let marker = if b.lit == want { "PASS" } else { "FAIL" };
        out.push_str(&format!(
            "
[HIL_EXPECT:{marker}] observe lit={} — wanted lit={want}",
            b.lit
        ));
    }
    Ok(out)
}

/// motion: did anything move across a burst of frames. Verdict via
/// expect_motion; a "yes" on LOW confidence is not evidence and cannot pass.
fn run_observe_motion(step: &HilStep, ctx: &ToolContext) -> Result<String, String> {
    let Some(paths) = step.paths.as_deref() else {
        return Err(
            "[InvalidInput] observe mode=motion requires paths (2+ frames, in capture order)"
                .to_string(),
        );
    };
    if paths.len() < 2 {
        return Err(
            "[InvalidInput] observe mode=motion requires at least 2 frames in paths".to_string(),
        );
    }
    let frames = load_observe_frames(ctx, paths)?;
    let roi = step.roi.map(|r| crate::tools::observe::Rect {
        x: r[0],
        y: r[1],
        w: r[2],
        h: r[3],
    });
    let m = crate::tools::observe::analyze_motion(
        &frames,
        roi,
        crate::tools::observe::MOTION_PIXEL_THRESHOLD,
    )
    .map_err(|e| format!("[InvalidInput] {e}"))?;
    let mut out = format!(
        "observe mode=motion ({} frames, {}x{})
  moving: {} (changed {:.1}% of ROI, mean abs diff {:.1})
  confidence: {} — {}",
        m.frames,
        frames[0].width(),
        frames[0].height(),
        if m.moving { "yes" } else { "no" },
        m.changed_fraction * 100.0,
        m.mean_abs_diff,
        m.confidence.name(),
        m.note,
    );
    if let Some(want) = step.expect_motion {
        // A "yes" verdict on LOW confidence (e.g. exposure drift) is not
        // evidence — the analysis itself said it cannot tell motion from the
        // camera auto-exposing.
        let ok = if want {
            m.moving && !matches!(m.confidence, crate::tools::observe::Confidence::Low)
        } else {
            !m.moving
        };
        let marker = if ok { "PASS" } else { "FAIL" };
        out.push_str(&format!(
            "
[HIL_EXPECT:{marker}] observe motion={} — wanted motion={want}{}",
            m.moving,
            if want && m.moving && !ok {
                " (low confidence)"
            } else {
                ""
            },
        ));
    }
    Ok(out)
}

/// blink: does a burst alternate, and at what frequency. expect_blinking is a
/// plain boolean match; expect_blink_hz additionally requires a time base, a
/// non-LOW confidence and the wanted value inside the measured range.
fn run_observe_blink(step: &HilStep, ctx: &ToolContext) -> Result<String, String> {
    let Some(paths) = step.paths.as_deref() else {
        return Err(
            "[InvalidInput] observe mode=blink requires paths (3+ frames, in capture order)"
                .to_string(),
        );
    };
    if paths.len() < 3 {
        return Err(
            "[InvalidInput] observe mode=blink requires at least 3 frames in paths".to_string(),
        );
    }
    let frames = load_observe_frames(ctx, paths)?;
    let roi = step.roi.map(|r| crate::tools::observe::Rect {
        x: r[0],
        y: r[1],
        w: r[2],
        h: r[3],
    });
    let interval_ms = step.interval_ms.unwrap_or(0);
    let samples: Vec<crate::tools::observe::Sample> = frames
        .iter()
        .enumerate()
        .map(|(i, f)| crate::tools::observe::Sample {
            t_ms: i as u64 * interval_ms,
            luma: crate::tools::observe::mean_luma(f, roi),
        })
        .collect();
    let b = crate::tools::observe::analyze_blink(&samples);
    let mut out = format!(
        "observe mode=blink ({} frames, interval_ms={})
  blinking: {}
  edges: {}
  confidence: {} — {}",
        b.samples,
        if interval_ms > 0 {
            interval_ms.to_string()
        } else {
            "not given".to_string()
        },
        if b.blinking { "yes" } else { "no" },
        b.edges,
        b.confidence.name(),
        b.note,
    );
    if let (Some(low), Some(high)) = (b.hz_low, b.hz_high) {
        out.push_str(&format!("\n  frequency: {low:.2} .. {high:.2} Hz"));
    }
    if let Some(want) = step.expect_blinking {
        let marker = if b.blinking == want { "PASS" } else { "FAIL" };
        out.push_str(&format!(
            "
[HIL_EXPECT:{marker}] observe blinking={} — wanted blinking={want}",
            b.blinking
        ));
    }
    if let Some(want) = step.expect_blink_hz {
        // Only a real measurement can answer this. A missing time base, LOW
        // confidence or an aliased capture are all "cannot confirm" — and a
        // LOW-confidence number must never pass an assertion.
        let (marker, reason) = if step.interval_ms.is_none() {
            (
                "FAIL",
                "interval_ms not given — cannot turn edges into a frequency",
            )
        } else if matches!(b.confidence, crate::tools::observe::Confidence::Low) {
            ("FAIL", "low confidence — not asserting a frequency on it")
        } else if let (Some(low), Some(high)) = (b.hz_low, b.hz_high) {
            if (low as f64) <= want && want <= (high as f64) {
                ("PASS", "inside the measured range")
            } else {
                ("FAIL", "outside the measured range")
            }
        } else {
            ("FAIL", "no frequency estimated (see note)")
        };
        out.push_str(&format!(
            "
[HIL_EXPECT:{marker}] observe blink_hz={want} — {reason}"
        ));
    }
    Ok(out)
}

/// diff: before/after comparison — did an agent's change actually move
/// pixels? Verdict via expect_diff; a "yes" on LOW confidence (reframed
/// camera) cannot pass.
fn run_observe_diff(step: &HilStep, ctx: &ToolContext) -> Result<String, String> {
    let Some(path) = step.file.as_deref() else {
        return Err(
            "[InvalidInput] observe mode=diff requires file (the before frame)".to_string(),
        );
    };
    let Some(after) = step.after.as_deref() else {
        return Err(
            "[InvalidInput] observe mode=diff requires after (the frame taken after the change)"
                .to_string(),
        );
    };
    let load = |p: &str| -> Result<image::RgbaImage, String> {
        let resolved = crate::tools::util::resolve_within(&ctx.cwd, p, &ctx.allowed_roots)
            .map_err(|e| format!("[Permission] {p}: {e}"))?;
        Ok(image::open(&resolved)
            .map_err(|e| format!("[Io] cannot decode {p}: {e}"))?
            .to_rgba8())
    };
    let before = load(path)?;
    let after_frame = load(after)?;
    let roi = step.roi.map(|r| crate::tools::observe::Rect {
        x: r[0],
        y: r[1],
        w: r[2],
        h: r[3],
    });
    let d = crate::tools::observe::analyze_diff(
        &before,
        &after_frame,
        roi,
        crate::tools::observe::MOTION_PIXEL_THRESHOLD,
    )
    .map_err(|e| format!("[InvalidInput] {e}"))?;
    let mut out = format!(
        "observe mode=diff ({}x{})
  changed: {} (mean delta {}{:.1}, {:.1}% of ROI)
  confidence: {} — {}",
        before.width(),
        before.height(),
        if d.changed { "yes" } else { "no" },
        if d.mean_delta >= 0.0 { "+" } else { "" },
        d.mean_delta,
        d.changed_fraction * 100.0,
        d.confidence.name(),
        d.note,
    );
    if let (Some(q), Some(share)) = (d.dominant_quadrant, d.quadrant_share) {
        out.push_str(&format!(
            "\n  change concentrates in: {q} ({:.0}% of it)",
            share * 100.0
        ));
    }
    if let Some(want) = step.expect_diff {
        let ok = if want {
            d.changed && !matches!(d.confidence, crate::tools::observe::Confidence::Low)
        } else {
            !d.changed
        };
        let marker = if ok { "PASS" } else { "FAIL" };
        out.push_str(&format!(
            "
[HIL_EXPECT:{marker}] observe diff={} — wanted diff={want}{}",
            d.changed,
            if want && d.changed && !ok {
                " (low confidence)"
            } else {
                ""
            },
        ));
    }
    Ok(out)
}

fn evaluate_expect(
    text: &str,
    contains: Option<&str>,
    regex: Option<&regex::Regex>,
) -> (usize, usize) {
    if let Some(pat) = contains {
        let matched = text.matches(pat).count();
        return (matched, matched);
    }
    if let Some(rx) = regex {
        let matched = rx.find_iter(text).count();
        return (matched, matched);
    }
    (0, 0)
}

#[allow(clippy::too_many_arguments)]
fn read_serial_with_expect(
    port: &str,
    baud: u32,
    timeout_ms: u64,
    elf: Option<&Path>,
    timestamp: bool,
    cancel: Option<firment_core::Cancellable>,
    autodetect: bool,
    expect_contains: Option<&str>,
    expect_regex: Option<&regex::Regex>,
    expect_count: usize,
) -> Result<String, String> {
    use std::io::Read;

    // autodetect baud if requested
    let resolved_baud = if autodetect {
        match detect_baud_inner(port, 300)? {
            Some(b) => b,
            None => {
                return Ok(format!(
                    "no valid data on {port} at any common baud rate; pass baud explicitly"
                ));
            }
        }
    } else {
        baud
    };

    let mut reader = serialport::new(port, resolved_baud)
        .timeout(Duration::from_millis(500))
        .open()
        .map_err(|e| format!("failed to open serial port {port}: {e}"))?;

    let start = Instant::now();
    let deadline = start + Duration::from_millis(timeout_ms);
    let mut buf = [0u8; 4096];
    let mut splitter = crate::utf8::LineSplitter::new(crate::utf8::MAX_LINE_BYTES);
    let mut lines: Vec<String> = Vec::new();
    let mut matched: usize = 0;

    let check_line = |line: &str, matched: &mut usize| {
        if let Some(pat) = expect_contains {
            if line.contains(pat) {
                *matched += 1;
            }
        } else if expect_regex.is_some_and(|rx| rx.is_match(line)) {
            *matched += 1;
        }
    };

    // Decode one complete line, run the expect assertions against it and
    // record it. Shared by the read loop and the trailing-partial-line
    // flush, which used to carry a second copy of this body.
    // Symbol index built once: per-line per-token ELF re-reads are the
    // dominant cost on address-heavy log streams.
    let symbol_index = elf.and_then(crate::decode::SymbolIndex::from_path);
    let handle_line = |line: &str, lines: &mut Vec<String>, matched: &mut usize| {
        let decoded = match &symbol_index {
            Some(index) => index.decode_line(line),
            None => line.to_string(),
        };
        let with_ts = if timestamp {
            let elapsed = Instant::now() - start;
            format!(
                "[{:02}.{:03}] {decoded}",
                elapsed.as_secs(),
                elapsed.subsec_millis()
            )
        } else {
            decoded.clone()
        };
        check_line(&decoded, matched);
        lines.push(with_ts);
    };

    loop {
        if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            lines.push("(monitor interrupted by turn cancellation)".to_string());
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        if (expect_contains.is_some() || expect_regex.is_some()) && matched >= expect_count {
            // got enough matches; still capture a tiny tail then break
            break;
        }
        match reader.read(&mut buf) {
            Ok(0) => continue,
            // Decode per line, not per read: a character split across two
            // reads would otherwise never match an expect assertion, since
            // the U+FFFD it turns into is not the text the device sent.
            Ok(n) => splitter.feed(&buf[..n], &mut |line| {
                handle_line(line, &mut lines, &mut matched)
            }),
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                continue;
            }
            Err(e) => return Err(format!("serial read failed: {e}")),
        }
        if matched >= expect_count
            && (expect_contains.is_some() || expect_regex.is_some())
            && !lines.is_empty()
        {
            break;
        }
    }
    if let Some(tail) = splitter.take_tail() {
        handle_line(&tail, &mut lines, &mut matched);
    }
    let text = lines.join("\n");
    if text.is_empty() {
        Ok(format!("no data received on {port} within {timeout_ms} ms"))
    } else {
        Ok(crate::tools::util::truncate(&text, 32_000))
    }
}

const BAUD_CANDIDATES: [u32; 9] = [
    9_600, 19_200, 38_400, 57_600, 74_880, 115_200, 230_400, 460_800, 921_600,
];

fn detect_baud_inner(port: &str, probe_ms: u64) -> Result<Option<u32>, String> {
    for baud in BAUD_CANDIDATES {
        if probe_baud_inner(port, baud, probe_ms)? {
            return Ok(Some(baud));
        }
    }
    Ok(None)
}

fn probe_baud_inner(port: &str, baud: u32, probe_ms: u64) -> Result<bool, String> {
    use std::io::Read;
    let Ok(mut reader) = serialport::new(port, baud)
        .timeout(Duration::from_millis(50))
        .open()
    else {
        return Ok(false);
    };
    let deadline = Instant::now() + Duration::from_millis(probe_ms);
    let mut bytes = 0u64;
    let mut junk = 0u64;
    let mut buf = [0u8; 256];
    loop {
        if Instant::now() >= deadline {
            break;
        }
        match reader.read(&mut buf) {
            Ok(0) => continue,
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == 0x00 || b == 0xFF {
                        junk += 1;
                    }
                }
                bytes += n as u64;
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                continue;
            }
            Err(_) => return Ok(false),
        }
    }
    Ok(bytes > 0 && (junk as f64 / bytes as f64) < 0.9)
}

async fn run_elf_step(
    step: &HilStep,
    ctx: &ToolContext,
    _remaining: u64,
) -> Result<String, String> {
    let file = step
        .elf
        .as_deref()
        .or(step.file.as_deref())
        .ok_or_else(|| "[InvalidInput] elf_analyze step requires file/elf".to_string())?;
    let elf = crate::tools::util::resolve_within(&ctx.cwd, file, &ctx.allowed_roots)
        .map_err(|e| format!("[Permission] {e}"))?;
    if !elf.is_file() {
        return Err(format!(
            "[NotFound] no ELF at {}: build first",
            elf.display()
        ));
    }
    // reuse the ElfAnalyze tool directly (bypass permission, call run)
    let tool = crate::tools::elf_analyze::ElfAnalyze;
    let args = json!({"file": file});
    // Build a minimal child context that inherits session_dir for baseline caching
    let child_ctx = ToolContext {
        cwd: ctx.cwd.clone(),
        permission: std::sync::Arc::new(firment_core::AutoApprove::everything()),
        allow_dangerous: ctx.allow_dangerous,
        journal: ctx.journal.clone(),
        verify_command: ctx.verify_command.clone(),
        allowed_roots: ctx.allowed_roots.clone(),
        symbols_backend: ctx.symbols_backend.clone(),
        build_command: ctx.build_command.clone(),
        default_chip: ctx.default_chip.clone(),
        monitor_port: ctx.monitor_port.clone(),
        monitor_baud: ctx.monitor_baud,
        subagent: None,
        subagent_depth: ctx.subagent_depth,
        max_subagent_depth: ctx.max_subagent_depth,
        asker: None,
        web_search_provider: None,
        web_search_api_key: None,
        session_dir: ctx.session_dir.clone(),
        ledger_path: ctx.ledger_path.clone(),
        device_log_dir: ctx.device_log_dir.clone(),
        providers: Vec::new(),
        cancel: ctx.cancel.clone(),
    };
    match tool.run(args, &child_ctx).await {
        Ok(out) => Ok(out.text),
        Err(e) => Err(e.message),
    }
}

// Build detection lives in build.rs (`detect_build_command`) and is shared
// here — the hil build step previously carried a verbatim copy, which is how
// the `cd <dir> &&` cmd-quoting bug survived its fix in the build tool.

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal};
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
            default_chip: Some("stm32f407vetx".to_string()),
            monitor_port: Some("COM_FAKE".to_string()),
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
            session_dir: Some(dir.join("session")),
            ..ToolContext::default()
        }
    }

    #[test]
    fn hil_toml_parses_suite_with_steps() {
        let toml_text = r#"
[suite.blink]
chip = "stm32g431rb"
port = "COM3"

[[suite.blink.steps]]
kind = "build"

[[suite.blink.steps]]
kind = "flash"
elf = "build/fw.elf"

[[suite.blink.steps]]
kind = "monitor"
timeout_ms = 5000
expect_contains = "LED ON"
expect_count = 2

[[suite.blink.steps]]
kind = "elf_analyze"
elf = "build/fw.elf"
"#;
        let file: HilFile = toml::from_str(toml_text).unwrap();
        assert_eq!(file.suite.len(), 1);
        let s = file.suite.get("blink").unwrap();
        assert_eq!(s.chip.as_deref(), Some("stm32g431rb"));
        assert_eq!(s.steps.len(), 4);
        assert_eq!(s.steps[2].expect_contains.as_deref(), Some("LED ON"));
        assert_eq!(s.steps[2].expect_count, Some(2));
    }

    #[test]
    fn inline_steps_parse_variants() {
        let v = json!([
            {"kind": "build"},
            {"kind": "monitor", "expect_contains": "ok", "expect_count": 3},
            {"kind": "delay", "duration_ms": 500}
        ]);
        let arr = v.as_array().unwrap();
        let mut steps = Vec::new();
        for v in arr {
            let s: HilStep = serde_json::from_value(v.clone()).unwrap();
            steps.push(s);
        }
        assert_eq!(steps[1].expect_contains.as_deref(), Some("ok"));
        assert_eq!(steps[2].duration_ms, Some(500));
    }

    #[test]
    fn evaluate_expect_counts() {
        let text = "LED ON\nLED OFF\nLED ON\n";
        let (m, _) = evaluate_expect(text, Some("LED ON"), None);
        assert_eq!(m, 2);
        let rx = regex::Regex::new("LED (ON|OFF)").unwrap();
        let (m2, _) = evaluate_expect(text, None, Some(&rx));
        assert_eq!(m2, 3);
    }

    #[tokio::test]
    async fn hil_dry_run_build_and_monitor() {
        let dir = tempdir().unwrap();
        // create a fake build system so build dry-run still shows simulated
        let c = ctx(dir.path());
        let tool = Hil;
        let args = json!({
            "steps": [
                {"kind": "build"},
                {"kind": "monitor", "port": "COM_FAKE", "timeout_ms": 100, "expect_contains": "hello"},
                {"kind": "delay", "duration_ms": 10}
            ],
            "dry_run": true
        });
        let res = tool.run(args, &c).await;
        // dry-run monitor expect will be marked fail (no hardware data), so overall fails
        assert!(res.is_err());
        let msg = res.unwrap_err().message;
        assert!(msg.contains("hil:"), "got: {msg}");
        assert!(msg.contains("replay:"), "got: {msg}");
    }

    #[tokio::test]
    async fn hil_replay_list_and_missing() {
        let dir = tempdir().unwrap();
        let c = ctx(dir.path());
        let tool = Hil;
        // list when empty
        let out = tool.run(json!({"replay": "list"}), &c).await.unwrap();
        assert!(out.text.contains("hil replays"), "got: {}", out.text);
        // missing id
        let err = tool
            .run(json!({"replay": "no-such-id"}), &c)
            .await
            .unwrap_err();
        assert!(err.message.contains("[NotFound]"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn hil_missing_suite_is_error() {
        let dir = tempdir().unwrap();
        let c = ctx(dir.path());
        let tool = Hil;
        let err = tool.run(json!({"suite": "blink"}), &c).await.unwrap_err();
        assert!(err.message.contains("hil"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn observe_step_dry_run_reports_simulated() {
        let dir = tempdir().unwrap();
        let step = HilStep {
            kind: "observe".to_string(),
            file: Some("led.png".to_string()),
            ..Default::default()
        };
        let out = run_observe_step(&step, &ctx(dir.path()), true, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[dry-run] observe"), "got: {out}");
        // No expectation was set, so there must be no verdict at all —
        // asserted on meaning rather than on the exact wording.
        assert!(!out.contains("[HIL_EXPECT"), "got: {out}");
    }

    #[tokio::test]
    async fn observe_step_dry_run_fails_when_expecting_lit() {
        let dir = tempdir().unwrap();
        let step = HilStep {
            kind: "observe".to_string(),
            file: Some("led.png".to_string()),
            expect_lit: Some(true),
            ..Default::default()
        };
        let out = run_observe_step(&step, &ctx(dir.path()), true, 60_000)
            .await
            .unwrap();
        // A dry run has no frame to measure: reporting Ok here would let the
        // expectation pass silently (the bug this test pins).
        assert!(
            out.contains("[HIL_EXPECT:FAIL]") && out.contains("cannot verify hardware"),
            "dry-run with an expectation must not pass: {out}"
        );
    }

    #[tokio::test]
    async fn observe_step_requires_file() {
        let dir = tempdir().unwrap();
        let step = HilStep {
            kind: "observe".to_string(),
            ..Default::default()
        };
        let err = run_observe_step(&step, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap_err();
        assert!(err.contains("requires file"), "got: {err}");
    }

    #[tokio::test]
    async fn observe_step_asserts_lit_verdict() {
        let dir = tempdir().unwrap();
        let frame = image::RgbaImage::from_pixel(64, 64, image::Rgba([250, 250, 250, 255]));
        frame.save(dir.path().join("led.png")).unwrap();
        let pass = HilStep {
            kind: "observe".to_string(),
            file: Some("led.png".to_string()),
            expect_lit: Some(true),
            ..Default::default()
        };
        let out = run_observe_step(&pass, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[HIL_EXPECT:PASS]"), "got: {out}");

        let fail = HilStep {
            expect_lit: Some(false),
            ..pass
        };
        let out = run_observe_step(&fail, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[HIL_EXPECT:FAIL]"), "got: {out}");
    }

    fn solid(luma: u8) -> image::RgbaImage {
        image::RgbaImage::from_pixel(64, 64, image::Rgba([luma, luma, luma, 255]))
    }

    fn frame_with_block(x: u32, y: u32, size: u32, luma: u8) -> image::RgbaImage {
        let mut f = solid(8);
        for py in y..y + size {
            for px in x..x + size {
                f.put_pixel(px, py, image::Rgba([luma, luma, luma, 255]));
            }
        }
        f
    }

    #[tokio::test]
    async fn observe_step_motion_verdict() {
        let dir = tempdir().unwrap();
        // The block MOVES between shots but total brightness stays the same,
        // so the pair is clean motion (HIGH), not exposure drift.
        frame_with_block(10, 10, 8, 240)
            .save(dir.path().join("m1.png"))
            .unwrap();
        frame_with_block(30, 30, 8, 240)
            .save(dir.path().join("m2.png"))
            .unwrap();
        let step = HilStep {
            kind: "observe".to_string(),
            mode: Some("motion".to_string()),
            paths: Some(vec!["m1.png".to_string(), "m2.png".to_string()]),
            expect_motion: Some(true),
            ..Default::default()
        };
        let out = run_observe_step(&step, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[HIL_EXPECT:PASS]"), "got: {out}");

        // Identical frames: no motion, the assertion must fail.
        let same = HilStep {
            paths: Some(vec!["m1.png".to_string(), "m1.png".to_string()]),
            ..step
        };
        let out = run_observe_step(&same, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[HIL_EXPECT:FAIL]"), "got: {out}");
    }

    #[tokio::test]
    async fn observe_step_motion_low_confidence_fails() {
        let dir = tempdir().unwrap();
        // Block moves AND the whole frame brightens: the analysis says "yes
        // but LOW" — a LOW-confidence yes is not evidence and must not pass.
        let a = frame_with_block(10, 10, 8, 240);
        let mut b = frame_with_block(30, 30, 8, 240);
        for px in b.pixels_mut() {
            px[0] = px[0].saturating_add(60);
            px[1] = px[1].saturating_add(60);
            px[2] = px[2].saturating_add(60);
        }
        a.save(dir.path().join("m1.png")).unwrap();
        b.save(dir.path().join("m2.png")).unwrap();
        let step = HilStep {
            kind: "observe".to_string(),
            mode: Some("motion".to_string()),
            paths: Some(vec!["m1.png".to_string(), "m2.png".to_string()]),
            expect_motion: Some(true),
            ..Default::default()
        };
        let out = run_observe_step(&step, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(
            out.contains("[HIL_EXPECT:FAIL]") && out.contains("low confidence"),
            "a LOW-confidence 'yes' must not pass: {out}"
        );
    }

    #[tokio::test]
    async fn observe_step_blink_hz_verdict() {
        let dir = tempdir().unwrap();
        // Square wave, 2 samples per half-cycle at 100 ms => 400 ms period,
        // i.e. 2.5 Hz. Ten frames is enough for a non-LOW estimate.
        for i in 0..10usize {
            let luma = if i % 4 < 2 { 240u8 } else { 10u8 };
            solid(luma)
                .save(dir.path().join(format!("b{i}.png")))
                .unwrap();
        }
        let paths: Vec<String> = (0..10).map(|i| format!("b{i}.png")).collect();
        let pass = HilStep {
            kind: "observe".to_string(),
            mode: Some("blink".to_string()),
            paths: Some(paths.clone()),
            interval_ms: Some(100),
            expect_blink_hz: Some(2.5),
            ..Default::default()
        };
        let out = run_observe_step(&pass, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[HIL_EXPECT:PASS]"), "got: {out}");

        // 5 Hz is outside the measured ~2.0–3.3 Hz range.
        let out_of_range = HilStep {
            expect_blink_hz: Some(5.0),
            ..pass
        };
        let out = run_observe_step(&out_of_range, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[HIL_EXPECT:FAIL]"), "got: {out}");
    }

    #[tokio::test]
    async fn observe_step_blink_hz_needs_interval_ms() {
        let dir = tempdir().unwrap();
        for i in 0..4usize {
            let luma = if i % 2 == 0 { 240 } else { 10 };
            solid(luma)
                .save(dir.path().join(format!("b{i}.png")))
                .unwrap();
        }
        let paths: Vec<String> = (0..4).map(|i| format!("b{i}.png")).collect();
        let step = HilStep {
            kind: "observe".to_string(),
            mode: Some("blink".to_string()),
            paths: Some(paths),
            expect_blink_hz: Some(2.5),
            ..Default::default()
        };
        let out = run_observe_step(&step, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(
            out.contains("[HIL_EXPECT:FAIL]") && out.contains("interval_ms"),
            "a missing time base must fail loudly: {out}"
        );
    }

    #[tokio::test]
    async fn observe_step_diff_verdict() {
        let dir = tempdir().unwrap();
        solid(8).save(dir.path().join("before.png")).unwrap();
        frame_with_block(10, 10, 8, 240)
            .save(dir.path().join("after.png"))
            .unwrap();
        let pass = HilStep {
            kind: "observe".to_string(),
            mode: Some("diff".to_string()),
            file: Some("before.png".to_string()),
            after: Some("after.png".to_string()),
            expect_diff: Some(true),
            ..Default::default()
        };
        let out = run_observe_step(&pass, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[HIL_EXPECT:PASS]"), "got: {out}");

        // Same frame on both sides: nothing changed.
        let no_change = HilStep {
            after: Some("before.png".to_string()),
            ..pass
        };
        let out = run_observe_step(&no_change, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[HIL_EXPECT:FAIL]"), "got: {out}");
    }
}
