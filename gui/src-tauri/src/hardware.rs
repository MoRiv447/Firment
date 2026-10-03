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
    /// Set when the reader loop has actually returned.
    ///
    /// A `watch` rather than a `Notify` because the signal must not be able to land before the
    /// waiter registers: a reader that finished in the gap between setting `stop` and waiting
    /// would leave a `notify_waiters()` with nobody listening, and the Stop would then wait out
    /// its whole budget for an exit that had already happened.
    exited: Arc<tokio::sync::watch::Sender<bool>>,
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
    let (exited, _) = tokio::sync::watch::channel(false);
    let exited = Arc::new(exited);
    // Insert AFTER the (blocking) open but under one lock pass with the
    // duplicate check: the check-to-insert window is now a few in-memory
    // operations instead of spanning the device-open call, so two rapid
    // starts can no longer both slip through and orphan the first reader.
    let monitor = Arc::new(SerialMonitor {
        write_port: Arc::new(tokio::sync::Mutex::new(write_port)),
        stop: stop.clone(),
        exited: exited.clone(),
    });
    {
        let mut map = shared
            .monitors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    let exited_r = exited.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut buf = [0u8; 4096];
        let mut splitter =
            firment_tools::utf8::LineSplitter::new(firment_tools::utf8::MAX_LINE_BYTES);
        loop {
            if stop_r.load(Ordering::Relaxed) {
                break;
            }
            match handle.read(&mut buf) {
                Ok(0) => continue,
                Ok(n) => {
                    // Complete lines first, each still carrying the '\n'
                    // the frontend uses to detect line boundaries (a
                    // newline-less chunk is glued onto the current line).
                    splitter.feed(&buf[..n], &mut |line| {
                        let decoded =
                            firment_tools::decode::decode_line(line, elf_path.as_deref());
                        let _ = app.emit(
                            "monitor-output",
                            json!({ "port": port_out, "kind": "stdout", "line": format!("{decoded}\n") }),
                        );
                    });
                    // Then flush whatever is left, so progress-style output
                    // (AT responses, boot progress, register dumps) still
                    // appears in real time instead of waiting for a '\n'.
                    // Only complete characters go out: an incomplete tail
                    // stays buffered for the next read rather than being
                    // frozen into a U+FFFD right now.
                    let rest = splitter.take_flushable();
                    if !rest.is_empty() {
                        let _ = app.emit(
                            "monitor-output",
                            json!({ "port": port_out, "kind": "stdout", "line": rest }),
                        );
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(_) => break,
            }
        }
        // Flush a character the port closed in the middle of, instead of
        // dropping it along with the buffer.
        if let Some(tail) = splitter.take_tail() {
            let _ = app.emit(
                "monitor-output",
                json!({ "port": port_out, "kind": "stdout", "line": tail }),
            );
        }
        // Dead port (device unplugged, driver error): remove the map entry so
        // the port can be started again and active_monitors stays truthful.
        // Compare by Arc pointer to only remove OUR entry (never a newer
        // monitor the user already restarted on the same port).
        {
            let mut map = shared_for_reader
                .monitors
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if map
                .get(&port_out)
                .is_some_and(|entry| Arc::ptr_eq(entry, &monitor_ref))
            {
                map.remove(&port_out);
            }
        }
        let _ = app.emit("monitor-exited", json!({ "port": port_out }));
        // Last, after the map entry is gone: a waiter woken here knows the port is free.
        let _ = exited_r.send(true);
    });

    Ok(())
}

/// Send raw bytes to the port of a running monitor.
pub async fn monitor_send(shared: Arc<Shared>, port: &str, data: &str) -> Result<(), String> {
    let monitor = shared
        .monitors
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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

/// Stop a monitor and wait for its reader to give the port back.
///
/// The `Err` is a timing report, not a failed stop: the flag is set and the entry removed either
/// way. Saying it out loud is the point — a reader still wedged in the driver means the next
/// `monitor_start` on that port will be told the device is busy, and a silent success here leaves
/// whoever reads that later failure with nothing to connect it to.
pub async fn monitor_stop(shared: Arc<Shared>, port: &str) -> Result<(), String> {
    let monitor = shared
        .monitors
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(port);
    let Some(m) = monitor else {
        return Ok(());
    };
    // Registered before the flag is set: the reader signals once, and a wait that started after
    // that would sit out its whole budget for an exit that had already happened.
    let mut waited = m.exited.subscribe();
    // Setting the flag makes the reader loop exit at its next check; the 100 ms read timeout
    // is what bounds that, because the loop cannot see the flag while blocked in `read`.
    m.stop.store(true, Ordering::Relaxed);
    // Our clone of the port goes here; the reader holds the other one, so the device is not free
    // until its loop returns. Waiting for that is what makes Stop-then-Start on the same port
    // work — without it the most obvious gesture in the UI failed with "port busy" for up to a
    // read timeout, which reads as the app holding the device hostage.
    // Either outcome means the reader finished: the signal arriving, or the sender being dropped
    // with it. Only the timeout means it is still holding the port.
    let gave_back = tokio::time::timeout(Duration::from_millis(500), waited.changed())
        .await
        .is_ok();
    drop(m);
    if gave_back {
        Ok(())
    } else {
        Err(format!(
            "the monitor on {port} was asked to stop and has not released the port yet; a start \
             on it may report the port busy for a moment"
        ))
    }
}

pub fn active_monitors(shared: &Arc<Shared>) -> Vec<String> {
    shared
        .monitors
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .keys()
        .cloned()
        .collect()
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
