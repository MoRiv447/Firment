use super::util::{resolve_within, truncate};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

pub struct Monitor;

/// How many times a port that died mid-capture is reopened before the capture is
/// given up on. Small on purpose: a probe that was physically unplugged does not
/// come back by being asked politely, and the turn has to end with a report rather
/// than with a hang.
///
/// Public because the CLI's `firm monitor` streams the same port and must not invent
/// a second budget for the same cable: one policy, two loops.
pub const RECONNECT_ATTEMPTS: u32 = 3;

/// Gap between those attempts.
pub const RECONNECT_DELAY: Duration = Duration::from_millis(300);

/// The port timeout, in milliseconds. Long enough that an idle port does not spin the
/// loop, short enough that cancellation is noticed promptly.
pub const PORT_TIMEOUT_MS: u64 = 500;

/// Open a serial port for reading.
///
/// Shared with the CLI so both surfaces open the same way. The timeout is what decides
/// whether silence reads as a timeout (fine: an idle port) or as an error (a reconnect),
/// so two values would mean two behaviours for the same cable.
pub fn open_port(port: &str, baud: u32) -> Result<Box<dyn Read>, String> {
    serialport::new(port, baud)
        .timeout(Duration::from_millis(PORT_TIMEOUT_MS))
        .open()
        .map(|p| Box::new(p) as Box<dyn Read>)
        .map_err(|e| format!("failed to open serial port {port}: {e}"))
}

/// What a capture ended as.
///
/// The lines come back either way, and that is the point: a cable that moves two
/// thirds of the way through a thirty-second capture must not cost the lines that
/// already arrived. A loss is reported *inside* the capture as a marker line, where
/// whoever reads it can see the gap, instead of replacing the capture with an error
/// string.
enum Capture {
    /// The deadline arrived, or the turn was cancelled.
    Ended(String),
    /// The port died. `captured` is everything that arrived before it did.
    Lost { captured: String, reason: String },
}

/// Say something happened, on its own line, in the text the agent will read.
///
/// Plain text rather than a timestamped line, matching the cancellation notice
/// below: this is the tool talking, not the target, and a marker that looks like
/// device output is a marker that gets decoded for symbols.
fn note(text: &mut String, line: &str) {
    // Closed on both sides: the captured chunk that follows a marker is a
    // `lines.join("\n")` with no trailing newline, so a marker without one would end
    // up glued to the first line of the resumed capture -- which is how a timeline
    // test caught this rather than a reader.
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
}

/// Count one reopen attempt, and say whether that was the last one allowed.
///
/// `attempts` is the number already spent, so the first call spends the first of
/// `RECONNECT_ATTEMPTS` and the budget runs out on the call that would exceed it --
/// which is what makes "gave up after N reopen attempts" print N and not N+1.
///
/// Public for the same reason the constants are: the CLI spends from this budget.
pub fn budget_spent(attempts: &mut u32) -> bool {
    if *attempts >= RECONNECT_ATTEMPTS {
        return true;
    }
    *attempts += 1;
    false
}

/// Open the port and read until the deadline, reopening it if the device drops.
fn read_serial(
    port: &str,
    baud: u32,
    timeout_ms: u64,
    elf: Option<&Path>,
    timestamp: bool,
    cancel: Option<firment_core::Cancellable>,
) -> Result<String, String> {
    let mut open = || open_port(port, baud);
    let start = Instant::now();
    let deadline = start + Duration::from_millis(timeout_ms);
    let text = capture_with_reconnect(
        &mut open,
        start,
        deadline,
        elf,
        timestamp,
        cancel,
        RECONNECT_DELAY,
    )?;
    // Only this wrapper knows the port name, so it owns the empty-case text.
    if text.is_empty() {
        Ok(format!("no data received on {port} within {timeout_ms} ms"))
    } else {
        Ok(text)
    }
}

