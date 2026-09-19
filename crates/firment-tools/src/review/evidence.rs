//! Hardware-behaviour review (plan §4-B).
//!
//! The other three review capabilities read code, a diff or a manifest. This one reads
//! **what the hardware did**: the replay log a HIL run leaves behind, one JSON line per
//! step, each carrying whether its assertion held, what the step actually said, and how
//! long it took.
//!
//! Two ideas from the plan's own review shape the report, and both are about not
//! overclaiming:
//!
//! * **The ladder is the summary.** Firment's whole pitch is that a claim is only as good
//!   as the rung that proves it — code, build, deploy, runtime, physical. A report that
//!   listed only failures would quietly imply the rest was proven, so the ladder is stated
//!   whether or not anything failed, and a rung the run never reached is a *note*.
//! * **"Would have checked" is not evidence.** A step that did not run leaves no
//!   measurement, and a suite that skipped the physical rung has no physical evidence —
//!   which is a fact about the run, not a defect in the firmware, so it is a note and
//!   never a finding (§16.4).

use firment_core::review::{Finding, ReviewReport, Severity};

/// One step of a stored HIL run.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EvidenceStep {
    /// 1-based, as the log writes it.
    pub step: usize,
    /// The step's tool name (`flash`, `observe`, …). The *rung* it reaches is
    /// [`ladder_rung`]'s business — the two vocabularies are deliberately separate.
    pub kind: String,
    pub ok: bool,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub elapsed_ms: u64,
}

/// The five rungs, weakest first. Index 0 is the floor every suite starts from.
pub const LADDER: [(u8, &str); 5] = [
    (1, "code"),
    (2, "build"),
    (3, "deploy"),
    (4, "runtime"),
    (5, "physical"),
];

/// Parse a replay log.
///
/// Returns the steps and the number of lines that could not be read. The count is not
/// decoration: a log written while the run was killed is half a log, and a report that
/// silently dropped those lines would describe a shorter run than actually happened.
pub fn parse_replay(log: &str) -> (Vec<EvidenceStep>, usize) {
    let mut steps = Vec::new();
    let mut unread = 0usize;
    for line in log.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<EvidenceStep>(line) {
            Ok(step) => steps.push(step),
            Err(_) => unread += 1,
        }
    }
    (steps, unread)
}

/// The highest rung the run actually reached.
///
/// A rung counts only when a step of that rung **succeeded**: a `flash` step that failed
/// means the deploy rung was attempted, not proven, and reporting it as reached would be
/// the exact overclaim this module exists to prevent.
pub fn highest_rung(steps: &[EvidenceStep]) -> u8 {
    steps
        .iter()
        .filter(|step| step.ok)
        .filter_map(|step| crate::tools::ladder_rung(&step.kind).map(|(level, _)| level))
        .max()
        .unwrap_or(1)
}

