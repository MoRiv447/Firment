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

/// Canonicalize an existing path; for a not-yet-existing path (e.g. a file to
/// be created), canonicalize its parent and re-attach the file name.
fn canonicalize_for_check(path: &Path) -> std::io::Result<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path);
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        let parent = fs::canonicalize(parent)?;
        return Ok(parent.join(path.file_name().unwrap_or_default()));
    }
    fs::canonicalize(path)
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
    let read_stdout = async { AsyncReadExt::read_to_end(&mut stdout, &mut out_buf).await };
    let read_stderr = async { AsyncReadExt::read_to_end(&mut stderr, &mut err_buf).await };
    let mut read_stdout = Box::pin(read_stdout);
    let mut read_stderr = Box::pin(read_stderr);

    let status = if timeout_ms == 0 {
        child.wait().await
    } else {
        tokio::select! {
            status = child.wait() => status,
            _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = (&mut read_stdout).await;
                let _ = (&mut read_stderr).await;
                return Ok((
                    format!(
                        "command: {command}\ntimed out after {timeout_ms} ms and was killed (child processes may survive)"
                    ),
                    None,
                ));
            }
        }
    };
    let _ = (&mut read_stdout).await;
    let _ = (&mut read_stderr).await;

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
}
