use super::util::{resolve_within, truncate};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

pub struct Monitor;

/// Open the port and hand the byte stream to [`read_serial_from`].
fn read_serial(
    port: &str,
    baud: u32,
    timeout_ms: u64,
    elf: Option<&Path>,
    timestamp: bool,
    cancel: Option<firment_core::Cancellable>,
) -> Result<String, String> {
    let mut reader = serialport::new(port, baud)
        .timeout(Duration::from_millis(500))
        .open()
        .map_err(|e| format!("failed to open serial port {port}: {e}"))?;
    let text = read_serial_from(&mut reader, timeout_ms, elf, timestamp, cancel)?;
    // Only this wrapper knows the port name, so it owns the empty-case text.
    if text.is_empty() {
        Ok(format!("no data received on {port} within {timeout_ms} ms"))
    } else {
        Ok(text)
    }
}

/// Blocking serial read loop: collect lines until the deadline, decoding hex
/// code addresses against an ELF when provided. With `timestamp`, each line
/// is prefixed with its arrival time `[SS.mmm]` relative to the read start.
///
/// Split out from [`read_serial`] so tests can drive it from a fake byte
/// source — a real port cannot be opened in CI, and the chunking that
/// breaks multi-byte characters only shows up on a live stream.
fn read_serial_from(
    reader: &mut impl Read,
    timeout_ms: u64,
    elf: Option<&Path>,
    timestamp: bool,
    cancel: Option<firment_core::Cancellable>,
) -> Result<String, String> {
    let start = Instant::now();
    let deadline = start + Duration::from_millis(timeout_ms);
    let mut buf = [0u8; 4096];
    let mut splitter = crate::utf8::LineSplitter::new(crate::utf8::MAX_LINE_BYTES);
    let mut lines: Vec<String> = Vec::new();
    // Build the symbol index ONCE: decoding used to re-read and re-parse the
    // whole ELF for every hex token in every line.
    let index = elf.and_then(crate::decode::SymbolIndex::from_path);
    let push_line = |line: &str, lines: &mut Vec<String>, start: Instant, timestamp: bool| {
        let decoded = match &index {
            Some(index) => index.decode_line(line),
            None => line.to_string(),
        };
        let with_ts = if timestamp {
            let elapsed = Instant::now() - start;
            format!(
                "[{:02}.{:03}] {decoded}",
                elapsed.as_secs(),
                elapsed.subsec_millis()
            )
        } else {
            decoded
        };
        lines.push(with_ts);
    };
    loop {
        // A cancelled turn must not keep holding the serial port until the
        // full timeout elapses.
        if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            lines.push("(monitor interrupted by turn cancellation)".to_string());
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        match reader.read(&mut buf) {
            Ok(0) => continue,
            // Decode each line only once its bytes are complete: decoding
            // per read turns a character split across two reads into
            // U+FFFD, which is the whole point of LineSplitter.
            Ok(n) => splitter.feed(&buf[..n], &mut |line| {
                push_line(line, &mut lines, start, timestamp)
            }),
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                continue;
            }
            Err(e) => return Err(format!("serial read failed: {e}")),
        }
    }
    if let Some(tail) = splitter.take_tail() {
        push_line(&tail, &mut lines, start, timestamp);
    }
    let text = lines.join("\n");
    if text.is_empty() {
        Ok(String::new())
    } else {
        Ok(truncate(&text, 32_000))
    }
}

/// Candidate baud rates tried by `autodetect`, slow to fast.
const BAUD_CANDIDATES: [u32; 9] = [
    9_600, 19_200, 38_400, 57_600, 74_880, 115_200, 230_400, 460_800, 921_600,
];

/// Probe each common baud rate for `probe_ms` and return the first one that
/// yields mostly-valid bytes (an incorrect baud rate reads noise — a high
/// ratio of 0x00/0xFF bytes).
fn detect_baud(port: &str, probe_ms: u64) -> Result<Option<u32>, String> {
    for baud in BAUD_CANDIDATES {
        if probe_baud(port, baud, probe_ms)? {
            return Ok(Some(baud));
        }
    }
    Ok(None)
}

