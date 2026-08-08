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
    let old_lines: Vec<&str> = old.split('\n').collect();
    let new_lines: Vec<&str> = new.split('\n').collect();
    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_lines.len().saturating_sub(prefix)
        && suffix < new_lines.len().saturating_sub(prefix)
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let old_mid = &old_lines[prefix..old_lines.len() - suffix];
    let new_mid = &new_lines[prefix..new_lines.len() - suffix];
    let mut out = String::new();
    out.push_str(&format!("--- {}\n+++ {}\n", path.display(), path.display()));
    out.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        prefix + 1,
        old_mid.len(),
        prefix + 1,
        new_mid.len()
    ));
    for line in old_mid {
        out.push_str(&format!("-{line}\n"));
    }
    for line in new_mid {
        out.push_str(&format!("+{line}\n"));
    }
    truncate(&out, max_chars)
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

    let status = tokio::select! {
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