/// Review one stored run: **what did the hardware prove?**
pub fn review_run(suite: &str, steps: &[EvidenceStep], unread_lines: usize) -> ReviewReport {
    let mut report = ReviewReport::new(format!("evidence: {suite}"));

    let reached = highest_rung(steps);
    let marks: Vec<String> = LADDER
        .iter()
        .map(|(level, label)| {
            let state = if *level < reached {
                "✓"
            } else if *level == reached && reached > 1 {
                "◐"
            } else {
                "○"
            };
            format!("{state} {label}")
        })
        .collect();
    report.detail(format!("ladder: {}", marks.join(" → ")));
    report.detail(format!("{} steps recorded", steps.len()));

    for step in steps.iter().filter(|step| !step.ok) {
        let rung = crate::tools::ladder_rung(&step.kind)
            .map(|(_, label)| label)
            .unwrap_or("unclassified");
        report.push(
            Finding::new(
                format!("evidence-step-{}-failed", step.step),
                format!("step {} ({}) failed at the {rung} rung", step.step, step.kind),
                // A hardware assertion that did not hold is the release-blocking kind:
                // this is the difference between "the code looks right" and "the board
                // does the right thing", and only the second one ships.
                Severity::High,
                "evidence",
                step.text.trim().to_string(),
            )
            .with_fix("Fix what the step measured, then re-run the suite — the ladder only counts a rung that succeeded.")
            .with_tags(&["evidence", "hil"]),
        );
    }

    // What the run did not prove. A note, never a finding: a suite that never reached the
    // physical rung has an incomplete run, not a broken firmware. The two cases are told
    // apart because they ask different things of the reader — "we tried and it failed"
    // points at the finding above, "we never tried" points at the suite.
    for (level, label) in LADDER {
        if level > reached {
            let attempted = steps
                .iter()
                .any(|step| crate::tools::ladder_rung(&step.kind).map(|(l, _)| l) == Some(level));
            report.note(if attempted {
                format!("the {label} rung was attempted and not proven (see the failed step above)")
            } else {
                format!("the {label} rung was never exercised — nothing in this run speaks for it")
            });
        }
    }
    if unread_lines > 0 {
        report.note(format!(
            "{unread_lines} replay line(s) could not be read — the run was killed mid-write, or the log format moved on"
        ));
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = r#"{"step":1,"kind":"build","ok":true,"text":"built","elapsed_ms":1200}
{"step":2,"kind":"flash","ok":true,"text":"flashed","elapsed_ms":3000}
{"step":3,"kind":"observe","ok":false,"text":"expect_lit: LED is dark (confidence HIGH)","elapsed_ms":900}
"#;

    #[test]
    fn a_replay_log_reads_back_step_for_step() {
        let (steps, unread) = parse_replay(LOG);
        assert_eq!(unread, 0);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[2].kind, "observe");
        assert!(!steps[2].ok);
        assert!(steps[2].text.contains("expect_lit"));
    }

    #[test]
    fn a_truncated_log_is_counted_rather_than_silently_shorter() {
        // A run killed mid-write leaves a half line. Dropping it quietly would describe a
        // shorter run than happened, and the report says so instead.
        let log = format!("{LOG}{{\"step\":4,\"kind\":\"obse");
        let (steps, unread) = parse_replay(&log);
        assert_eq!(steps.len(), 3);
        assert_eq!(unread, 1);
    }

    #[test]
    fn a_failed_step_is_high_and_says_what_it_measured() {
        let (steps, unread) = parse_replay(LOG);
        let report = review_run("blink", &steps, unread);

        assert_eq!(report.counts(), (1, 0));
        let finding = &report.findings[0];
        assert_eq!(finding.severity, Severity::High);
        assert!(
            finding
                .title
                .contains("step 3 (observe) failed at the physical rung")
        );
        assert!(
            finding.description.contains("LED is dark"),
            "the measurement is the evidence: {finding:?}"
        );
    }

    #[test]
    fn the_ladder_says_what_was_and_was_not_proven() {
        let (steps, unread) = parse_replay(LOG);
        let report = review_run("blink", &steps, unread);

        // build and flash succeeded, observe failed: the run reached the deploy rung.
        let ladder = report
            .details
            .iter()
            .find(|d| d.starts_with("ladder:"))
            .unwrap();
        assert!(ladder.contains("✓ build"), "{ladder}");
        assert!(ladder.contains("◐ deploy"), "{ladder}");
        assert!(ladder.contains("○ physical"), "{ladder}");

        // Physical was *attempted* (and failed); runtime was not touched at all. The two
        // notes read differently because they ask different things of the reader.
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.contains("physical rung was attempted and not proven")),
            "notes: {:?}",
            report.notes
        );
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.contains("runtime rung was never exercised")),
            "notes: {:?}",
            report.notes
        );
    }

    #[test]
    fn a_run_that_never_left_the_code_rung_says_exactly_that() {
        // The "would have checked" case: a dry run, or a suite with no hardware steps.
        let (steps, unread) =
            parse_replay("{\"step\":1,\"kind\":\"delay\",\"ok\":true,\"text\":\"waited\"}\n");
        let report = review_run("rehearsal", &steps, unread);

        // Nothing failed, and the report still refuses to sound proven: four rungs are
        // named as unproven.
        assert!(report.is_clean());
        assert_eq!(report.notes.len(), 4, "notes: {:?}", report.notes);
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.contains("physical rung was never exercised"))
        );
        // `summary()` must not call this clean: it has notes.
        assert!(
            report.summary().contains("unchecked"),
            "{}",
            report.summary()
        );
    }
}
