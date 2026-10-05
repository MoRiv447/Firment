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
// pub(crate): the red team runner passes a recovery step through
// `flash_recovery_step` + `run_flash_step` without touching the fields.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct HilStep {
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
    // observe motion/diff: per-pixel luma change that counts as "moved"
    // (same knob as the observe tool; default 16).
    #[serde(default)]
    pixel_threshold: Option<u8>,
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
    // la step: capture from a logic analyzer (or assert against a stored
    // capture via `capture`) and check measurements. See tools/la.rs.
    // `duration_ms` doubles as the capture window (time_ms bound).
    #[serde(default)]
    driver: Option<String>,
    #[serde(default)]
    channels: Option<String>,
    #[serde(default)]
    samplerate: Option<String>,
    #[serde(default)]
    samples: Option<u64>,
    #[serde(default)]
    capture: Option<String>,
    #[serde(default)]
    channel: Option<usize>,
    #[serde(default)]
    decoder: Option<String>,
    #[serde(default)]
    decoder_opts: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    expect_frequency_hz: Option<f64>,
    #[serde(default)]
    expect_duty: Option<f64>,
    #[serde(default)]
    expect_edges: Option<usize>,
    #[serde(default)]
    expect_decoded: Option<String>,
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
                "steps": {"type": "array", "description": "Inline steps [{kind, file, elf, chip, probe, port, baud, clk_hz, timeout_ms, duration_ms, expect_contains, expect_regex, expect_count, mode, roi, threshold, pixel_threshold, expect_lit, save, paths, after, interval_ms, expect_blinking, expect_blink_hz, expect_motion, expect_diff, driver, channels, samplerate, samples, capture, channel, decoder, decoder_opts, expect_frequency_hz, expect_duty, expect_edges, expect_decoded}] — kinds: build/flash/run/monitor/trace/observe/la/elf_analyze/delay; flash/run/elf auto-infer .pio/build/*/firmware.elf when elf omitted; observe uses mode=brightness|motion|blink|diff with file/paths/after, roi [x,y,w,h], threshold, interval_ms (blink), save, and asserts via expect_lit/expect_motion/expect_diff/expect_blinking/expect_blink_hz; la captures via driver/channels/samplerate + samples|duration_ms (or asserts a stored capture= id) and checks expect_frequency_hz/expect_duty/expect_edges/expect_decoded (channel= selects the line, decoder/decoder_opts for decoded text)"},
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
        // What the suite actually proved, step by step — not what it planned.
        let mut reached_level = 1u8;
        let mut reached_name = "code";
        let mut aborted_at: Option<usize> = None;
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
                aborted_at = Some(idx);
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
                aborted_at = Some(idx);
                break;
            }

            let step_start = Instant::now();
            let kind = step.inner.kind.as_str();
            let remaining =
                total_timeout.saturating_sub(overall_start.elapsed().as_millis() as u64);

            // Refused before the step runs its hardware: a suite whose `expect_regex` does not
            // compile must not spend a flash to find out, and a step that cannot be checked is
            // not a step that passed.
            let expectation = Expectation::of(&step.inner)
                .map_err(|e| ToolError::new(format!("step {} ({kind}): {e}", idx + 1)))?;

            let result: Result<String, String> = match kind {
                "build" => run_build_step(&step.inner, ctx, dry_run, remaining).await,
                "flash" => run_flash_step(&step.inner, ctx, dry_run, remaining).await,
                "run" => run_run_step(&step.inner, ctx, dry_run, remaining).await,
                "monitor" => run_monitor_step(&step.inner, ctx, dry_run, remaining).await,
                "trace" => run_trace_step(&step.inner, ctx, dry_run, remaining).await,
                "observe" => run_observe_step(&step.inner, ctx, dry_run, remaining).await,
                "la" => run_la_step(&step.inner, ctx, dry_run, remaining).await,
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
                    "[InvalidInput] unknown hil step kind: {kind} (expected build/flash/run/monitor/trace/observe/la/elf_analyze/delay)"
                )),
            };

            let elapsed = step_start.elapsed().as_millis() as u64;
            match result {
                Ok(text) => {
                    let (text, expect_failed) = step_verdict(kind, expectation, text);
                    if expect_failed {
                        failed_expect = true;
                        overall_ok = false;
                    }
                    write_replay_line(&replay_path, idx, kind, !expect_failed, &text, elapsed);
                    if let Some((l, n)) = ladder_rung(kind)
                        && !expect_failed
                        && l > reached_level
                    {
                        reached_level = l;
                        reached_name = n;
                    }
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
                    aborted_at = Some(idx);
                    // Hard failures stop the suite (build/flash/run timeouts, etc.)
                    // Monitor expect failures already handled as Ok with marker, so this is hard Io
                    break;
                }
            }
        }

        let total_elapsed = overall_start.elapsed().as_millis() as u64;
        let status = if overall_ok { "PASS" } else { "FAIL" };
        // Evidence tag (verification ladder in the system prompt): the
        // highest level the steps that ACTUALLY RAN reached, so an aborted
        // suite is never mistaken for the hardware validation it planned.
        let evidence = if dry_run {
            "dry-run — nothing was executed".to_string()
        } else {
            let reached = format!("reached level {reached_level} ({reached_name})");
            match aborted_at {
                Some(i) if i + 1 < resolved_steps.len() => format!(
                    "{reached} — aborted at step {}/{}, {} later step(s) never ran",
                    i + 1,
                    resolved_steps.len(),
                    resolved_steps.len() - i - 1
                ),
                _ => reached,
            }
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

/// Verification ladder rung a hil step reaches when it succeeds. Level 1
/// ("code") is the floor every suite starts from; `elf_analyze` and `delay`
/// do not advance the ladder.
///
/// Public because the TUI's EVIDENCE panel displays this ladder. Note the two
/// vocabularies and do not conflate them: the *kind* is the tool/step name
/// (`flash`), while the *label* is the rung's name (`deploy`).
pub fn ladder_rung(kind: &str) -> Option<(u8, &'static str)> {
    match kind {
        "build" => Some((2, "build")),
        "flash" => Some((3, "deploy")),
        "run" | "monitor" => Some((4, "runtime")),
        // SWO/ITM is RUNTIME observability, not physical behavior — level 5
        // belongs to the observe step.
        "trace" => Some((4, "runtime")),
        // Waveforms are physical behaviour measured, not asserted — same
        // rung as observe.
        "observe" | "la" => Some((5, "physical")),
        _ => None,
    }
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

/// Which kinds have already answered for the `expect_contains`/`expect_regex`/`expect_count`
/// triple inside their own runner.
///
/// `run`, `monitor` and `trace` do -- they read the port or the RTT stream and report against it.
/// Nothing else did: `build` and `flash` accepted the triple in their schema and ignored it, so a
/// suite could ask that the build log mention something and be told PASS by a build that never
/// mentioned it; `observe`, `la` and `elf_analyze` ignore it too, each for its own richer fields.
/// A new kind therefore defaults to "checked here", because forgetting in this list means the
/// dispatcher checks the step -- the safe direction.
fn answers_for_itself(kind: &str) -> bool {
    matches!(kind, "run" | "monitor" | "trace")
}

/// One step's text and whether it failed its declared expectation.
///
/// The tail is where the question gets asked once, rather than at each runner's option. For the
/// kinds that answer for themselves the marker they embed is the verdict, and the triple is NOT
/// re-checked here: their own text quotes what was wanted, so re-running `contains` over it would
/// find the expected string in the report of the failure and call it a pass.
fn step_verdict(kind: &str, expectation: Option<Expectation>, text: String) -> (String, bool) {
    if answers_for_itself(kind) {
        let failed = text.contains("[HIL_EXPECT:FAIL]");
        return (text, failed);
    }
    match expectation {
        Some(exp) => {
            let (passed, verdict) = exp.verdict(kind, &text);
            (format!("{text}\n{verdict}"), !passed)
        }
        None => (text, false),
    }
}

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
    let (text, code, end) =
        crate::tools::util::run_command(&command, &work_dir, timeout, None, Some(&ctx.cancel))
            .await
            .map_err(|e| format!("[Io] build spawn failed: {e}"))?;
    match (code, end) {
        (Some(0), _) => Ok(format!("{note}build passed (exit 0)\n{text}")),
        (_, crate::tools::util::End::Cancelled) => Err(format!(
            "[Cancelled] build step was interrupted by turn cancellation\n{note}{text}"
        )),
        (_, crate::tools::util::End::TimedOut) => Err(format!(
            "[Timeout] build timed out after {timeout} ms\n{note}{text}"
        )),
        (_, crate::tools::util::End::Killed) => Err(format!(
            "[Io] build step's process was killed by a signal (not by us)\n{note}{text}"
        )),
        (Some(c), _) => Err(format!(
            "[CompileError] build failed (exit {c})\n{note}{text}"
        )),
        (None, _) => Err(format!(
            "[Io] build step reported no exit code\n{note}{text}"
        )),
    }
}

/// Construct the flash step the red team runner uses to revive a crashed
/// target: same fields, same code path as a suite's own flash step.
pub(crate) fn flash_recovery_step(elf: &str, chip: Option<&str>, probe: Option<&str>) -> HilStep {
    HilStep {
        kind: "flash".to_string(),
        elf: Some(elf.to_string()),
        chip: chip.map(|c| c.to_string()),
        probe: probe.map(|p| p.to_string()),
        ..Default::default()
    }
}

/// Shared with the red team runner: recovery after a crash re-flashes the
/// suite's ELF through exactly this code path (one implementation, no
/// verbatim copy — see the build-step lesson comment near the bottom).
pub(crate) async fn run_flash_step(
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
    let probe_ok = crate::tools::util::probe_rs_present().await;
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

/// What a step declares it expects to see, resolved the same way for every step kind.
///
/// The check used to live inside `run_monitor_step` and nowhere else, so a `run` step carrying
/// `expect_contains` had its firmware started, any output at all accepted, and a passing line
/// written into the report: an expectation the suite paid for and never got. Both kinds now
/// resolve and evaluate through here, so the second one cannot be added and forgotten again.
#[derive(Debug)]
struct Expectation {
    contains: Option<String>,
    /// Compiled from `regex_src`, which is kept because the failure text quotes what was wanted.
    regex: Option<regex::Regex>,
    regex_src: Option<String>,
    count: usize,
}

impl Expectation {
    /// `None` when the step declares nothing to check. An `expect_regex` that does not compile is
    /// an error rather than a silently unchecked expectation.
    fn of(step: &HilStep) -> Result<Option<Self>, String> {
        let contains = step
            .expect_contains
            .clone()
            .or_else(|| step.expect.as_ref().and_then(|e| e.contains.clone()));
        let regex_src = step
            .expect_regex
            .clone()
            .or_else(|| step.expect.as_ref().and_then(|e| e.regex.clone()));
        if contains.is_none() && regex_src.is_none() {
            return Ok(None);
        }
        let regex = match &regex_src {
            Some(src) => Some(
                regex::Regex::new(src)
                    .map_err(|e| format!("[InvalidInput] expect_regex invalid: {e}"))?,
            ),
            None => None,
        };
        Ok(Some(Self {
            contains,
            regex,
            regex_src,
            count: step
                .expect_count
                .or_else(|| step.expect.as_ref().and_then(|e| e.count))
                .unwrap_or(1)
                // expect_count = 0 would pass vacuously before any byte arrives.
                .max(1),
        }))
    }

    /// `[HIL_EXPECT:PASS|FAIL] …` against captured output — `(passed, text)`.
    fn verdict(&self, what: &str, captured: &str) -> (bool, String) {
        let (matched, _total) =
            evaluate_expect(captured, self.contains.as_deref(), self.regex.as_ref());
        let ok = matched >= self.count;
        let marker = if ok { "PASS" } else { "FAIL" };
        let head = format!(
            "[HIL_EXPECT:{marker}] {what} expect matched {matched}/{}",
            self.count
        );
        if ok {
            return (true, head);
        }
        (
            false,
            format!(
                "{head} — wanted contains={:?} regex={:?} in:\n{captured}",
                self.contains, self.regex_src
            ),
        )
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
    let probe_ok = crate::tools::util::probe_rs_present().await;
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
    // Resolved before the run so an invalid `expect_regex` fails the step rather than being
    // dropped after the board has already been touched.
    let expectation = Expectation::of(step)?;
    let outcome =
        crate::tools::util::run_probe_rs(args, &ctx.cwd, timeout, Some(ctx.cancel.clone()), &[])
            .await;
    match outcome {
        Ok((text, Some(0))) => Ok(with_expect(
            &expectation,
            format!(
                "run finished (exit 0)\n{}",
                crate::forensic::append_fault_marker(text.clone())
            ),
            &text,
            "run",
        )),
        Ok((text, Some(c))) => Err(format!("[Io] run failed (exit {c})\n{text}")),
        Ok((text, None)) => Ok(with_expect(
            &expectation,
            format!("run timed out after {timeout} ms; captured:\n{text}"),
            &text,
            "run",
        )),
        Err(e) if e.contains("[Timeout]") => Ok(with_expect(
            &expectation,
            format!("run captured {timeout} ms (window closed, probe-rs timed out)\n{e}"),
            &e,
            "run",
        )),
        Err(e) => Err(crate::tools::util::probe_rs_err(e).message),
    }
}

/// A step's text with its expectation's marker appended, when it declared one. The verdict is
/// taken on `captured` — the program's own output — rather than on the decorated body, so a header
/// word cannot satisfy an expectation the firmware never met.
///
/// The marker is the whole reporting mechanism here: a failed expectation is not an error from the
/// step, it is a line the orchestrator reads, so a run that produced the wrong output still hands
/// back the output it produced.
fn with_expect(
    expectation: &Option<Expectation>,
    body: String,
    captured: &str,
    what: &str,
) -> String {
    match expectation {
        Some(e) => {
            let (_ok, marker) = e.verdict(what, captured);
            format!("{body}\n{marker}")
        }
        None => body,
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
        // Both halves of the declaration count. This used to read `expect_contains` alone, so a
        // monitor step whose expectation was a regex rehearsed as "simulated" with no verdict at
        // all — the dry run reporting nothing asked for, when something had been asked.
        if let Some(e) = Expectation::of(step)? {
            let wanted = match (e.contains.as_deref(), e.regex_src.as_deref()) {
                (Some(pat), None) => format!("expect_contains={pat:?}"),
                (None, Some(pat)) => format!("expect_regex={pat:?}"),
                (Some(c), Some(r)) => format!("expect_contains={c:?} expect_regex={r:?}"),
                (None, None) => "nothing".to_string(),
            };
            return Ok(format!(
                "[dry-run] monitor {port_display} simulated ({timeout} ms) — would check {wanted}\n[HIL_EXPECT:FAIL] dry-run cannot verify hardware output (no data)"
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

    // Expect config, resolved through the same path the `run` step uses — see `Expectation`.
    let expectation = Expectation::of(step)?;
    let expect_count = expectation
        .as_ref()
        .map(|e| e.count)
        .unwrap_or(1)
        // expect_count = 0 would pass vacuously before any byte arrives.
        .max(1);

    // Run blocking serial read in spawn_blocking, but with expect-aware early exit
    let port_clone = port.clone();
    let cancel = ctx.cancel.clone();
    let expected_text = expectation.as_ref().and_then(|e| e.contains.clone());
    let regex_for_thread = expectation.as_ref().and_then(|e| e.regex.clone());
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
    if let Some(e) = &expectation {
        // Append marker so the orchestrator can detect failure while still returning text
        let (_ok, detail) = e.verdict("monitor", &captured);
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
    let probe_ok = crate::tools::util::probe_rs_present().await;
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
    // Expectations are read once, and graded whichever way the capture window closed. The
    // resolution goes through `Expectation` like every other step that accepts `expect_*`; the
    // wording below stays trace's own, because its report line has always carried the wanted
    // pattern even when it matched.
    let expectation = Expectation::of(step)?;
    // The note used to be built only inside the exit-0 arm below. A target that never writes ITM
    // ends its window through the timeout arm — the normal shape of a dead or quiet trace stream —
    // and there the expectation was never checked: no `[HIL_EXPECT:FAIL]` marker, so
    // `expect_failed` stayed false and the suite reported level 4 (runtime) as reached while the
    // firmware had printed nothing at all.
    let expect_note = if let Some(e) = &expectation {
        let (matched, _) = evaluate_expect(&text, e.contains.as_deref(), e.regex.as_ref());
        Some(format!(
            "\n[HIL_EXPECT:{}] trace expect matched {matched}/{} — wanted contains={:?} regex={:?}",
            if matched >= e.count { "PASS" } else { "FAIL" },
            e.count,
            e.contains,
            e.regex_src,
        ))
    } else {
        None
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
            if let Some(note) = expect_note {
                out.push_str(&note);
            }
            Ok(out)
        }
        None => {
            let mut out = format!(
                "trace: capture window closed after {outer} ms on an idle SWO stream (no ITM packets — firmware must write ITM ports, e.g. ITM_SendChar)\n{text}"
            );
            if let Some(note) = expect_note {
                out.push_str(&note);
            }
            Ok(out)
        }
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
) -> Result<(Vec<image::RgbaImage>, Vec<std::path::PathBuf>), String> {
    let mut frames = Vec::with_capacity(paths.len());
    let mut sources = Vec::with_capacity(paths.len());
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
        sources.push(resolved);
    }
    Ok((frames, sources))
}

/// Validate an observe `roi` against the frame it will be applied to —
/// the same rules as the observe tool's `parse_roi` (w/h >= 1, inside the
/// frame; the `[u32; 4]` deserializer already guarantees four non-negative
/// integers). HIL used to build the Rect unchecked: the analysers clamp
/// out-of-range pixels silently, so a typo'd roi measured an empty region
/// and a brightness step could pass `expect_lit = false` on a measurement
/// of nothing.
fn validate_observe_roi(
    roi: Option<[u32; 4]>,
    width: u32,
    height: u32,
) -> Result<Option<crate::tools::observe::Rect>, String> {
    let Some(r) = roi else {
        return Ok(None);
    };
    let (x, y, w, h) = (r[0], r[1], r[2], r[3]);
    if w == 0 || h == 0 {
        return Err("[InvalidInput] observe roi width/height must be >= 1".to_string());
    }
    if x.saturating_add(w) > width || y.saturating_add(h) > height {
        return Err(format!(
            "[InvalidInput] observe roi [{x},{y},{w},{h}] exceeds the frame ({width}x{height})"
        ));
    }
    Ok(Some(crate::tools::observe::Rect { x, y, w, h }))
}

/// Copy one measured frame into `.firment/observe/` and append the result to
/// the step output. Archiving is a side effect of a measurement that already
/// succeeded: a failed copy warns but never discards the verdict or the
/// `[HIL_EXPECT]` marker that follows.
fn hil_save(ctx: &ToolContext, src: &std::path::Path, tag: &str, out: &mut String) {
    match crate::tools::observe::save_copy(ctx, src, tag) {
        Ok(dest) => out.push_str(&format!("\n  saved: {}", dest.display())),
        Err(e) => out.push_str(&format!("\n  save failed: {e}")),
    }
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
    let roi = validate_observe_roi(step.roi, frame.width(), frame.height())?;
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
        hil_save(ctx, &resolved, "", &mut out);
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
    let (frames, sources) = load_observe_frames(ctx, paths)?;
    let roi = validate_observe_roi(step.roi, frames[0].width(), frames[0].height())?;
    let m = crate::tools::observe::analyze_motion(
        &frames,
        roi,
        step.pixel_threshold
            .unwrap_or(crate::tools::observe::MOTION_PIXEL_THRESHOLD),
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
    if step.save == Some(true) {
        for (i, src) in sources.iter().enumerate() {
            hil_save(ctx, src, &format!("-{i}"), &mut out);
        }
    }
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
    let (frames, sources) = load_observe_frames(ctx, paths)?;
    // One burst = one camera session: frames of mixed dimensions would
    // silently produce a garbage luma series. motion/diff already error on
    // a size mismatch — blink must too.
    let (w0, h0) = (frames[0].width(), frames[0].height());
    if frames.iter().any(|f| f.width() != w0 || f.height() != h0) {
        return Err(
            "[InvalidInput] observe mode=blink requires all frames to share the same dimensions"
                .to_string(),
        );
    }
    let roi = validate_observe_roi(step.roi, w0, h0)?;
    // A frequency needs a time base; a burst of photos carries none unless
    // the user states the gap. Without it the sample series is t_ms = 0
    // everywhere, which turns period into 0 and the range into `inf .. 1000
    // Hz` — refuse instead (same rule as the tool itself).
    let Some(interval_ms) = step.interval_ms else {
        return Err(
            "[InvalidInput] observe mode=blink requires interval_ms — the gap between shots, \
             which only you know. Without a time base a frequency is invented."
                .to_string(),
        );
    };
    if interval_ms == 0 {
        return Err("[InvalidInput] interval_ms must be > 0".to_string());
    }
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
        "observe mode=blink ({} frames, interval_ms={interval_ms})
  blinking: {}
  edges: {}
  confidence: {} — {}",
        b.samples,
        if b.blinking { "yes" } else { "no" },
        b.edges,
        b.confidence.name(),
        b.note,
    );
    if let (Some(low), Some(high)) = (b.hz_low, b.hz_high) {
        out.push_str(&format!("\n  frequency: {low:.2} .. {high:.2} Hz"));
    }
    if step.save == Some(true) {
        for (i, src) in sources.iter().enumerate() {
            hil_save(ctx, src, &format!("-{i}"), &mut out);
        }
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
        // Only a real measurement can answer this. A LOW-confidence number
        // must never pass an assertion.
        let (marker, reason) = if matches!(b.confidence, crate::tools::observe::Confidence::Low) {
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
    let load = |p: &str| -> Result<(image::RgbaImage, std::path::PathBuf), String> {
        let resolved = crate::tools::util::resolve_within(&ctx.cwd, p, &ctx.allowed_roots)
            .map_err(|e| format!("[Permission] {p}: {e}"))?;
        Ok((
            image::open(&resolved)
                .map_err(|e| format!("[Io] cannot decode {p}: {e}"))?
                .to_rgba8(),
            resolved,
        ))
    };
    let (before, before_src) = load(path)?;
    let (after_frame, after_src) = load(after)?;
    let roi = validate_observe_roi(step.roi, before.width(), before.height())?;
    let d = crate::tools::observe::analyze_diff(
        &before,
        &after_frame,
        roi,
        step.pixel_threshold
            .unwrap_or(crate::tools::observe::MOTION_PIXEL_THRESHOLD),
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
    if step.save == Some(true) {
        for (src, tag) in [(&before_src, "-before"), (&after_src, "-after")] {
            hil_save(ctx, src, tag, &mut out);
        }
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

/// la: capture from a logic analyzer (or assert against a stored capture)
/// and check measurements. A LOW-confidence "yes" is not evidence — the
/// same rule the observe steps apply.
async fn run_la_step(
    step: &HilStep,
    ctx: &ToolContext,
    dry_run: bool,
    remaining: u64,
) -> Result<String, String> {
    run_la_step_with(
        &crate::tools::la::La::default(),
        step,
        ctx,
        dry_run,
        remaining,
    )
    .await
}

async fn run_la_step_with(
    tool: &crate::tools::la::La,
    step: &HilStep,
    ctx: &ToolContext,
    dry_run: bool,
    remaining: u64,
) -> Result<String, String> {
    use firment_core::Tool as _;
    let mut wants = Vec::new();
    if let Some(hz) = step.expect_frequency_hz {
        wants.push(format!("expect_frequency_hz={hz}"));
    }
    if let Some(d) = step.expect_duty {
        wants.push(format!("expect_duty={d}"));
    }
    if let Some(n) = step.expect_edges {
        wants.push(format!("expect_edges={n}"));
    }
    if let Some(s) = &step.expect_decoded {
        wants.push(format!("expect_decoded='{s}'"));
    }
    if dry_run {
        // Mirror run_observe_step: with no expectation there is nothing to
        // fake, so the suite stays green. Any expectation must FAIL — a dry
        // run captured no samples, and "would have checked" is not evidence.
        return Ok(if wants.is_empty() {
            "[dry-run] la simulated — would capture, nothing to assert".to_string()
        } else {
            format!(
                "[dry-run] la simulated — would capture and check {}\n\
                 [HIL_EXPECT:FAIL] dry-run cannot verify hardware output (no samples)",
                wants.join(", ")
            )
        });
    }
    // The suite-level approval already covered hardware steps: auto-approve
    // the child context (same mechanism as the elf step).
    let child_ctx = firment_core::ToolContext {
        permission: std::sync::Arc::new(firment_core::AutoApprove::everything()),
        ..ctx.clone()
    };
    let id = match &step.capture {
        Some(existing) => existing.clone(),
        None => {
            let mut args = serde_json::json!({"action": "capture"});
            for (k, v) in [
                ("driver", &step.driver),
                ("channels", &step.channels),
                ("samplerate", &step.samplerate),
            ] {
                if let Some(v) = v {
                    args[k] = serde_json::json!(v);
                }
            }
            // The suite's total-time budget is a hard ceiling: a capture
            // window that cannot fit in the remaining ms must be refused
            // (or clamped for time-based windows), not silently overrun.
            if let Some(n) = step.samples {
                let sr = step
                    .samplerate
                    .as_deref()
                    .or(ctx.la.as_ref().and_then(|l| l.samplerate.as_deref()))
                    .and_then(crate::la_cmd::samplerate_hz);
                if let Some(sr) = sr {
                    let est_ms = n as f64 / sr * 1000.0;
                    if est_ms > remaining as f64 {
                        return Err(format!(
                            "[InvalidInput] la capture of {n} samples at {sr:.0} Hz needs \
                             ~{est_ms:.0} ms but only {remaining} ms of suite budget remain — \
                             shorten the window or raise the suite timeout_ms"
                        ));
                    }
                }
                args["samples"] = serde_json::json!(n);
            }
            if let Some(t) = step.duration_ms {
                args["time_ms"] = serde_json::json!(t.min(remaining.max(1)));
            }
            let out = tool.run(args, &child_ctx).await.map_err(|e| e.message)?;
            out.text
                .lines()
                .next()
                .and_then(|l| l.strip_prefix("[la] captured "))
                .map(|s| s.trim().to_string())
                .ok_or_else(|| "[Io] la capture returned no capture id".to_string())?
        }
    };
    let mut out = format!("la capture {id}");
    let needs_waves = step.expect_frequency_hz.is_some()
        || step.expect_duty.is_some()
        || step.expect_edges.is_some();
    let cap = if needs_waves {
        Some(crate::tools::la::load_capture_waves(&child_ctx, &id)?)
    } else {
        None
    };
    let channel = step.channel.unwrap_or(0);
    let wave_of = || -> Result<&Vec<u8>, String> {
        let cap = cap.as_ref().expect("loaded above");
        cap.waves.get(channel).ok_or_else(|| {
            format!(
                "[InvalidInput] channel {channel} out of range ({} captured)",
                cap.channel_count
            )
        })
    };
    let marker = |ok: bool| if ok { "PASS" } else { "FAIL" };
    if let Some(want) = step.expect_frequency_hz {
        let wave = wave_of()?;
        let hz = cap.as_ref().unwrap().samplerate_hz.ok_or(
            "[InvalidInput] expect_frequency_hz needs a capture with a known samplerate — set samplerate= on the step or [tools.la]",
        )?;
        let f = crate::la_measure::measure_frequency(wave, hz);
        let (ok, why) = match &f {
            Some(f)
                if !matches!(f.confidence, crate::tools::observe::Confidence::Low)
                    && f.hz_low <= want
                    && want <= f.hz_high =>
            {
                (
                    true,
                    format!(
                        "{want} Hz inside the measured {}..{} Hz range",
                        f.hz_low, f.hz_high
                    ),
                )
            }
            Some(f) if matches!(f.confidence, crate::tools::observe::Confidence::Low) => {
                (false, format!("low confidence — {}", f.note))
            }
            Some(f) => (
                false,
                format!(
                    "{want} Hz outside the measured {}..{} Hz range",
                    f.hz_low, f.hz_high
                ),
            ),
            None => (
                false,
                "not periodic (fewer than two rising edges)".to_string(),
            ),
        };
        out.push_str(&format!(
            "\n[HIL_EXPECT:{}] la frequency: {why}",
            marker(ok)
        ));
    }
    if let Some(want) = step.expect_duty {
        let wave = wave_of()?;
        let d = crate::la_measure::measure_duty(wave);
        let (ok, why) = match &d {
            Some(d)
                if !matches!(d.confidence, crate::tools::observe::Confidence::Low)
                    && (d.fraction - want).abs() <= 0.1 =>
            {
                (
                    true,
                    format!(
                        "duty {:.1}% within ±10 points of {want}",
                        d.fraction * 100.0
                    ),
                )
            }
            Some(d) if matches!(d.confidence, crate::tools::observe::Confidence::Low) => {
                (false, format!("low confidence — {}", d.note))
            }
            Some(d) => (
                false,
                format!(
                    "duty {:.1}% more than 10 points from {want}",
                    d.fraction * 100.0
                ),
            ),
            None => (false, "not periodic (needs two rising edges)".to_string()),
        };
        out.push_str(&format!("\n[HIL_EXPECT:{}] la duty: {why}", marker(ok)));
    }
    if let Some(want) = step.expect_edges {
        let wave = wave_of()?;
        let got = crate::la_measure::count_edges(wave, crate::la_measure::EdgeKind::Both);
        let ok = got == want;
        out.push_str(&format!(
            "\n[HIL_EXPECT:{}] la edges: {got} transitions, wanted {want}",
            marker(ok)
        ));
    }
    if let Some(want) = &step.expect_decoded {
        let mut args = serde_json::json!({
            "action": "decode", "capture": id,
            "decoder": step.decoder.as_deref().unwrap_or("uart"),
        });
        if let Some(opts) = &step.decoder_opts {
            args["decoder_opts"] = serde_json::to_value(opts).unwrap_or(serde_json::Value::Null);
        }
        let text = tool
            .run(args, &child_ctx)
            .await
            .map_err(|e| e.message)?
            .text;
        let ok = text.contains(want.as_str());
        out.push_str(&format!(
            "\n[HIL_EXPECT:{}] la decoded: {}",
            marker(ok),
            if ok {
                format!("'{want}' found in the decoded frames")
            } else {
                format!("'{want}' not present in the decoded output")
            }
        ));
    }
    if wants.is_empty() {
        out.push_str("\n(no expectations — capture stored for later la measure/decode)");
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
    use crate::tools::monitor::{RECONNECT_ATTEMPTS, RECONNECT_DELAY, budget_spent, open_port};
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

    let mut reader = open_port(port, resolved_baud)?;
    let mut reopen_attempts: u32 = 0;
    // The count of **device** lines at the moment of the last drop: a capture that grew since
    // then is evidence the port works, and the next drop is a new outage with a fresh budget.
    // The delivered count is tracked apart from `lines` because `lines` is also where the drop
    // and reopen notes go, and a budget reset by one's own message is a budget that never runs
    // out. `monitor`'s loop gets this right by reading its device-only string.
    let mut delivered: usize = 0;
    let mut lines_at_loss: usize = 0;

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
    // The same ceiling as `monitor`'s loop, from the same constant: a chatty target under a long
    // step timeout was only ever cut on the way out, after every byte had been collected. The
    // assertions below keep running on every line either way — cutting the stored text must not
    // silently decide an expectation.
    let mut stored_bytes = 0usize;
    let mut stored_capped = false;
    let mut handle_line = |line: &str, lines: &mut Vec<String>, matched: &mut usize| {
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
        if stored_bytes < crate::tools::monitor::CAPTURE_CAP_BYTES {
            stored_bytes += with_ts.len();
            lines.push(with_ts);
        } else if !stored_capped {
            lines.push(format!(
                "(hil monitor step: capture reached {} MiB and stopped storing lines — \
                 expectations were still checked against what followed)",
                crate::tools::monitor::CAPTURE_CAP_BYTES / (1024 * 1024)
            ));
            stored_capped = true;
        }
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
                delivered += 1;
                handle_line(line, &mut lines, &mut matched)
            }),
            // Silence is an idle port, not a fault.
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                continue;
            }
            Err(e) => {
                // A cable that moves mid-capture must not fail the run and throw the
                // capture away: the expect assertions are the point, and the lines the
                // device already printed are the evidence for why they were or were not
                // met. Same policy and budget as `monitor`'s capture, including the
                // reset on a delivering stretch.
                if delivered > lines_at_loss {
                    reopen_attempts = 0;
                }
                lines_at_loss = delivered;
                lines.push(format!("(serial port dropped: {e})"));
                if budget_spent(&mut reopen_attempts) {
                    lines.push(format!(
                        "(serial port lost after {reopen_attempts} reopen attempt(s): {e})"
                    ));
                    break;
                }
                std::thread::sleep(RECONNECT_DELAY);
                match open_port(port, resolved_baud) {
                    Ok(reopened) => {
                        reader = reopened;
                        lines.push(format!(
                            "(serial port back after {reopen_attempts} of {RECONNECT_ATTEMPTS} reopen attempt(s))"
                        ));
                    }
                    Err(e) => lines.push(format!("(reopen failed: {e})")),
                }
            }
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
    // Build a child context that inherits everything except the interactive
    // and self-referential parts: permission is replaced with auto-approve
    // (the suite approval already covered this step), and subagent/asker/web
    // are stripped. Struct-update on a clone so the next ToolContext field
    // does not silently miss the inheritance (the old full literal meant a
    // new field was a compile error HERE, easy to fix wrong under time
    // pressure).
    let child_ctx = ToolContext {
        permission: std::sync::Arc::new(firment_core::AutoApprove::everything()),
        subagent: None,
        asker: None,
        web_search_provider: None,
        web_search_api_key: None,
        providers: Vec::new(),
        ..ctx.clone()
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

    #[test]
    fn ladder_rung_maps_steps_to_verification_levels() {
        assert_eq!(ladder_rung("build"), Some((2, "build")));
        assert_eq!(ladder_rung("flash"), Some((3, "deploy")));
        assert_eq!(ladder_rung("run"), Some((4, "runtime")));
        // SWO/ITM is runtime observability, not physical behavior.
        assert_eq!(ladder_rung("trace"), Some((4, "runtime")));
        assert_eq!(ladder_rung("observe"), Some((5, "physical")));
        assert_eq!(ladder_rung("la"), Some((5, "physical")));
        assert_eq!(ladder_rung("delay"), None);
        assert_eq!(ladder_rung("elf_analyze"), None);
    }

    #[tokio::test]
    async fn aborted_suite_reports_only_the_level_it_reached() {
        let dir = tempdir().unwrap();
        let mut c = ctx(dir.path());
        // Any real command that exists makes the build step pass.
        c.build_command = Some("cargo --version".to_string());
        let err = Hil
            .run(
                json!({"steps": [
                    {"kind": "build"},
                    // No elf/file: a hard [InvalidInput] failure that stops
                    // the suite before the observe step ever runs.
                    {"kind": "flash"},
                    {"kind": "observe", "file": "frame.png"}
                ]}),
                &c,
            )
            .await
            .unwrap_err();
        let msg = err.message;
        assert!(
            msg.contains("reached level 2 (build)"),
            "must report the rung the succeeding steps reached: {msg}"
        );
        assert!(
            !msg.contains("level 5"),
            "a never-run observe step must not buy physical evidence: {msg}"
        );
        assert!(
            msg.contains("aborted at step 2/3, 1 later step(s) never ran"),
            "got: {msg}"
        );
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

    /// A `run` step accepts `expect_contains` / `expect_regex` / `expect_count` and used to ignore
    /// all three: the check lived inside `run_monitor_step` only, so a suite that declared what it
    /// expected to see from a run got a passing step whatever the firmware printed.
    /// The tail that decides, for the kinds that never decided for themselves.
    #[test]
    fn a_build_or_flash_step_that_ignores_its_expectation_no_longer_passes() {
        let contains = |kind: &str, needle: &str| {
            Expectation::of(&HilStep {
                kind: kind.to_string(),
                expect_contains: Some(needle.to_string()),
                ..Default::default()
            })
            .expect("resolves")
            .expect("the step declared one")
        };
        let regex = |kind: &str, pattern: &str| {
            Expectation::of(&HilStep {
                kind: kind.to_string(),
                expect_regex: Some(pattern.to_string()),
                ..Default::default()
            })
            .expect("compiles")
            .expect("the step declared one")
        };

        // The case that used to be a silent pass: a build that finished and never mentioned the
        // figure the suite asked for.
        let (text, failed) = step_verdict(
            "build",
            Some(contains("build", "Flash: 81% used")),
            "Compiling board.c\nFinished in 4.2s\n".to_string(),
        );
        assert!(failed, "an unmet expectation must fail the step: {text}");
        assert!(text.contains("[HIL_EXPECT:FAIL]"), "{text}");
        assert!(
            text.contains("Flash: 81% used"),
            "the report must name what was wanted: {text}"
        );

        let (text, failed) = step_verdict(
            "flash",
            Some(regex("flash", "Download completed")),
            "probe-rs download\nWrote 4096 bytes in 0.8s\n".to_string(),
        );
        assert!(
            failed,
            "flash accepted `expect_regex` and read it nowhere: {text}"
        );

        // Met, so the tail is not simply failing everything: the same step with the line present.
        let (text, failed) = step_verdict(
            "flash",
            Some(regex("flash", "Download completed")),
            "probe-rs download\nDownload completed in 0.8s\n".to_string(),
        );
        assert!(!failed, "{text}");
        assert!(text.contains("[HIL_EXPECT:PASS]"), "{text}");

        // The trap that keeps the kinds which grade themselves out of this tail: their own text
        // quotes the expectation, so re-checking `contains` over it would find the wanted string
        // in the report of a failure. A `run` step's marker is the verdict, unchanged.
        let self_graded = "rtt: expected boot ok\n[HIL_EXPECT:FAIL] run expect matched 0/1\n";
        let (text, failed) = step_verdict(
            "run",
            Some(contains("run", "boot ok")),
            self_graded.to_string(),
        );
        assert!(failed, "the step's own verdict still decides: {text}");
        assert_eq!(text, self_graded, "and it is not graded a second time");

        // Nothing declared is not a failure, for either family of kind.
        let (text, failed) = step_verdict("build", None, "ok\n".to_string());
        assert!(!failed);
        assert_eq!(text, "ok\n");
    }

    #[test]
    fn a_declared_expectation_grades_whatever_output_a_step_produced() {
        let step = HilStep {
            kind: "run".to_string(),
            expect_contains: Some("boot ok".to_string()),
            ..Default::default()
        };
        let expect = Expectation::of(&step)
            .expect("resolves")
            .expect("the step declared one");
        let (ok, text) = expect.verdict("run", "reset\nboot ok\nready\n");
        assert!(ok, "{text}");
        assert!(
            text.contains("[HIL_EXPECT:PASS] run expect matched 1/1"),
            "{text}"
        );

        let (ok, text) = expect.verdict("run", "silence");
        assert!(!ok, "nothing matched, so the step did not pass: {text}");
        assert!(text.contains("[HIL_EXPECT:FAIL]"), "{text}");

        // Nothing declared is not a failed check; an unusable pattern is an error rather than an
        // expectation that quietly goes unmade.
        assert!(Expectation::of(&HilStep::default()).unwrap().is_none());
        let bad = HilStep {
            expect_regex: Some("(".to_string()),
            ..Default::default()
        };
        assert!(
            Expectation::of(&bad)
                .unwrap_err()
                .contains("expect_regex invalid")
        );
    }

    #[tokio::test]
    async fn a_dry_run_reports_an_expectation_it_cannot_check_even_as_a_regex() {
        let dir = tempdir().unwrap();
        let step = HilStep {
            kind: "monitor".to_string(),
            port: Some("COM9".to_string()),
            expect_regex: Some("LED ON".to_string()),
            ..Default::default()
        };
        let out = run_monitor_step(&step, &ctx(dir.path()), true, 60_000)
            .await
            .unwrap();
        assert!(out.contains("expect_regex=\"LED ON\""), "got: {out}");
        assert!(out.contains("[HIL_EXPECT:FAIL]"), "got: {out}");
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

    #[tokio::test]
    async fn observe_step_validates_roi_like_the_tool() {
        let dir = tempdir().unwrap();
        solid(240).save(dir.path().join("led.png")).unwrap();
        // An in-bounds roi must behave exactly as before the validation.
        let in_bounds = HilStep {
            kind: "observe".to_string(),
            file: Some("led.png".to_string()),
            roi: Some([60, 60, 4, 4]),
            expect_lit: Some(true),
            ..Default::default()
        };
        let out = run_observe_step(&in_bounds, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[HIL_EXPECT:PASS]"), "got: {out}");

        // An out-of-bounds roi used to clamp to an empty measurement region
        // and pass `expect_lit = false` on a measurement of nothing. It must
        // fail loudly instead.
        let out_of_bounds = HilStep {
            roi: Some([60, 60, 10, 10]),
            expect_lit: Some(false),
            ..in_bounds
        };
        let err = run_observe_step(&out_of_bounds, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap_err();
        assert!(
            err.contains("exceeds the frame"),
            "a typo'd roi must fail loudly: {err}"
        );
    }

    #[tokio::test]
    async fn observe_step_blink_rejects_mixed_frame_sizes() {
        let dir = tempdir().unwrap();
        for i in 0..3usize {
            let luma = if i % 2 == 0 { 240u8 } else { 10u8 };
            let f = if i == 2 {
                image::RgbaImage::from_pixel(32, 32, image::Rgba([luma, luma, luma, 255]))
            } else {
                solid(luma)
            };
            f.save(dir.path().join(format!("b{i}.png"))).unwrap();
        }
        let paths: Vec<String> = (0..3).map(|i| format!("b{i}.png")).collect();
        let step = HilStep {
            kind: "observe".to_string(),
            mode: Some("blink".to_string()),
            paths: Some(paths),
            interval_ms: Some(100),
            expect_blinking: Some(true),
            ..Default::default()
        };
        let err = run_observe_step(&step, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap_err();
        assert!(
            err.contains("same dimensions"),
            "a mixed-size burst must fail loudly, not produce garbage: {err}"
        );
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
    async fn observe_step_save_covers_sequence_and_diff_modes() {
        // `save` used to be implemented for brightness only; motion and diff
        // silently ignored it. Every mode must copy what it measured.
        let dir = tempdir().unwrap();
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
            save: Some(true),
            ..Default::default()
        };
        let out = run_observe_step(&step, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert_eq!(out.matches("saved:").count(), 2, "got: {out}");

        let diff = HilStep {
            kind: "observe".to_string(),
            mode: Some("diff".to_string()),
            file: Some("m1.png".to_string()),
            after: Some("m2.png".to_string()),
            save: Some(true),
            ..Default::default()
        };
        let out = run_observe_step(&diff, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert_eq!(out.matches("saved:").count(), 2, "got: {out}");
        assert!(out.contains("-before-m1.png"), "got: {out}");
        assert!(out.contains("-after-m2.png"), "got: {out}");
        // 2 motion frames + 2 diff frames.
        assert_eq!(
            std::fs::read_dir(dir.path().join(".firment/observe"))
                .unwrap()
                .count(),
            4
        );
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
        let err = run_observe_step(&step, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap_err();
        assert!(
            err.contains("interval_ms"),
            "a missing time base must fail loudly: {err}"
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

    // ----- la step ---------------------------------------------------------

    /// Write a stored capture (raw-bit sidecar + meta) straight to disk —
    /// the assertion path reads files, it does not exec sigrok-cli.
    fn write_capture(dir: &std::path::Path, id: &str, bytes: &[u8], sr_hz: Option<f64>) {
        let la = dir.join(".firment").join("la");
        std::fs::create_dir_all(&la).unwrap();
        std::fs::write(la.join(format!("{id}.bin")), bytes).unwrap();
        std::fs::write(la.join(format!("{id}.sr")), b"stub").unwrap();
        let meta = serde_json::json!({
            "id": id, "driver": "demo", "channels": "0", "channel_count": 1,
            "samplerate": "8m", "samplerate_hz": sr_hz, "samples": bytes.len(),
            "time_ms": null, "created_unix": 0, "has_binary": true
        });
        std::fs::write(la.join(format!("{id}.meta.json")), meta.to_string()).unwrap();
    }

    /// 1-channel square wave: `period` samples per cycle, 50% duty, starting
    /// high. One byte per sample, channel 0 in bit 0.
    fn square_bytes(period: usize, cycles: usize) -> Vec<u8> {
        (0..period * cycles)
            .map(|i| if i % period < period / 2 { 1 } else { 0 })
            .collect()
    }

    #[test]
    fn hil_toml_parses_la_step_fields() {
        let toml_text = r#"
[suite.wave]
[[suite.wave.steps]]
kind = "la"
driver = "fx2lafw"
channels = "0,1"
samplerate = "8m"
samples = 400000
expect_frequency_hz = 1000000.0
expect_edges = 9
decoder = "uart"
decoder_opts = { rx = "0", baudrate = "115200" }
expect_decoded = "0x55"
"#;
        let file: HilFile = toml::from_str(toml_text).unwrap();
        let s = &file.suite.get("wave").unwrap().steps[0];
        assert_eq!(s.driver.as_deref(), Some("fx2lafw"));
        assert_eq!(s.expect_frequency_hz, Some(1e6));
        assert_eq!(s.expect_edges, Some(9));
        assert_eq!(
            s.decoder_opts
                .as_ref()
                .unwrap()
                .get("baudrate")
                .map(String::as_str),
            Some("115200")
        );
        // deny_unknown_fields still guards the new surface.
        assert!(
            toml::from_str::<HilFile>(
                "[suite.x]\n[[suite.x.steps]]\nkind = \"la\"\nexpect_freq = 1\n"
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn la_step_dry_run_fails_expectations() {
        let dir = tempdir().unwrap();
        let step = HilStep {
            kind: "la".to_string(),
            capture: Some("whatever".to_string()),
            expect_frequency_hz: Some(1e6),
            ..Default::default()
        };
        let out = run_la_step(&step, &ctx(dir.path()), true, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[dry-run]"), "got: {out}");
        assert!(
            out.contains("[HIL_EXPECT:FAIL]") && out.contains("no samples"),
            "dry-run must not fake hardware evidence: {out}"
        );
    }

    #[tokio::test]
    async fn la_step_capture_window_respects_suite_budget() {
        // 1e9 samples at 8 MHz is a 125-second window — with only 1 s of
        // suite budget left the step must refuse BEFORE touching hardware.
        let dir = tempdir().unwrap();
        let step = HilStep {
            kind: "la".to_string(),
            driver: Some("demo".to_string()),
            channels: Some("0".to_string()),
            samplerate: Some("8m".to_string()),
            samples: Some(1_000_000_000),
            ..Default::default()
        };
        let err = run_la_step(&step, &ctx(dir.path()), false, 1_000)
            .await
            .unwrap_err();
        assert!(err.contains("budget"), "got: {err}");
    }

    #[tokio::test]
    async fn la_step_dry_run_without_expectations_does_not_fail_the_suite() {
        let dir = tempdir().unwrap();
        // A dry run with nothing to assert is not a failed expectation — it
        // is a rehearsal. Only an expectation must fail (no samples to
        // measure), which is what the neighbouring test pins.
        let step = HilStep {
            kind: "la".to_string(),
            capture: Some("whatever".to_string()),
            ..Default::default()
        };
        let out = run_la_step(&step, &ctx(dir.path()), true, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[dry-run]"), "got: {out}");
        assert!(
            !out.contains("[HIL_EXPECT"),
            "a rehearsal with no expectation must not fail the suite: {out}"
        );
    }

    #[tokio::test]
    async fn la_step_asserts_stored_capture() {
        let dir = tempdir().unwrap();
        // 8-sample period at 8 MHz = 1 MHz, four rising edges (HIGH), nine
        // transitions total.
        write_capture(dir.path(), "t1", &square_bytes(8, 5), Some(8e6));
        let freq = HilStep {
            kind: "la".to_string(),
            capture: Some("t1".to_string()),
            expect_frequency_hz: Some(1e6),
            ..Default::default()
        };
        let out = run_la_step(&freq, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[HIL_EXPECT:PASS]"), "got: {out}");

        let wrong = HilStep {
            expect_frequency_hz: Some(5e6),
            ..freq
        };
        let out = run_la_step(&wrong, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[HIL_EXPECT:FAIL]"), "got: {out}");
        assert!(out.contains("outside the measured"), "got: {out}");

        let edges = HilStep {
            expect_frequency_hz: None,
            expect_edges: Some(9),
            ..wrong
        };
        let out = run_la_step(&edges, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[HIL_EXPECT:PASS]"), "got: {out}");

        let duty = HilStep {
            expect_edges: None,
            expect_duty: Some(0.5),
            ..edges
        };
        let out = run_la_step(&duty, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[HIL_EXPECT:PASS]"), "got: {out}");
    }

    #[tokio::test]
    async fn la_step_low_confidence_cannot_pass() {
        let dir = tempdir().unwrap();
        // 3-sample period at 8 MHz: under 4x oversampling — the measurement
        // is LOW confidence and must not carry an assertion.
        write_capture(dir.path(), "t2", &square_bytes(3, 5), Some(8e6));
        let step = HilStep {
            kind: "la".to_string(),
            capture: Some("t2".to_string()),
            expect_frequency_hz: Some(8e6 / 3.0),
            ..Default::default()
        };
        let out = run_la_step(&step, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(
            out.contains("[HIL_EXPECT:FAIL]") && out.contains("low confidence"),
            "a LOW-confidence frequency must not pass: {out}"
        );
    }

    #[tokio::test]
    async fn la_step_decoded_expectation_uses_the_backend() {
        let dir = tempdir().unwrap();
        write_capture(dir.path(), "t1", &square_bytes(8, 5), Some(8e6));
        let tool = crate::tools::la::La::with_backend(std::sync::Arc::new(
            crate::tools::la::FakeBackend::new(vec![]),
        ));
        let step = HilStep {
            kind: "la".to_string(),
            capture: Some("t1".to_string()),
            decoder: Some("uart".to_string()),
            expect_decoded: Some("0x55".to_string()),
            ..Default::default()
        };
        let out = run_la_step_with(&tool, &step, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[HIL_EXPECT:PASS]"), "got: {out}");

        let missing = HilStep {
            expect_decoded: Some("0xEE".to_string()),
            ..step
        };
        let out = run_la_step_with(&tool, &missing, &ctx(dir.path()), false, 60_000)
            .await
            .unwrap();
        assert!(out.contains("[HIL_EXPECT:FAIL]"), "got: {out}");
    }

    /// `docs/hil-example.toml` is the file a user copies to `.firment/hil.toml`, and nothing ever
    /// parsed it. Every suite struct here is `deny_unknown_fields` precisely so a typo'd
    /// expectation fails loudly — which means a renamed field would turn the *published example*
    /// into a file that cannot be loaded, and the error would land on the person who followed the
    /// instructions rather than on whoever renamed the field.
    #[test]
    fn the_hil_example_still_parses_as_a_suite_file() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/hil-example.toml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let file: HilFile = toml::from_str(&text)
            .unwrap_or_else(|e| panic!("docs/hil-example.toml no longer parses: {e}"));
        // A floor, so this cannot pass by parsing an empty file: the example exists to show every
        // step kind, and it is where a user learns the vocabulary.
        assert!(
            file.suite.len() >= 5,
            "expected the showcase suites; found {}",
            file.suite.len()
        );
        for (name, suite) in &file.suite {
            assert!(
                !suite.steps.is_empty(),
                "suite {name:?} in the published example has no steps"
            );
        }
    }
}