fn probe_baud(port: &str, baud: u32, probe_ms: u64) -> Result<bool, String> {
    use std::io::Read;
    let Ok(mut reader) = serialport::new(port, baud)
        .timeout(Duration::from_millis(50))
        .open()
    else {
        return Ok(false); // port busy on this attempt; try the next rate
    };
    let deadline = Instant::now() + Duration::from_millis(probe_ms);
    let mut bytes = 0u64;
    let mut junk = 0u64;
    let mut buf = [0u8; 256];
    loop {
        if Instant::now() >= deadline {
            break;
        }
        match reader.read(&mut buf) {
            Ok(0) => continue,
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == 0x00 || b == 0xFF {
                        junk += 1;
                    }
                }
                bytes += n as u64;
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                continue;
            }
            Err(_) => return Ok(false),
        }
    }
    Ok(bytes > 0 && (junk as f64 / bytes as f64) < 0.9)
}

/// Human-readable description of a serial port (manufacturer / product /
/// serial number when available) for the missing-port error message.
pub fn port_label(port: &serialport::SerialPortInfo) -> String {
    match &port.port_type {
        serialport::SerialPortType::UsbPort(usb) => {
            let mut parts: Vec<String> = Vec::new();
            for s in [
                usb.manufacturer.clone(),
                usb.product.clone(),
                usb.serial_number.clone(),
            ]
            .into_iter()
            .flatten()
            {
                parts.push(s);
            }
            if parts.is_empty() {
                port.port_name.clone()
            } else {
                format!("{} ({})", port.port_name, parts.join(" "))
            }
        }
        _ => port.port_name.clone(),
    }
}

/// Enumerate serial ports so the agent can pick one when neither the call
/// nor [tools] monitor_port provides it (instead of failing blind).
pub fn enumerate_ports() -> String {
    match serialport::available_ports() {
        Ok(ports) if !ports.is_empty() => {
            ports.iter().map(port_label).collect::<Vec<_>>().join(", ")
        }
        _ => "none detected".to_string(),
    }
}

/// Bare port names (`COM10`, `/dev/ttyUSB0`) for exact matching. The joined
/// `enumerate_ports()` string must NOT be substring-matched: `"COM1"` is a
/// substring of `"COM10 (...)"` and would report an attached port that
/// isn't there.
pub fn port_names() -> Vec<String> {
    serialport::available_ports()
        .map(|ports| ports.into_iter().map(|p| p.port_name).collect())
        .unwrap_or_default()
}

