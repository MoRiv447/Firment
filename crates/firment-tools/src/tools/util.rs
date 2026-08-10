use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

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

pub(crate) fn read_text(path: &Path) -> Result<String, String> {
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

pub(crate) fn truncate(text: &str, max_chars: usize) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    if chars.len() > max_chars {
        chars.truncate(max_chars);
        chars.push('…');
    }
    chars.into_iter().collect()
}

/// Validate a value that will be spliced unquoted into a shell command line
/// (chip id, probe serial). Only plain token characters are allowed so the
/// value can never break out of the shell or inject extra commands.
pub(crate) fn token_arg(value: &str, what: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err(format!("[InvalidInput] {what} must not be empty"));
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

/// Quote a value for safe interpolation into the platform shell used by
/// `run_command` (cmd.exe on Windows, sh on Unix).
pub(crate) fn shell_quote(arg: &str) -> String {
    if cfg!(windows) {
        // cmd.exe: double quotes are the quoting mechanism, but `%` expansion
        // and `^` escaping still apply inside them, so neutralise both.
        let escaped = arg.replace('%', "%%").replace('^', "^^");
        format!("\"{escaped}\"")
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

/// Run a command through the platform shell, capture output, enforce a
/// timeout. Returns (formatted text, exit code); exit code `None` means the
/// process was killed by the timeout.
pub(crate) async fn run_command(
    command: &str,
    cwd: &Path,
    timeout_ms: u64,
    env: Option<&HashMap<String, String>>,
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

    let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout handle unavailable".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "stderr handle unavailable".to_string())?;
    let mut out_buf: Vec<u8> = Vec::new();
    let mut err_buf: Vec<u8> = Vec::new();
    let read_streams = async {
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            let _ = AsyncReadExt::read_to_end(&mut stdout, &mut out_buf).await;
            let _ = AsyncReadExt::read_to_end(&mut stderr, &mut err_buf).await;
        })
        .await;
    };
    let mut read_streams = Box::pin(read_streams);

    let status = if timeout_ms == 0 {
        child.wait().await
    } else {
        tokio::select! {
            status = child.wait() => status,
            _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
                if let Some(pid) = child.id() {
                    // Kill the process tree: the direct child may have spawned
                    // background processes that inherit our pipe handles and
                    // would otherwise keep the read futures open forever.
                    kill_process_tree(pid);
                }
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Ok((
                    format!(
                        "command: {command}\ntimed out after {timeout_ms} ms and was killed \
                         (process tree terminated)"
                    ),
                    None,
                ));
            }
        }
    };
    // Drain what the process left behind with a firm deadline: a background
    // grandchild holding the pipe write-ends would otherwise block forever.
    let _ = (&mut read_streams).await;
    drop(read_streams);

    let stdout = truncate(&String::from_utf8_lossy(&out_buf), 32_000);
    let stderr = truncate(&String::from_utf8_lossy(&err_buf), 32_000);
    let status = status.map_err(|e| format!("wait failed: {e}"))?;
    let code = status.code();
    let status_text = code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "signal".to_string());
    Ok((
        format!(
            "command: {command}\nexit code: {status_text}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
        ),
        code,
    ))
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
    // `-pid` targets the whole process group set up via process_group(0).
    let _ = std::process::Command::new("kill")
        .args(["-9", &format!("-{pid}")])
        .status();
}
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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
        let slow = if cfg!(windows) {
            "ping -n 30 127.0.0.1 >nul"
        } else {
            "sleep 30"
        };
        let started = std::time::Instant::now();
        let (text, code) = run_command(slow, dir.path(), 400, None)
            .await
            .expect("run_command returns Ok");
        assert!(code.is_none(), "timeout must report a killed process");
        assert!(text.contains("timed out"), "got: {text}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "timeout returned too late: {:?}",
            started.elapsed()
        );
    }
}
