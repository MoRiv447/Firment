use firment_core::{Cancellable, ToolError};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{ChildStderr, ChildStdout, Command};

pub(crate) fn resolve(cwd: &Path, path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() { p } else { cwd.join(p) }
}

/// Resolve `path` and enforce the workspace boundary: the canonical target
/// must live under `cwd` or one of `extra_roots` (e.g. the session spill dir).
/// Returns the un-canonicalized resolved path on success (for display/use).
pub(crate) fn resolve_within(
    cwd: &Path,
    path: &str,
    extra_roots: &[PathBuf],
) -> Result<PathBuf, String> {
    let target = resolve(cwd, path);
    let cwd_canon = fs::canonicalize(cwd)
        .map_err(|e| format!("cannot canonicalize cwd {}: {e}", cwd.display()))?;
    let target_canon = canonicalize_for_check(&target)
        .map_err(|e| format!("cannot resolve {}: {e}", target.display()))?;
    let inside = target_canon.starts_with(&cwd_canon)
        || extra_roots.iter().any(|root| {
            fs::canonicalize(root)
                .map(|root_canon| target_canon.starts_with(&root_canon))
                .unwrap_or(false)
        });
    if !inside {
        return Err(format!(
            "[Permission] path is outside the workspace: {} (workspace: {})",
            target.display(),
            cwd_canon.display()
        ));
    }
    Ok(target)
}

/// Canonicalize a path for boundary checking. An existing path is canonicalized
/// directly; for a not-yet-existing path (e.g. a file to be created), the
/// deepest existing ancestor is canonicalized and the missing suffix is
/// re-attached, so files inside not-yet-created directories still resolve.
fn canonicalize_for_check(path: &Path) -> std::io::Result<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path);
    }
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut probe = path;
    loop {
        if probe.exists() {
            let canon = fs::canonicalize(probe)?;
            return Ok(missing.iter().rev().fold(canon, |acc, seg| acc.join(seg)));
        }
        let Some(name) = probe.file_name() else {
            return fs::canonicalize(path);
        };
        missing.push(name.to_os_string());
        let Some(parent) = probe.parent() else {
            return fs::canonicalize(path);
        };
        if parent.as_os_str().is_empty() {
            return fs::canonicalize(path);
        }
        probe = parent;
    }
}

/// Upper bound for a single text file the tools read into memory. `grep` and
/// `symbols` call [`read_text`] for EVERY file they walk, so one 4 GB log or a
/// build artifact on the path would otherwise be slurped whole.
pub(crate) const MAX_TEXT_FILE_BYTES: u64 = 32 * 1024 * 1024;