#[async_trait]
impl Tool for Monitor {
    fn name(&self) -> &'static str {
        "monitor"
    }

    fn description(&self) -> &'static str {
        "Open a serial port (UART) for a bounded time and return the captured log lines, each prefixed with its arrival time [SS.mmm]. Optionally decode hex code addresses (e.g. panic backtraces) using an ELF. With autodetect=true, tries common baud rates (9600..921600) and reports the first one that yields valid data. Use after flash/run when the target logs over a physical UART. If no port is passed or configured, the tool reports the serial ports it detected on this host."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "port": {"type": "string", "description": "Serial port, e.g. COM3 or /dev/ttyUSB0 (defaults to [tools] monitor_port)"},
                "baud": {"type": "integer", "minimum": 1, "description": "Baud rate (defaults to [tools] monitor_baud)"},
                "autodetect": {"type": "boolean", "default": false, "description": "Probe common baud rates (9600..921600) and use the first that yields valid data; overrides baud"},
                "timestamp": {"type": "boolean", "default": true, "description": "Prefix each line with its arrival time [SS.mmm]"},
                "elf": {"type": "string", "description": "Optional path to the firmware ELF (inside the workspace) for decoding hex code addresses in log lines"},
                "timeout_ms": {"type": "integer", "minimum": 1, "default": 10000, "description": "How long to listen before returning captured output"}
            }
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        let port = args
            .get("port")
            .and_then(|p| p.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "configured port".to_string());
        Some(format!("⚠ open serial port for monitoring: {port}"))
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let port = args
            .get("port")
            .and_then(|p| p.as_str())
            .map(|s| s.to_string())
            .or_else(|| ctx.monitor_port.clone())
            .ok_or_else(|| {
                ToolError::new(format!(
                    "[InvalidInput] missing port: pass a port parameter (e.g. COM3) or set \
                     monitor_port in [tools] of config.toml. Detected ports: {}",
                    enumerate_ports()
                ))
            })?;
        let baud = match args.get("baud").and_then(|b| b.as_u64()) {
            // u32 cast would silently wrap a nonsense value (2^32 + k → k).
            Some(b) if b <= u32::MAX as u64 => b as u32,
            Some(b) => {
                return Err(ToolError::new(format!(
                    "[InvalidInput] baud {b} is out of range (max {})",
                    u32::MAX
                )));
            }
            None => ctx.monitor_baud,
        };
        let autodetect = args
            .get("autodetect")
            .and_then(|a| a.as_bool())
            .unwrap_or(false);
        let timestamp = args
            .get("timestamp")
            .and_then(|t| t.as_bool())
            .unwrap_or(true);
        let elf: Option<std::path::PathBuf> = args
            .get("elf")
            .and_then(|e| e.as_str())
            .map(|e| resolve_within(&ctx.cwd, e, &ctx.allowed_roots))
            .transpose()
            .map_err(ToolError::new)?;
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|t| t.as_u64())
            .unwrap_or(10_000);

        let port_clone = port.clone();
        let cancel = ctx.cancel.clone();
        let captured = tokio::task::spawn_blocking(move || -> Result<String, String> {
            let baud = if autodetect {
                match detect_baud(&port_clone, 300)? {
                    Some(found) => found,
                    None => {
                        return Ok(format!(
                            "no valid data on {port_clone} at any common baud rate; \
                             pass baud explicitly"
                        ));
                    }
                }
            } else {
                baud
            };
            let text = read_serial(
                &port_clone,
                baud,
                timeout_ms,
                elf.as_deref(),
                timestamp,
                Some(cancel.clone()),
            )?;
            let header = if autodetect {
                format!("monitor {port_clone} (autodetected {baud} baud, {timeout_ms} ms)")
            } else {
                format!("monitor {port_clone} ({baud} baud, {timeout_ms} ms)")
            };
            Ok(format!("{header}\n{text}"))
        })
        .await
        .map_err(|e| ToolError::new(format!("[Io] monitor task failed: {e}")))?
        .map_err(ToolError::new)?;
        Ok(ToolOutput { text: captured })
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

    #[test]
    fn baud_candidates_cover_common_rates_slow_to_fast() {
        assert!(BAUD_CANDIDATES.windows(2).all(|w| w[0] < w[1]));
        assert!(BAUD_CANDIDATES.contains(&115_200));
        assert!(BAUD_CANDIDATES.contains(&9_600));
        assert!(BAUD_CANDIDATES.contains(&921_600));
    }

    #[test]
    fn timestamp_format_is_seconds_and_millis() {
        let elapsed = Duration::from_millis(12_345);
        let ts = format!(
            "[{:02}.{:03}] hello",
            elapsed.as_secs(),
            elapsed.subsec_millis()
        );
        assert_eq!(ts, "[12.345] hello");
        assert!(ts.starts_with('['), "got: {ts}");
    }

    #[test]
    fn port_label_joins_usb_identity_and_falls_back_to_name() {
        let usb = serialport::SerialPortInfo {
            port_name: "COM7".into(),
            port_type: serialport::SerialPortType::UsbPort(serialport::UsbPortInfo {
                vid: 0x0483,
                pid: 0x374e,
                serial_number: Some("SN123".into()),
                manufacturer: Some("STMicroelectronics".into()),
                product: Some("STM32 STLink".into()),
            }),
        };
        assert_eq!(
            port_label(&usb),
            "COM7 (STMicroelectronics STM32 STLink SN123)"
        );
        let plain = serialport::SerialPortInfo {
            port_name: "COM9".into(),
            port_type: serialport::SerialPortType::Unknown,
        };
        assert_eq!(port_label(&plain), "COM9");
    }

    #[test]
    fn enumerate_ports_returns_string() {
        let text = enumerate_ports();
        assert!(!text.is_empty(), "enumerate_ports must never return empty");
    }

    fn ctx(dir: &Path, monitor_port: Option<&str>) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(EditJournal::new(dir.join("undo")))),
            verify_command: None,
            symbols_backend: None,
            build_command: None,
            default_chip: Some("stm32f407vetx".to_string()),
            monitor_port: monitor_port.map(|s| s.to_string()),
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        }
    }

    #[tokio::test]
    async fn missing_port_is_an_error() {
        let dir = tempdir().unwrap();
        let err = Monitor
            .run(json!({}), &ctx(dir.path(), None))
            .await
            .unwrap_err();
        assert!(err.message.contains("monitor_port"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn elf_outside_workspace_is_rejected() {
        let dir = tempdir().unwrap();
        let outside = dir.path().parent().unwrap().join("evil.elf");
        std::fs::write(&outside, b"x").unwrap();
        let err = Monitor
            .run(
                json!({"port": "COM_FAKE", "elf": outside.to_string_lossy()}),
                &ctx(dir.path(), None),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("outside the workspace"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn unopenable_port_returns_error() {
        let dir = tempdir().unwrap();
        let err = Monitor
            .run(
                json!({"port": "COM_DOES_NOT_EXIST_12345"}),
                &ctx(dir.path(), None),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("failed to open serial port"),
            "got: {}",
            err.message
        );
    }

    /// Hands back one byte per `read()` and then behaves like an idle port.
    /// One byte at a time is the worst case for multi-byte characters: every
    /// CJK glyph arrives split three ways.
    struct FakeReader {
        data: Vec<u8>,
        pos: usize,
    }

    impl Read for FakeReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.data.get(self.pos) {
                Some(&b) => {
                    buf[0] = b;
                    self.pos += 1;
                    Ok(1)
                }
                None => {
                    // Emulate the port timeout rather than spinning hot; the
                    // loop re-checks its deadline and stops there.
                    std::thread::sleep(Duration::from_millis(5));
                    Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "no data"))
                }
            }
        }
    }

    fn fake(data: &[u8]) -> FakeReader {
        FakeReader {
            data: data.to_vec(),
            pos: 0,
        }
    }

    #[test]
    fn read_serial_from_reassembles_byte_at_a_time() {
        let text = read_serial_from(&mut fake(b"hello\nworld\n"), 200, None, false, None).unwrap();
        assert_eq!(text, "hello\nworld");
    }

    #[test]
    fn read_serial_from_flushes_trailing_partial_line() {
        let text = read_serial_from(&mut fake(b"a\nb\npartial"), 200, None, false, None).unwrap();
        assert_eq!(text, "a\nb\npartial");
    }

    #[test]
    fn read_serial_from_returns_empty_when_silent() {
        let text = read_serial_from(&mut fake(b""), 60, None, false, None).unwrap();
        assert!(text.is_empty(), "got: {text:?}");
    }

    #[test]
    fn read_serial_from_prefixes_timestamps() {
        let text = read_serial_from(&mut fake(b"hello\n"), 200, None, true, None).unwrap();
        assert!(text.starts_with('['), "got: {text:?}");
        assert!(text.contains("hello"), "got: {text:?}");
    }

    #[test]
    fn read_serial_from_stops_on_cancel() {
        let cancel = firment_core::Cancellable::new();
        cancel.cancel();
        let text =
            read_serial_from(&mut fake(b"forever\n"), 5_000, None, false, Some(cancel)).unwrap();
        assert!(text.contains("interrupted"), "got: {text:?}");
    }

    // -- the regression these tests exist for ------------------------------

    #[test]
    fn read_serial_from_keeps_cjk_split_across_reads() {
        // One byte per read(), so every 3-byte CJK glyph arrives split three
        // ways — the exact shape that produced U+FFFD before.
        let text = read_serial_from(
            &mut fake("传感器就绪\n温度 25.6°C\n".as_bytes()),
            200,
            None,
            false,
            None,
        )
        .unwrap();
        assert_eq!(text, "传感器就绪\n温度 25.6°C");
        assert!(!text.contains('\u{FFFD}'), "got: {text:?}");
    }

    #[test]
    fn read_serial_from_keeps_partial_cjk_tail() {
        // No trailing newline: the final line is flushed by take_tail, and
        // it has to survive the byte-at-a-time delivery intact too.
        let text =
            read_serial_from(&mut fake("启动中".as_bytes()), 200, None, false, None).unwrap();
        assert_eq!(text, "启动中");
        assert!(!text.contains('\u{FFFD}'), "got: {text:?}");
    }

    #[test]
    fn read_serial_from_strips_one_trailing_cr() {
        let text = read_serial_from(&mut fake(b"a\r\nb\r\n"), 200, None, false, None).unwrap();
        assert_eq!(text, "a\nb");
    }
}
