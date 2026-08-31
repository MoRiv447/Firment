//! sigrok-cli command construction and output parsing for the `la` tool —
//! the pure half. Nothing here spawns a process; `tools/la.rs` feeds the
//! argv arrays to `util::run_argv` and the raw bytes/text back to these
//! parsers.
//!
//! sigrok-cli is invoked as an EXTERNAL BINARY with an argv array (never a
//! shell string, never a linked library): the GPL of sigrok/libsigrok does
//! not reach an MIT program that merely exec()s it and reads stdout, the
//! same way `probe-rs` is used as an external CLI. Do NOT add a libsigrok
//! crate dependency — that WOULD link GPL code into this binary.
//!
//! The `.sr` session files sigrok writes are a GPL-owned container format:
//! they are archived for the user's own PulseView review, never parsed
//! here. Measurements read the `-O binary` sidecar instead.

use std::collections::BTreeMap;
use std::path::Path;

/// One capture request: driver + optional channel list + optional sample
/// rate, bounded by either a sample count or a wall-clock window.
#[derive(Debug, Clone)]
pub struct CaptureRequest {
    pub driver: String,
    /// sigrok channel spec, e.g. `"0,1,2-3"` or `"0=SCLK,1=MOSI"`.
    pub channels: Option<String>,
    /// sigrok samplerate token, e.g. `"8m"` (8 MHz), `"100k"`.
    pub samplerate: Option<String>,
    pub samples: Option<u64>,
    pub time_ms: Option<u64>,
}

/// Validate a driver/decoder token: plain command-token characters only,
/// no leading dash (it would read as a flag). Reuses the same rules as
/// chip/probe ids.
pub fn sanitize_driver(value: &str) -> Result<String, String> {
    crate::tools::util::token_arg(value, "driver")
}

/// Validate a sigrok channel spec: letters, digits, `,` `-` `=` (ranges and
/// labels), no spaces, no leading dash.
pub fn sanitize_channels(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err("[InvalidInput] channels must not be empty".to_string());
    }
    if value.starts_with('-') {
        return Err("[InvalidInput] channels must not start with '-'".to_string());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ',' | '-' | '=' | '_'))
    {
        return Err(
            "[InvalidInput] channels may only contain letters, digits and `,` `-` `=` `_` \
             (e.g. \"0,1,2-3\" or \"0=SCLK,1=MOSI\")"
                .to_string(),
        );
    }
    Ok(value.to_string())
}

/// Validate a sigrok samplerate token: `8000000`, `8m`, `100k`, `1.6m` …
pub fn sanitize_samplerate(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let digits: &[u8] = match bytes.last() {
        Some(b'k' | b'K' | b'm' | b'M' | b'g' | b'G') => &bytes[..bytes.len() - 1],
        _ => bytes,
    };
    let malformed = || {
        Err(
            "[InvalidInput] samplerate must be a number with an optional k/m/g suffix \
             (e.g. 8000000 or 8m)"
                .to_string(),
        )
    };
    if digits.is_empty() {
        return malformed();
    }
    let mut dots = 0usize;
    for b in digits {
        match b {
            b'.' => dots += 1,
            d if d.is_ascii_digit() => {}
            _ => return malformed(),
        }
    }
    if dots > 1 || digits[0] == b'.' || digits[digits.len() - 1] == b'.' {
        return malformed();
    }
    Ok(value.to_string())
}

/// Validate a decoder option key: bare identifier (`rx`, `baudrate`, …).
fn sanitize_opt_key(value: &str) -> Result<String, String> {
    if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        Ok(value.to_string())
    } else {
        Err(format!(
            "[InvalidInput] decoder option key '{value}' must be letters, digits and `_`"
        ))
    }
}

/// Validate a decoder option value: token characters plus `:` (channel
/// specs like `rx=0:tx=1` are spelled per-option by sigrok, but values such
/// as `1:2` appear in some decoders).
fn sanitize_opt_value(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err("[InvalidInput] decoder option value must not be empty".to_string());
    }
    if value.starts_with('-') {
        return Err("[InvalidInput] decoder option value must not start with '-'".to_string());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/'))
    {
        return Err(format!(
            "[InvalidInput] decoder option value '{value}' contains characters that are not \
             allowed in a command token"
        ));
    }
    Ok(value.to_string())
}

