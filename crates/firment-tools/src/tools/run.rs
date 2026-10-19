use super::util::{probe_rs_err, resolve_within, run_streaming, shell_quote, token_arg};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::path::Path;

pub struct Run;

/// Build the probe-rs run command line (exposed for tests). Arguments are
/// shell-quoted so hostile values cannot break out of the command line.
pub fn run_command_line(chip: &str, file: &str, probe: Option<&str>) -> String {
    let mut cmd = format!("probe-rs run --chip {}", shell_quote(chip));
    if let Some(probe) = probe {
        cmd.push_str(&format!(" --probe {}", shell_quote(probe)));
    }
    cmd.push_str(&format!(" {}", shell_quote(file)));
    cmd
}

/// Executes `probe-rs run` with an argument array (no shell).
///
/// Like flash, running through `cmd /C <string>` would leak the quotes of
/// `--chip "STM32G431RB"` to probe-rs, making chip lookup fail. `probe-rs run`
/// is a long-lived process that streams RTT logs; we capture output until the
/// timeout elapses and then kill the tree, returning whatever was captured.
/// A turn cancellation (Esc) also kills the child: an orphaned probe-rs keeps
/// the debug probe locked and blocks every later flash/debug until it exits on
/// its own — with timeout_ms = 0, forever. Returns (text, exit code, why it stopped).
async fn run_probe_rs_run(
    chip: &str,
    file: &Path,
    probe: Option<&str>,
    cwd: &Path,
    timeout_ms: u64,
    cancel: &firment_core::Cancellable,
) -> Result<(String, Option<i32>, super::util::End), String> {
    let mut args: Vec<std::ffi::OsString> = vec!["run".into(), "--chip".into(), chip.into()];
    if let Some(probe) = probe {
        args.extend(["--probe".into(), probe.into()]);
    }
    args.push(file.as_os_str().to_os_string());
    run_streaming("probe-rs", &args, cwd, timeout_ms, cancel).await
}

