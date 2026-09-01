//! `la` — logic analyzer capture, measurement and protocol decode, giving
//! the agent evidence at rung 5 of the verification ladder: waveforms are
//! physical behaviour, measured not asserted.
//!
//! sigrok-cli is invoked as an EXTERNAL binary with an argv array (no shell,
//! no linked library — the same probe-rs pattern, and the reason the GPL of
//! sigrok stays out of this MIT binary). The `.sr` session files it writes
//! are archived for the user's own PulseView review and NEVER parsed here;
//! measurements read the raw-bit export instead, through
//! [`crate::la_cmd::unpack_bitstream`], and protocol semantics are delegated
//! to sigrok's own decoders.
//!
//! Capture is the only hardware-touching action and the only one that asks
//! for approval; everything else is read-only over stored captures.

use super::util::{resolve_within, run_argv, truncate};
use crate::la_cmd::{
    self, CaptureRequest, build_capture_argv, build_decode_argv, build_export_binary_argv,
    build_info_argv, build_session_show_argv, count_channels, drivers_argv, parse_pd_annotations,
    parse_sigrok_version, samplerate_hz, sanitize_channels, sanitize_driver, version_argv,
};
use crate::la_measure::{
    EdgeKind, count_edges, estimate_bitrate, measure_duty, measure_frequency, measure_pulse_widths,
};
use async_trait::async_trait;
use firment_core::config::LaConfig;
use firment_core::{Cancellable, Tool, ToolContext, ToolError, ToolOutput};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What the tool needs from the outside world: run one sigrok-cli
/// invocation. A trait so tests inject [`FakeBackend`] and a future Saleae
/// REST backend can slot in beside [`SigrokCli`].
#[async_trait]
pub trait CaptureBackend: Send + Sync {
    /// Run `bin` with the given argv (no shell); returns combined
    /// stdout+stderr and the exit code (`None` = killed by timeout/cancel).
    async fn exec(
        &self,
        bin: &str,
        argv: &[String],
        cwd: &Path,
        timeout_ms: u64,
        cancel: Option<Cancellable>,
    ) -> Result<(String, Option<i32>), String>;
}

/// Production backend: spawn sigrok-cli directly, argv array, timeout and
/// cancellation handled by the shared external-CLI runner.
pub struct SigrokCli;

#[async_trait]
impl CaptureBackend for SigrokCli {
    async fn exec(
        &self,
        bin: &str,
        argv: &[String],
        cwd: &Path,
        timeout_ms: u64,
        cancel: Option<Cancellable>,
    ) -> Result<(String, Option<i32>), String> {
        run_argv(bin, argv.to_vec(), cwd, timeout_ms, cancel, &[]).await
    }
}

/// The `la` tool. `backend` is the injectable seam; the registry uses the
/// default (real sigrok-cli).
pub struct La {
    backend: Arc<dyn CaptureBackend>,
}

impl Default for La {
    fn default() -> Self {
        Self {
            backend: Arc::new(SigrokCli),
        }
    }
}

impl La {
    #[cfg(test)]
    pub fn with_backend(backend: Arc<dyn CaptureBackend>) -> Self {
        Self { backend }
    }
}

/// One stored capture, as recorded next to its files.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CaptureMeta {
    id: String,
    driver: String,
    channels: String,
    channel_count: usize,
    #[serde(default)]
    samplerate: Option<String>,
    #[serde(default)]
    samplerate_hz: Option<f64>,
    #[serde(default)]
    samples: Option<u64>,
    #[serde(default)]
    time_ms: Option<u64>,
    created_unix: u64,
    has_binary: bool,
}

/// Where captures live inside the workspace.
pub(crate) fn la_dir(cwd: &Path) -> PathBuf {
    cwd.join(".firment").join("la")
}

/// Resolve a `capture` argument: either an id (a name without separators,
/// looked up under `.firment/la/`) or an explicit workspace path. Returns
/// (stem path without extension, meta).
fn resolve_capture(ctx: &ToolContext, arg: &str) -> Result<(PathBuf, CaptureMeta), ToolError> {
    let dir = la_dir(&ctx.cwd);
    let looks_like_id = !arg.contains('/') && !arg.contains('\\') && !arg.contains(".sr");
    let stem = if looks_like_id {
        // Same charset rule as hil replay ids: `dir.join(arg)` with ".." or
        // a dot-name would walk out of .firment/la/ (join replaces on
        // absolute paths, and ".." resolves upward).
        if arg.is_empty()
            || !arg
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            || arg.starts_with('.')
        {
            return Err(ToolError::new(format!(
                "[InvalidInput] capture id '{arg}' is not a plain name (letters, digits, - _ . \
                 ; no leading dot)"
            )));
        }
        dir.join(arg)
    } else {
        let resolved = resolve_within(&ctx.cwd, arg, &ctx.allowed_roots).map_err(ToolError::new)?;
        // strip a trailing .sr/.bin to get the stem
        match resolved.extension() {
            Some(ext) if ext == "sr" || ext == "bin" => resolved.with_extension(""),
            _ => resolved,
        }
    };
    let meta_path = stem.with_extension("meta.json");
    let text = std::fs::read_to_string(&meta_path).map_err(|_| {
        ToolError::new(format!(
            "[NotFound] no capture '{arg}' (looked for {}) — run action=capture first, or \
             action=list_captures to see what exists",
            meta_path.display()
        ))
    })?;
    let meta: CaptureMeta = serde_json::from_str(&text)
        .map_err(|e| ToolError::new(format!("[Io] corrupt meta {}: {e}", meta_path.display())))?;
    Ok((stem, meta))
}

