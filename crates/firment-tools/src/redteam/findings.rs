//! Structured vulnerability findings — the red team's report format.
//!
//! The rule that makes the report worth reading: a finding is only as
//! strong as its evidence. `finalize` caps any finding whose evidence files
//! are missing (or absent) to `low` severity + `UNVERIFIED` — the observe
//! lesson ("a low-confidence answer can never pass an assertion") applied
//! to reports. And the reproducer is `seed + case id`, so every finding is
//! re-runnable without an LLM in the loop.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn name(self) -> &'static str {
        match self {
            Severity::Low => "low",
            Severity::Medium => "medium",
            Severity::High => "high",
            Severity::Critical => "critical",
        }
    }
}

/// How to re-run the case that produced the finding — no model required.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reproducer {
    pub suite: String,
    pub case: String,
}

/// One finding, JSONL-round-trippable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub finding_id: String,
    pub severity: Severity,
    /// crash | reboot | hang — the oracle's class.
    pub class: String,
    /// bitflip | oversize | … | campaign (LLM-discovered, corpus-back-ported).
    pub strategy: String,
    pub case_id: String,
    pub seed: u64,
    pub payload_hex: String,
    /// e.g. `uart@COM3` or `mqtt@node/cmd`.
    pub interface: String,
    /// What was observed, verbatim-ish (fault marker, banner, silence).
    pub observed: String,
    /// Evidence references: `file#Lline` or file paths inside the run dir.
    pub evidence: Vec<String>,
    pub reproducer: Reproducer,
    /// HIGH | UNVERIFIED — set by `finalize`, never by hand.
    pub confidence: String,
}

impl Finding {
    /// Cap unverified findings. `evidence_exists` is given each evidence
    /// reference and answers whether the file actually exists (the runner
    /// passes a real fs check; tests pass a closure).
    pub fn finalize(&mut self, evidence_exists: impl Fn(&str) -> bool) {
        let verified = !self.evidence.is_empty()
            && self
                .evidence
                .iter()
                .all(|e| is_safe_relative_path(e) && evidence_exists(e));
        if verified {
            self.confidence = "HIGH".to_string();
        } else {
            self.confidence = "UNVERIFIED".to_string();
            if self.severity > Severity::Low {
                self.severity = Severity::Low;
            }
        }
    }

    /// The strict gate for LLM-campaign findings, where the evidence list
    /// is MODEL-authored and therefore untrusted. A hallucinated finding
    /// must not ride in on "report.md exists": every evidence reference
    /// has to be a relative path inside the run dir, not the run's own
    /// report files, actually present, AND actually containing the quoted
    /// `observed` line. No payload_hex means no reproducer means UNVERIFIED
    /// — the prompt promised this; here it is enforced.
    pub fn finalize_strict(&mut self, dir: &Path) {
        // The modulo form is on purpose: `is_multiple_of` stabilised in
        // 1.87, past this workspace's declared MSRV 1.85.
        #[allow(clippy::manual_is_multiple_of)]
        let payload_ok = !self.payload_hex.is_empty()
            && self.payload_hex.len() % 2 == 0
            && self.payload_hex.bytes().all(|b| b.is_ascii_hexdigit());
        let observed = self.observed.clone();
        let evidence_ok = !self.evidence.is_empty()
            && self.evidence.iter().all(|e| {
                if !is_safe_relative_path(e) {
                    return false;
                }
                let name = Path::new(e)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if name == "report.md" || name == "findings.jsonl" {
                    return false; // the run's own outputs are not evidence
                }
                let path = dir.join(e);
                // join of a relative path cannot escape, but double-check.
                path.is_file()
                    && path.starts_with(dir)
                    && std::fs::read_to_string(&path)
                        .map(|t| !observed.is_empty() && t.contains(observed.as_str()))
                        .unwrap_or(false)
            });
        if payload_ok && evidence_ok {
            self.confidence = "HIGH".to_string();
        } else {
            self.confidence = "UNVERIFIED".to_string();
            if self.severity > Severity::Low {
                self.severity = Severity::Low;
            }
        }
    }

