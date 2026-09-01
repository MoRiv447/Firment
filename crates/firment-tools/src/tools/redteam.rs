//! `redteam` — runtime adversarial verification: the agent attacks the
//! firmware it wrote and only counts what the target's own output proves.
//!
//! Design (v0.8.0):
//! * **Declarative suites** in `.firment/redteam.toml`, same skeleton as
//!   HIL: `deny_unknown_fields` (a typo'd key must never silently drop an
//!   expectation), one approval covering the whole suite, JSONL replay,
//!   dry-run that cannot fake evidence.
//! * **Two-layer pyramid**: the deterministic mutation corpus
//!   ([`crate::redteam::mutate`]) is the CI-runnable regression floor — a
//!   finding's reproducer is `seed + case id`, no LLM needed. The LLM
//!   attacker campaign explores on top (action=campaign) and its findings
//!   must be back-ported into corpus cases to enter the report.
//! * **Evidence-capped findings**: the oracle classifies captured output;
//!   findings cite the capture files and are capped to low/UNVERIFIED
//!   without existing evidence.
//! * **Recovery**: a crashed target is re-flashed or reset between cases
//!   (the suite declares how); a target that cannot be revived aborts the
//!   run rather than producing garbage verdicts against a dead board.

use super::util::{resolve_within, truncate};
use crate::redteam::findings::{self, Finding, Severity};
use crate::redteam::mutate::{self, Mutation};
use crate::redteam::oracle::{self, OracleCfg, Verdict};
use async_trait::async_trait;
use firment_core::{Cancellable, Tool, ToolContext, ToolError, ToolOutput};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Suite schema
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RedteamFile {
    suite: HashMap<String, RedteamSuite>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct RedteamSuite {
    pub chip: Option<String>,
    pub elf: Option<String>,
    pub probe: Option<String>,
    /// reflash | reset | none — how to revive the target after a finding.
    pub recovery: Option<String>,
    pub budget: Budget,
    pub allowed_actions: Vec<String>,
    pub interfaces: Vec<Interface>,
    pub oracle: OracleCfg,
    pub mutation: Option<MutationCfg>,
    pub report: ReportCfg,
    /// Opt the LLM attacker campaign into a run (default off): the campaign
    /// explores on top of the deterministic corpus but is interactive-only
    /// unless explicitly enabled — an unattended model-driven attack is
    /// neither reviewable nor reproducible.
    pub llm_phase: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Budget {
    pub max_cases: usize,
    pub max_duration_ms: u64,
    pub per_case_timeout_ms: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_cases: 200,
            max_duration_ms: 600_000,
            per_case_timeout_ms: 3_000,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Interface {
    /// uart (implemented) | rtt | device_cmd (reserved for v0.9.x).
    pub kind: String,
    pub port: Option<String>,
    pub baud: Option<u32>,
    pub node: Option<String>,
    /// Seed frame the mutations operate on: `hex:55AA0102` or
    /// `text:LED ON\n`.
    pub baseline: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct MutationCfg {
    /// REQUIRED when strategies are listed: an unseeded corpus is not
    /// reproducible, and reproducibility is the whole point.
    pub seed: Option<u64>,
    pub strategies: Vec<Mutation>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct ReportCfg {
    pub dir: Option<String>,
    pub min_severity: Option<Severity>,
}

fn parse_baseline(s: Option<&str>) -> Result<Vec<u8>, String> {
    let Some(s) = s else {
        return Ok(Vec::new());
    };
    if let Some(hex) = s.strip_prefix("hex:") {
        let hex = hex.trim();
        if hex.len() % 2 != 0 || hex.chars().any(|c| !c.is_ascii_hexdigit()) {
            return Err(format!(
                "[InvalidInput] baseline hex '{hex}' is not an even-length hex string"
            ));
        }
        return Ok((0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("validated"))
            .collect());
    }
    if let Some(text) = s.strip_prefix("text:") {
        return Ok(text.as_bytes().to_vec());
    }
    Err(format!(
        "[InvalidInput] baseline '{s}' needs a prefix: hex:55AA… or text:… — guessing the \
         encoding of an attack seed is not allowed"
    ))
}

fn validate_suite(suite: &RedteamSuite) -> Result<(), String> {
    if suite.interfaces.is_empty() {
        return Err("[InvalidInput] suite has no [[interfaces]] — nothing to attack".to_string());
    }
    for iface in &suite.interfaces {
        match iface.kind.as_str() {
            "uart" => {
                if iface.port.is_none() {
                    return Err(
                        "[InvalidInput] uart interface needs port (e.g. \"COM3\")".to_string()
                    );
                }
            }
            "rtt" | "device_cmd" => {
                return Err(format!(
                    "[InvalidInput] interface kind='{}' is not implemented in v0.8.0 — use \
                     uart (or the device_cmd tool directly for MQTT nodes)",
                    iface.kind
                ));
            }
            other => {
                return Err(format!(
                    "[InvalidInput] unknown interface kind '{other}' (uart | rtt | device_cmd)"
                ));
            }
        }
    }
    let Some(m) = &suite.mutation else {
        return Err(
            "[InvalidInput] suite needs a [mutation] block — without a corpus there is no \
             deterministic attack surface"
                .to_string(),
        );
    };
    if m.seed.is_none() {
        return Err(
            "[InvalidInput] mutation.seed is required — an unseeded corpus is not reproducible, \
             and a finding nobody can re-run is not a finding"
                .to_string(),
        );
    }
    if m.strategies.is_empty() {
        return Err(
            "[InvalidInput] mutation.strategies is empty — nothing would be sent".to_string(),
        );
    }
    let recovery = suite.recovery.as_deref().unwrap_or("reset");
    match recovery {
        "reflash" => {
            if !suite.allowed_actions.iter().any(|a| a == "flash") {
                return Err(
                    "[InvalidInput] recovery=reflash requires \"flash\" in allowed_actions — \
                     the suite must declare the revive path it depends on"
                        .to_string(),
                );
            }
            if suite.elf.is_none() {
                return Err(
                    "[InvalidInput] recovery=reflash needs elf (the image to re-flash)".to_string(),
                );
            }
        }
        "reset" => {
            if !suite.allowed_actions.iter().any(|a| a == "reset") {
                return Err(
                    "[InvalidInput] recovery=reset requires \"reset\" in allowed_actions"
                        .to_string(),
                );
            }
        }
        "none" => {}
        other => {
            return Err(format!(
                "[InvalidInput] recovery='{other}' — use reflash | reset | none"
            ));
        }
    }
    // An empty fault signature matches EVERY window (`"".contains` is a
    // subset of every string), which would turn the whole campaign into
    // "everything crashed" — the loudest possible false-positive generator,
    // and one a stray `signatures = [""]` in the TOML produces by accident.
    if let Some(empty) = suite
        .oracle
        .extra_fault_signatures
        .iter()
        .find(|s| s.trim().is_empty())
    {
        return Err(format!(
            "[InvalidInput] oracle.extra_fault_signatures contains an empty signature \
             ({empty:?}) — an empty string matches every window and would mark every case \
             as a crash"
        ));
    }
    // Fail at validation, not mid-attack: every pure thing the live loop
    // does (baseline parsing, oracle regex compiling) is checked here so a
    // dry-run that passes cannot blow up after payloads are already flying.
    for iface in &suite.interfaces {
        parse_baseline(iface.baseline.as_deref())?;
    }
    if let Some(hb) = suite.oracle.heartbeat_regex.as_deref() {
        regex::Regex::new(hb).map_err(|e| {
            format!("[InvalidInput] oracle.heartbeat_regex is not a valid regex: {e}")
        })?;
    }
    if let Some(bb) = suite.oracle.boot_banner.as_deref() {
        regex::Regex::new(bb)
            .map_err(|e| format!("[InvalidInput] oracle.boot_banner is not a valid regex: {e}"))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Injectable execution layer (serial links, recovery, forensic)
// ---------------------------------------------------------------------------

/// What one case window captured from the target.
pub(crate) struct Capture {
    pub text: String,
    /// The window expired without the early-exit condition (heartbeat or
    /// fault) being met.
    pub timed_out: bool,
}

/// One live interface to the target.
#[async_trait]
pub(crate) trait TargetLink: Send {
    async fn send_and_capture(
        &mut self,
        payload: &[u8],
        window_ms: u64,
        oracle: &OracleCfg,
        cancel: &Cancellable,
    ) -> Result<Capture, String>;
    /// Read-only window (no payload) — used to check liveness after recovery.
    /// Takes the turn's cancellation: a liveness probe must not outrun an
    /// interrupt.
    async fn listen(
        &mut self,
        window_ms: u64,
        oracle: &OracleCfg,
        cancel: &Cancellable,
    ) -> Result<Capture, String>;
}

/// The hardware-facing seams of a run, injectable for tests.
#[async_trait]
pub(crate) trait Executor: Send + Sync {
    async fn open_link(&self, iface: &Interface) -> Result<Box<dyn TargetLink>, String>;
    /// Revive the target (reflash / reset). The runner has ALREADY dropped
    /// its serial link when this is called, and re-opens one afterwards to
    /// probe liveness — recovery must never touch the port itself (a second
    /// open of an owned serial port fails on both Windows and Linux, and
    /// after a USB re-enumeration the old handle is dead anyway).
    /// `budget_ms` caps the recovery work so a stuck probe-rs cannot
    /// overrun the suite budget.
    async fn recover(
        &self,
        suite: &RedteamSuite,
        ctx: &ToolContext,
        budget_ms: u64,
    ) -> Result<(), String>;
    /// Fault post-mortem text, if the probe can produce one.
    async fn forensic(&self, suite: &RedteamSuite, ctx: &ToolContext) -> Option<String>;
}

/// Production executor: real serial ports, probe-rs recovery, Debug forensic.
pub(crate) struct LiveExecutor;

/// The port lives in an Option so the blocking IO closure can take it out
/// and hand it back — serialport reads/writes are blocking syscalls and run
/// on `spawn_blocking` (the monitor/hil precedent), never on the async
/// worker that drives the TUI/GUI event pump.
struct UartLink {
    port: Option<Box<dyn serialport::SerialPort>>,
}

/// How long to keep listening after a heartbeat was seen: a payload that
/// corrupts state can kill the target a few hundred ms AFTER its last tick,
/// and closing the window on the first heartbeat would attribute that crash
/// to the NEXT case.
const HEARTBEAT_TAIL_MS: u64 = 300;

fn read_window(
    port: &mut dyn serialport::SerialPort,
    window_ms: u64,
    oracle: &OracleCfg,
    cancel: &Cancellable,
) -> Capture {
    let heartbeat = oracle
        .heartbeat_regex
        .as_deref()
        .and_then(|h| regex::Regex::new(h).ok());
    let start = Instant::now();
    let deadline = start + Duration::from_millis(window_ms);
    let mut tail_deadline: Option<Instant> = None;
    let mut buf = [0u8; 4096];
    let mut acc: Vec<u8> = Vec::new();
    let mut timed_out = true;
    loop {
        if cancel.is_cancelled() {
            timed_out = false;
            break;
        }
        let stop = match tail_deadline {
            Some(t) => t.min(deadline),
            None => deadline,
        };
        if Instant::now() >= stop {
            break;
        }
        match port.read(&mut buf) {
            Ok(n) if n > 0 => {
                acc.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&acc);
                // A fault scene is ephemeral (watchdog!) — exit on it
                // immediately.
                let fault = crate::forensic::fault_detected_marker(&text).is_some()
                    || oracle
                        .extra_fault_signatures
                        .iter()
                        .any(|s| !s.is_empty() && text.contains(s.as_str()));
                if fault {
                    timed_out = false;
                    break;
                }
                // A heartbeat means the case is survived — but keep a short
                // tail window so a delayed crash still lands on THIS case.
                if tail_deadline.is_none()
                    && heartbeat.as_ref().is_some_and(|rx| rx.is_match(&text))
                {
                    tail_deadline = Some(Instant::now() + Duration::from_millis(HEARTBEAT_TAIL_MS));
                }
            }
            Ok(_) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => {
                acc.extend_from_slice(format!("\n[read error: {e}]\n").as_bytes());
                break;
            }
        }
    }
    Capture {
        text: String::from_utf8_lossy(&acc).to_string(),
        timed_out,
    }
}

/// Blocking half of `send_and_capture`: chunked write under a deadline,
/// then the read window. Runs on the blocking pool; the port travels in and
/// back out so the link stays reusable.
fn blocking_send_capture(
    mut port: Box<dyn serialport::SerialPort>,
    payload: &[u8],
    window_ms: u64,
    oracle: &OracleCfg,
    cancel: &Cancellable,
) -> (Box<dyn serialport::SerialPort>, Result<Capture, String>) {
    use std::io::Write;
    // A 64 KiB oversize case at 9600 baud is ~67 s of writing — it must
    // stay cancellable and bounded, so write in chunks and check the clock
    // between them.
    let write_deadline = Instant::now() + Duration::from_millis(window_ms.saturating_add(10_000));
    let mut sent = 0usize;
    while sent < payload.len() {
        if cancel.is_cancelled() {
            return (
                port,
                Err("[Cancelled] redteam write interrupted by turn cancel".to_string()),
            );
        }
        if Instant::now() >= write_deadline {
            return (
                port,
                Err(format!(
                    "[Timeout] uart write stalled after {sent}/{} bytes — the target is not \
                     draining the line (baud mismatch?)",
                    payload.len()
                )),
            );
        }
        let end = (sent + 1024).min(payload.len());
        if let Err(e) = port.write_all(&payload[sent..end]) {
            return (
                port,
                Err(format!("[Io] uart write failed after {sent} bytes: {e}")),
            );
        }
        let _ = port.flush();
        sent = end;
    }
    let cap = read_window(&mut *port, window_ms, oracle, cancel);
    (port, Ok(cap))
}

impl UartLink {
    /// Run a blocking closure with the port on the blocking pool, returning
    /// the port to the link afterwards.
    async fn with_port<T, F>(&mut self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(Box<dyn serialport::SerialPort>) -> (Box<dyn serialport::SerialPort>, T)
            + Send
            + 'static,
    {
        let port = self
            .port
            .take()
            .ok_or("[Io] redteam link already closed (used after drop)")?;
        let (port, out) = tokio::task::spawn_blocking(move || f(port))
            .await
            .map_err(|e| format!("[Io] serial task failed: {e}"))?;
        self.port = Some(port);
        Ok(out)
    }
}

#[async_trait]
impl TargetLink for UartLink {
    async fn send_and_capture(
        &mut self,
        payload: &[u8],
        window_ms: u64,
        oracle: &OracleCfg,
        cancel: &Cancellable,
    ) -> Result<Capture, String> {
        let payload = payload.to_vec();
        let oracle = oracle.clone();
        let cancel = cancel.clone();
        self.with_port(move |port| {
            blocking_send_capture(port, &payload, window_ms, &oracle, &cancel)
        })
        .await?
    }

    async fn listen(
        &mut self,
        window_ms: u64,
        oracle: &OracleCfg,
        cancel: &Cancellable,
    ) -> Result<Capture, String> {
        let oracle = oracle.clone();
        let cancel = cancel.clone();
        self.with_port(move |mut port| {
            let cap = read_window(&mut *port, window_ms, &oracle, &cancel);
            (port, Ok(cap))
        })
        .await?
    }
}

#[async_trait]
impl Executor for LiveExecutor {
    async fn open_link(&self, iface: &Interface) -> Result<Box<dyn TargetLink>, String> {
        let port_name = iface
            .port
            .as_deref()
            .ok_or("[InvalidInput] uart needs port")?;
        let baud = iface.baud.unwrap_or(115_200);
        let port = serialport::new(port_name, baud)
            .timeout(Duration::from_millis(200))
            .open()
            .map_err(|e| {
                format!(
                    "[Io] cannot open {port_name} at {baud} baud: {e} — is it plugged in, and \
                     is the baud right? (another program holding the port, or a stale COM number \
                     after a USB re-enumeration, also lands here)"
                )
            })?;
        Ok(Box::new(UartLink { port: Some(port) }))
    }

    async fn recover(
        &self,
        suite: &RedteamSuite,
        ctx: &ToolContext,
        budget_ms: u64,
    ) -> Result<(), String> {
        // The runner dropped its serial link before calling this; recovery
        // is probe-side only (reflash / reset). Liveness is probed by the
        // runner on a FRESH link — after a reflash the USB serial may have
        // re-enumerated and the old handle is worthless.
        match suite.recovery.as_deref().unwrap_or("reset") {
            "reflash" => {
                let hil_step = crate::tools::hil::flash_recovery_step(
                    suite.elf.as_deref().ok_or("recovery: no elf")?,
                    suite.chip.as_deref().or(ctx.default_chip.as_deref()),
                    suite.probe.as_deref(),
                );
                let timeout = 180_000.min(budget_ms.max(1_000));
                super::hil::run_flash_step(&hil_step, ctx, false, timeout).await?;
            }
            "reset" => {
                let chip = suite
                    .chip
                    .clone()
                    .or(ctx.default_chip.clone())
                    .ok_or("recovery=reset needs chip")?;
                let chip = crate::tools::util::token_arg(&chip, "chip")?;
                let mut args = vec!["reset".to_string(), "--chip".to_string(), chip];
                if let Some(p) = &suite.probe {
                    args.push("--probe".to_string());
                    args.push(crate::tools::util::token_arg(p, "probe")?);
                }
                let timeout = 15_000.min(budget_ms.max(1_000));
                crate::tools::util::run_probe_rs(
                    args,
                    &ctx.cwd,
                    timeout,
                    Some(ctx.cancel.clone()),
                    &[],
                )
                .await
                .map_err(|e| crate::tools::util::probe_rs_err(e).message)?;
            }
            other => {
                return Err(format!(
                    "[InvalidInput] recovery='{other}' cannot revive the target"
                ));
            }
        }
        Ok(())
    }

    async fn forensic(&self, suite: &RedteamSuite, ctx: &ToolContext) -> Option<String> {
        let elf = suite.elf.as_ref()?;
        let mut args = json!({"action": "forensic", "elf": elf});
        if let Some(chip) = suite.chip.as_ref().or(ctx.default_chip.as_ref()) {
            args["chip"] = json!(chip);
        }
        if let Some(probe) = &suite.probe {
            args["probe"] = json!(probe);
        }
        // Auto-approve child context: the suite approval covered forensics as
        // part of the attack loop (and the scene is ephemeral — a popup
        // would lose it to the watchdog, same reasoning as debug forensic).
        let child_ctx = ToolContext {
            permission: std::sync::Arc::new(firment_core::AutoApprove::everything()),
            ..ctx.clone()
        };
        use firment_core::Tool as _;
        match super::debug::Debug.run(args, &child_ctx).await {
            Ok(out) => Some(out.text),
            Err(e) => Some(format!("(forensic unavailable: {})", e.message)),
        }
    }
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

pub(crate) struct RunOutcome {
    pub findings: Vec<Finding>,
    pub cases_sent: usize,
    pub aborted: Option<String>,
    pub run_id: String,
    pub dir: PathBuf,
}

fn redteam_dir_for(
    ctx: &ToolContext,
    suite: &RedteamSuite,
    run_id: &str,
) -> Result<PathBuf, String> {
    let base = match suite.report.dir.as_deref() {
        // No silent fallback: a report.dir that escapes the workspace roots
        // must be an error, not a path quietly joined onto cwd (an absolute
        // `d` would REPLACE the base and write evidence outside any root).
        Some(d) => resolve_within(&ctx.cwd, d, &ctx.allowed_roots).map_err(|e| {
            format!(
                "[Permission] report.dir {:?} is outside the workspace: {e}",
                d
            )
        })?,
        None => ctx.cwd.join(".firment").join("redteam"),
    };
    Ok(base.join(run_id))
}

/// Did the target answer the liveness probe? The regexes were compiled at
/// validation time, so a compile failure here can only mean the caller
/// bypassed validation — treat it as alive rather than silently dead.
fn heartbeat_seen(text: &str, oracle: &OracleCfg) -> bool {
    match oracle.heartbeat_regex.as_deref() {
        Some(hb) => regex::Regex::new(hb)
            .map(|rx| rx.is_match(text))
            .unwrap_or(true),
        None => !text.trim().is_empty(),
    }
}

fn replay_dir(ctx: &ToolContext) -> PathBuf {
    ctx.session_dir
        .clone()
        .unwrap_or_else(|| ctx.cwd.join(".firment").join("work"))
        .join("redteam")
}

fn write_replay_line(path: &Path, line: &Value) {
    if let Ok(s) = serde_json::to_string(line) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{s}");
        }
    }
}

/// Core loop, injectable executor. One approval covered the whole suite, so
/// links/recovery run without further prompts.
///
/// Error discipline: once hardware has been attacked, NOTHING may lose the
/// evidence. Mid-run failures (port busy, write stall, bad classify input)
/// set `aborted` and fall through to the report writers instead of `?` —
/// findings already captured are the whole point of the run.
pub(crate) async fn run_suite_with(
    suite_label: &str,
    suite: &RedteamSuite,
    ctx: &ToolContext,
    exec: &dyn Executor,
) -> Result<RunOutcome, String> {
    validate_suite(suite)?;
    let m = suite.mutation.as_ref().expect("validated");
    let seed = m.seed.expect("validated");
    let run_id = uuid::Uuid::new_v4().to_string();
    let dir = redteam_dir_for(ctx, suite, &run_id)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("[Io] create {}: {e}", dir.display()))?;
    let replay_path = replay_dir(ctx).join(format!("{run_id}.jsonl"));
    if let Some(parent) = replay_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let overall = Instant::now();
    let mut findings = Vec::new();
    let mut cases_sent = 0usize;
    let mut aborted = None;

    'interfaces: for iface in &suite.interfaces {
        // Baselines and oracle regexes were validated up front, so these
        // pure re-parses cannot fail here.
        let baseline = parse_baseline(iface.baseline.as_deref())?;
        let cases = mutate::corpus(seed, &baseline, &m.strategies);
        let iface_label = format!("{}@{}", iface.kind, iface.port.as_deref().unwrap_or("-"));
        let mut link = match exec.open_link(iface).await {
            Ok(link) => link,
            Err(e) => {
                aborted = Some(format!(
                    "interface {iface_label}: {e} — run stopped early, findings so far are valid"
                ));
                break 'interfaces;
            }
        };
        let mut saw_traffic = false;
        for case in &cases {
            if cases_sent >= suite.budget.max_cases {
                aborted = Some(format!(
                    "budget: max_cases {} reached",
                    suite.budget.max_cases
                ));
                break 'interfaces;
            }
            if overall.elapsed() >= Duration::from_millis(suite.budget.max_duration_ms) {
                aborted = Some(format!(
                    "budget: max_duration_ms {} reached",
                    suite.budget.max_duration_ms
                ));
                break 'interfaces;
            }
            if ctx.cancel.is_cancelled() {
                aborted = Some("cancelled by turn interrupt".to_string());
                break 'interfaces;
            }
            let cap = match link
                .send_and_capture(
                    &case.payload,
                    suite.budget.per_case_timeout_ms,
                    &suite.oracle,
                    &ctx.cancel,
                )
                .await
            {
                Ok(cap) => cap,
                Err(e) => {
                    aborted = Some(format!(
                        "case {} on {iface_label}: {e} — run stopped early, findings so far are \
                         valid",
                        case.id
                    ));
                    break 'interfaces;
                }
            };
            cases_sent += 1;
            let verdict =
                match oracle::classify(&cap.text, saw_traffic, cap.timed_out, &suite.oracle) {
                    Ok(v) => v,
                    Err(e) => {
                        aborted = Some(format!("case {}: {e}", case.id));
                        break 'interfaces;
                    }
                };
            if !cap.text.trim().is_empty() {
                saw_traffic = true;
            }
            let class = match &verdict {
                Verdict::Crash(_) => Some("crash"),
                Verdict::Reboot => Some("reboot"),
                Verdict::Hang => Some("hang"),
                Verdict::Alive => None,
            };
            let mut line = json!({
                "case": case.id,
                "interface": iface_label,
                "verdict": class.unwrap_or("alive"),
            });
            let Some(class) = class else {
                write_replay_line(&replay_path, &line);
                continue;
            };
            let fid = format!("F-{:03}", findings.len() + 1);
            let mut ev: Vec<String> = Vec::new();
            if !cap.text.trim().is_empty() {
                let log = dir.join(format!("capture-{fid}.log"));
                let _ = std::fs::write(&log, &cap.text);
                ev.push(format!("capture-{fid}.log"));
            }
            if class == "crash"
                && let Some(report) = exec.forensic(suite, ctx).await
            {
                let fpath = dir.join(format!("forensic-{fid}.txt"));
                let _ = std::fs::write(&fpath, &report);
                ev.push(format!("forensic-{fid}.txt"));
            }
            let mut f = Finding {
                finding_id: fid.clone(),
                severity: findings::default_severity(class),
                class: class.to_string(),
                strategy: mutate::strategy_name(case.strategy).to_string(),
                case_id: case.id.clone(),
                seed,
                payload_hex: findings::hex_encode(&case.payload),
                interface: iface_label.clone(),
                observed: match &verdict {
                    Verdict::Crash(r) => r.clone(),
                    Verdict::Reboot => "boot banner reappeared mid-stream".to_string(),
                    Verdict::Hang => "no heartbeat within the window".to_string(),
                    Verdict::Alive => unreachable!(),
                },
                evidence: ev,
                reproducer: findings::Reproducer {
                    suite: suite_label.to_string(),
                    case: case.id.clone(),
                },
                confidence: String::new(),
            };
            let dir_for_check = dir.clone();
            f.finalize(|e| dir_for_check.join(e).is_file());
            line["finding"] = json!(&f.finding_id);
            findings.push(f);
            // Record the finding BEFORE attempting recovery: a failed revive
            // must not erase the replay line of what was already proven.
            write_replay_line(&replay_path, &line);
            // Recovery: drop the link first (the port must be free — and
            // after a reflash the USB serial may have re-enumerated, so the
            // old handle is worthless), revive, reopen, probe liveness.
            let recovery = suite.recovery.as_deref().unwrap_or("reset");
            drop(link);
            if recovery == "none" {
                aborted = Some(format!(
                    "target {} on case {} and recovery='none' — run aborted, findings so far \
                     are valid",
                    class, case.id
                ));
                break 'interfaces;
            }
            let remaining = suite
                .budget
                .max_duration_ms
                .saturating_sub(overall.elapsed().as_millis() as u64);
            let alive = match exec.recover(suite, ctx, remaining).await {
                Err(e) => {
                    aborted = Some(format!(
                        "recovery failed after {} on case {}: {e} — run aborted, findings so far \
                         are valid",
                        class, case.id
                    ));
                    break 'interfaces;
                }
                Ok(()) => match exec.open_link(iface).await {
                    Err(e) => {
                        aborted = Some(format!(
                            "cannot reopen {iface_label} after recovery: {e} — run aborted, \
                             findings so far are valid"
                        ));
                        break 'interfaces;
                    }
                    Ok(fresh) => {
                        link = fresh;
                        match link.listen(3_000, &suite.oracle, &ctx.cancel).await {
                            Ok(probe) => heartbeat_seen(&probe.text, &suite.oracle),
                            Err(e) => {
                                aborted = Some(format!(
                                    "liveness probe on {iface_label} failed: {e} — run aborted, \
                                     findings so far are valid"
                                ));
                                break 'interfaces;
                            }
                        }
                    }
                },
            };
            if !alive {
                aborted = Some(format!(
                    "target dead after {} on case {} (recovery='{}' did not revive it) — run \
                     aborted, findings so far are valid",
                    class, case.id, recovery
                ));
                break 'interfaces;
            }
            saw_traffic = true;
        }
    }

    // Reports.
    let jsonl: String = findings.iter().map(|f| f.to_json_line()).collect();
    std::fs::write(dir.join("findings.jsonl"), jsonl)
        .map_err(|e| format!("[Io] write findings: {e}"))?;
    let min = suite.report.min_severity.unwrap_or(Severity::Low);
    let shown: Vec<Finding> = findings
        .iter()
        .filter(|f| f.severity >= min)
        .cloned()
        .collect();
    std::fs::write(
        dir.join("report.md"),
        findings::render_report_md(suite_label, &run_id, &shown),
    )
    .map_err(|e| format!("[Io] write report: {e}"))?;

    Ok(RunOutcome {
        findings,
        cases_sent,
        aborted,
        run_id,
        dir,
    })
}

/// Dry-run: generate the corpus, report the plan, touch nothing. Like HIL's
/// dry-run, it can never produce evidence — the output says so.
fn dry_run_plan(suite_label: &str, suite: &RedteamSuite) -> Result<String, String> {
    validate_suite(suite)?;
    let m = suite.mutation.as_ref().expect("validated");
    let seed = m.seed.expect("validated");
    let mut out = format!(
        "[dry-run] redteam suite '{suite_label}' — no hardware touched, no findings can be \
         claimed\n  budget: max_cases={} max_duration_ms={} per_case_timeout_ms={}\n  recovery: \
         {}\n  oracle: heartbeat={} banner={}\n",
        suite.budget.max_cases,
        suite.budget.max_duration_ms,
        suite.budget.per_case_timeout_ms,
        suite.recovery.as_deref().unwrap_or("reset"),
        suite.oracle.heartbeat_regex.as_deref().unwrap_or("-"),
        suite.oracle.boot_banner.as_deref().unwrap_or("-"),
    );
    for iface in &suite.interfaces {
        let baseline = parse_baseline(iface.baseline.as_deref())?;
        let cases = mutate::corpus(seed, &baseline, &m.strategies);
        let shown_id = iface
            .port
            .clone()
            .or_else(|| iface.node.clone())
            .unwrap_or_else(|| "-".to_string());
        out.push_str(&format!(
            "  interface {}@{}: baseline {} bytes, {} cases (first: {})\n",
            iface.kind,
            shown_id,
            baseline.len(),
            cases.len(),
            cases
                .first()
                .map(|c| format!("{} {} bytes", c.id, c.payload.len()))
                .unwrap_or_default(),
        ));
    }
    out.push_str("[HIL_EXPECT:FAIL] dry-run cannot verify hardware behavior");
    Ok(out)
}

// ---------------------------------------------------------------------------
// LLM attacker campaign (exploration layer on top of the corpus)
// ---------------------------------------------------------------------------

/// Prompt for the attacker subagent. The corpus summary is handed in so the
/// model explores what the deterministic engine did NOT cover.
pub(crate) fn campaign_prompt(label: &str, suite: &RedteamSuite, corpus_note: &str) -> String {
    let ifaces: Vec<String> = suite
        .interfaces
        .iter()
        .map(|i| {
            format!(
                "  - {}@{} (baud {})",
                i.kind,
                i.port
                    .clone()
                    .or_else(|| i.node.clone())
                    .unwrap_or_default(),
                i.baud.unwrap_or(115200)
            )
        })
        .collect();
    format!(
        "You are the red team campaign for suite '{label}'. The deterministic corpus already \
         ran: {corpus_note}\n\n\
         Target interfaces (you may ONLY touch these — anything else is denied):\n{}\n\
         Liveness oracle: heartbeat={} | boot banner={} | extra fault signatures={:?}\n\n\
         Explore what the corpus missed: probe the protocol, craft structured-but-wrong frames, \
         race two inputs, feed what the target's own logs hint at. Prefer cheap observations \
         first (monitor a quiet window, read the ELF for parsers).\n\n\
         Hard rules:\n\
         - A finding is only real if captured output shows it — quote the exact line.\n\
         - On a suspected crash, run debug action=forensic IMMEDIATELY (the scene is ephemeral).\n\
         - Do NOT reflash or reset the target; the suite handles recovery.\n\
         - ATTENTION: every debug action (forensic included) leaves the target HALTED. Silence \
           you capture after touching the debugger is YOUR artifact — never report it as a \
           hang, and say in `why` that you halted the target.\n\
         - End your reply with a fenced block (empty array is an honest answer):\n\
         ```redteam-findings\n[{{\"severity\":\"high|medium|low\",\"class\":\"crash|reboot|hang\",\
         \"payload_hex\":\"...\",\"interface\":\"uart@COM3\",\"observed\":\"exact line\",\
         \"evidence\":[\"file.log\"],\"why\":\"one sentence\"}}]\n```\n\
         A finding without payload_hex cannot be reproduced and will be marked UNVERIFIED.",
        ifaces.join("\n"),
        suite.oracle.heartbeat_regex.as_deref().unwrap_or("-"),
        suite.oracle.boot_banner.as_deref().unwrap_or("-"),
        suite.oracle.extra_fault_signatures,
    )
}

/// Parse the campaign's fenced findings block. Malformed JSON or a missing
/// block yields nothing — the campaign's TEXT report is still shown, but
/// only structured, payload-bearing entries become findings.
pub(crate) fn extract_findings_block(
    text: &str,
    suite_label: &str,
    seed: u64,
    next_id: usize,
) -> Vec<Finding> {
    let Some(open) = text.find("```redteam-findings") else {
        return Vec::new();
    };
    let rest = &text[open + "```redteam-findings".len()..];
    let Some(close) = rest.find("```") else {
        return Vec::new();
    };
    let Ok(items) = serde_json::from_str::<Vec<Value>>(&rest[..close]) else {
        return Vec::new();
    };
    items
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let sev = match v.get("severity").and_then(|s| s.as_str()) {
                Some("critical") => Severity::Critical,
                Some("high") => Severity::High,
                Some("medium") => Severity::Medium,
                _ => Severity::Low,
            };
            Finding {
                finding_id: format!("F-{:03}", next_id + i),
                severity: sev,
                class: v
                    .get("class")
                    .and_then(|s| s.as_str())
                    .unwrap_or("crash")
                    .to_string(),
                strategy: "campaign".to_string(),
                case_id: format!("campaign-{}", next_id + i),
                seed,
                payload_hex: v
                    .get("payload_hex")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                interface: v
                    .get("interface")
                    .and_then(|s| s.as_str())
                    .unwrap_or("?")
                    .to_string(),
                observed: v
                    .get("observed")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                evidence: v
                    .get("evidence")
                    .and_then(|e| e.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                reproducer: findings::Reproducer {
                    suite: suite_label.to_string(),
                    case: format!("campaign-{}", next_id + i),
                },
                confidence: String::new(),
            }
        })
        .collect()
}

/// Run the campaign: clone the attacker runner with a target-locked
/// permission, hand it the prompt, parse its findings block.
pub(crate) async fn run_campaign(
    label: &str,
    suite: &RedteamSuite,
    ctx: &ToolContext,
    corpus_note: &str,
    next_id: usize,
) -> Result<Vec<Finding>, String> {
    use firment_core::SubagentFactory as _;
    let base = ctx
        .attacker
        .as_ref()
        .ok_or("[InvalidInput] campaign needs an attacker runner (interactive agent session)")?;
    let locked: Vec<String> = suite
        .interfaces
        .iter()
        .filter_map(|i| i.port.clone().or_else(|| i.node.clone()))
        .collect();
    let attacker = firment_core::SubagentRunner {
        permission: Arc::new(firment_core::TargetLockPermission::new(
            base.permission.clone(),
            locked,
        )),
        ..(**base).clone()
    };
    let seed = suite.mutation.as_ref().and_then(|m| m.seed).unwrap_or(0);
    let prompt = campaign_prompt(label, suite, corpus_note);
    let text = attacker
        .run_subagent(
            &prompt,
            ctx.cwd.clone(),
            None,
            None,
            ctx.subagent_depth + 1,
            ctx.cancel.clone(),
        )
        .await?;
    Ok(extract_findings_block(&text, label, seed, next_id))
}

// ---------------------------------------------------------------------------
// Suite file discovery
// ---------------------------------------------------------------------------

fn find_redteam_file(cwd: &Path) -> Option<PathBuf> {
    for dir in cwd.ancestors() {
        let p = dir.join(".firment").join("redteam.toml");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn load_suites(cwd: &Path) -> Result<(PathBuf, RedteamFile), String> {
    let path = find_redteam_file(cwd)
        .ok_or("[NotFound] no .firment/redteam.toml (searched cwd and ancestors)")?;
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("[Io] read {}: {e}", path.display()))?;
    let file: RedteamFile =
        toml::from_str(&text).map_err(|e| format!("[InvalidInput] redteam.toml: {e}"))?;
    Ok((path, file))
}

// ---------------------------------------------------------------------------
// Tool
// ---------------------------------------------------------------------------

pub struct Redteam;

#[async_trait]
impl Tool for Redteam {
    fn name(&self) -> &'static str {
        "redteam"
    }

    fn description(&self) -> &'static str {
        "Runtime red team: attack the firmware on the target with a deterministic mutated-input corpus and report only what the captured output proves. Suites live in .firment/redteam.toml: interfaces (uart port/baud + baseline frame), mutation (seed REQUIRED + strategies), oracle (heartbeat/boot-banner/fault signatures), budget (max_cases/max_duration_ms/per_case_timeout_ms), recovery (reflash|reset|none). One approval covers the whole suite; every finding cites capture files (missing evidence caps severity to low/UNVERIFIED) and is reproducible from seed + case id alone — no LLM needed. dry_run rehearses the corpus without touching hardware (and can never claim evidence). Supports replay (JSONL) and list_suites."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "suite": {"type": "string", "description": "Suite name defined in .firment/redteam.toml"},
                "dry_run": {"type": "boolean", "default": false, "description": "Generate and report the corpus without touching hardware; cannot produce findings"},
                "live": {"type": "boolean", "default": false, "description": "Explicitly allow a live run in headless mode (interactive sessions approve via popup instead)"},
                "replay": {"type": "string", "description": "Replay a previous run by id, or 'list' to list replays"},
                "list_suites": {"type": "boolean", "default": false, "description": "List suites defined in .firment/redteam.toml"}
            }
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        if args.get("replay").is_some()
            || args
                .get("list_suites")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            || args
                .get("dry_run")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        {
            return None;
        }
        let suite = args.get("suite").and_then(|v| v.as_str()).unwrap_or("?");
        Some(format!(
            "⚠ red team suite '{suite}': sends mutated payloads to the target's interfaces and \
             may reset/reflash it between cases (the suite's allowed_actions and budget define \
             the scope — read .firment/redteam.toml before approving)"
        ))
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        if args
            .get("list_suites")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Ok(ToolOutput {
                text: list_suites(ctx),
            });
        }
        if let Some(replay) = args.get("replay").and_then(|v| v.as_str()) {
            return handle_replay(replay, ctx);
        }
        let label = args.get("suite").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::new("[InvalidInput] redteam needs 'suite' (see list_suites)")
        })?;
        let (_path, file) = load_suites(&ctx.cwd).map_err(ToolError::new)?;
        let suite = file
            .suite
            .get(label)
            .ok_or_else(|| {
                ToolError::new(format!(
                    "[NotFound] no suite '{label}' in .firment/redteam.toml"
                ))
            })?
            .clone();
        let dry = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if dry {
            let text = dry_run_plan(label, &suite).map_err(ToolError::new)?;
            return Ok(ToolOutput {
                text: truncate(&text, 32_000),
            });
        }
        // Headless live runs need an explicit --live: an unattended attack
        // loop with nobody to review the approval is how boards get bricked
        // in CI. Interactive sessions gate on the approval popup instead.
        // This is an Err, not an Ok: a CI script that ran `firm redteam`
        // expecting an attack must get a non-zero exit when none happened —
        // a green "success" here would be a false pass.
        let live = args.get("live").and_then(|v| v.as_bool()).unwrap_or(false);
        if !live && ctx.asker.is_none() {
            let text = dry_run_plan(label, &suite).map_err(ToolError::new)?;
            return Err(ToolError::new(format!(
                "[redteam] headless live run refused — pass live=true (CLI: --live) to attack \
                 hardware without an interactive approver. Corpus rehearsal instead:\n{text}"
            )));
        }
        let mut outcome = run_suite_with(label, &suite, ctx, &LiveExecutor)
            .await
            .map_err(ToolError::new)?;
        // Campaign layer: only when the suite opts in AND an interactive
        // approver exists to watch the attack stream. Campaign findings are
        // finalized against the run dir like corpus ones.
        let mut campaign_note = String::new();
        if suite.llm_phase == Some(true) {
            if ctx.asker.is_some() && ctx.attacker.is_some() {
                let note = format!(
                    "{} cases sent, {} findings so far",
                    outcome.cases_sent,
                    outcome.findings.len()
                );
                match run_campaign(label, &suite, ctx, &note, outcome.findings.len() + 1).await {
                    Ok(mut extra) => {
                        let dir = outcome.dir.clone();
                        // Campaign evidence is model-authored: the strict
                        // gate (payload required, references must exist and
                        // contain the quoted line, no self-references) is
                        // what keeps a hallucination out of the report.
                        for f in extra.iter_mut() {
                            f.finalize_strict(&dir);
                        }
                        campaign_note =
                            format!("campaign added {} candidate finding(s)", extra.len());
                        outcome.findings.extend(extra);
                        // Rewrite reports with the campaign findings included.
                        let jsonl: String =
                            outcome.findings.iter().map(|f| f.to_json_line()).collect();
                        let _ = std::fs::write(dir.join("findings.jsonl"), jsonl);
                        let min = suite.report.min_severity.unwrap_or(Severity::Low);
                        let shown: Vec<Finding> = outcome
                            .findings
                            .iter()
                            .filter(|f| f.severity >= min)
                            .cloned()
                            .collect();
                        let _ = std::fs::write(
                            dir.join("report.md"),
                            findings::render_report_md(label, &outcome.run_id, &shown),
                        );
                    }
                    Err(e) => campaign_note = format!("campaign failed: {e}"),
                }
            } else {
                campaign_note = "campaign skipped: needs an interactive session".to_string();
            }
        }
        let mut out = format!(
            "[redteam] suite '{label}' — {} cases sent, {} findings ({} verified)\n  run: {}\n  \
             report: {}\n",
            outcome.cases_sent,
            outcome.findings.len(),
            outcome
                .findings
                .iter()
                .filter(|f| f.confidence == "HIGH")
                .count(),
            outcome.run_id,
            outcome.dir.join("report.md").display(),
        );
        for f in &outcome.findings {
            out.push_str(&format!(
                "  {} {} [{}] {} {} — {}\n",
                f.finding_id,
                f.severity.name(),
                f.confidence,
                f.class,
                f.case_id,
                f.observed,
            ));
        }
        if let Some(reason) = &outcome.aborted {
            out.push_str(&format!("  aborted: {reason}\n"));
        }
        if !campaign_note.is_empty() {
            out.push_str(&format!("  campaign: {campaign_note}\n"));
        }
        out.push_str(&format!(
            "\nevidence: reached level 5 (physical) — findings cite captured output\nreplay: \
             redteam replay {}  |  list: redteam replay list",
            outcome.run_id
        ));
        Ok(ToolOutput {
            text: truncate(&out, 64_000),
        })
    }
}

