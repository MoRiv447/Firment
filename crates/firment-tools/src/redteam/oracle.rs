//! The crash oracle: pure classification of a captured output window after
//! one attack payload. No serial ports here — the runner feeds the text and
//! the timing facts in, and this module says what they mean.
//!
//! Priority is deliberate: a fault signature outranks a reboot banner
//! (crashed-then-restarted is a crash, not a mystery), and a reboot outranks
//! a missing heartbeat (a restarted board answers eventually — a hung one
//! never does). Silence with no heartbeat defined is NOT a hang: you cannot
//! claim an absence you never established a presence for.

use crate::forensic;
use regex::Regex;
use serde::Deserialize;

/// What the suite declares as "the target is alive" and "the target just
/// booted", plus any project-specific fault strings beyond the built-in
/// Cortex-M / panic signatures.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct OracleCfg {
    /// Regex matched against the window: a hit means the firmware is still
    /// ticking (e.g. `tick=\d+`). Absent = liveness is not asserted.
    pub heartbeat_regex: Option<String>,
    /// Regex for the boot banner. A hit AFTER traffic has been seen means
    /// the target restarted mid-run — an unsolicited reboot is a finding.
    pub boot_banner: Option<String>,
    /// Extra fault strings (e.g. "stack underflow") appended to the
    /// built-in signatures.
    pub extra_fault_signatures: Vec<String>,
}

/// The oracle's verdict for one case window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A fault signature appeared; the payload carries the matched marker.
    Crash(String),
    /// The boot banner reappeared mid-stream: the target restarted itself
    /// (watchdog or brown-out after the payload).
    Reboot,
    /// The window expired with no heartbeat — the target stopped ticking.
    Hang,
    /// Heartbeat present (or liveness undefined): the payload did not kill
    /// it.
    Alive,
}

/// Classify one captured window.
///
/// * `saw_traffic_before` — the target had produced output BEFORE this
///   window; only then does a banner hit mean "restarted" rather than
///   "this is the first boot log".
/// * `timed_out` — the read window expired without the case completing;
///   with no heartbeat in the text this is the Hang evidence.
pub fn classify(
    text: &str,
    saw_traffic_before: bool,
    timed_out: bool,
    cfg: &OracleCfg,
) -> Result<Verdict, String> {
    if let Some(marker) = forensic::fault_detected_marker(text) {
        return Ok(Verdict::Crash(format!("fault signature: {marker}")));
    }
    for sig in &cfg.extra_fault_signatures {
        if text.contains(sig.as_str()) {
            return Ok(Verdict::Crash(format!("extra signature: {sig}")));
        }
    }
    if let Some(banner) = &cfg.boot_banner {
        let rx = Regex::new(banner)
            .map_err(|e| format!("[InvalidInput] oracle boot_banner regex: {e}"))?;
        if saw_traffic_before && rx.is_match(text) {
            return Ok(Verdict::Reboot);
        }
    }
    if let Some(hb) = &cfg.heartbeat_regex {
        let rx =
            Regex::new(hb).map_err(|e| format!("[InvalidInput] oracle heartbeat_regex: {e}"))?;
        if rx.is_match(text) {
            return Ok(Verdict::Alive);
        }
        if timed_out {
            return Ok(Verdict::Hang);
        }
        // Window closed on its own without a heartbeat but without timing
        // out either: inconclusive, not a hang claim.
        return Ok(Verdict::Alive);
    }
    Ok(Verdict::Alive)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> OracleCfg {
        OracleCfg {
            heartbeat_regex: Some(r"tick=\d+".to_string()),
            boot_banner: Some("boot v2".to_string()),
            extra_fault_signatures: vec!["stack underflow".to_string()],
        }
    }

    #[test]
    fn hardfault_text_is_a_crash() {
        let v = classify("tick=41\ntick=42\nHardFault_Handler\n", true, false, &cfg()).unwrap();
        assert!(matches!(v, Verdict::Crash(_)), "got {v:?}");
    }

    #[test]
    fn extra_signature_is_a_crash_too() {
        let v = classify("tick=1\nstack underflow detected\n", true, false, &cfg()).unwrap();
        assert_eq!(
            v,
            Verdict::Crash("extra signature: stack underflow".to_string())
        );
    }

    #[test]
    fn crash_outranks_reboot() {
        // Fault AND banner in the same window: it crashed, then restarted —
        // the crash is the finding.
        let v = classify("HardFault_Handler\nboot v2\n", true, false, &cfg()).unwrap();
        assert!(matches!(v, Verdict::Crash(_)), "got {v:?}");
    }

    #[test]
    fn banner_after_traffic_is_a_reboot() {
        let v = classify("tick=99\nboot v2\n", true, false, &cfg()).unwrap();
        assert_eq!(v, Verdict::Reboot);
    }

    #[test]
    fn first_banner_is_not_a_reboot() {
        let v = classify("boot v2\ntick=1\n", false, false, &cfg()).unwrap();
        assert_eq!(v, Verdict::Alive, "no prior traffic = this is the boot log");
    }

    #[test]
    fn timeout_without_heartbeat_is_a_hang() {
        let v = classify("tick=40\n", true, true, &cfg()).unwrap();
        assert_eq!(v, Verdict::Alive, "heartbeat WAS seen in the window");
        let v = classify("nothing at all\n", true, true, &cfg()).unwrap();
        assert_eq!(v, Verdict::Hang);
    }

    #[test]
    fn silence_without_heartbeat_defined_is_not_a_hang() {
        let cfg = OracleCfg::default();
        let v = classify("", true, true, &cfg).unwrap();
        assert_eq!(
            v,
            Verdict::Alive,
            "cannot claim an absence never established as a presence"
        );
    }

    #[test]
    fn bad_regexes_are_reported_not_panics() {
        let cfg = OracleCfg {
            heartbeat_regex: Some("(unclosed".to_string()),
            boot_banner: None,
            extra_fault_signatures: vec![],
        };
        assert!(
            classify("x", false, false, &cfg)
                .unwrap_err()
                .contains("[InvalidInput]")
        );
    }
}