/// Build the argv for a capture: `-d <driver> [-C <ch>] [-c samplerate=<sr>]
/// (--samples N | --time MS) -o <out.sr>`.
///
/// One capture, one `.sr` session file — sigrok writes exactly one output
/// format per invocation, and capturing twice would measure two different
/// windows. The measurement layer gets its raw bits by EXPORTING the stored
/// session ([`build_export_binary_argv`]), and the protocol decoder reads
/// the `.sr` directly.
pub fn build_capture_argv(req: &CaptureRequest, out_sr: &Path) -> Result<Vec<String>, String> {
    let mut argv = vec!["-d".to_string(), sanitize_driver(&req.driver)?];
    if let Some(ch) = &req.channels {
        argv.push("-C".to_string());
        argv.push(sanitize_channels(ch)?);
    }
    if let Some(sr) = &req.samplerate {
        argv.push("-c".to_string());
        argv.push(format!("samplerate={}", sanitize_samplerate(sr)?));
    }
    match (req.samples, req.time_ms) {
        (Some(n), _) => {
            argv.push("--samples".to_string());
            argv.push(n.to_string());
        }
        (None, Some(t)) => {
            argv.push("--time".to_string());
            argv.push(t.to_string());
        }
        (None, None) => {
            return Err(
                "[InvalidInput] capture needs a bound: 'samples' or 'time_ms' — an unbounded \
                 capture never returns"
                    .to_string(),
            );
        }
    }
    argv.push("-o".to_string());
    argv.push(out_sr.display().to_string());
    Ok(argv)
}

/// Build the argv that exports a stored `.sr` session to raw bits
/// (`-i <capture.sr> -O binary -o <capture.bin>`) for the measurement layer.
pub fn build_export_binary_argv(capture_sr: &Path, out_bin: &Path) -> Vec<String> {
    vec![
        "-i".to_string(),
        capture_sr.display().to_string(),
        "-O".to_string(),
        "binary".to_string(),
        "-o".to_string(),
        out_bin.display().to_string(),
    ]
}

/// Build the argv for a protocol decode: `-i <capture.sr> -P <decoder>:k=v…`.
/// sigrok decoders read the `.sr` session file, so this one takes the sr
/// path, not the binary sidecar.
pub fn build_decode_argv(
    capture_sr: &Path,
    decoder: &str,
    opts: &BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    let mut spec = crate::tools::util::token_arg(decoder, "decoder")?;
    for (k, v) in opts {
        spec.push(':');
        spec.push_str(&sanitize_opt_key(k)?);
        spec.push('=');
        spec.push_str(&sanitize_opt_value(v)?);
    }
    Ok(vec![
        "-i".to_string(),
        capture_sr.display().to_string(),
        "-P".to_string(),
        spec,
    ])
}

/// Build the argv for device capability info: `-d <driver> --show`.
pub fn build_info_argv(driver: &str) -> Result<Vec<String>, String> {
    Ok(vec![
        "-d".to_string(),
        sanitize_driver(driver)?,
        "--show".to_string(),
    ])
}

/// The version probe argv: `--version`.
pub fn version_argv() -> Vec<String> {
    vec!["--version".to_string()]
}

/// The driver-list probe argv: `-L`.
pub fn drivers_argv() -> Vec<String> {
    vec!["-L".to_string()]
}

/// Parse `sigrok-cli 0.7.2` (or `0.8.0-rc1`) into a triple. Returns `None`
/// for anything that is not a sigrok-cli banner, so the caller can refuse
/// an unknown binary instead of guessing flags at it.
pub fn parse_sigrok_version(text: &str) -> Option<(u8, u8, u8)> {
    let line = text.lines().next()?.trim();
    let rest = line.strip_prefix("sigrok-cli")?.trim();
    let core = rest.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().unwrap_or(0);
    Some((major, minor, patch))
}

/// Unpack sigrok-cli's `-O binary` output into per-channel 0/1 vectors.
///
/// sigrok packs each SAMPLE GROUP as `ceil(n_channels / 8)` bytes with
/// channel 0 in the LSB of the first byte, channel 8 in the LSB of the
/// second, and so on. This layout is locked by the tests below; if a real
/// device disagrees, THIS function is the single place to fix — the
/// analysers only ever see the unpacked vectors.
pub fn unpack_bitstream(bytes: &[u8], n_channels: usize) -> Result<Vec<Vec<u8>>, String> {
    if n_channels == 0 {
        return Err("[InvalidInput] unpack needs at least one channel".to_string());
    }
    if n_channels > 512 {
        return Err("[InvalidInput] implausible channel count".to_string());
    }
    let group = n_channels.div_ceil(8);
    // The modulo form on purpose: `usize::is_multiple_of` (what clippy
    // suggests) stabilised in 1.87, past this workspace's declared MSRV 1.85.
    #[allow(clippy::manual_is_multiple_of)]
    if bytes.len() % group != 0 {
        return Err(format!(
            "[Io] truncated bitstream: {} bytes is not a whole number of {group}-byte sample \
             groups",
            bytes.len()
        ));
    }
    let n_samples = bytes.len() / group;
    let mut channels: Vec<Vec<u8>> = (0..n_channels)
        .map(|_| Vec::with_capacity(n_samples))
        .collect();
    for i in 0..n_samples {
        for ch in 0..n_channels {
            let byte = bytes[i * group + ch / 8];
            channels[ch].push((byte >> (ch % 8)) & 1);
        }
    }
    Ok(channels)
}