/// Effective timeout for one sigrok-cli invocation: explicit config wins;
/// otherwise a minute plus the capture window itself.
fn exec_timeout(cfg: &LaConfig, time_ms: Option<u64>) -> u64 {
    cfg.timeout_ms.unwrap_or(60_000 + time_ms.unwrap_or(0))
}

/// A stored capture loaded for measurement: unpacked per-channel waves plus
/// the metadata the analysers need. Shared with the HIL `la` step so the
/// assertion logic reads exactly what `la measure` reads.
pub(crate) struct LaCapture {
    pub id: String,
    pub channel_count: usize,
    pub samplerate_hz: Option<f64>,
    pub waves: Vec<Vec<u8>>,
}

pub(crate) fn load_capture_waves(ctx: &ToolContext, arg: &str) -> Result<LaCapture, String> {
    let (stem, meta) = resolve_capture(ctx, arg).map_err(|e| e.message)?;
    if !meta.has_binary {
        return Err(format!(
            "capture {} has no raw-bit sidecar (export failed at capture time) — measure is \
             unavailable; decode still works",
            meta.id
        ));
    }
    let bytes = std::fs::read(stem.with_extension("bin")).map_err(|_| {
        format!(
            "capture {} has no raw-bit sidecar (export failed at capture time)",
            meta.id
        )
    })?;
    // Size sanity: a half-written export must not be measured as if it were
    // the whole capture — truncated waveforms fabricate confident garbage.
    if let Some(n) = meta.samples {
        let expected = n as usize * meta.channel_count.div_ceil(8);
        if bytes.len() != expected {
            return Err(format!(
                "[Io] capture {} bitstream is {} bytes, expected {expected} for {} samples × {} \
                 channels — the export is truncated; re-capture",
                meta.id,
                bytes.len(),
                n,
                meta.channel_count
            ));
        }
    }
    let waves = la_cmd::unpack_bitstream(&bytes, meta.channel_count)?;
    Ok(LaCapture {
        id: meta.id,
        channel_count: meta.channel_count,
        samplerate_hz: meta.samplerate_hz,
        waves,
    })
}

/// `sigrok-cli -L` prints FIVE sections (hardware drivers, input formats,
/// output formats, transform modules, protocol decoders). Matching a driver
/// name against the whole blob would report `uart` (a decoder) or `srzip`
/// (a format) as a supported driver — only the first section counts.
fn drivers_section(text: &str) -> &str {
    let start = match text.find("Supported hardware drivers:") {
        Some(i) => i + "Supported hardware drivers:".len(),
        None => return "",
    };
    let rest = &text[start..];
    match rest.find("\nSupported ") {
        Some(end) => &rest[..end],
        None => rest,
    }
}