pub(crate) fn read_text(path: &Path) -> Result<String, String> {
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size > MAX_TEXT_FILE_BYTES {
        return Err(format!(
            "{} is {} bytes, over the {}-byte text cap; narrow the search path or read \
             it with offset/limit instead of whole",
            path.display(),
            size,
            MAX_TEXT_FILE_BYTES
        ));
    }
    let bytes = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if bytes.contains(&0) {
        return Err(format!(
            "{} is a binary file ({} bytes); text tools refuse to read it",
            path.display(),
            bytes.len()
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub(crate) fn rel_str(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// 8-hex prefix of the SHA-256 of a normalized line (no trailing CR/LF).
/// `edit_file` hashline anchors use the same prefix (matched with starts_with).
pub(crate) fn line_hash_prefix(line: &str) -> String {
    firment_core::hash::sha256_hex(line.as_bytes())
        .chars()
        .take(8)
        .collect()
}

/// Truncate long text to `max_chars`, keeping the HEAD and the TAIL of the
/// input: errors (compiler output, tool logs) usually appear at the end, and
/// dropping the tail would hide the actual failure.
pub(crate) fn truncate(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    if max_chars < 16 {
        return chars
            .into_iter()
            .take(max_chars)
            .chain(std::iter::once('…'))
            .collect();
    }
    // Keep the head (context) and tail (errors), mark the middle as dropped.
    let head = max_chars * 2 / 3;
    let tail = max_chars - head - 1; // 1 for the ellipsis
    let mut out: Vec<char> = chars[..head].to_vec();
    out.extend(format!("…[{} chars dropped]…", chars.len() - head - tail).chars());
    let tail_start = chars.len() - tail;
    out.extend(chars[tail_start..].iter());
    out.into_iter().collect()
}

/// Validate a value that will be spliced unquoted into a shell command line
/// (chip id, probe serial). Only plain token characters are allowed so the
/// value can never break out of the shell or inject extra commands.
pub(crate) fn token_arg(value: &str, what: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err(format!("[InvalidInput] {what} must not be empty"));
    }
    // A leading dash would make the value look like a CLI flag to probe-rs
    // (argv-array exec — no shell injection, but argument confusion).
    if value.starts_with('-') {
        return Err(format!("[InvalidInput] {what} must not start with '-'"));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/'))
    {
        return Err(format!(
            "[InvalidInput] {what} contains characters that are not allowed in a command \
             token; use only letters, digits, and `- _ . : /`"
        ));
    }
    Ok(value.to_string())
}

/// Quote a value for interpolation into a shell command STRING that is only
/// ever DISPLAYED (error text, previews, `run_command_line`). The actual
/// probe-rs invocations use argv arrays and must never go through this.
///
/// Plain double quotes, no `%`/`^` doubling: the doubling is BATCH-file
/// syntax that would corrupt the displayed value on a `cmd /C` command
/// line (`rev^2_100%` showed up as `rev^^2_100%%` in flash errors), and
/// since the string is never executed, no shell metacharacter needs
/// neutralising. A `"` cannot occur in a Windows file name.
pub(crate) fn shell_quote(arg: &str) -> String {
    if cfg!(windows) {
        format!("\"{arg}\"")
    } else {
        // sh: single quotes; embedded single quotes are spliced out.
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// Minimal unified diff for permission previews: trims common prefix/suffix
/// lines into a single hunk, capped at `max_chars`.
pub(crate) fn simple_diff(path: &Path, old: &str, new: &str, max_chars: usize) -> String {
    let mut out = format!("--- {}\n+++ {}\n", path.display(), path.display());
    out.push_str(&firment_core::journal::line_diff(old, new, max_chars));
    out
}

/// Reads a child's stdout and stderr CONCURRENTLY into buffers the caller can
/// inspect at any time.
///
/// Both properties are load-bearing. Reading the pipes one after the other
/// deadlocks: a child blocked on a full stderr pipe never closes stdout, so
/// the stdout read never reaches EOF. Keeping the buffers shared instead of
/// moving them into the reader task means a caller that gives up on a deadline
/// still holds what was captured — a timed-out build's compiler errors are
/// precisely the output the agent needs.
pub(crate) struct PipeDrain {
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    task: tokio::task::JoinHandle<()>,
}

impl PipeDrain {
    pub(crate) fn start(stdout: ChildStdout, stderr: ChildStderr) -> Self {
        let out: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let err: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let task = {
            let (out_handle, err_handle) = (out.clone(), err.clone());
            tokio::spawn(async move {
                tokio::join!(read_into(stdout, out_handle), read_into(stderr, err_handle));
            })
        };
        Self {
            stdout: out,
            stderr: err,
            task,
        }
    }

    /// Wait up to `grace` for both pipes to reach EOF, then return the bytes
    /// captured so far and whether the deadline cut collection short.
    pub(crate) async fn finish(self, grace: Duration) -> (Vec<u8>, Vec<u8>, bool) {
        let mut task = self.task;
        let timed_out = !matches!(tokio::time::timeout(grace, &mut task).await, Ok(Ok(())));
        if timed_out {
            // Nothing left to wait for: the caller has the bytes read so far.
            task.abort();
        }
        (
            drain_buffer(&self.stdout),
            drain_buffer(&self.stderr),
            timed_out,
        )
    }
}

async fn read_into<R: AsyncRead + Unpin>(mut reader: R, buf: Arc<Mutex<Vec<u8>>>) {
    let mut chunk = [0u8; 16 * 1024];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                // Lock only across the copy, never across an await.
                lock_sharing(&buf).extend_from_slice(&chunk[..n]);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

fn drain_buffer(buf: &Mutex<Vec<u8>>) -> Vec<u8> {
    std::mem::take(&mut *lock_sharing(buf))
}

fn lock_sharing(buf: &Mutex<Vec<u8>>) -> std::sync::MutexGuard<'_, Vec<u8>> {
    buf.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Run a command through the platform shell, capture output, enforce a
/// timeout. `cancel` (the turn-level cancellation signal) stops the process
/// tree promptly when the turn is interrupted. Returns (formatted text, exit
/// code); exit code `None` means the process was killed (timeout or cancel).
pub(crate) async fn run_command(
    command: &str,
    cwd: &Path,
    timeout_ms: u64,
    env: Option<&HashMap<String, String>>,
    cancel: Option<&Cancellable>,
) -> Result<(String, Option<i32>), String> {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        // Put the whole tree in its own process group so the timeout can
        // kill backgrounded grandchildren too, not just the shell.
        // (tokio::process::Command provides process_group; the std import
        // would be unused and fails `cargo clippy -D warnings` on Linux.)
        cmd.process_group(0);
    }
    cmd.current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(env) = env {
        for (k, v) in env {
            cmd.env(k, v);
        }
    }

    let mut child = cmd
        .kill_on_drop(true) // drop-safety net: see comment above the drain task
        .spawn()
        .map_err(|e| format!("spawn failed: {e}"))?;
    // If the surrounding future is dropped by an outer wave cancellation
    // before our own cancel branch can run, the direct child is still
    // terminated instead of orphaned (tokio does NOT kill on drop by
    // default). The tree-kill paths below remain the thorough mechanism.

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout handle unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "stderr handle unavailable".to_string())?;

    // Drive the drains CONCURRENTLY with wait(): a child emitting more than
    // the OS pipe buffer (~64 KB per pipe) blocks on write and would never
    // exit, turning long builds into spurious timeouts with all output lost.
    let drain = PipeDrain::start(stdout, stderr);

    let cancel_fut = cancel.map(|c| Box::pin(c.cancelled()));

    // Kill/timeout/cancel outcomes are RECORDED, not returned immediately:
    // the buffered partial output is collected below and appended. Discarding
    // what an already-killed process had printed (e.g. a timed-out build's
    // compiler errors) was half of the original pipe bug.
    let mut interrupted: Option<String> = None;
    let mut exit: Option<std::io::Result<std::process::ExitStatus>> = None;
    if timeout_ms == 0 {
        match cancel_fut {
            Some(cancel_fut) => {
                tokio::select! {
                    st = child.wait() => exit = Some(st),
                    _ = cancel_fut => {
                        let (text, _) = kill_tree_and_report(
                            &mut child,
                            command,
                            "cancelled: interrupted before the command finished (process tree \
                             terminated)".to_string(),
                        )
                        .await;
                        interrupted = Some(text);
                    }
                }
            }
            None => {
                exit = Some(child.wait().await);
            }
        }
    } else {
        tokio::select! {
            st = child.wait() => exit = Some(st),
            _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
                let (text, _) = kill_tree_and_report(
                    &mut child,
                    command,
                    format!(
                        "timed out after {timeout_ms} ms and was killed (process tree terminated)"
                    ),
                )
                .await;
                interrupted = Some(text);
            }
            _ = async {
                if let Some(c) = cancel.as_ref() {
                    Box::pin(c.cancelled()).await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {
                let (text, _) = kill_tree_and_report(
                    &mut child,
                    command,
                    "cancelled: interrupted before the command finished (process tree \
                     terminated)".to_string(),
                )
                .await;
                interrupted = Some(text);
            }
        }
    };
    // Collect the output with a firm deadline: after the process exited, a
    // background grandchild holding the pipe write-ends would otherwise block
    // forever. If the deadline fires we keep what was captured and mark
    // truncation instead of hanging.
    let (out_buf, err_buf, drain_timed_out) = drain.finish(Duration::from_secs(15)).await;
    let drain_note = if drain_timed_out {
        "\n[output truncated: the process finished but output was still streaming after 15s]"
    } else {
        ""
    };

    if let Some(reason) = interrupted {
        let stdout = truncate(&String::from_utf8_lossy(&out_buf), 32_000);
        let stderr = truncate(&String::from_utf8_lossy(&err_buf), 32_000);
        return Ok((
            format!(
                "{reason}\n--- partial stdout ---\n{stdout}\n--- partial stderr ---\n{stderr}{drain_note}"
            ),
            None,
        ));
    }

    let status = exit
        .expect("neither exited nor interrupted")
        .map_err(|e| format!("wait failed: {e}"))?;
    let stdout = truncate(&String::from_utf8_lossy(&out_buf), 32_000);
    let stderr = truncate(&String::from_utf8_lossy(&err_buf), 32_000);
    let code = status.code();
    let status_text = code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "signal".to_string());
    Ok((
        format!(
            "command: {command}\nexit code: {status_text}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}{drain_note}"
        ),
        code,
    ))
}

/// Kill the direct child plus its whole tree (timeout or cancellation) and
/// report the interruption. `exit code: None` tells the caller the process did
/// not finish on its own.
async fn kill_tree_and_report(
    child: &mut tokio::process::Child,
    command: &str,
    reason: String,
) -> (String, Option<i32>) {
    if let Some(pid) = child.id() {
        kill_process_tree(pid);
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
    (format!("command: {command}\n{reason}"), None)
}

/// Run a `probe-rs` subcommand directly with an explicit argument array (no
/// shell), capturing combined stdout+stderr. Enforces a timeout; a process
/// killed by the timeout or by turn cancellation reports exit code `None`.
///
/// Running through `cmd /C <string>` is NOT used here: cmd.exe keeps the
/// quotes inside quoted arguments when the whole command is passed as one
/// string (e.g. `--chip "STM32G431RB"` arrives at probe-rs as `"STM32G431RB"`
/// including the quotes), which makes chip lookup fail with "chip not found".
/// Spawning with explicit args avoids quoting entirely and is equally safe.
pub(crate) async fn run_probe_rs(
    args: Vec<String>,
    cwd: &Path,
    timeout_ms: u64,
    cancel: Option<Cancellable>,
    envs: &[(String, String)],
) -> Result<(String, Option<i32>), String> {
    run_argv("probe-rs", args, cwd, timeout_ms, cancel, envs).await
}

/// Generalization of [`run_probe_rs`] to any external CLI invoked with an
/// argv array and no shell — probe-rs today, sigrok-cli for the `la` tool.
/// The program name appears in timeout/cancel messages so the user can tell
/// which binary was killed.
pub(crate) async fn run_argv(
    program: &str,
    args: Vec<String>,
    cwd: &Path,
    timeout_ms: u64,
    cancel: Option<Cancellable>,
    envs: &[(String, String)],
) -> Result<(String, Option<i32>), String> {
    let mut cmd = Command::new(program);
    cmd.args(&args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        cmd.env(key, value);
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .kill_on_drop(true) // same drop-safety net as run_command
        .spawn()
        .map_err(|e| format!("spawn failed: {e}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout handle unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "stderr handle unavailable".to_string())?;

    // Drain CONCURRENTLY with wait(): probe-rs emits progress/log lines the
    // whole run; without a concurrent reader, output beyond the OS pipe
    // buffer blocks the child and turns real runs into spurious timeouts.
    let drain = PipeDrain::start(stdout, stderr);

    let status = tokio::select! {
        status = child.wait() => status,
        _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
            // Kill then wait: without wait() the dead child lingers as a
            // zombie on Unix until this process exits.
            let _ = child.kill().await;
            let _ = child.wait().await;
            // Whatever the program printed before it was killed IS the
            // diagnostic; a bare timeout message leaves the agent guessing.
            let (out_buf, err_buf, _) = drain.finish(Duration::ZERO).await;
            return Err(format!(
                "[Timeout] {program} timed out after {timeout_ms} ms and was killed{}",
                captured_note(&out_buf, &err_buf)
            ));
        }
        _ = async {
            if let Some(c) = cancel.as_ref() {
                Box::pin(c.cancelled()).await;
            } else {
                std::future::pending::<()>().await;
            }
        } => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let (out_buf, err_buf, _) = drain.finish(Duration::ZERO).await;
            return Err(format!(
                "[Cancelled] {program} was interrupted by turn cancellation{}",
                captured_note(&out_buf, &err_buf)
            ));
        }
    };
    // Post-exit collection with a firm deadline: a grandchild holding the
    // pipe write-ends would otherwise hang here forever.
    let (out_buf, err_buf, drain_timed_out) = drain.finish(Duration::from_secs(15)).await;
    let code = status.map_err(|e| format!("wait failed: {e}"))?.code();

    let stdout = String::from_utf8_lossy(&out_buf).to_string();
    let stderr = String::from_utf8_lossy(&err_buf).to_string();
    let mut text = format!("{stdout}{stderr}");
    if drain_timed_out {
        text.push_str(
            "\n[output truncated: the process finished but output was still streaming after 15s]",
        );
    }
    Ok((text, code))
}

/// Append whatever a killed child had already printed to an error message.
fn captured_note(stdout: &[u8], stderr: &[u8]) -> String {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    );
    if text.trim().is_empty() {
        return String::new();
    }
    format!("; captured output:\n{}", truncate(&text, 4_000))
}

/// Run a long-lived program with an argv array, streaming its output until it
/// exits, `timeout_ms` elapses (0 = wait forever), or the turn is cancelled.
/// Returns (captured output, exit code, cancelled); exit code `None` means the
/// process was killed.
///
/// Split out of the `run` tool so the drain/wait race is testable with any
/// program — probe-rs itself is not installed on CI runners.
pub(crate) async fn run_streaming(
    program: &str,
    args: &[std::ffi::OsString],
    cwd: &Path,
    timeout_ms: u64,
    cancel: &Cancellable,
) -> Result<(String, Option<i32>, bool), String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    // kill_on_drop is the drop-safety net for outer wave cancellation; the
    // select branches below cover the awaited paths explicitly.
    cmd.kill_on_drop(true)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout handle unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "stderr handle unavailable".to_string())?;
    let drain = PipeDrain::start(stdout, stderr);

    enum Outcome {
        Status(std::io::Result<std::process::ExitStatus>),
        TimedOut,
        Cancelled,
    }
    let outcome = if timeout_ms == 0 {
        tokio::select! {
            status = child.wait() => Outcome::Status(status),
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                Outcome::Cancelled
            }
        }
    } else {
        tokio::select! {
            status = child.wait() => Outcome::Status(status),
            _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                Outcome::TimedOut
            }
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                Outcome::Cancelled
            }
        }
    };
    let (out_buf, err_buf, drain_timed_out) = drain.finish(Duration::from_secs(15)).await;

    let mut text = String::from_utf8_lossy(&out_buf).to_string();
    text.push_str(&String::from_utf8_lossy(&err_buf));
    if drain_timed_out {
        text.push_str("\n[output truncated: still streaming after 15s]");
    }
    match outcome {
        Outcome::Cancelled => Ok((text, None, true)),
        Outcome::TimedOut => Ok((text, None, false)),
        Outcome::Status(s) => {
            let code = s.map_err(|e| format!("wait failed: {e}"))?.code();
            Ok((text, code, false))
        }
    }
}

/// Map a raw probe-rs invocation error into a tagged tool error.
///
/// - Missing binary -> `[NotFound]` with an install hint.
/// - A stuck ST-Link session on Windows (`reset not supported by WinUSB`, or
///   the probe refusing to open at all) -> `[Io]` with replug guidance.
///   probe-rs leaves the ST-Link USB session busy when a previous process did
///   not close it cleanly; on WinUSB drivers `libusb_reset_device` is not
///   supported so the recovery path fails too (probe-rs issue #2207).
/// - Anything else -> generic `[Io]`.
pub(crate) fn probe_rs_err(e: String) -> ToolError {
    if e.contains("spawn failed") {
        ToolError::new(
            "[NotFound] probe-rs is not installed or not on PATH: install it with \
             `cargo install probe-rs-tools` or download from the probe-rs GitHub Releases",
        )
    } else if e.contains("[Cancelled]") {
        // Turn cancellation is not an I/O failure — keep the honest tag
        // instead of relabeling it [Io].
        ToolError::new(e)
    } else if e.contains("reset not supported by WinUSB")
        || e.contains("Failed to open the debug probe")
    {
        ToolError::new(format!(
            "[Io] cannot open the ST-Link probe: its USB session is still busy (left over from a \
             previous probe-rs/ST tool that did not close it cleanly; known probe-rs issue #2207 \
             on Windows/WinUSB). Unplug and replug the ST-Link, close any program holding the \
             probe (STM32CubeProgrammer / CubeIDE / Keil / OpenOCD), then retry.\n{e}"
        ))
    } else {
        ToolError::new(format!("[Io] {e}"))
    }
}

/// Terminate a process and everything underneath it. Background jobs spawned by
/// the command inherit our pipe handles; killing only the direct child would
/// leave those handles open and the capture would never reach EOF.
#[cfg(windows)]
fn kill_process_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(0x0800_0000)
        .status();
}

#[cfg(not(windows))]
fn kill_process_tree(pid: u32) {
    // Kill every descendant directly (discovered recursively via `ps`)
    // instead of signalling a process group. Group kills are unsafe on
    // hosted runners: when the child's pgid resolves to the job's own
    // process group -- or when the `our_pgid` probe races/fails and
    // returns 0, which defeats the `!= our_pgid` guard -- `kill -9
    // -<pgid>` SIGKILLs the whole step, including this process itself
    // (observed as `Killed` / exit 137 on ubuntu-22.04). Direct kills
    // of individual pids can never hit a foreign group, so they are
    // safe even under probe races.
    for d in ps_descendants(pid).iter().rev() {
        eprintln!("[kill_process_tree] killing descendant {d}");
        let _ = std::process::Command::new("kill")
            .args(["-9", &d.to_string()])
            .status();
    }
    eprintln!("[kill_process_tree] killing {pid}");
    let _ = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status();
}

#[cfg(not(windows))]
fn ps_descendants(pid: u32) -> Vec<u32> {
    // Snapshot the process table once, then walk parent links from `pid`
    // recursively. Returns post-order (children before parents) so a
    // grandchild is killed before the child that may be waiting on it.
    let out = std::process::Command::new("ps")
        .args(["-A", "--no-headers", "-o", "ppid=", "-o", "pid="])
        .output()
        .ok();
    let Some(out) = out else {
        return Vec::new();
    };
    let mut table: Vec<(u32, u32)> = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut it = line.split_whitespace();
        if let (Some(pp_str), Some(pid_str)) = (it.next(), it.next())
            && let (Ok(pp), Ok(pid)) = (pp_str.parse::<u32>(), pid_str.parse::<u32>())
        {
            table.push((pp, pid));
        }
    }
    fn collect(table: &[(u32, u32)], parent: u32, result: &mut Vec<u32>) {
        for &(pp, p) in table {
            if pp == parent {
                collect(table, p, result);
                result.push(p);
            }
        }
    }
    let mut result = Vec::new();
    collect(&table, pid, &mut result);
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn probe_rs_err_classifies_missing_binary_stuck_probe_and_generic() {
        let missing = probe_rs_err("spawn failed: no such file".to_string());
        assert!(missing.message.contains("[NotFound]"), "got: {missing}");
        assert!(missing.message.contains("probe-rs"), "got: {missing}");

        let stuck = probe_rs_err(
            "Failed to open the debug probe.\nreset not supported by WinUSB".to_string(),
        );
        assert!(stuck.message.contains("[Io]"), "got: {stuck}");
        assert!(stuck.message.contains("Unplug and replug"), "got: {stuck}");

        // The WinUSB marker alone is enough to trigger the guidance.
        let winusb_only = probe_rs_err("reset not supported by WinUSB".to_string());
        assert!(
            winusb_only.message.contains("Unplug and replug"),
            "got: {winusb_only}"
        );

        let generic = probe_rs_err("some other failure".to_string());
        assert!(generic.message.contains("[Io]"), "got: {generic}");
        assert!(!generic.message.contains("Unplug"), "got: {generic}");
    }

    #[test]
    fn resolve_within_enforces_workspace_boundary() {
        let dir = tempdir().unwrap();
        let inside = resolve_within(dir.path(), "a.txt", &[]).unwrap();
        assert_eq!(inside, dir.path().join("a.txt"));

        let err = resolve_within(dir.path(), "../outside.txt", &[]).unwrap_err();
        assert!(err.contains("outside the workspace"), "got: {err}");

        let extra = tempdir().unwrap();
        let target = extra.path().join("x.txt");
        assert!(resolve_within(dir.path(), &target.to_string_lossy(), &[]).is_err());
        let ok = resolve_within(
            dir.path(),
            &target.to_string_lossy(),
            &[extra.path().to_path_buf()],
        )
        .unwrap();
        assert!(ok.starts_with(extra.path()));
    }

    #[tokio::test]
    async fn timeout_kills_long_running_command_promptly() {
        let dir = tempdir().unwrap();
        // Print something FIRST, then hang: the kill path must preserve the
        // partial output instead of discarding it with the process.
        let slow = if cfg!(windows) {
            "echo build-ok& ping -n 30 127.0.0.1 >nul"
        } else {
            "echo build-ok; sleep 30"
        };
        let started = std::time::Instant::now();
        let (text, code) = run_command(slow, dir.path(), 400, None, None)
            .await
            .expect("run_command returns Ok");
        assert!(code.is_none(), "timeout must report a killed process");
        assert!(text.contains("timed out"), "got: {text}");
        assert!(
            text.contains("build-ok"),
            "partial output must survive the kill, got: {text}"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "timeout returned too late: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn cancel_stops_long_running_command_promptly() {
        let dir = tempdir().unwrap();
        let slow = if cfg!(windows) {
            "ping -n 30 127.0.0.1 >nul"
        } else {
            "sleep 30"
        };
        let cancel = firment_core::Cancellable::new();
        let c = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            c.cancel();
        });
        let started = std::time::Instant::now();
        let (text, code) = run_command(slow, dir.path(), 0, None, Some(&cancel))
            .await
            .expect("run_command returns Ok");
        assert!(code.is_none(), "cancel must report a killed process");
        assert!(text.contains("cancelled"), "got: {text}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "cancel returned too late: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn run_streaming_drains_both_pipes_past_the_os_buffer() {
        let dir = tempdir().unwrap();
        // ~4x the 64 KB pipe buffer per stream: a reader that waits for
        // stdout EOF before touching stderr, or that only starts draining
        // after the child exits, deadlocks right here.
        fs::write(dir.path().join("flood.txt"), "x".repeat(256 * 1024)).unwrap();
        let args: Vec<std::ffi::OsString> = if cfg!(windows) {
            vec!["/C".into(), "type flood.txt & type flood.txt 1>&2".into()]
        } else {
            vec!["-c".into(), "cat flood.txt; cat flood.txt >&2".into()]
        };
        let program = if cfg!(windows) { "cmd" } else { "sh" };
        let started = std::time::Instant::now();
        let (text, code, cancelled) =
            run_streaming(program, &args, dir.path(), 30_000, &Cancellable::new())
                .await
                .expect("child runs");
        assert_eq!(code, Some(0), "child exited abnormally: {text}");
        assert!(!cancelled);
        assert_eq!(
            text.matches('x').count(),
            2 * 256 * 1024,
            "both pipes must reach EOF"
        );
        assert!(!text.contains("output truncated"), "got: {text}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(20),
            "the pipes were not drained while the child ran: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn abandoning_a_drain_keeps_the_bytes_already_read() {
        let dir = tempdir().unwrap();
        let (program, args): (&str, Vec<&str>) = if cfg!(windows) {
            (
                "cmd",
                vec!["/C", "echo first-line& ping -n 30 127.0.0.1 >nul"],
            )
        } else {
            ("sh", vec!["-c", "echo first-line; sleep 30"])
        };
        let mut child = Command::new(program)
            .args(&args)
            .current_dir(dir.path())
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let drain = PipeDrain::start(stdout, stderr);
        // Let the reader actually observe the first line, then give up: the
        // child is still alive, so EOF never arrives.
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        let (out_buf, err_buf, timed_out) = drain.finish(Duration::ZERO).await;
        let _ = child.kill().await;
        assert!(
            timed_out,
            "a live child must not look like a finished drain"
        );
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out_buf),
            String::from_utf8_lossy(&err_buf)
        );
        assert!(
            text.contains("first-line"),
            "bytes already read must survive the abandoned drain, got: {text}"
        );
    }

    #[test]
    fn truncate_reports_the_real_dropped_char_count() {
        let out = truncate(&"z".repeat(1_000), 100);
        assert!(
            out.contains("901 chars dropped"),
            "the marker must interpolate the count, got: {out}"
        );
        assert!(!out.contains('{'), "got: {out}");
    }

    #[test]
    fn read_text_refuses_files_over_the_cap() {
        let dir = tempdir().unwrap();
        let huge = dir.path().join("huge.log");
        // Extend to the size without writing it: read_text must not find out
        // by running out of memory.
        fs::File::create(&huge)
            .unwrap()
            .set_len(MAX_TEXT_FILE_BYTES + 1)
            .unwrap();
        let err = read_text(&huge).unwrap_err();
        assert!(err.contains("text cap"), "got: {err}");

        let small = dir.path().join("small.txt");
        fs::write(&small, "hello").unwrap();
        assert_eq!(read_text(&small).unwrap(), "hello");
    }
}