fn list_suites(ctx: &ToolContext) -> String {
    match load_suites(&ctx.cwd) {
        Ok((path, file)) => {
            if file.suite.is_empty() {
                return format!("redteam: no suites in {}", path.display());
            }
            let mut names: Vec<&String> = file.suite.keys().collect();
            names.sort();
            let mut out = format!("redteam suites in {}:\n", path.display());
            for name in names {
                let s = &file.suite[name];
                out.push_str(&format!(
                    "  - {name}: {} interface(s), recovery={}, budget {} cases\n",
                    s.interfaces.len(),
                    s.recovery.as_deref().unwrap_or("reset"),
                    s.budget.max_cases,
                ));
            }
            out.push_str("\nrun: redteam suite=<name>  |  dry: redteam suite=<name> dry_run=true");
            out
        }
        Err(e) => e,
    }
}

fn handle_replay(arg: &str, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
    let base = replay_dir(ctx);
    if arg == "list" {
        let Ok(read) = std::fs::read_dir(&base) else {
            return Ok(ToolOutput {
                text: format!("redteam replays in {}:\n(none yet)", base.display()),
            });
        };
        let mut entries: Vec<_> = read.flatten().collect();
        entries.sort_by_key(|e| e.path());
        let mut out = format!("redteam replays in {}:\n", base.display());
        for e in entries.iter().rev().take(20) {
            out.push_str(&format!("  {}\n", e.file_name().to_string_lossy()));
        }
        return Ok(ToolOutput { text: out });
    }
    // Same charset rule as hil: a replay id must not be an absolute path.
    if !arg
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ToolError::new(format!(
            "[InvalidInput] replay id must be alphanumeric/-/_ , got: {arg}"
        )));
    }
    let path = base.join(format!("{arg}.jsonl"));
    let text = std::fs::read_to_string(&path).map_err(|_| {
        ToolError::new(format!(
            "[NotFound] no redteam replay {arg} in {}",
            base.display()
        ))
    })?;
    Ok(ToolOutput {
        text: truncate(&text, 64_000),
    })
}

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
            journal: Arc::new(Mutex::new(EditJournal::new(dir.join("undo")))),
            monitor_port: Some("COM_FAKE".to_string()),
            ..ToolContext::default()
        }
    }

    fn suite() -> RedteamSuite {
        RedteamSuite {
            recovery: Some("none".to_string()),
            allowed_actions: vec!["send".to_string()],
            interfaces: vec![Interface {
                kind: "uart".to_string(),
                port: Some("COM_FAKE".to_string()),
                baud: Some(115200),
                node: None,
                baseline: Some("hex:55AA0102".to_string()),
            }],
            oracle: OracleCfg {
                heartbeat_regex: Some(r"tick=\d+".to_string()),
                boot_banner: Some("boot v2".to_string()),
                extra_fault_signatures: vec![],
            },
            mutation: Some(MutationCfg {
                seed: Some(1234),
                strategies: vec![Mutation::Bitflip, Mutation::Oversize],
            }),
            budget: Budget {
                max_cases: 500,
                max_duration_ms: 60_000,
                per_case_timeout_ms: 50,
            },
            ..Default::default()
        }
    }

    #[test]
    fn bad_oracle_regex_or_baseline_fails_validation_before_hardware() {
        // Everything the live loop does purely must be checked at validation
        // time: a dry-run that passes cannot blow up after payloads fly.
        let mut s = suite();
        s.oracle.heartbeat_regex = Some("(unclosed".to_string());
        let err = validate_suite(&s).unwrap_err();
        assert!(err.contains("heartbeat_regex"), "got: {err}");

        let mut s = suite();
        s.oracle.boot_banner = Some("*bad".to_string());
        assert!(validate_suite(&s).unwrap_err().contains("boot_banner"));

        let mut s = suite();
        s.interfaces[0].baseline = Some("55AA".to_string());
        assert!(validate_suite(&s).unwrap_err().contains("prefix"));
    }

    #[test]
    fn empty_fault_signature_is_rejected() {
        // `"".contains` is true for every window: leaving an empty entry in
        // extra_fault_signatures would mark every single case as a crash.
        let mut s = suite();
        s.oracle.extra_fault_signatures = vec!["".to_string()];
        let err = validate_suite(&s).unwrap_err();
        assert!(
            err.contains("empty signature"),
            "an empty signature must be refused: {err}"
        );
        // Whitespace-only is the same vacuous match.
        s.oracle.extra_fault_signatures = vec!["stack underflow".to_string(), "   ".to_string()];
        assert!(validate_suite(&s).is_err());
        // A real signature list passes.
        s.oracle.extra_fault_signatures = vec!["stack underflow".to_string()];
        assert!(validate_suite(&s).is_ok());
    }

    /// Scripted link: the executor's GLOBAL send counter crashes on the
    /// configured send index — so a link reopened after recovery continues
    /// the same script instead of crashing again at its own send zero.
    struct ScriptLink {
        crash_at: usize,
        total: Arc<std::sync::atomic::AtomicUsize>,
        listen_text: &'static str,
    }

    #[async_trait]
    impl TargetLink for ScriptLink {
        async fn send_and_capture(
            &mut self,
            _payload: &[u8],
            _window_ms: u64,
            _oracle: &OracleCfg,
            _cancel: &Cancellable,
        ) -> Result<Capture, String> {
            let i = self.total.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if i == self.crash_at {
                return Ok(Capture {
                    text: "tick=41\nHardFault_Handler\n".to_string(),
                    timed_out: false,
                });
            }
            Ok(Capture {
                text: format!("tick={}\n", i + 1),
                timed_out: false,
            })
        }
        async fn listen(
            &mut self,
            _w: u64,
            _o: &OracleCfg,
            _cancel: &Cancellable,
        ) -> Result<Capture, String> {
            Ok(Capture {
                text: self.listen_text.to_string(),
                timed_out: false,
            })
        }
    }

    struct MockExecutor {
        crash_at: usize,
        /// false → recover() returns Err (the runner must abort with
        /// "recovery failed" and STILL write the reports).
        revive: bool,
        /// What the liveness probe on the reopened link sees.
        listen_text: &'static str,
        /// open_link fails from this call number on (0 = never).
        fail_open_from: usize,
        opens: Arc<std::sync::atomic::AtomicUsize>,
        total: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl MockExecutor {
        fn alive(crash_at: usize) -> Self {
            Self {
                crash_at,
                revive: true,
                listen_text: "tick=99\n",
                fail_open_from: 0,
                opens: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                total: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait]
    impl Executor for MockExecutor {
        async fn open_link(&self, _iface: &Interface) -> Result<Box<dyn TargetLink>, String> {
            let n = self.opens.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if self.fail_open_from != 0 && n >= self.fail_open_from {
                return Err("mock: port busy".to_string());
            }
            Ok(Box::new(ScriptLink {
                crash_at: self.crash_at,
                total: self.total.clone(),
                listen_text: self.listen_text,
            }))
        }
        async fn recover(
            &self,
            _suite: &RedteamSuite,
            _ctx: &ToolContext,
            _budget_ms: u64,
        ) -> Result<(), String> {
            if self.revive {
                Ok(())
            } else {
                Err("mock: revive failed".to_string())
            }
        }
        async fn forensic(&self, _suite: &RedteamSuite, _ctx: &ToolContext) -> Option<String> {
            Some("forensic scene: PC=0x08001234 HardFault".to_string())
        }
    }

    #[tokio::test]
    async fn crash_produces_evidence_backed_finding_and_aborts_when_dead() {
        let dir = tempdir().unwrap();
        let s = suite(); // recovery none → runner aborts without reviving
        let outcome = run_suite_with("uart-fuzz", &s, &ctx(dir.path()), &MockExecutor::alive(2))
            .await
            .unwrap();
        assert_eq!(outcome.findings.len(), 1, "one crash before abort");
        let f = &outcome.findings[0];
        assert_eq!(f.class, "crash");
        assert_eq!(f.confidence, "HIGH", "capture + forensic files exist");
        assert_eq!(f.severity, Severity::High);
        assert!(f.evidence.iter().any(|e| e.starts_with("capture-")));
        assert!(f.evidence.iter().any(|e| e.starts_with("forensic-")));
        assert!(
            outcome
                .aborted
                .as_ref()
                .unwrap()
                .contains("recovery='none'"),
            "got: {:?}",
            outcome.aborted
        );
        // Reports on disk.
        assert!(outcome.dir.join("findings.jsonl").is_file());
        let md = std::fs::read_to_string(outcome.dir.join("report.md")).unwrap();
        assert!(md.contains("F-001"), "got: {md}");
    }

    #[tokio::test]
    async fn recovery_drops_and_reopens_the_link_then_continues() {
        // The port-exclusivity regression: recovery must NOT probe liveness
        // on the link it still holds. The runner drops the link, recovers,
        // and opens a FRESH one — proven by the open counter reaching 2.
        let dir = tempdir().unwrap();
        let mut s = suite();
        s.recovery = Some("reset".to_string());
        s.allowed_actions = vec!["send".to_string(), "reset".to_string()];
        s.budget.max_cases = 5;
        let exec = MockExecutor::alive(0); // crash on the very first send
        let outcome = run_suite_with("uart-fuzz", &s, &ctx(dir.path()), &exec)
            .await
            .unwrap();
        assert_eq!(outcome.findings.len(), 1, "the crash is found once");
        assert_eq!(
            exec.opens.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "initial open + reopen after recovery"
        );
        assert!(
            outcome.aborted.as_ref().unwrap().contains("max_cases"),
            "the run CONTINUED past the crash: {:?}",
            outcome.aborted
        );
        assert_eq!(outcome.cases_sent, 5);
    }

    #[tokio::test]
    async fn failed_recovery_keeps_findings_and_reports() {
        let dir = tempdir().unwrap();
        let mut s = suite();
        s.recovery = Some("reset".to_string());
        s.allowed_actions = vec!["send".to_string(), "reset".to_string()];
        let mut exec = MockExecutor::alive(0);
        exec.revive = false;
        let outcome = run_suite_with("uart-fuzz", &s, &ctx(dir.path()), &exec)
            .await
            .unwrap();
        assert_eq!(outcome.findings.len(), 1);
        assert!(
            outcome
                .aborted
                .as_ref()
                .unwrap()
                .contains("recovery failed"),
            "got: {:?}",
            outcome.aborted
        );
        assert!(outcome.dir.join("findings.jsonl").is_file());
        assert!(outcome.dir.join("report.md").is_file());
    }

    #[tokio::test]
    async fn reopen_failure_after_recovery_still_writes_reports() {
        let dir = tempdir().unwrap();
        let mut s = suite();
        s.recovery = Some("reset".to_string());
        s.allowed_actions = vec!["send".to_string(), "reset".to_string()];
        let mut exec = MockExecutor::alive(0);
        exec.fail_open_from = 2; // the reopen after recovery fails
        let outcome = run_suite_with("uart-fuzz", &s, &ctx(dir.path()), &exec)
            .await
            .unwrap();
        assert_eq!(outcome.findings.len(), 1);
        assert!(
            outcome.aborted.as_ref().unwrap().contains("cannot reopen"),
            "got: {:?}",
            outcome.aborted
        );
        assert!(outcome.dir.join("findings.jsonl").is_file());
    }

    #[tokio::test]
    async fn dead_liveness_probe_aborts_with_reports() {
        let dir = tempdir().unwrap();
        let mut s = suite();
        s.recovery = Some("reset".to_string());
        s.allowed_actions = vec!["send".to_string(), "reset".to_string()];
        let mut exec = MockExecutor::alive(0);
        exec.listen_text = ""; // reopened link hears nothing → target dead
        let outcome = run_suite_with("uart-fuzz", &s, &ctx(dir.path()), &exec)
            .await
            .unwrap();
        assert_eq!(outcome.findings.len(), 1);
        assert!(
            outcome.aborted.as_ref().unwrap().contains("target dead"),
            "got: {:?}",
            outcome.aborted
        );
        assert!(outcome.dir.join("report.md").is_file());
    }

    #[tokio::test]
    async fn alive_target_yields_no_findings() {
        let dir = tempdir().unwrap();
        let outcome = run_suite_with(
            "uart-fuzz",
            &suite(),
            &ctx(dir.path()),
            &MockExecutor::alive(usize::MAX),
        )
        .await
        .unwrap();
        assert!(outcome.findings.is_empty());
        assert_eq!(
            outcome.cases_sent, 35,
            "32 exhaustive bitflips (4-byte baseline) + 3 oversize"
        );
        assert!(outcome.aborted.is_none());
    }

    #[tokio::test]
    async fn budget_max_cases_stops_the_run() {
        let dir = tempdir().unwrap();
        let mut s = suite();
        s.budget.max_cases = 3;
        let outcome = run_suite_with(
            "uart-fuzz",
            &s,
            &ctx(dir.path()),
            &MockExecutor::alive(usize::MAX),
        )
        .await
        .unwrap();
        assert_eq!(outcome.cases_sent, 3);
        assert!(outcome.aborted.unwrap().contains("max_cases"));
    }

    #[tokio::test]
    async fn crash_with_capture_stays_high_without_forensic() {
        let dir = tempdir().unwrap();
        // Crash WITH non-empty captured text: the capture file exists, so
        // the finding stays HIGH even when no forensic report is available.
        struct SilentCrash;
        #[async_trait]
        impl TargetLink for SilentCrash {
            async fn send_and_capture(
                &mut self,
                _p: &[u8],
                _w: u64,
                _o: &OracleCfg,
                _c: &Cancellable,
            ) -> Result<Capture, String> {
                Ok(Capture {
                    text: "HardFault_Handler\n".to_string(),
                    timed_out: false,
                })
            }
            async fn listen(
                &mut self,
                _w: u64,
                _o: &OracleCfg,
                _c: &Cancellable,
            ) -> Result<Capture, String> {
                Ok(Capture {
                    text: String::new(),
                    timed_out: true,
                })
            }
        }
        struct NoForensic;
        #[async_trait]
        impl Executor for NoForensic {
            async fn open_link(&self, _i: &Interface) -> Result<Box<dyn TargetLink>, String> {
                Ok(Box::new(SilentCrash))
            }
            async fn recover(
                &self,
                _s: &RedteamSuite,
                _c: &ToolContext,
                _b: u64,
            ) -> Result<(), String> {
                Ok(())
            }
            async fn forensic(&self, _s: &RedteamSuite, _c: &ToolContext) -> Option<String> {
                None
            }
        }
        let outcome = run_suite_with("x", &suite(), &ctx(dir.path()), &NoForensic)
            .await
            .unwrap();
        let f = &outcome.findings[0];
        assert_eq!(f.confidence, "HIGH");
        assert_eq!(f.severity, Severity::High);
        assert!(f.evidence.iter().all(|e| !e.starts_with("forensic-")));
    }

    #[test]
    fn validation_refuses_unreproducible_or_unrevivable_suites() {
        let mut s = suite();
        s.mutation.as_mut().unwrap().seed = None;
        assert!(validate_suite(&s).unwrap_err().contains("seed"));

        let mut s = suite();
        s.recovery = Some("reflash".to_string());
        assert!(
            validate_suite(&s).unwrap_err().contains("allowed_actions"),
            "reflash without declared flash permission is refused"
        );

        let mut s = suite();
        s.interfaces[0].kind = "rtt".to_string();
        assert!(validate_suite(&s).unwrap_err().contains("not implemented"));

        assert!(parse_baseline(Some("55AA")).unwrap_err().contains("prefix"));
        assert_eq!(parse_baseline(Some("hex:55aa")).unwrap(), vec![0x55, 0xAA]);
        assert_eq!(parse_baseline(Some("text:hi")).unwrap(), b"hi");
        assert!(parse_baseline(Some("hex:5")).is_err());
    }

    #[test]
    fn toml_suite_parses_and_rejects_typos() {
        let text = r#"
[suite.uart-fuzz]
chip = "stm32g431rb"
elf = "build/fw.elf"
recovery = "reset"
allowed_actions = ["send", "reset"]
budget = { max_cases = 10, max_duration_ms = 5000, per_case_timeout_ms = 200 }

[[suite.uart-fuzz.interfaces]]
kind = "uart"
port = "COM3"
baud = 115200
baseline = "hex:55AA0102"

[suite.uart-fuzz.oracle]
heartbeat_regex = "tick=\\d+"
boot_banner = "boot v2"

[suite.uart-fuzz.mutation]
seed = 1234
strategies = ["boundary", "bitflip"]
"#;
        let file: RedteamFile = toml::from_str(text).unwrap();
        let s = file.suite.get("uart-fuzz").unwrap();
        assert_eq!(s.budget.max_cases, 10);
        assert_eq!(s.mutation.as_ref().unwrap().strategies.len(), 2);
        assert!(
            toml::from_str::<RedteamFile>(&text.replace("boot_banner", "boot_baner")).is_err(),
            "deny_unknown_fields must catch the typo"
        );
    }

    #[tokio::test]
    async fn dry_run_never_touches_hardware_or_claims_evidence() {
        let text = dry_run_plan("uart-fuzz", &suite()).unwrap();
        assert!(text.contains("[dry-run]"), "got: {text}");
        assert!(text.contains("bitflip"), "got: {text}");
        assert!(text.contains("[HIL_EXPECT:FAIL]"), "got: {text}");
    }

    #[tokio::test]
    async fn headless_live_is_refused_without_live_flag() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".firment")).unwrap();
        std::fs::write(
            dir.path().join(".firment/redteam.toml"),
            r#"
[suite.s]
recovery = "none"
allowed_actions = ["send"]
[[suite.s.interfaces]]
kind = "uart"
port = "COM_FAKE"
baseline = "hex:AA"
[suite.s.mutation]
seed = 1
strategies = ["oversize"]
"#,
        )
        .unwrap();
        // asker: None → headless; live not set → must refuse (Err, so the
        // CLI exits non-zero) and still show the corpus rehearsal.
        let err = Redteam
            .run(json!({"suite": "s"}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(err.message.contains("refused"), "got: {}", err.message);
        assert!(err.message.contains("[dry-run]"), "got: {}", err.message);
    }

    #[test]
    fn registered_in_all_and_absent_from_plan_registry() {
        assert!(crate::tools::all().iter().any(|t| t.name() == "redteam"));
        assert!(crate::plan_registry().get("redteam").is_none());
    }

    #[test]
    fn attacker_registry_has_hardware_no_self_replication() {
        let reg = crate::attacker_registry();
        for name in [
            "monitor",
            "debug",
            "la",
            "elf_analyze",
            "observe",
            "device_cmd",
        ] {
            assert!(
                reg.get(name).is_some(),
                "{name} must be available to the attacker"
            );
        }
        for name in [
            "flash",
            "run",
            "shell",
            "write_file",
            "edit_file",
            "task",
            "redteam",
        ] {
            assert!(
                reg.get(name).is_none(),
                "{name} must NOT be reachable by the attacker"
            );
        }
    }

    #[test]
    fn campaign_prompt_names_interfaces_and_the_findings_block() {
        let p = campaign_prompt("uart-fuzz", &suite(), "35 cases, 0 findings");
        assert!(p.contains("COM_FAKE"), "got: {p}");
        assert!(p.contains("redteam-findings"), "got: {p}");
        assert!(p.contains("35 cases"), "got: {p}");
        assert!(p.contains("Do NOT reflash"), "got: {p}");
        // The debugger halts the target — the prompt must say so, or the
        // attacker reports its own silence as a hang.
        assert!(
            p.contains("leaves the target HALTED"),
            "the campaign must warn that debug stops the target: {p}"
        );
    }

    #[test]
    fn extract_findings_block_parses_and_tolerates_garbage() {
        let text = "prose …\n```redteam-findings\n[{\"severity\":\"high\",\"class\":\"crash\",\
                    \"payload_hex\":\"55aa01a2\",\"interface\":\"uart@COM3\",\
                    \"observed\":\"HardFault_Handler\",\"evidence\":[\"capture-F-001.log\"],\
                    \"why\":\"bit flip in length byte\"}]\n```\ntrailing";
        let fs = extract_findings_block(text, "uart-fuzz", 1234, 2);
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].finding_id, "F-002");
        assert_eq!(fs[0].severity, Severity::High);
        assert_eq!(fs[0].strategy, "campaign");
        assert_eq!(fs[0].payload_hex, "55aa01a2");

        // No block / bad JSON / empty array → nothing.
        assert!(extract_findings_block("no block here", "s", 1, 1).is_empty());
        assert!(extract_findings_block("```redteam-findings\nnot json\n```", "s", 1, 1).is_empty());
        assert!(extract_findings_block("```redteam-findings\n[]\n```", "s", 1, 1).is_empty());
        // Missing payload_hex survives extraction but the finalize rule
        // (no evidence file) will cap it — the payload is the reproducer.
        let loose = extract_findings_block(
            "```redteam-findings\n[{\"class\":\"hang\"}]\n```",
            "s",
            1,
            1,
        );
        assert_eq!(loose.len(), 1);
        assert!(loose[0].payload_hex.is_empty());
    }

    #[tokio::test]
    async fn campaign_without_attacker_runner_is_actionable_error() {
        let dir = tempdir().unwrap();
        let err = run_campaign("x", &suite(), &ctx(dir.path()), "note", 1)
            .await
            .unwrap_err();
        assert!(err.contains("attacker runner"), "got: {err}");
    }
}