impl La {
    async fn run_detect(
        &self,
        cfg: &LaConfig,
        args: &Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let bin = cfg.bin.as_deref().unwrap_or("sigrok-cli");
        let (ver_text, code) = self
            .backend
            .exec(
                bin,
                &version_argv(),
                &ctx.cwd,
                exec_timeout(cfg, None),
                Some(ctx.cancel.clone()),
            )
            .await
            .map_err(|e| ToolError::new(la_cmd::sigrok_err_hint(&e)))?;
        if code != Some(0) {
            return Ok(ToolOutput {
                text: format!(
                    "[la] sigrok-cli probe failed (exit {:?}):\n{}",
                    code,
                    truncate(&ver_text, 4000)
                ),
            });
        }
        let version = parse_sigrok_version(&ver_text);
        let (drv_text, _) = self
            .backend
            .exec(
                bin,
                &drivers_argv(),
                &ctx.cwd,
                exec_timeout(cfg, None),
                Some(ctx.cancel.clone()),
            )
            .await
            .map_err(|e| ToolError::new(la_cmd::sigrok_err_hint(&e)))?;
        let want = args
            .get("driver")
            .and_then(|v| v.as_str())
            .or(if cfg.driver.is_empty() {
                None
            } else {
                Some(cfg.driver.as_str())
            });
        let mut text = match version {
            Some((maj, min, pat)) => format!("[la] sigrok-cli {maj}.{min}.{pat} found at '{bin}'"),
            None => format!(
                "[la] '{bin}' answered but not with a sigrok-cli banner:\n{}",
                truncate(&ver_text, 500)
            ),
        };
        if let Some(want) = want {
            let listed = drivers_section(&drv_text).lines().any(|l| {
                l.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
                    .any(|tok| tok == want)
            });
            text.push_str(&format!(
                "\n  configured driver '{want}': {}",
                if listed {
                    "supported by this sigrok-cli build"
                } else {
                    "NOT in this build's driver list — run `sigrok-cli -L` to see what is"
                }
            ));
        } else {
            text.push_str(
                "\n  no driver configured — set [tools.la] driver=... or pass driver explicitly",
            );
        }
        let n_drivers = drivers_section(&drv_text)
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with("Supported"))
            .count();
        text.push_str(&format!("\n  {n_drivers} hardware driver lines reported"));
        Ok(ToolOutput {
            text: truncate(&text, 32_000),
        })
    }

    async fn run_info(
        &self,
        cfg: &LaConfig,
        args: &Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let driver = args
            .get("driver")
            .and_then(|v| v.as_str())
            .or(if cfg.driver.is_empty() {
                None
            } else {
                Some(cfg.driver.as_str())
            })
            .ok_or_else(|| {
                ToolError::new(
                    "[InvalidInput] info needs a driver — pass driver=... or configure \
                     [tools.la] driver",
                )
            })?;
        let argv = build_info_argv(driver).map_err(ToolError::new)?;
        let bin = cfg.bin.as_deref().unwrap_or("sigrok-cli");
        let (text, code) = self
            .backend
            .exec(
                bin,
                &argv,
                &ctx.cwd,
                exec_timeout(cfg, None),
                Some(ctx.cancel.clone()),
            )
            .await
            .map_err(|e| ToolError::new(la_cmd::sigrok_err_hint(&e)))?;
        if code != Some(0) {
            return Err(ToolError::new(la_cmd::sigrok_err_hint(&text)));
        }
        Ok(ToolOutput {
            text: format!("[la] {driver} capabilities:\n{}", truncate(&text, 32_000)),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_capture(
        &self,
        cfg: &LaConfig,
        ctx: &ToolContext,
        driver: &str,
        channels: &str,
        samplerate: Option<&str>,
        samples: Option<u64>,
        time_ms: Option<u64>,
    ) -> Result<ToolOutput, ToolError> {
        let channel_count = count_channels(channels).ok_or_else(|| {
            ToolError::new(format!(
                "[InvalidInput] channels '{channels}' is not a parsable sigrok spec \
                 (e.g. \"D0,D1\" or \"0,1,2-3\")"
            ))
        })?;
        if let Some(n) = samples
            && n > cfg.max_samples
        {
            return Err(ToolError::new(format!(
                "[InvalidInput] samples {n} exceeds the configured cap max_samples = {} \
                 ([tools.la]) — shorten the window or raise the cap deliberately",
                cfg.max_samples
            )));
        }
        if let Some(t) = time_ms
            && t > cfg.max_time_ms
        {
            return Err(ToolError::new(format!(
                "[InvalidInput] time_ms {t} exceeds the configured cap max_time_ms = {} \
                 ([tools.la]) — a one-hour window belongs in PulseView, not an agent turn",
                cfg.max_time_ms
            )));
        }
        let req = CaptureRequest {
            driver: driver.to_string(),
            channels: Some(channels.to_string()),
            samplerate: samplerate.map(|s| s.to_string()),
            samples,
            time_ms,
        };
        let dir = la_dir(&ctx.cwd);
        std::fs::create_dir_all(&dir)
            .map_err(|e| ToolError::new(format!("[Io] create {}: {e}", dir.display())))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let id = format!("la-{}-{:06}", now.as_secs(), now.subsec_micros());
        let stem = dir.join(&id);
        let sr_path = stem.with_extension("sr");
        let bin_path = stem.with_extension("bin");
        let argv = build_capture_argv(&req, &sr_path).map_err(ToolError::new)?;
        let cli_bin = cfg.bin.as_deref().unwrap_or("sigrok-cli");
        let timeout = exec_timeout(cfg, time_ms);

        let (text, code) = match self
            .backend
            .exec(cli_bin, &argv, &ctx.cwd, timeout, Some(ctx.cancel.clone()))
            .await
        {
            Ok(pair) => pair,
            Err(e) => {
                // A spawn/timeout failure can still leave a half-written .sr
                // behind — sweep it so list_captures never sees a ghost.
                let _ = std::fs::remove_file(&sr_path);
                return Err(ToolError::new(la_cmd::sigrok_err_hint(&e)));
            }
        };
        if code != Some(0) {
            let _ = std::fs::remove_file(&sr_path);
            return Err(ToolError::new(la_cmd::sigrok_err_hint(&text)));
        }

        // Read the session's REAL channel count and samplerate back from
        // sigrok itself: the device snaps a requested rate to the nearest
        // supported one (fx2lafw knows only 6/12/24/48 MHz), and measuring
        // against the REQUESTED value would bias every frequency.
        let mut actual_channels = channel_count;
        let mut actual_hz = samplerate.and_then(samplerate_hz);
        let mut readback_note = String::new();
        match self
            .backend
            .exec(
                cli_bin,
                &build_session_show_argv(&sr_path),
                &ctx.cwd,
                timeout,
                Some(ctx.cancel.clone()),
            )
            .await
        {
            Ok((show_text, Some(0))) => {
                let info = la_cmd::parse_session_show(&show_text);
                if let Some(n) = info.channels {
                    actual_channels = n;
                }
                if let Some(hz) = info.samplerate_hz {
                    if Some(hz) != actual_hz {
                        readback_note = format!(
                            "\n  note: device samplerate is {hz:.0} Hz (requested {})",
                            samplerate.unwrap_or("default")
                        );
                    }
                    actual_hz = Some(hz);
                }
            }
            _ => {
                readback_note =
                    "\n  note: session readback failed — channel count and samplerate are the \
                     requested values, frequencies may be biased if the device snapped the rate"
                        .to_string();
            }
        }

        // Export the stored session to raw bits for the measurement layer.
        // A failure here is not fatal: the .sr is still valid for decode and
        // for the user's PulseView — but a half-written .bin IS fatal for
        // measure (truncated waveforms are worse than none), so it is swept.
        let export_argv = build_export_binary_argv(&sr_path, &bin_path);
        let has_binary = matches!(
            self.backend
                .exec(
                    cli_bin,
                    &export_argv,
                    &ctx.cwd,
                    timeout,
                    Some(ctx.cancel.clone()),
                )
                .await,
            Ok((_, Some(0)))
        );
        if !has_binary {
            let _ = std::fs::remove_file(&bin_path);
        }

        let meta = CaptureMeta {
            id: id.clone(),
            driver: driver.to_string(),
            channels: channels.to_string(),
            channel_count: actual_channels,
            samplerate: samplerate.map(|s| s.to_string()),
            samplerate_hz: actual_hz,
            samples,
            time_ms,
            created_unix: now.as_secs(),
            has_binary,
        };
        std::fs::write(
            stem.with_extension("meta.json"),
            serde_json::to_vec_pretty(&meta)
                .map_err(|e| ToolError::new(format!("[Io] encode meta: {e}")))?,
        )
        .map_err(|e| ToolError::new(format!("[Io] write meta: {e}")))?;

        let bound = match (samples, time_ms) {
            (Some(n), _) => format!("{n} samples"),
            (None, Some(t)) => format!("{t} ms"),
            (None, None) => unreachable!("argv builder required a bound"),
        };
        let mut text = format!(
            "[la] captured {id}\n  driver: {driver}  channels: {channels} ({actual_channels})  \
             samplerate: {}  window: {bound}\n  session: {}\n",
            actual_hz
                .map(|h| format!("{h:.0} Hz"))
                .unwrap_or_else(|| "(device default)".to_string()),
            sr_path.display(),
        );
        if has_binary {
            text.push_str(&format!("  bits:    {}\n", bin_path.display()));
        } else {
            text.push_str(
                "  bits:    export failed — action=measure unavailable for this capture, \
                 action=decode still works\n",
            );
        }
        text.push_str(&readback_note);
        text.push_str(&format!(
            "  next: la measure capture={id} channel=0 what=frequency|duty|edges|pulse_widths|bitrate \
             · la decode capture={id} decoder=uart opts={{\"rx\":\"0\",\"baudrate\":\"115200\"}}"
        ));
        Ok(ToolOutput {
            text: truncate(&text, 32_000),
        })
    }

    fn run_measure(&self, args: &Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let capture = args
            .get("capture")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] measure needs 'capture' (id or path)"))?;
        let what = args
            .get("what")
            .and_then(|v| v.as_str())
            .unwrap_or("frequency");
        let channel = args.get("channel").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        // Shared loader: has_binary + size sanity + unpack in one place, so
        // `la measure` and the HIL `la` step read exactly the same bytes.
        let cap = load_capture_waves(ctx, capture).map_err(ToolError::new)?;
        if channel >= cap.channel_count {
            return Err(ToolError::new(format!(
                "[InvalidInput] channel {channel} out of range — capture has {} channel(s)",
                cap.channel_count
            )));
        }
        let wave = &cap.waves[channel];
        let sr = cap.samplerate_hz;
        let need_hz = |what: &str| -> Result<f64, ToolError> {
            sr.ok_or_else(|| {
                ToolError::new(format!(
                    "[InvalidInput] {what} needs a known samplerate; this capture stored none — \
                     capture again with samplerate=... set"
                ))
            })
        };
        let mut text = format!(
            "[la] measure capture={} channel={channel} ({what})\n  samples: {}\n",
            cap.id,
            wave.len()
        );
        match what {
            "frequency" => {
                let hz = need_hz("frequency")?;
                match measure_frequency(wave, hz) {
                    Some(f) => {
                        text.push_str(&format!(
                            "  frequency: {} .. {} Hz (~{:.2})\n  rising edges: {}\n  confidence: \
                             {} — {}\n",
                            f.hz_low,
                            f.hz_high,
                            f.hz,
                            f.rising_edges,
                            f.confidence.name(),
                            f.note,
                        ));
                    }
                    None => text.push_str(
                        "  frequency: not periodic (fewer than two rising edges — one transition \
                         is not a period)\n",
                    ),
                }
            }
            "duty" => match measure_duty(wave) {
                Some(d) => text.push_str(&format!(
                    "  duty: {:.1}% high\n  confidence: {} — {}\n",
                    d.fraction * 100.0,
                    d.confidence.name(),
                    d.note,
                )),
                None => text.push_str("  duty: not periodic (needs two rising edges)\n"),
            },
            "edges" => {
                let kind = match args.get("edge").and_then(|v| v.as_str()).unwrap_or("both") {
                    "rising" => EdgeKind::Rising,
                    "falling" => EdgeKind::Falling,
                    _ => EdgeKind::Both,
                };
                text.push_str(&format!(
                    "  edges ({}): {}\n",
                    args.get("edge").and_then(|v| v.as_str()).unwrap_or("both"),
                    count_edges(wave, kind)
                ));
            }
            "pulse_widths" => {
                let hz = need_hz("pulse_widths")?;
                match measure_pulse_widths(wave, hz) {
                    Some(p) => text.push_str(&format!(
                        "  pulses: {}  width: {:.0} .. {:.0} ns\n  confidence: {} — {}\n",
                        p.pulses,
                        p.min_ns,
                        p.max_ns,
                        p.confidence.name(),
                        p.note,
                    )),
                    None => text.push_str("  pulse widths: the channel never goes high\n"),
                }
            }
            "bitrate" => {
                let hz = need_hz("bitrate")?;
                match estimate_bitrate(wave, hz) {
                    Some(b) => text.push_str(&format!(
                        "  bitrate estimate: {:.0} bps\n  confidence: {} — {}\n",
                        b.bps,
                        b.confidence.name(),
                        b.note,
                    )),
                    None => text.push_str("  bitrate: fewer than two transitions to space\n"),
                }
            }
            other => {
                return Err(ToolError::new(format!(
                    "[InvalidInput] what='{other}' — use frequency/duty/edges/pulse_widths/bitrate"
                )));
            }
        }
        text.push_str("[evidence: physical — logic capture]");
        Ok(ToolOutput {
            text: truncate(&text, 32_000),
        })
    }

    async fn run_decode(
        &self,
        cfg: &LaConfig,
        args: &Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let capture = args
            .get("capture")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] decode needs 'capture' (id or path)"))?;
        let decoder = args
            .get("decoder")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] decode needs 'decoder' (e.g. uart)"))?;
        let mut opts: BTreeMap<String, String> = BTreeMap::new();
        if let Some(obj) = args.get("decoder_opts").and_then(|v| v.as_object()) {
            for (k, v) in obj {
                let value = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                opts.insert(k.clone(), value);
            }
        }
        let (stem, meta) = resolve_capture(ctx, capture)?;
        let sr_path = stem.with_extension("sr");
        let argv = build_decode_argv(&sr_path, decoder, &opts).map_err(ToolError::new)?;
        let bin = cfg.bin.as_deref().unwrap_or("sigrok-cli");
        let (text, code) = self
            .backend
            .exec(
                bin,
                &argv,
                &ctx.cwd,
                exec_timeout(cfg, None),
                Some(ctx.cancel.clone()),
            )
            .await
            .map_err(|e| ToolError::new(la_cmd::sigrok_err_hint(&e)))?;
        if code != Some(0) {
            return Err(ToolError::new(la_cmd::sigrok_err_hint(&text)));
        }
        let frames = parse_pd_annotations(&text);
        let mut out = format!(
            "[la] decode capture={} decoder={decoder}\n  frames: {}\n",
            meta.id,
            frames.len()
        );
        for f in frames.iter().take(200) {
            out.push_str(&format!("  {}: {}\n", f.decoder, f.text));
        }
        if frames.len() > 200 {
            out.push_str(&format!("  … {} more frames omitted\n", frames.len() - 200));
        }
        if frames.is_empty() && !text.trim().is_empty() {
            out.push_str(&format!(
                "  (no annotations parsed; raw output:\n{})\n",
                truncate(&text, 2000)
            ));
        }
        out.push_str("[evidence: physical — logic capture]");
        Ok(ToolOutput {
            text: truncate(&out, 32_000),
        })
    }

    fn run_list_captures(&self, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let dir = la_dir(&ctx.cwd);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => {
                return Ok(ToolOutput {
                    text: "[la] no captures yet — action=capture first".to_string(),
                });
            }
        };
        let mut rows = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Ok(m) = serde_json::from_str::<CaptureMeta>(&text) {
                rows.push(format!(
                    "  {}  {} ch={} sr={} window={} bits={}",
                    m.id,
                    m.driver,
                    m.channels,
                    m.samplerate.as_deref().unwrap_or("-"),
                    match (m.samples, m.time_ms) {
                        (Some(n), _) => format!("{n} samples"),
                        (None, Some(t)) => format!("{t} ms"),
                        (None, None) => "-".to_string(),
                    },
                    if m.has_binary { "yes" } else { "no" },
                ));
            }
        }
        rows.sort();
        if rows.is_empty() {
            return Ok(ToolOutput {
                text: "[la] no captures yet — action=capture first".to_string(),
            });
        }
        Ok(ToolOutput {
            text: format!("[la] captures in {}:\n{}", dir.display(), rows.join("\n")),
        })
    }
}