#[async_trait]
impl Tool for Run {
    fn name(&self) -> &'static str {
        "run"
    }

    fn description(&self) -> &'static str {
        "Flash and run the firmware on the target, streaming RTT logs (semihosting through the CLI fallback). Use a bounded timeout so you can observe the output. Firmware that prints over a UART is not visible here — use `monitor` for that."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file": {"type": "string", "description": "Path to the firmware ELF (must be inside the workspace)"},
                "chip": {"type": "string", "description": "probe-rs chip id, e.g. stm32f407vetx"},
                "probe": {"type": "string", "description": "Optional probe serial/id"},
                "timeout_ms": {"type": "integer", "minimum": 0, "default": 30000, "description": "0 = wait forever"}
            },
            "required": ["file"]
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        let file = args.get("file").and_then(|f| f.as_str()).unwrap_or("?");
        Some(format!("⚠ flash and run target: {file}"))
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let file = args
            .get("file")
            .and_then(|f| f.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing 'file'"))?;
        let resolved =
            resolve_within(&ctx.cwd, file, &ctx.allowed_roots).map_err(ToolError::new)?;
        let chip = args
            .get("chip")
            .and_then(|c| c.as_str())
            .map(|s| token_arg(s, "chip"))
            .transpose()
            .map_err(ToolError::new)?
            .or_else(|| ctx.default_chip.clone())
            .ok_or_else(|| {
                ToolError::new(
                    "[InvalidInput] missing chip: pass a chip parameter or set default_chip in \
                     [tools] of config.toml",
                )
            })?;
        let probe = args
            .get("probe")
            .and_then(|p| p.as_str())
            .map(|s| token_arg(s, "probe"))
            .transpose()
            .map_err(ToolError::new)?;
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|t| t.as_u64())
            .unwrap_or(30_000);

        // 先走库：烧录 + RTT，同一个会话。没有 CLI 也能跑成这一条路径。
        // RTT 起不来（固件用 semihosting、或没初始化控制块）时回退 `probe-rs run`，
        // 那条路径还负责 semihosting 与它自己的退出码语义。
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watcher = {
            let s = stop.clone();
            let c = ctx.cancel.clone();
            tokio::spawn(async move {
                c.cancelled().await;
                s.store(true, std::sync::atomic::Ordering::Relaxed);
            })
        };
        // 三种结局，而不是"成功/失败"两种：
        //   Done   —— 烧好了，也收到了 RTT 字节；
        //   NoRtt  —— 烧好了，但这个固件没有 RTT 控制块。**不能再烧一遍**：
        //             固件已经在跑，回退 `probe-rs run` 会重新烧录，而它要做的
        //             只是 attach（semihosting 也走 attach）；
        //   Failed —— 库没烧成，交给 CLI 那条完整路径。
        enum LibRun {
            Done {
                segs: usize,
                bytes: usize,
                out: Vec<u8>,
            },
            NoRtt {
                segs: usize,
                why: String,
            },
            Failed,
        }

        let lib_run = {
            let chip_lib = chip.clone();
            let probe_lib = probe.clone();
            let file_lib = resolved.clone();
            let stop_lib = stop.clone();
            let dur = std::time::Duration::from_millis(timeout_ms);
            tokio::task::spawn_blocking(move || {
                let segs = match crate::probe::flash_elf(&chip_lib, probe_lib.as_deref(), &file_lib)
                {
                    Ok(n) => n,
                    Err(_) => return LibRun::Failed,
                };
                let mut out: Vec<u8> = Vec::new();
                match crate::probe::rtt_stream(
                    &chip_lib,
                    probe_lib.as_deref(),
                    dur,
                    &stop_lib,
                    |chunk| out.extend_from_slice(chunk),
                ) {
                    Ok(bytes) => LibRun::Done { segs, bytes, out },
                    Err(e) => LibRun::NoRtt {
                        segs,
                        why: e.message().to_string(),
                    },
                }
            })
            .await
            .unwrap_or(LibRun::Failed)
        };
        watcher.abort();
        match lib_run {
            LibRun::Done { segs, bytes, out } => {
                if ctx.cancel.is_cancelled() {
                    return Err(ToolError::new(format!(
                        "[Cancelled] run interrupted by turn cancellation after {bytes} RTT bytes; \
                         captured output://n{}",
                        String::from_utf8_lossy(&out)
                    )));
                }
                let text = String::from_utf8_lossy(&out).into_owned();
                return Ok(ToolOutput {
                    text: format!(
                        "run timed out after {timeout_ms} ms; captured output [probe-rs lib]\n\
                         (flashed {segs} loadable segment(s), {bytes} RTT bytes)\n{}",
                        crate::forensic::append_fault_marker(text)
                    ),
                });
            }
            LibRun::NoRtt { segs, why } => {
                // probe-rs 的这句话后面跟着四行"怎么让 RTT 工作"的建议；对一次
                // 明确的 "this firmware has no RTT" 来说那是噪音，只取第一行。
                let why = why.lines().next().unwrap_or(&why).to_string();
                // 已经烧好了。这里只 attach，**不再烧** —— 一次 `run` 只该烧一次。
                // 先放开我们自己的句柄，否则 CLI 打不开探针（实测 error 5）。
                crate::probe::release();
                let mut args = vec!["attach".to_string(), "--chip".to_string(), chip.clone()];
                if let Some(p) = probe.as_ref() {
                    args.push("--probe".to_string());
                    args.push(p.clone());
                }
                let hint = "hint: `run` only carries RTT (and semihosting via the CLI). \
                            Firmware that logs over a UART — most of this project's — shows \
                            nothing here; use `monitor` on that port instead.";
                let streamed = match super::util::run_probe_rs(
                    args,
                    &ctx.cwd,
                    timeout_ms,
                    Some(ctx.cancel.clone()),
                    &[],
                )
                .await
                {
                    Ok((text, _)) => text,
                    // attach 本来就会一直跑；到这个工具的窗口结束是正常收尾，不是失败。
                    // 超时错误里带着已经捕获的输出（），保留它。
                    Err(e) if e.starts_with("[Timeout]") => {
                        format!("attached for {timeout_ms} ms, stream ended by the window\n{e}")
                    }
                    Err(e) => format!("(could not stream with the CLI: {e})"),
                };
                return Ok(ToolOutput {
                    text: format!(
                        "flashed {segs} loadable segment(s) [probe-rs lib], then no RTT control \
                         block ({why})\n{streamed}\n\n{hint}"
                    ),
                });
            }
            LibRun::Failed => {
                // 下面若走 CLI 的 `probe-rs run`，同样要先把句柄放掉。
                crate::probe::release();
            }
        }

        let probe_rs_ok = super::util::probe_rs_present().await;
        if !probe_rs_ok {
            return Err(ToolError::new(
                "[NotFound] probe-rs is not installed or not on PATH: install it with \
                 `cargo install probe-rs-tools` or download from the probe-rs GitHub Releases",
            ));
        }

        let command = run_command_line(&chip, &resolved.to_string_lossy(), probe.as_deref());
        match run_probe_rs_run(
            &chip,
            &resolved,
            probe.as_deref(),
            &ctx.cwd,
            timeout_ms,
            &ctx.cancel,
        )
        .await
        {
            Ok((text, Some(0), _)) => Ok(ToolOutput {
                text: format!(
                    "run finished (exit 0)\n{}",
                    crate::forensic::append_fault_marker(text)
                ),
            }),
            Ok((text, Some(code), _)) => Err(ToolError::new(format!(
                "[Io] run failed (exit {code})\ncommand: {command}\n{text}"
            ))),
            Ok((text, _, super::util::End::Cancelled)) => Err(ToolError::new(format!(
                "[Cancelled] run interrupted by turn cancellation (probe-rs killed so it does \
                 not hold the debug probe); captured output:\n{text}"
            ))),
            // A bounded `run` is the normal case for this tool, so a deadline expiring is a
            // result rather than an error — but only when a deadline was set. A nil exit code
            // from a signal used to fall in here too and print "timed out after N ms" for a
            // timeout that never fired.
            Ok((text, _, super::util::End::TimedOut)) if timeout_ms > 0 => Ok(ToolOutput {
                text: format!(
                    "run timed out after {timeout_ms} ms; captured output:\n{}",
                    crate::forensic::append_fault_marker(text)
                ),
            }),
            Ok((text, _, super::util::End::TimedOut)) => Err(ToolError::new(format!(
                "[Io] run stopped with no exit code, and no timeout was set\ncommand: \
                 {command}\n{text}"
            ))),
            Ok((text, _, super::util::End::Killed)) => Err(ToolError::new(format!(
                "[Io] run's process was killed by a signal, not by this tool's timeout\ncommand: \
                 {command}\n{text}"
            ))),
            Ok((text, _, super::util::End::Exited)) => Err(ToolError::new(format!(
                "[Io] run ended without an exit code\ncommand: {command}\n{text}"
            ))),
            Err(e) => Err(probe_rs_err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal};
    use serde_json::json;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    /// 端到端：真板 + 会打印 RTT 的夹具固件（`fixtures/rtt-probe-fw`）。
    ///
    /// ```text
    /// FIRMENT_TEST_RTT_ELF=fixtures/rtt-probe-fw/target/thumbv7em-none-eabihf/release/rtt-probe-fw \
    ///   cargo test -p firment-tools --lib streams_rtt_through_the_tool -- --ignored
    /// ```
    ///
    /// 它把三段拼起来验：库烧录 → RTT attach → 收到的字节里有递增的序号。
    /// 夹具在这里是固件，不是 CH340 —— 这条链只碰 ST-Link。
    #[tokio::test]
    #[ignore = "needs a NUCLEO-G431RB and the RTT fixture firmware"]
    async fn streams_rtt_through_the_tool() {
        let elf = std::env::var("FIRMENT_TEST_RTT_ELF").expect("set FIRMENT_TEST_RTT_ELF");
        let dir = tempdir().unwrap();
        std::fs::copy(&elf, dir.path().join("rtt.elf")).unwrap();
        let mut c = ctx(dir.path());
        c.default_chip = Some("stm32g431rb".to_string());

        let out = Run
            .run(
                json!({"file": "rtt.elf", "chip": "stm32g431rb", "timeout_ms": 4000}),
                &c,
            )
            .await
            .expect("run through the tool");
        assert!(out.text.contains("[probe-rs lib]"), "{}", out.text);
        assert!(
            out.text.contains("firment-rtt-smoke"),
            "expected RTT content, got: {}",
            out.text
        );
    }

    /// 没有 RTT 的固件：**只烧一次**，然后 attach，并把"日志可能走 UART"说清楚。
    ///
    /// ```text
    /// FIRMENT_TEST_ELF=<一份不初始化 RTT 的 ELF> \
    ///   cargo test -p firment-tools --lib reports_no_rtt_instead_of_reflashing -- --ignored
    /// ```
    #[tokio::test]
    #[ignore = "needs a NUCLEO-G431RB and a firmware ELF without RTT"]
    async fn reports_no_rtt_instead_of_reflashing() {
        let elf = std::env::var("FIRMENT_TEST_ELF").expect("set FIRMENT_TEST_ELF");
        let dir = tempdir().unwrap();
        std::fs::copy(&elf, dir.path().join("no-rtt.elf")).unwrap();
        let mut c = ctx(dir.path());
        c.default_chip = Some("stm32g431rb".to_string());

        let out = Run
            .run(
                json!({"file": "no-rtt.elf", "chip": "stm32g431rb", "timeout_ms": 2000}),
                &c,
            )
            .await
            .expect("run through the tool");
        // 真机验证时用 --nocapture 看这段原文：断言只证明包含什么，
        // 而人要看的是整段输出读起来像不像话。
        println!(
            "--- run output ---
{}
--- end ---",
            out.text
        );
        assert!(
            out.text.contains("no RTT control block"),
            "expected the NoRtt branch, got: {}",
            out.text
        );
        // 关键：不能再走 `probe-rs run`（那会重烧一遍）。
        assert!(
            !out.text.contains("probe-rs run"),
            "the fallback must attach, not re-flash: {}",
            out.text
        );
        assert!(
            out.text.contains("monitor"),
            "the hint should point at monitor: {}",
            out.text
        );
    }

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
            monitor_port: None,
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        }
    }

    #[test]
    fn run_command_line_builds_probe_rs_invocation() {
        let cmd = run_command_line("stm32f407vetx", "target/out.elf", None);
        assert!(cmd.contains("probe-rs run --chip"));
        assert!(cmd.contains("stm32f407vetx"));
        assert!(cmd.contains("target/out.elf"));
        assert!(cmd.contains(&shell_quote("stm32f407vetx")));
    }

    #[test]
    fn run_command_line_quotes_hostile_values() {
        let cmd = run_command_line("x & whoami", "a'$(rm -rf /).elf", Some("p`p"));
        assert!(
            cmd.contains(&shell_quote("x & whoami")),
            "chip must stay a single token: {cmd}"
        );
        assert!(
            cmd.contains(&shell_quote("a'$(rm -rf /).elf")),
            "file must be shell-safe: {cmd}"
        );
        assert!(
            cmd.contains(&shell_quote("p`p")),
            "probe must stay a single token: {cmd}"
        );
    }

    #[tokio::test]
    async fn file_outside_workspace_is_rejected() {
        let dir = tempdir().unwrap();
        let outside = dir.path().parent().unwrap().join("evil.elf");
        std::fs::write(&outside, b"x").unwrap();
        let err = Run
            .run(json!({"file": outside.to_string_lossy()}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("outside the workspace"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn missing_probe_rs_gives_install_hint() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("out.elf"), b"x").unwrap();
        if std::process::Command::new("probe-rs")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return;
        }
        let err = Run
            .run(json!({"file": "out.elf"}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("probe-rs is not installed"),
            "got: {}",
            err.message
        );
    }
}