/// One decoded protocol annotation line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdFrame {
    pub decoder: String,
    pub text: String,
}

/// Parse sigrok-cli's annotation output (`-P …` prints `decoder: payload`
/// lines to stdout). Unknown shapes are skipped, not fatal: progress noise
/// and future decoder output formats must not break a capture that decoded
/// fine.
pub fn parse_pd_annotations(stdout: &str) -> Vec<PdFrame> {
    let mut frames = Vec::new();
    for line in stdout.lines() {
        let line = line.trim_end_matches('\r');
        let Some((decoder, text)) = line.split_once(": ") else {
            continue;
        };
        if decoder.is_empty()
            || text.is_empty()
            || !decoder
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic())
            || !decoder
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            continue;
        }
        frames.push(PdFrame {
            decoder: decoder.to_string(),
            text: text.to_string(),
        });
    }
    frames
}

/// Map a raw sigrok-cli invocation failure to an actionable message. The
/// input is the combined stdout+stderr (or the spawn error text).
pub fn sigrok_err_hint(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    if raw.contains("spawn failed") {
        return "[NotFound] sigrok-cli is not installed or not on PATH. Windows: grab the \
                self-extracting package from sigrok.org/download (a CN mirror such as \
                ghproxy-style mirrors works when sigrok.org is slow) and either add it to \
                PATH or point [tools.la] bin at sigrok-cli.exe; Linux: your distro packages \
                it (apt install sigrok-cli); macOS: brew install sigrok-cli.\n{raw}"
            .to_string();
    }
    if lower.contains("device busy")
        || lower.contains("resource busy")
        || lower.contains("usb_open error")
        || lower.contains("libusb")
    {
        return format!(
            "[Io] cannot open the capture device: its USB session is still busy (PulseView or a \
             previous sigrok-cli that did not close it cleanly). Close those, unplug and replug \
             the analyzer, then retry.\n{raw}"
        );
    }
    if lower.contains("no driver")
        || lower.contains("driver not found")
        || lower.contains("unknown driver")
    {
        return format!(
            "[InvalidInput] sigrok-cli has no driver by that name — run `la` action=detect to \
             see what this build supports.\n{raw}"
        );
    }
    if lower.contains("unknown") && lower.contains("key") {
        return format!(
            "[InvalidInput] sigrok rejected a config/decoder key — run `la` action=info to see \
             what this device and decoder accept.\n{raw}"
        );
    }
    format!("[Io] {raw}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn req() -> CaptureRequest {
        CaptureRequest {
            driver: "fx2lafw".to_string(),
            channels: Some("0,1,2-3".to_string()),
            samplerate: Some("8m".to_string()),
            samples: Some(1_000_000),
            time_ms: None,
        }
    }

    #[test]
    fn capture_argv_is_a_plain_array() {
        let argv = build_capture_argv(&req(), &PathBuf::from("c.sr")).unwrap();
        assert_eq!(
            argv,
            vec![
                "-d",
                "fx2lafw",
                "-C",
                "0,1,2-3",
                "-c",
                "samplerate=8m",
                "--samples",
                "1000000",
                "-o",
                "c.sr",
            ]
        );
    }

    #[test]
    fn capture_argv_time_variant_and_omissions() {
        let r = CaptureRequest {
            driver: "demo".to_string(),
            channels: None,
            samplerate: None,
            samples: None,
            time_ms: Some(500),
        };
        let argv = build_capture_argv(&r, &PathBuf::from("d.sr")).unwrap();
        assert_eq!(argv, vec!["-d", "demo", "--time", "500", "-o", "d.sr"]);
    }

    #[test]
    fn binary_export_reads_the_stored_session() {
        assert_eq!(
            build_export_binary_argv(&PathBuf::from("c.sr"), &PathBuf::from("c.bin")),
            vec!["-i", "c.sr", "-O", "binary", "-o", "c.bin"]
        );
    }

    #[test]
    fn unbounded_capture_is_refused() {
        let r = CaptureRequest {
            driver: "fx2lafw".to_string(),
            channels: None,
            samplerate: None,
            samples: None,
            time_ms: None,
        };
        let err = build_capture_argv(&r, &PathBuf::from("x.sr")).unwrap_err();
        assert!(err.contains("[InvalidInput]"), "got: {err}");
    }

    #[test]
    fn injection_never_reaches_argv() {
        let r = CaptureRequest {
            driver: "fx2lafw; rm -rf /".to_string(),
            ..req()
        };
        assert!(build_capture_argv(&r, &PathBuf::from("x.sr")).is_err());
        let r = CaptureRequest {
            channels: Some("0 1 --evil".to_string()),
            ..req()
        };
        assert!(build_capture_argv(&r, &PathBuf::from("x.sr")).is_err());
        let r = CaptureRequest {
            samplerate: Some("8m; ls".to_string()),
            ..req()
        };
        assert!(build_capture_argv(&r, &PathBuf::from("x.sr")).is_err());
        // A leading dash would read as a flag even without shell magic.
        assert!(sanitize_driver("--evil").is_err());
        assert!(sanitize_channels("-5").is_err());
    }

    #[test]
    fn samplerate_forms_accepted() {
        for ok in ["8000000", "8m", "100k", "1.6m", "2G"] {
            assert_eq!(sanitize_samplerate(ok).unwrap(), ok, "{ok} must pass");
        }
        for bad in ["", "m", "8 m", "eight", "-8m", "8mm", "1.2.3m"] {
            assert!(sanitize_samplerate(bad).is_err(), "{bad} must fail");
        }
    }

    #[test]
    fn decode_argv_splices_validated_options() {
        let mut opts = BTreeMap::new();
        opts.insert("rx".to_string(), "0".to_string());
        opts.insert("baudrate".to_string(), "115200".to_string());
        let argv = build_decode_argv(&PathBuf::from("cap.sr"), "uart", &opts).unwrap();
        assert_eq!(argv[0], "-i");
        assert_eq!(argv[2], "-P");
        assert_eq!(argv[3], "uart:baudrate=115200:rx=0");
        // A hostile option value cannot smuggle a flag.
        opts.insert("rx".to_string(), "--evil".to_string());
        assert!(build_decode_argv(&PathBuf::from("cap.sr"), "uart", &opts).is_err());
    }

    #[test]
    fn version_banner_parses_and_garbage_refuses() {
        assert_eq!(parse_sigrok_version("sigrok-cli 0.7.2\n"), Some((0, 7, 2)));
        assert_eq!(
            parse_sigrok_version("sigrok-cli 0.8.0-rc1  (git …)\n"),
            Some((0, 8, 0))
        );
        assert_eq!(parse_sigrok_version("sigrok-cli 0.7\n"), Some((0, 7, 0)));
        assert_eq!(parse_sigrok_version("some other tool 1.2.3"), None);
        assert_eq!(parse_sigrok_version(""), None);
    }

    #[test]
    fn bitstream_lsb_first_channel_major_ordering_is_locked() {
        // 2 channels, 4 samples: byte 0b01 = ch0 high, byte 0b10 = ch1 high.
        let bytes = [0b01u8, 0b10, 0b11, 0b00];
        let ch = unpack_bitstream(&bytes, 2).unwrap();
        assert_eq!(ch[0], vec![1, 0, 1, 0]);
        assert_eq!(ch[1], vec![0, 1, 1, 0]);
    }

    #[test]
    fn bitstream_multi_byte_groups() {
        // 9 channels: two bytes per sample. Sample 0: ch0 and ch8 high.
        let bytes = [0b0000_0001u8, 0b0000_0001, 0, 0];
        let ch = unpack_bitstream(&bytes, 9).unwrap();
        assert_eq!(ch[0], vec![1, 0]);
        assert_eq!(ch[8], vec![1, 0]);
        assert_eq!(ch[4], vec![0, 0]);
    }

    #[test]
    fn truncated_bitstream_is_an_error_not_a_panic() {
        // 9 channels need 2-byte sample groups: one byte is half a sample.
        assert!(unpack_bitstream(&[0], 9).is_err());
        assert!(unpack_bitstream(&[], 1).is_ok()); // zero samples is fine
        assert!(unpack_bitstream(&[0], 0).is_err());
    }

    #[test]
    fn annotations_parse_leniently() {
        let stdout = "uart: TX: (0x55)\nuart: RX: (0xAA)\nnot an annotation line\n\
                      12345: bogus decoder name\n";
        let frames = parse_pd_annotations(stdout);
        assert_eq!(frames.len(), 2, "got {frames:?}");
        assert_eq!(frames[0].decoder, "uart");
        assert_eq!(frames[0].text, "TX: (0x55)");
    }

    #[test]
    fn error_hints_route_to_the_right_advice() {
        assert!(sigrok_err_hint("spawn failed: program not found").contains("[NotFound]"));
        assert!(sigrok_err_hint("Device busy.").contains("replug"));
        assert!(sigrok_err_hint("No driver for foo.").contains("detect"));
        assert!(sigrok_err_hint("Unknown config key: xyz").contains("info"));
        assert!(sigrok_err_hint("something else entirely").starts_with("[Io]"));
    }
}