#[async_trait]
impl Tool for La {
    fn name(&self) -> &'static str {
        "la"
    }

    fn description(&self) -> &'static str {
        "Logic analyzer: capture digital waveforms through sigrok-cli and turn them into PHYSICAL evidence (verification ladder rung 5). Actions: detect (is sigrok-cli installed, does it know the driver), info (device capabilities), capture (bounded acquisition stored under .firment/la/), measure (frequency as a range, duty, edge counts, pulse widths, bitrate — deterministic math on the raw bits, every verdict carries a confidence), decode (protocol annotations via sigrok decoders: uart/spi/i2c/1-wire/NEC/CAN …), list_captures. Configure the device once in [tools.la] (sigrok driver name); capture is the only action that touches hardware and it asks for approval. The .sr sessions are archived for PulseView; measurements never parse them."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["detect", "info", "capture", "measure", "decode", "list_captures"],
                    "description": "detect: probe the sigrok-cli install. info: device capabilities. capture: acquire a bounded window. measure: frequency/duty/edges/pulse_widths/bitrate on a stored capture. decode: run a sigrok protocol decoder on a stored capture. list_captures: what is on disk."
                },
                "driver": {"type": "string", "description": "capture/info/detect: sigrok driver name (falls back to [tools.la] driver)."},
                "channels": {"type": "string", "description": "capture: sigrok channel spec — names as the driver reports them (fx2lafw/demo: \"D0,D1\", ranges like \"D0-D7\"), optionally labelled \"D0=SCLK,D1=MOSI\" (falls back to [tools.la] channels). Required for measure-capable captures."},
                "samplerate": {"type": "string", "description": "capture: sigrok samplerate token, e.g. \"8m\" (8 MHz). Without it frequency/pulse/bitrate measures cannot run."},
                "samples": {"type": "integer", "minimum": 1, "description": "capture: sample count to acquire (bounded by [tools.la] max_samples)."},
                "time_ms": {"type": "integer", "minimum": 1, "description": "capture: wall-clock window in ms — the other way to bound an acquisition."},
                "capture": {"type": "string", "description": "measure/decode: capture id (from capture/list_captures) or a workspace path to a stored session."},
                "channel": {"type": "integer", "minimum": 0, "description": "measure: which captured channel to analyse (default 0)."},
                "what": {"type": "string", "enum": ["frequency", "duty", "edges", "pulse_widths", "bitrate"], "description": "measure: what to measure (default frequency)."},
                "edge": {"type": "string", "enum": ["rising", "falling", "both"], "description": "measure what=edges: which transitions to count (default both)."},
                "decoder": {"type": "string", "description": "decode: sigrok protocol decoder name (uart, spi, i2c, 1-wire, nfc-*, can, …)."},
                "decoder_opts": {"type": "object", "description": "decode: decoder options, e.g. {\"rx\":\"0\",\"baudrate\":\"115200\"}."}
            },
            "required": ["action"]
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        // Only capture touches the USB device; everything else is read-only
        // over stored files or a version/driver query.
        if args.get("action").and_then(|v| v.as_str()) == Some("capture") {
            let driver = args
                .get("driver")
                .and_then(|v| v.as_str())
                .unwrap_or("(configured)");
            let bound = match (
                args.get("samples").and_then(|v| v.as_u64()),
                args.get("time_ms").and_then(|v| v.as_u64()),
            ) {
                (Some(n), _) => format!("{n} samples"),
                (None, Some(t)) => format!("{t} ms"),
                (None, None) => "configured window".to_string(),
            };
            Some(format!(
                "⚠ la capture: {driver} — {bound}; occupies the logic analyzer's USB session"
            ))
        } else {
            None
        }
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing 'action'"))?;
        let cfg = ctx.la.clone().unwrap_or_default();
        match action {
            "detect" => self.run_detect(&cfg, &args, ctx).await,
            "info" => self.run_info(&cfg, &args, ctx).await,
            "list_captures" => self.run_list_captures(ctx),
            "measure" => self.run_measure(&args, ctx),
            "decode" => self.run_decode(&cfg, &args, ctx).await,
            "capture" => {
                let driver = args
                    .get("driver")
                    .and_then(|v| v.as_str())
                    .or(if cfg.driver.is_empty() {
                        None
                    } else {
                        Some(cfg.driver.as_str())
                    })
                    .ok_or_else(|| {
                        ToolError::new(
                            "[InvalidInput] capture needs a driver — pass driver=... or configure \
                             [tools.la] driver",
                        )
                    })?;
                sanitize_driver(driver).map_err(ToolError::new)?;
                let channels = args
                    .get("channels")
                    .and_then(|v| v.as_str())
                    .or(cfg.channels.as_deref())
                    .ok_or_else(|| {
                        ToolError::new(
                            "[InvalidInput] capture needs channels (e.g. \"0,1,2-3\") — measure \
                             must know the channel count to decode the raw bits",
                        )
                    })?;
                sanitize_channels(channels).map_err(ToolError::new)?;
                let samplerate = args
                    .get("samplerate")
                    .and_then(|v| v.as_str())
                    .or(cfg.samplerate.as_deref());
                if let Some(sr) = samplerate {
                    la_cmd::sanitize_samplerate(sr).map_err(ToolError::new)?;
                }
                let samples = args.get("samples").and_then(|v| v.as_u64());
                let time_ms = args.get("time_ms").and_then(|v| v.as_u64());
                if samples.is_none() && time_ms.is_none() {
                    return Err(ToolError::new(
                        "[InvalidInput] capture needs a bound: samples=... or time_ms=... — an \
                         unbounded capture never returns",
                    ));
                }
                self.run_capture(&cfg, ctx, driver, channels, samplerate, samples, time_ms)
                    .await
            }
            other => Err(ToolError::new(format!(
                "[InvalidInput] action='{other}' — use detect/info/capture/measure/decode/\
                 list_captures"
            ))),
        }
    }
}

