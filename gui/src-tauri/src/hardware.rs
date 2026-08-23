use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tauri::Emitter;

use crate::state::Shared;

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
    let mut handle = serialport::new(&port, baud)
        .timeout(Duration::from_millis(100))
        .open()
        .map_err(|e| format!("failed to open serial port {port}: {e}"))?;

    let write_port = handle
        .try_clone()
        .map_err(|e| format!("failed to clone serial port {port}: {e}"))?;

    let stop = Arc::new(AtomicBool::new(false));
    // Insert AFTER the (blocking) open but under one lock pass with the
    // duplicate check: the check-to-insert window is now a few in-memory
    // operations instead of spanning the device-open call, so two rapid
    // starts can no longer both slip through and orphan the first reader.
    let monitor = Arc::new(SerialMonitor {
        write_port: Arc::new(tokio::sync::Mutex::new(write_port)),
        stop: stop.clone(),
    });
    {
        let mut map = shared.monitors.lock().unwrap();
        if map.contains_key(&port) {
            return Err(format!("a monitor is already running on {port}"));
        }
        map.insert(port.clone(), monitor.clone());
    }

    // ---- reader task ----
    let app = shared.app.clone();
    let shared_for_reader = shared.clone();
    let port_out = port.clone();
    let elf_path = elf.map(PathBuf::from);
    let stop_r = stop.clone();
    let monitor_ref = monitor.clone();
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
        // Dead port (device unplugged, driver error): remove the map entry so
        // the port can be started again and active_monitors stays truthful.
        // Compare by Arc pointer to only remove OUR entry (never a newer
        // monitor the user already restarted on the same port).
        {
            let mut map = shared_for_reader.monitors.lock().unwrap();
            if map
                .get(&port_out)
                .is_some_and(|entry| Arc::ptr_eq(entry, &monitor_ref))
            {
                map.remove(&port_out);
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
    w.flush()
        .map_err(|e| format!("flush to {port} failed: {e}"))?;
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

/// Flash (`kind = "flash"`) or flash+run (`kind = "run"`) in-process via the
/// shared [`firment_tools::hardware`] entry points — the same probe-rs
/// pipeline the agent tools use, without shelling out to a `firm` binary.
/// Emits the collected output on the `hardware-exit` event.
pub async fn run_hardware_command(
    shared: Arc<Shared>,
    kind: String,
    file: String,
    chip: Option<String>,
    probe: Option<String>,
    cwd: Option<String>,
    timeout_secs: u64,
) -> Result<(), String> {
    let cwd = match cwd.as_deref().filter(|d| !d.trim().is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
    let timeout_ms = timeout_secs.max(5) * 1_000;
    let result = match kind.as_str() {
        "flash" => {
            firment_tools::hardware::flash_elf(
                &cwd,
                &file,
                chip.as_deref(),
                probe.as_deref(),
                timeout_ms,
            )
            .await
        }
        "run" => {
            firment_tools::hardware::run_elf(
                &cwd,
                &file,
                chip.as_deref(),
                probe.as_deref(),
                timeout_ms,
            )
            .await
        }
        other => Err(format!("unknown hardware command kind: {other}")),
    };
    let _ = shared.app.emit(
        "hardware-exit",
        json!({
            "kind": kind,
            "code": if result.is_ok() { 0 } else { 1 },
            "stdout": result.clone().unwrap_or_default(),
            "stderr": result.clone().err().unwrap_or_default()
        }),
    );
    result.map(|_| ())
}

pub fn list_serial_ports() -> Vec<String> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .collect()
}