    pub fn to_json_line(&self) -> String {
        let mut line = serde_json::to_string(self).expect("Finding is serializable");
        line.push('\n');
        line
    }

    pub fn from_json_line(line: &str) -> Result<Self, String> {
        serde_json::from_str(line.trim_end_matches('\n'))
            .map_err(|e| format!("[Io] corrupt finding line: {e}"))
    }
}

/// An evidence reference must be a plain relative path: no absolute paths
/// (`Path::join` with an absolute path REPLACES the base — an escape hatch),
/// no parent-dir traversal. Checks components, not `is_absolute`: on
/// Windows "/etc/hosts" is NOT absolute (no drive prefix) but still has a
/// RootDir component that `join` would honour.
fn is_safe_relative_path(e: &str) -> bool {
    use std::path::Component;
    let p = Path::new(e);
    !e.is_empty()
        && !p.components().any(|c| {
            matches!(
                c,
                Component::Prefix(_) | Component::RootDir | Component::ParentDir
            )
        })
}

/// Lowercase hex of a payload, for the report and the reproducer.
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Default severity the oracle assigns per verdict class.
pub fn default_severity(class: &str) -> Severity {
    match class {
        "crash" => Severity::High,
        "reboot" => Severity::Medium,
        "hang" => Severity::Medium,
        _ => Severity::Low,
    }
}