/// Canned sigrok-cli for tests (this crate's, including the HIL `la` step
/// tests): answers version/driver/info/decode queries and writes a fixed
/// raw-bit file when asked to export binary.
#[cfg(test)]
pub(crate) struct FakeBackend {
    pub bin_bytes: Vec<u8>,
    pub decode_out: String,
    pub fail_capture: bool,
}

#[cfg(test)]
impl FakeBackend {
    pub(crate) fn new(bin_bytes: Vec<u8>) -> Self {
        Self {
            bin_bytes,
            decode_out: "uart: TX: (0x55)\n".to_string(),
            fail_capture: false,
        }
    }
}

#[cfg(test)]
#[async_trait]
impl CaptureBackend for FakeBackend {
    async fn exec(
        &self,
        _bin: &str,
        argv: &[String],
        _cwd: &Path,
        _timeout_ms: u64,
        _cancel: Option<Cancellable>,
    ) -> Result<(String, Option<i32>), String> {
        let joined = argv.join(" ");
        if joined.contains("--version") {
            return Ok(("sigrok-cli 0.7.2\n".to_string(), Some(0)));
        }
        if joined.contains("-L") {
            // Realistic five-section -L layout: the driver check must not
            // mistake a protocol decoder for a hardware driver.
            return Ok((
                "Supported hardware drivers:\n  fx2lafw - FTDI 2-channel logic analyzer\n  \
                 saleae-logic - Saleae Logic\n\nSupported protocol decoders:\n  uart - UART\n"
                    .to_string(),
                Some(0),
            ));
        }
        if joined.contains("--show") {
            return Ok(("Channels: 2\nSamplerate: 8000000\n".to_string(), Some(0)));
        }
        if joined.contains("-P") {
            return Ok((self.decode_out.clone(), Some(0)));
        }
        if joined.contains("-O binary") {
            let out = argv[argv.iter().position(|a| a == "-o").unwrap() + 1].clone();
            std::fs::write(&out, &self.bin_bytes).unwrap();
            return Ok((String::new(), Some(0)));
        }
        // the capture invocation itself
        if self.fail_capture {
            return Ok(("Device busy.".to_string(), Some(1)));
        }
        let out = argv[argv.iter().position(|a| a == "-o").unwrap() + 1].clone();
        std::fs::write(&out, b"sr-stub").unwrap();
        Ok((String::new(), Some(0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal};
    use std::sync::Mutex;
    use tempfile::tempdir;

    fn ctx_with(dir: &Path, la: Option<LaConfig>) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            journal: Arc::new(Mutex::new(EditJournal::new(dir.join("undo")))),
            la,
            ..ToolContext::default()
        }
    }

    fn cfg() -> LaConfig {
        LaConfig {
            driver: "fx2lafw".to_string(),
            channels: Some("0,1".to_string()),
            samplerate: Some("8m".to_string()),
            ..LaConfig::default()
        }
    }

    /// 2 channels × 4 samples packed LSB-first: ch0 = 1,0,1,0; ch1 = 0,1,1,0.
    const BITS: [u8; 4] = [0b01, 0b10, 0b11, 0b00];

    fn fake_la(bin_bytes: Vec<u8>) -> La {
        La::with_backend(Arc::new(FakeBackend::new(bin_bytes)))
    }

    #[tokio::test]
    async fn detect_reports_version_and_configured_driver() {
        let dir = tempdir().unwrap();
        let out = fake_la(vec![])
            .run(
                json!({"action": "detect"}),
                &ctx_with(dir.path(), Some(cfg())),
            )
            .await
            .unwrap()
            .text;
        assert!(out.contains("0.7.2"), "got: {out}");
        assert!(out.contains("'fx2lafw': supported"), "got: {out}");
    }

    #[tokio::test]
    async fn capture_stores_session_bits_and_meta_then_measures() {
        let dir = tempdir().unwrap();
        let la = fake_la(BITS.to_vec());
        let out = la
            .run(
                json!({"action": "capture", "samples": 4}),
                &ctx_with(dir.path(), Some(cfg())),
            )
            .await
            .unwrap()
            .text;
        assert!(out.contains("[la] captured la-"), "got: {out}");
        let ctx = ctx_with(dir.path(), Some(cfg()));
        let out = la
            .run(
                json!({"action": "measure", "capture": capture_id(dir.path()), "channel": 0, "what": "edges"}),
                &ctx,
            )
            .await
            .unwrap()
            .text;
        assert!(out.contains("edges (both): 3"), "got: {out}");
        assert!(
            out.contains("[evidence: physical — logic capture]"),
            "got: {out}"
        );
    }

    fn capture_id(dir: &Path) -> String {
        let mut names: Vec<String> = std::fs::read_dir(dir.join(".firment/la"))
            .unwrap()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.strip_suffix(".meta.json").map(|s| s.to_string())
            })
            .collect();
        names.sort();
        names.pop().expect("one capture")
    }

    #[tokio::test]
    async fn capture_busy_device_gets_replug_hint() {
        let dir = tempdir().unwrap();
        let la = La::with_backend(Arc::new(FakeBackend {
            bin_bytes: vec![],
            decode_out: String::new(),
            fail_capture: true,
        }));
        let err = la
            .run(
                json!({"action": "capture", "samples": 4}),
                &ctx_with(dir.path(), Some(cfg())),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("replug"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn capture_respects_max_samples() {
        let dir = tempdir().unwrap();
        let mut c = cfg();
        c.max_samples = 10;
        let err = fake_la(vec![])
            .run(
                json!({"action": "capture", "samples": 11}),
                &ctx_with(dir.path(), Some(c)),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("max_samples"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn capture_without_driver_or_channels_is_actionable() {
        let dir = tempdir().unwrap();
        let err = fake_la(vec![])
            .run(
                json!({"action": "capture", "samples": 4}),
                &ctx_with(dir.path(), None),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("driver"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn decode_returns_annotations_with_evidence() {
        let dir = tempdir().unwrap();
        let la = fake_la(BITS.to_vec());
        la.run(
            json!({"action": "capture", "samples": 4}),
            &ctx_with(dir.path(), Some(cfg())),
        )
        .await
        .unwrap();
        let ctx = ctx_with(dir.path(), Some(cfg()));
        let out = la
            .run(
                json!({"action": "decode", "capture": capture_id(dir.path()), "decoder": "uart",
                       "decoder_opts": {"rx": "0", "baudrate": 115200}}),
                &ctx,
            )
            .await
            .unwrap()
            .text;
        assert!(out.contains("uart: TX: (0x55)"), "got: {out}");
        assert!(
            out.contains("[evidence: physical — logic capture]"),
            "got: {out}"
        );
    }

    #[tokio::test]
    async fn list_captures_shows_stored_runs() {
        let dir = tempdir().unwrap();
        let la = fake_la(BITS.to_vec());
        let empty_ctx = ctx_with(dir.path(), Some(cfg()));
        assert!(
            la.run(json!({"action": "list_captures"}), &empty_ctx)
                .await
                .unwrap()
                .text
                .contains("no captures yet")
        );
        la.run(
            json!({"action": "capture", "samples": 4}),
            &ctx_with(dir.path(), Some(cfg())),
        )
        .await
        .unwrap();
        let out = la
            .run(json!({"action": "list_captures"}), &empty_ctx)
            .await
            .unwrap()
            .text;
        assert!(out.contains("fx2lafw"), "got: {out}");
    }

    #[tokio::test]
    async fn capture_id_traversal_is_refused() {
        // ".." has no separator and no ".sr" — without the charset rule it
        // would join out of .firment/la/ and read a meta file elsewhere.
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".firment")).unwrap();
        std::fs::write(
            dir.path().join(".firment").join("meta.json"),
            r#"{"id":"x","driver":"d","channels":"0","channel_count":1,"has_binary":true}"#,
        )
        .unwrap();
        let err = La::default()
            .run(
                json!({"action": "measure", "capture": ".."}),
                &ctx_with(dir.path(), Some(cfg())),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("[InvalidInput]"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn only_capture_asks_approval() {
        let la = La::default();
        assert!(
            la.approval(&json!({"action": "capture", "samples": 4}))
                .is_some()
        );
        assert!(la.approval(&json!({"action": "detect"})).is_none());
        assert!(
            la.approval(&json!({"action": "measure", "capture": "x"}))
                .is_none()
        );
        assert!(
            la.approval(&json!({"action": "decode", "capture": "x", "decoder": "uart"}))
                .is_none()
        );
    }

    #[tokio::test]
    async fn measure_refuses_truncated_or_missing_sidecar() {
        let dir = tempdir().unwrap();
        let ctx = ctx_with(dir.path(), Some(cfg()));
        // A meta claiming 10 samples with a 4-byte sidecar: half-written
        // export must NOT be measured as if complete.
        let la_dir = dir.path().join(".firment/la");
        std::fs::create_dir_all(&la_dir).unwrap();
        std::fs::write(la_dir.join("t1.bin"), BITS).unwrap();
        std::fs::write(la_dir.join("t1.sr"), b"stub").unwrap();
        std::fs::write(
            la_dir.join("t1.meta.json"),
            json!({"id":"t1","driver":"demo","channels":"0,1","channel_count":2,
                   "samplerate":"8m","samplerate_hz":8000000.0,"samples":10,
                   "time_ms":null,"created_unix":0,"has_binary":true})
            .to_string(),
        )
        .unwrap();
        let err = La::default()
            .run(
                json!({"action": "measure", "capture": "t1", "what": "edges"}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("truncated"), "got: {}", err.message);

        // has_binary = false → measure refuses even if a stale .bin exists.
        std::fs::write(
            la_dir.join("t2.meta.json"),
            json!({"id":"t2","driver":"demo","channels":"0,1","channel_count":2,
                   "samplerate":"8m","samplerate_hz":8000000.0,"samples":4,
                   "time_ms":null,"created_unix":0,"has_binary":false})
            .to_string(),
        )
        .unwrap();
        std::fs::write(la_dir.join("t2.bin"), BITS).unwrap();
        std::fs::write(la_dir.join("t2.sr"), b"stub").unwrap();
        let err = La::default()
            .run(
                json!({"action": "measure", "capture": "t2", "what": "edges"}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("no raw-bit sidecar"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn registered_in_all_and_absent_from_plan_registry() {
        assert!(crate::tools::all().iter().any(|t| t.name() == "la"));
        assert!(crate::plan_registry().get("la").is_none());
    }
}