/// Read from whatever `open` hands back, reopening it when the device drops.
///
/// `open` is a parameter rather than a call to `serialport` in here so that the
/// reconnect path can be tested: a real port cannot be opened in CI, and "the cable
/// moved mid-capture" is not a thing that can be arranged on demand with hardware
/// either. `start` and `deadline` are absolute so that every attempt shares one
/// clock -- a reopened capture whose timestamps restarted at `[00.000]` would hide
/// the very gap the marker is there to report.
fn capture_with_reconnect(
    open: &mut dyn FnMut() -> Result<Box<dyn Read>, String>,
    start: Instant,
    deadline: Instant,
    elf: Option<&Path>,
    timestamp: bool,
    cancel: Option<firment_core::Cancellable>,
    reconnect_delay: Duration,
) -> Result<String, String> {
    let mut first = true;
    let mut text = String::new();
    let mut attempts: u32 = 0;
    let mut lost_at: Option<Instant> = None;
    loop {
        // Both arms either return or `continue`, so this is a statement: there is no
        // value to bind, and the port handle is consumed inside the arm that opened it.
        match open() {
            Ok(mut reader) => {
                if attempts > 0 {
                    let gap = lost_at.take().map(|t| t.elapsed()).unwrap_or_default();
                    note(
                        &mut text,
                        &format!(
                            "(serial port back after {attempts} reopen attempt(s), {:.1}s later)",
                            gap.as_secs_f64()
                        ),
                    );
                }
                first = false;
                match read_serial_from(&mut reader, start, deadline, elf, timestamp, cancel.clone())
                {
                    Capture::Ended(chunk) => {
                        text.push_str(&chunk);
                        return Ok(text);
                    }
                    Capture::Lost { captured, reason } => {
                        // Keep the lines FIRST, then decide whether to try again: the
                        // capture is the deliverable, and the reconnect is a courtesy.
                        //
                        // A stretch that delivered lines is evidence the port works, so
                        // the outage ending it is a NEW outage: without this, a cable
                        // replugged four times over one capture loses the fourth to a
                        // budget the first three spent. A stretch that delivered nothing
                        // is the opposite signal -- a port that opens and dies instantly
                        // -- which is the case the budget is for.
                        if !captured.trim().is_empty() {
                            attempts = 0;
                        }
                        text.push_str(&captured);
                        lost_at = Some(Instant::now());
                        if budget_spent(&mut attempts) {
                            note(
                                &mut text,
                                &format!(
                                    "(serial port lost after {attempts} reopen attempt(s): {reason})"
                                ),
                            );
                            return Ok(text);
                        }
                        std::thread::sleep(reconnect_delay);
                        continue;
                    }
                }
            }
            Err(e) => {
                // The first open failing is the ordinary "no such port" case and has
                // nothing to preserve.
                if first {
                    return Err(e);
                }
                lost_at.get_or_insert_with(Instant::now);
                if budget_spent(&mut attempts) {
                    note(
                        &mut text,
                        &format!("(serial port lost after {attempts} reopen attempt(s): {e})"),
                    );
                    return Ok(text);
                }
                std::thread::sleep(reconnect_delay);
            }
        }
    }
}

/// Blocking serial read loop: collect lines until `deadline`, decoding hex
/// code addresses against an ELF when provided. With `timestamp`, each line
/// is prefixed with its arrival time `[SS.mmm]` relative to `start`.
///
/// Split out from [`read_serial`] so tests can drive it from a fake byte
/// source — a real port cannot be opened in CI, and the chunking that
/// breaks multi-byte characters only shows up on a live stream.
fn read_serial_from(
    reader: &mut impl Read,
    start: Instant,
    deadline: Instant,
    elf: Option<&Path>,
    timestamp: bool,
    cancel: Option<firment_core::Cancellable>,
) -> Capture {
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
            // Silence is not a fault: an idle port with nothing to say is the
            // normal case for most of a capture.
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                continue;
            }
            // Everything else is the device going away, and the caller is the only
            // place that can reopen it -- so the lines come back with the reason
            // rather than being dropped for an error string.
            Err(e) => {
                if let Some(tail) = splitter.take_tail() {
                    push_line(&tail, &mut lines, start, timestamp);
                }
                return Capture::Lost {
                    captured: join_capture(lines),
                    reason: e.to_string(),
                };
            }
        }
    }
    if let Some(tail) = splitter.take_tail() {
        push_line(&tail, &mut lines, start, timestamp);
    }
    Capture::Ended(join_capture(lines))
}

