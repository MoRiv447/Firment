use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tauri::Emitter;
use tokio::process::Command;
use tokio::time::timeout;

use crate::state::Shared;

pub fn find_firm_binary() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".firment").join("bin").join("firm.exe"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("firm.exe"));
            candidates.push(dir.join("..").join("debug").join("firm.exe"));
        }
    }
    for c in candidates {
        if c.exists() {
            return Some(c);
        }
    }
    // fall back to PATH lookup
    let out = std::process::Command::new("where")
        .arg("firm")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string());
    out.and_then(|s| s.lines().next().map(|l| PathBuf::from(l.trim())))
        .filter(|p| p.exists())
}

/// Handle to a live serial monitor. The reader thread owns one clone of the
/// port; `write_port` is the other clone used by `monitor_send`.
pub struct SerialMonitor {
    pub write_port: Arc<tokio::sync::Mutex<Box<dyn serialport::SerialPort>>>,
    pub stop: Arc<AtomicBool>,
}

/// Open the serial port directly (no `firm monitor` subprocess) and stream
/// raw reads to the frontend as they arrive.
///
/// Why not the subprocess? `firm monitor` buffers output by line and only
/// prints on `\n`; with `--timeout 0` a device that emits no newlines (boot
/// progress, AT responses, register dumps) would never show anything. Reading
/// the port directly here and emitting every read chunk makes the monitor
/// behave like a normal serial terminal.
pub async fn monitor_start(
    shared: Arc<Shared>,
    port: String,
    baud: u32,
    elf: Option<String>,
) -> Result<(), String> {
    if shared.monitors.lock().unwrap().contains_key(&port) {
        return Err(format!("a monitor is already running on {port}"));
    }
    let mut handle = serialport::new(&port, baud)
        .timeout(Duration::from_millis(100))
        .open()
        .map_err(|e| format!("failed to open serial port {port}: {e}"))?;

    let write_port = handle
        .try_clone()
        .map_err(|e| format!("failed to clone serial port {port}: {e}"))?;

    let stop = Arc::new(AtomicBool::new(false));
    let monitor = SerialMonitor {
        write_port: Arc::new(tokio::sync::Mutex::new(write_port)),
        stop: stop.clone(),
    };
    shared
        .monitors
        .lock()
        .unwrap()
        .insert(port.clone(), Arc::new(monitor));

    // ---- reader task ----
    let app = shared.app.clone();
    let port_out = port.clone();
    let elf_path = elf.map(PathBuf::from);
    let stop_r = stop.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut buf = [0u8; 4096];
        let mut partial = String::new();
        loop {
            if stop_r.load(Ordering::Relaxed) {
                break;
            }
            match handle.read(&mut buf) {
                Ok(0) => continue,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    partial.push_str(&text);
                    // Emit complete lines immediately, KEEPING the trailing
                    // '\n'. The frontend uses the '\n' to detect line
                    // boundaries (a newline-less chunk is glued onto the
                    // current line), so stripping it here merges every line
                    // into one.
                    while let Some(pos) = partial.find('\n') {
                        let line = partial[..=pos].trim_end().to_string();
                        let decoded =
                            firment_tools::decode::decode_line(&line, elf_path.as_deref());
                        let _ = app.emit(
                            "monitor-output",
                            json!({ "port": port_out, "kind": "stdout", "line": format!("{decoded}\n") }),
                        );
                        partial.drain(..=pos);
                    }
                    // No newline in this chunk: still flush what we have so
                    // progress-style output appears in real time instead of
                    // being stuck until a '\n' finally arrives.
                    if !partial.is_empty() {
                        let chunk = std::mem::take(&mut partial);
                        let _ = app.emit(
                            "monitor-output",
                            json!({ "port": port_out, "kind": "stdout", "line": chunk }),
                        );
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(_) => break,
            }
        }
        let _ = app.emit("monitor-exited", json!({ "port": port_out }));
    });

    Ok(())
}

/// Send raw bytes to the port of a running monitor.
pub async fn monitor_send(shared: Arc<Shared>, port: &str, data: &str) -> Result<(), String> {
    let monitor = shared
        .monitors
        .lock()
        .unwrap()
        .get(port)
        .cloned()
        .ok_or_else(|| format!("no monitor running on {port}"))?;
    let mut w = monitor.write_port.lock().await;
    w.write_all(data.as_bytes())
        .map_err(|e| format!("write to {port} failed: {e}"))?;
    w.flush().map_err(|e| format!("flush to {port} failed: {e}"))?;
    Ok(())
}

pub async fn monitor_stop(shared: Arc<Shared>, port: &str) {
    let monitor = shared.monitors.lock().unwrap().remove(port);
    if let Some(m) = monitor {
        // Setting the flag makes the reader loop exit; the read timeout
        // (100ms) bounds how long the blocked read can delay that.
        m.stop.store(true, Ordering::Relaxed);
        // Dropping the write port closes the device, unblocking the reader.
        drop(m);
    }
}

pub fn active_monitors(shared: &Arc<Shared>) -> Vec<String> {
    shared.monitors.lock().unwrap().keys().cloned().collect()
}

/// Run a one-shot `firm flash`/`firm run` and emit the collected output.
/// `cwd` (optional) sets the working directory of the firm subprocess so the
/// CLI's workspace sandbox resolves relative paths against it.
pub async fn run_hardware_command(
    shared: Arc<Shared>,
    kind: String,
    args: Vec<String>,
    cwd: Option<String>,
    timeout_secs: u64,
) -> Result<(), String> {
    let firm = find_firm_binary().ok_or_else(|| {
        "firm binary not found - install via 'firm install' or build it first".to_string()
    })?;
    let mut cmd = Command::new(firm);
    cmd.args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(dir) = cwd.as_ref().filter(|d| !d.trim().is_empty()) {
        cmd.current_dir(dir);
    }
    let out = timeout(
        Duration::from_secs(timeout_secs.max(5)),
        cmd.output(),
    )
    .await
    .map_err(|_| format!("{kind} timed out after {timeout_secs}s"))?
    .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let code = out.status.code().unwrap_or(-1);
    let _ = shared.app.emit(
        "hardware-exit",
        json!({
            "kind": kind,
            "code": code,
            "stdout": stdout,
            "stderr": stderr
        }),
    );
    if !out.status.success() {
        // Include stdout/stderr in the Err so the IDE's catch fallback (when
        // the emit didn't render in time) still has the real probe-rs
        // diagnostic. The frontend will normally have already received the
        // emit and rendered these in the Alert description.
        let mut msg = format!("{kind} failed with exit code {code}");
        if !stdout.trim().is_empty() {
            msg.push_str(&format!("\n--- stdout ---\n{stdout}"));
        }
        if !stderr.trim().is_empty() {
            msg.push_str(&format!("\n--- stderr ---\n{stderr}"));
        }
        return Err(msg);
    }
    Ok(())
}

pub fn list_serial_ports() -> Vec<String> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .collect()
}
