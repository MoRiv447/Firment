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

/// Validate a driver token, optionally with sigrok's inline config after a
/// colon: `fx2lafw`, `demo`, `demo:logic-channels=8`. The name part is a
/// plain command token (no leading dash); the config part may carry
/// `key=value` pairs — the `=` is what `token_arg` alone would reject, and
/// without it the hardware-free `demo` driver (and CI) is unreachable.
pub fn sanitize_driver(value: &str) -> Result<String, String> {
    match value.split_once(':') {
        Some((driver, config)) => {
            crate::tools::util::token_arg(driver, "driver")?;
            if config.is_empty()
                || !config.bytes().all(|b| {
                    b.is_ascii_alphanumeric()
                        || matches!(b, b'-' | b'_' | b'.' | b'/' | b'=' | b',' | b';')
                })
            {
                return Err(format!(
                    "[InvalidInput] driver inline config '{config}' after ':' must be \
                     key=value pairs (letters, digits and - _ . / = , ;)"
                ));
            }
            Ok(value.to_string())
        }
        None => crate::tools::util::token_arg(value, "driver"),
    }
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

/// Numeric Hz behind a validated samplerate token (`8m` -> 8e6). `None` for
/// malformed input — the measurement layer refuses to divide by a guess.
pub fn samplerate_hz(token: &str) -> Option<f64> {
    let (num, mult) = match token.as_bytes().last()? {
        b'k' | b'K' => (&token[..token.len() - 1], 1e3),
        b'm' | b'M' => (&token[..token.len() - 1], 1e6),
        b'g' | b'G' => (&token[..token.len() - 1], 1e9),
        _ => (token, 1.0),
    };
    num.parse::<f64>()
        .ok()
        .map(|v| v * mult)
        .filter(|v| *v > 0.0)
}

/// Number of channels a sigrok channel spec selects: `0,1,2-3` -> 4,
/// `D0,D1` -> 2, `0=SCLK,1=MOSI` -> 2. `None` on malformed input (the
/// unpack layer needs an exact count; guessing one would mis-decode every
/// sample).
///
/// sigrok-cli matches channels BY NAME — and the mainstream drivers name
/// them `D0..D7` (fx2lafw, demo), not `0..7`. So a non-numeric item is a
/// named channel and counts as one; only ranges (`2-5`) require numbers,
/// and — like sigrok's own parser — a range must be strict (`lo < hi`).
pub fn count_channels(spec: &str) -> Option<usize> {
    let mut total = 0usize;
    for item in spec.split(',') {
        let item = item.split('=').next()?.trim();
        if item.is_empty() {
            return None;
        }
        if let Some((lo, hi)) = item.split_once('-') {
            let lo: usize = lo.trim().parse().ok()?;
            let hi: usize = hi.trim().parse().ok()?;
            if hi <= lo {
                return None;
            }
            // checked all the way: `0-18446744073709551615` would overflow
            // even the width of the range itself.
            total = total.checked_add(hi.checked_sub(lo)?.checked_add(1)?)?;
        } else if item.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            // numeric index or named channel — either way, one line
            total = total.checked_add(1)?;
        } else {
            return None;
        }
        // Beyond this the bitstream unpacking is implausible anyway; also
        // caps `0-18446744073709551615` style overflow attempts.
        if total > 512 {
            return None;
        }
    }
    if total == 0 { None } else { Some(total) }
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

/// Strip sigrok's optional sample-range prefix: with an elevated loglevel
/// annotation lines arrive as `1000-1008 uart: 0x55`.
fn strip_sample_prefix(line: &str) -> &str {
    let Some(sp) = line.find(' ') else {
        return line;
    };
    let head = &line[..sp];
    match head.split_once('-') {
        Some((a, b))
            if !a.is_empty()
                && !b.is_empty()
                && a.bytes().all(|c| c.is_ascii_digit())
                && b.bytes().all(|c| c.is_ascii_digit()) =>
        {
            &line[sp + 1..]
        }
        _ => line,
    }
}

/// Parse sigrok-cli's annotation output (`-P …` prints `decoder: payload`
/// lines to stdout). Unknown shapes are skipped, not fatal — but libsigrok
/// LOG lines (`sr: …`, `sigrok-cli: …`) are filtered explicitly: they share
/// the `word: text` shape and would otherwise masquerade as decoded frames.
pub fn parse_pd_annotations(stdout: &str) -> Vec<PdFrame> {
    let mut frames = Vec::new();
    for line in stdout.lines() {
        let line = strip_sample_prefix(line.trim_end_matches('\r').trim_start());
        if line.starts_with("sr:") || line.starts_with("sigrok-cli") {
            continue;
        }
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

/// Build the argv that inspects a stored session: `-i <capture.sr> --show`.
/// sigrok-cli reports the session's REAL channel count and samplerate here —
/// the device may have snapped the requested rate to the nearest supported
/// one, and measuring against the requested value would bias every
/// frequency.
pub fn build_session_show_argv(capture_sr: &Path) -> Vec<String> {
    vec![
        "-i".to_string(),
        capture_sr.display().to_string(),
        "--show".to_string(),
    ]
}

/// What `--show` revealed about a stored session.
#[derive(Debug, Clone, Copy, Default)]
pub struct SessionInfo {
    pub channels: Option<usize>,
    pub samplerate_hz: Option<f64>,
}

/// Lenient parse of `sigrok-cli -i … --show` output: the first integer
/// after `Channels:` and after `Samplerate`. Anything unrecognized yields
/// `None` — the caller falls back to the requested values and says so.
pub fn parse_session_show(text: &str) -> SessionInfo {
    let mut info = SessionInfo::default();
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if info.channels.is_none()
            && let Some(pos) = lower.find("channels:")
        {
            let rest = line[pos + "channels:".len()..].trim_start();
            let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            info.channels = num.parse().ok().filter(|n: &usize| *n > 0);
        }
        if info.samplerate_hz.is_none()
            && let Some(pos) = lower.find("samplerate")
        {
            let rest = &line[pos..];
            let run: String = rest
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(v) = run.parse::<f64>()
                && v > 0.0
            {
                info.samplerate_hz = Some(v);
            }
        }
    }
    info
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
    fn samplerate_tokens_become_hz() {
        assert_eq!(samplerate_hz("8m"), Some(8e6));
        assert_eq!(samplerate_hz("100k"), Some(1e5));
        assert_eq!(samplerate_hz("1.6m"), Some(1.6e6));
        assert_eq!(samplerate_hz("2000000"), Some(2e6));
        assert_eq!(samplerate_hz("8x"), None);
        assert_eq!(samplerate_hz(""), None);
    }

    #[test]
    fn channel_specs_count_exactly() {
        assert_eq!(count_channels("0,1,2-3"), Some(4));
        assert_eq!(count_channels("0=SCLK,1=MOSI"), Some(2));
        assert_eq!(count_channels("0-7"), Some(8));
        assert_eq!(count_channels("3"), Some(1));
        // sigrok-cli matches channels BY NAME and the mainstream drivers
        // name them D0..D7 — named items must count, not be refused.
        assert_eq!(count_channels("D0,D1"), Some(2));
        assert_eq!(
            count_channels("D0-D7"),
            None,
            "ranges are numeric-only in sigrok"
        );
        // Malformed specs must NOT guess a count — a wrong count mis-decodes
        // every sample downstream.
        assert_eq!(count_channels("5-2"), None);
        assert_eq!(count_channels("3-3"), None, "sigrok ranges are strict");
        assert_eq!(count_channels(""), None);
        assert_eq!(count_channels("a,b"), Some(2), "named channels are legal");
        // Overflow attempt: the range width alone would wrap a usize.
        assert_eq!(count_channels("0-18446744073709551615"), None);
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
        // Default sigrok-cli output has no TX/RX words — just `decoder: payload`.
        let stdout = "uart: 0x55\nuart: 0xAA\nnot an annotation line\n\
                      12345: bogus decoder name\n";
        let frames = parse_pd_annotations(stdout);
        assert_eq!(frames.len(), 2, "got {frames:?}");
        assert_eq!(frames[0].decoder, "uart");
        assert_eq!(frames[0].text, "0x55");
    }

    #[test]
    fn annotations_strip_sample_prefix_and_filter_log_noise() {
        // Elevated loglevel prefixes annotation lines with the sample range…
        let stdout = "1000-1008 uart: 0x55\n";
        let frames = parse_pd_annotations(stdout);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].decoder, "uart");
        assert_eq!(frames[0].text, "0x55");
        // …and libsigrok log lines share the `word: text` shape — they must
        // not masquerade as decoded frames.
        let noisy = "sr: uart: some internal log\nsigrok-cli: starting session\nuart: 0x41\n";
        let frames = parse_pd_annotations(noisy);
        assert_eq!(frames.len(), 1, "got {frames:?}");
        assert_eq!(frames[0].text, "0x41");
    }

    #[test]
    fn driver_inline_config_is_allowed_after_a_colon() {
        // demo:logic-channels=8 is the hardware-free CI path; plain token_arg
        // would reject the '='.
        assert_eq!(
            sanitize_driver("demo:logic-channels=8").unwrap(),
            "demo:logic-channels=8"
        );
        assert_eq!(sanitize_driver("fx2lafw").unwrap(), "fx2lafw");
        assert!(sanitize_driver("demo:").is_err(), "empty config");
        assert!(sanitize_driver("demo:logic-channels=$(x)").is_err());
        assert!(sanitize_driver("--evil").is_err());
    }

    #[test]
    fn session_show_reads_back_channels_and_samplerate() {
        let info = parse_session_show("Channels: 8\nSamplerate: 12000000 Hz\nLogic unitsize: 1\n");
        assert_eq!(info.channels, Some(8));
        assert_eq!(info.samplerate_hz, Some(12e6));
        // Garbage yields None fields — the caller falls back and says so.
        let info = parse_session_show("sigrok-cli 0.7.2\n");
        assert_eq!(info.channels, None);
        assert_eq!(info.samplerate_hz, None);
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