/// The captured lines as text, with the forensic marker when they carry a fault
/// signature -- applied to a partial capture too, because a crash is exactly what
/// a port that just disappeared tends to be.
fn join_capture(lines: Vec<String>) -> String {
    let text = lines.join("\n");
    if text.is_empty() {
        return text;
    }
    let text = crate::forensic::append_fault_marker(text);
    truncate(&text, 32_000)
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

    /// A port that dies once its data runs out: the error a yanked USB cable gives,
    /// as opposed to the timeout an idle port gives.
    struct DroppingReader {
        data: Vec<u8>,
        pos: usize,
    }

    impl Read for DroppingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.data.get(self.pos) {
                Some(&b) => {
                    buf[0] = b;
                    self.pos += 1;
                    Ok(1)
                }
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "device disconnected",
                )),
            }
        }
    }

    fn dropping(data: &[u8]) -> DroppingReader {
        DroppingReader {
            data: data.to_vec(),
            pos: 0,
        }
    }

    /// The capture from a read that is expected to end on its own terms.
    fn captured(
        reader: &mut impl Read,
        timeout_ms: u64,
        elf: Option<&Path>,
        timestamp: bool,
        cancel: Option<firment_core::Cancellable>,
    ) -> String {
        let start = Instant::now();
        match read_serial_from(
            reader,
            start,
            start + Duration::from_millis(timeout_ms),
            elf,
            timestamp,
            cancel,
        ) {
            Capture::Ended(text) => text,
            Capture::Lost { captured, reason } => {
                panic!("the port must not drop here: {reason} (captured {captured:?})")
            }
        }
    }
    #[test]
    fn read_serial_from_reassembles_byte_at_a_time() {
        let text = captured(&mut fake(b"hello\nworld\n"), 200, None, false, None);
        assert_eq!(text, "hello\nworld");
    }

    #[test]
    fn read_serial_from_flushes_trailing_partial_line() {
        let text = captured(&mut fake(b"a\nb\npartial"), 200, None, false, None);
        assert_eq!(text, "a\nb\npartial");
    }

    #[test]
    fn read_serial_from_returns_empty_when_silent() {
        let text = captured(&mut fake(b""), 60, None, false, None);
        assert!(text.is_empty(), "got: {text:?}");
    }

    #[test]
    fn read_serial_from_prefixes_timestamps() {
        let text = captured(&mut fake(b"hello\n"), 200, None, true, None);
        assert!(text.starts_with('['), "got: {text:?}");
        assert!(text.contains("hello"), "got: {text:?}");
    }

    #[test]
    fn read_serial_from_stops_on_cancel() {
        let cancel = firment_core::Cancellable::new();
        cancel.cancel();
        let text = captured(&mut fake(b"forever\n"), 5_000, None, false, Some(cancel));
        assert!(text.contains("interrupted"), "got: {text:?}");
    }

    // -- the regression these tests exist for ------------------------------

    #[test]
    fn read_serial_from_keeps_cjk_split_across_reads() {
        // One byte per read(), so every 3-byte CJK glyph arrives split three
        // ways — the exact shape that produced U+FFFD before.
        let text = captured(
            &mut fake("传感器就绪\n温度 25.6°C\n".as_bytes()),
            200,
            None,
            false,
            None,
        );
        assert_eq!(text, "传感器就绪\n温度 25.6°C");
        assert!(!text.contains('\u{FFFD}'), "got: {text:?}");
    }

    #[test]
    fn read_serial_from_keeps_partial_cjk_tail() {
        // No trailing newline: the final line is flushed by take_tail, and
        // it has to survive the byte-at-a-time delivery intact too.
        let text = captured(&mut fake("启动中".as_bytes()), 200, None, false, None);
        assert_eq!(text, "启动中");
        assert!(!text.contains('\u{FFFD}'), "got: {text:?}");
    }

    #[test]
    fn read_serial_from_strips_one_trailing_cr() {
        let text = captured(&mut fake(b"a\r\nb\r\n"), 200, None, false, None);
        assert_eq!(text, "a\nb");
    }

    // -- a cable that moves mid-capture -------------------------------------
    //
    // None of this can be arranged with hardware on demand, which is why `open` is a
    // parameter of `capture_with_reconnect`: the reader that drops is a fake, and the
    // reconnect is a second fake, so "the port came back" is a thing a test can assert.

    #[test]
    fn a_dropped_port_is_reopened_and_the_capture_survives() {
        let mut opens = 0;
        let mut open = || {
            opens += 1;
            Ok(if opens == 1 {
                Box::new(dropping(b"before\n")) as Box<dyn Read>
            } else {
                // Back, and then idle until the deadline ends the capture.
                Box::new(fake(b"after\n")) as Box<dyn Read>
            })
        };
        let start = Instant::now();
        let text = capture_with_reconnect(
            &mut open,
            start,
            start + Duration::from_millis(200),
            None,
            false,
            None,
            Duration::ZERO,
        )
        .unwrap();

        assert_eq!(opens, 2, "one reopen, not a loop of them");
        // By line, not by `contains`: the marker's own wording ("back after 1 reopen
        // attempt") holds the word the resumed line would be searched for, and a
        // substring test would happily find that instead and report the order as
        // broken. What matters is the order, so the assertion is on the order.
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "got: {text:?}");
        assert_eq!(lines[0], "before");
        assert!(
            lines[1].contains("back after 1 reopen attempt"),
            "the gap marker sits between the halves: {text:?}"
        );
        assert_eq!(lines[2], "after");
    }

    #[test]
    fn a_port_that_stays_gone_is_reported_rather_than_hung_on() {
        let mut opens = 0;
        let mut open = || {
            opens += 1;
            if opens == 1 {
                Ok(Box::new(dropping(b"only line\n")) as Box<dyn Read>)
            } else {
                Err("failed to open serial port COM9: not found".to_string())
            }
        };
        let start = Instant::now();
        let text = capture_with_reconnect(
            &mut open,
            start,
            start + Duration::from_millis(200),
            None,
            false,
            None,
            Duration::ZERO,
        )
        .unwrap();

        // What arrived before the cable moved is still the answer, and the reason it
        // stops there is in the text rather than in a returned error.
        assert!(text.contains("only line"), "got: {text:?}");
        assert!(
            text.contains("lost after 3 reopen attempt(s)"),
            "got: {text:?}"
        );
        assert_eq!(
            opens,
            1 + RECONNECT_ATTEMPTS as usize,
            "the budget is the budget"
        );
    }

    #[test]
    fn a_port_that_keeps_working_gets_a_fresh_budget_each_time() {
        // Five replugs over one capture, each followed by real output. Every stretch
        // that delivered lines is evidence the port works, so each new outage starts
        // from a full budget -- otherwise the fourth replug is lost to a budget the
        // first three spent, which is the capture failing at the thing it is for.
        let mut opens = 0;
        let mut open = || {
            opens += 1;
            Ok(if opens <= 5 {
                let line = format!("line {opens}\n");
                Box::new(dropping(line.as_bytes())) as Box<dyn Read>
            } else {
                Box::new(fake(b"")) as Box<dyn Read>
            })
        };
        let start = Instant::now();
        let text = capture_with_reconnect(
            &mut open,
            start,
            start + Duration::from_millis(300),
            None,
            false,
            None,
            Duration::ZERO,
        )
        .unwrap();

        for n in 1..=5 {
            assert!(
                text.contains(&format!("line {n}")),
                "line {n} missing: {text:?}"
            );
        }
        assert!(
            !text.contains("lost after"),
            "the port kept working: {text:?}"
        );
        assert_eq!(opens, 6, "five replugs plus the final quiet read");
    }

    #[test]
    fn a_port_that_never_delivers_still_runs_out_of_budget() {
        // The counter-example to the test above: the budget has to survive a port that
        // opens successfully and dies before a single line arrives, or a wedged adapter
        // would be reopened every 300ms until the capture's deadline.
        let mut opens = 0;
        let mut open = || {
            opens += 1;
            Ok(Box::new(dropping(b"")) as Box<dyn Read>)
        };
        let start = Instant::now();
        let text = capture_with_reconnect(
            &mut open,
            start,
            start + Duration::from_millis(300),
            None,
            false,
            None,
            Duration::ZERO,
        )
        .unwrap();

        assert!(
            text.contains("lost after 3 reopen attempt(s)"),
            "got: {text:?}"
        );
        assert_eq!(
            opens,
            1 + RECONNECT_ATTEMPTS as usize,
            "the budget is the budget"
        );
    }

    #[test]
    fn an_idle_port_is_not_a_disconnect() {
        let mut opens = 0;
        let mut open = || {
            opens += 1;
            Ok(Box::new(fake(b"")) as Box<dyn Read>)
        };
        let start = Instant::now();
        let text = capture_with_reconnect(
            &mut open,
            start,
            start + Duration::from_millis(60),
            None,
            false,
            None,
            Duration::ZERO,
        )
        .unwrap();

        assert!(text.is_empty(), "got: {text:?}");
        // The distinction the whole feature rests on: a timeout is an idle port, and
        // a capture where nothing was said must not look like a capture that failed.
        assert_eq!(opens, 1, "silence is not a disconnect");
    }

    #[test]
    fn a_reopened_capture_keeps_one_timeline() {
        let mut opens = 0;
        let mut open = || {
            opens += 1;
            Ok(if opens == 1 {
                Box::new(dropping(b"early\n")) as Box<dyn Read>
            } else {
                Box::new(fake(b"resumed\n")) as Box<dyn Read>
            })
        };
        // Started five seconds ago, so a second attempt that restarted its own clock
        // would print `[00.` for the resumed line -- and hide the gap the marker is
        // there to report.
        let start = Instant::now() - Duration::from_secs(5);
        let text = capture_with_reconnect(
            &mut open,
            start,
            start + Duration::from_secs(6),
            None,
            true,
            None,
            Duration::ZERO,
        )
        .unwrap();

        // Taken by position, not by search: the marker's own wording ends in
        // "...s later", so `contains("late")` finds the marker -- and a substring test
        // that can match the annotation instead of the data reports on the wrong line.
        let late = text
            .lines()
            .last()
            .expect("a capture with the resumed line");
        assert!(
            late.contains("resumed"),
            "the last line is what the reopened port delivered: {text:?}"
        );
        // Parsed, not prefix-matched: the point is that the clock did not restart, and
        // the exact second depends on how loaded the machine is when the suite runs
        // (`[05.`, `[06.`, ... are all the same answer). A restarted clock would print
        // something under a second, which is what this rules out.
        let stamp: f64 = late
            .trim_start_matches('[')
            .split(']')
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("the resumed line carries a timestamp: {late:?}"));
        assert!(
            stamp >= 5.0,
            "the clock must not restart after a reopen: {late:?}"
        );
    }
}