/// Render the human-facing report.md. Findings are expected finalized.
pub fn render_report_md(suite: &str, run_id: &str, findings: &[Finding]) -> String {
    let mut out = format!(
        "# Red team report — suite `{suite}` (run {run_id})\n\n\
         Findings: {} ({} verified)\n\n",
        findings.len(),
        findings.iter().filter(|f| f.confidence == "HIGH").count(),
    );
    if findings.is_empty() {
        out.push_str("No findings: every case left the target alive.\n");
    }
    for f in findings {
        out.push_str(&format!(
            "## {} — {} [{}] ({})\n\n\
             - class: {}\n\
             - interface: {}\n\
             - payload: `{}`\n\
             - observed: {}\n\
             - evidence: {}\n\
             - reproducer: seed {}, case `{}` (suite `{}`)\n\n",
            f.finding_id,
            f.severity.name(),
            f.confidence,
            f.strategy,
            f.class,
            f.interface,
            f.payload_hex,
            f.observed,
            if f.evidence.is_empty() {
                "—".to_string()
            } else {
                f.evidence.join(", ")
            },
            f.seed,
            f.case_id,
            f.reproducer.suite,
        ));
    }
    out.push_str("\nevidence: reached level 5 (physical) — findings cite captured output\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding() -> Finding {
        Finding {
            finding_id: "F-001".to_string(),
            severity: Severity::High,
            class: "crash".to_string(),
            strategy: "bitflip".to_string(),
            case_id: "bitflip-17".to_string(),
            seed: 1234,
            payload_hex: hex_encode(&[0x55, 0xAA]),
            interface: "uart@COM3".to_string(),
            observed: "HardFault_Handler".to_string(),
            evidence: vec!["capture-017.log".to_string()],
            reproducer: Reproducer {
                suite: "uart-fuzz".to_string(),
                case: "bitflip-17".to_string(),
            },
            confidence: String::new(),
        }
    }

    #[test]
    fn finalize_keeps_verified_findings_at_their_severity() {
        let mut f = finding();
        f.finalize(|_| true);
        assert_eq!(f.confidence, "HIGH");
        assert_eq!(f.severity, Severity::High);
    }

    #[test]
    fn finalize_caps_unverified_findings_to_low() {
        let mut f = finding();
        f.finalize(|_| false);
        assert_eq!(f.confidence, "UNVERIFIED");
        assert_eq!(
            f.severity,
            Severity::Low,
            "missing evidence must not carry a high"
        );

        let mut empty = finding();
        empty.evidence.clear();
        empty.finalize(|_| true);
        assert_eq!(empty.confidence, "UNVERIFIED");
        assert_eq!(empty.severity, Severity::Low);
    }

    #[test]
    fn jsonl_round_trips() {
        let mut f = finding();
        f.finalize(|_| true);
        let line = f.to_json_line();
        let back = Finding::from_json_line(&line).unwrap();
        assert_eq!(f, back);
        assert!(line.ends_with('\n'));
        assert!(Finding::from_json_line("not json").is_err());
    }

    #[test]
    fn report_lists_findings_and_reproducers() {
        let mut f = finding();
        f.finalize(|_| true);
        let md = render_report_md("uart-fuzz", "run-9", &[f]);
        assert!(md.contains("F-001"), "got: {md}");
        assert!(md.contains("bitflip-17"), "got: {md}");
        assert!(md.contains("seed 1234"), "got: {md}");
        assert!(md.contains("level 5 (physical)"), "got: {md}");
        let empty = render_report_md("uart-fuzz", "run-9", &[]);
        assert!(empty.contains("No findings"), "got: {empty}");
    }

    #[test]
    fn hex_encoding_is_lowercase_and_padded() {
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xff]), "000fff");
        assert_eq!(hex_encode(&[]), "");
    }

    #[test]
    fn severity_ordering_is_the_contract() {
        assert!(Severity::Low < Severity::Medium);
        assert!(Severity::High < Severity::Critical);
        assert_eq!(default_severity("crash"), Severity::High);
        assert_eq!(default_severity("mystery"), Severity::Low);
    }

    #[test]
    fn finalize_rejects_absolute_and_traversing_evidence() {
        // Path::join with an absolute path REPLACES the base — an evidence
        // entry like "/etc/hosts" must never count as verification.
        let mut f = finding();
        f.evidence = vec!["/etc/hosts".to_string()];
        f.finalize(|_| true);
        assert_eq!(f.confidence, "UNVERIFIED");
        assert_eq!(f.severity, Severity::Low);

        let mut f = finding();
        f.evidence = vec!["../outside.log".to_string()];
        f.finalize(|_| true);
        assert_eq!(f.confidence, "UNVERIFIED");
    }

    #[test]
    fn finalize_strict_demands_payload_and_real_quoted_evidence() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("capture-F-002.log"),
            "tick=9\nHardFault_Handler\n",
        )
        .unwrap();

        // A well-formed campaign finding: payload + evidence containing the
        // quoted line → HIGH.
        let mut good = finding();
        good.strategy = "campaign".to_string();
        good.observed = "HardFault_Handler".to_string();
        good.evidence = vec!["capture-F-002.log".to_string()];
        good.finalize_strict(dir.path());
        assert_eq!(good.confidence, "HIGH", "got: {good:?}");

        // Missing payload_hex → no reproducer → UNVERIFIED + low.
        let mut no_payload = good.clone();
        no_payload.payload_hex = String::new();
        no_payload.finalize_strict(dir.path());
        assert_eq!(no_payload.confidence, "UNVERIFIED");
        assert_eq!(no_payload.severity, Severity::Low);

        // Evidence that exists but does NOT contain the quoted line (the
        // hallucination ride-in) → UNVERIFIED.
        let mut unrelated = good.clone();
        unrelated.observed = "everything is on fire".to_string();
        unrelated.finalize_strict(dir.path());
        assert_eq!(unrelated.confidence, "UNVERIFIED", "got: {unrelated:?}");

        // The run's own report files are not evidence.
        std::fs::write(dir.path().join("report.md"), "HardFault_Handler").unwrap();
        let mut self_ref = good.clone();
        self_ref.evidence = vec!["report.md".to_string()];
        self_ref.finalize_strict(dir.path());
        assert_eq!(self_ref.confidence, "UNVERIFIED");

        // Absolute-path evidence → UNVERIFIED.
        let mut abs = good.clone();
        abs.evidence = vec![
            dir.path()
                .join("capture-F-002.log")
                .to_string_lossy()
                .into_owned(),
        ];
        abs.finalize_strict(dir.path());
        assert_eq!(abs.confidence, "UNVERIFIED");
    }
}
