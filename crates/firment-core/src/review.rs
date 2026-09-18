//! The shared vocabulary of every review capability (plan §4).
//!
//! The four review directions — change self-review (A), hardware-behaviour review (B),
//! static code review (C) and dependency/supply-chain review (D) — differ completely in
//! where they look and agree completely in what they say. So the schema is defined once
//! here (it is the `Finding` shape verified in the desktop CodeLens project), and a
//! finding from the dependency scanner reads exactly like one from a self-review: one
//! renderer, one badge, one Markdown export.
//!
//! Two rules come from the plan's own negative-optimisation review and are load-bearing
//! for every capability built on this module:
//!
//! * **A report is not an error.** `notes` carries what could *not* be checked and why
//!   (an advisory database that is not installed, a board that is not connected). It is
//!   neutral text, never a finding, because a review that turns "I could not look" into
//!   "this is broken" teaches the reader to ignore it (§16.4).
//! * **Severity is about the reader's next action.** `High` is a thing that should stop
//!   a release; `Medium` is a thing to look at. Red is reserved for the first of those.

pub mod deps;
pub mod self_review;

use serde::{Deserialize, Serialize};

/// What a finding cost, from the reader's point of view rather than a CVSS score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Worth knowing; the release can proceed.
    Medium,
    /// Should stop a release until someone decides it is fine.
    High,
}

impl Severity {
    pub fn label(&self) -> &'static str {
        match self {
            Severity::High => "high",
            Severity::Medium => "medium",
        }
    }

    /// The glyph a compact UI can put in front of a title.
    pub fn mark(&self) -> &'static str {
        match self {
            Severity::High => "■",
            Severity::Medium => "□",
        }
    }
}

/// One thing a review found.
///
/// Fields are optional where a capability genuinely cannot fill them: a dependency
/// finding has a package name and no symbol, a code finding has a symbol and no
/// version. Empty rather than absent would read as "checked and empty", which is the
/// distinction this struct exists to keep.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// Stable across runs, so a UI can keep a "dismissed" set.
    pub id: String,
    pub title: String,
    pub severity: Severity,
    /// Which capability produced it: `dependency`, `supply-chain`, `self-review`,
    /// `hardware`, `static`.
    pub category: String,
    /// Path, when the finding is about a file (or the manifest that pulled a crate in).
    pub file: Option<String>,
    pub symbol: Option<String>,
    /// What was observed, in the reviewer's words.
    pub description: String,
    /// Why it matters. Kept separate from the description so a UI can show one line and
    /// expand into the other.
    pub impact: Option<String>,
    /// The offending text, when there is one and it is short.
    pub code: Option<String>,
    /// How to check it by hand.
    pub steps: Vec<String>,
    /// What to do about it.
    pub fix: Option<String>,
    pub tags: Vec<String>,
}

impl Finding {
    /// A finding with only the fields every capability can supply.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        severity: Severity,
        category: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            severity,
            category: category.into(),
            file: None,
            symbol: None,
            description: description.into(),
            impact: None,
            code: None,
            steps: Vec::new(),
            fix: None,
            tags: Vec::new(),
        }
    }

    pub fn with_impact(mut self, impact: impl Into<String>) -> Self {
        self.impact = Some(impact.into());
        self
    }

    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    pub fn with_fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    pub fn with_tags(mut self, tags: &[&str]) -> Self {
        self.tags = tags.iter().map(|t| t.to_string()).collect();
        self
    }
}

/// What one review pass produced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReviewReport {
    /// What was reviewed, in the user's terms: `dependencies`, a path, a diff.
    pub scope: String,
    pub findings: Vec<Finding>,
    /// Neutral statements about the *review*, not the code: what could not be checked.
    pub notes: Vec<String>,
    /// Extra lines a specific capability wants to show, printed after the summary and
    /// before the findings (a license distribution, for instance).
    pub details: Vec<String>,
}

impl ReviewReport {
    pub fn new(scope: impl Into<String>) -> Self {
        Self {
            scope: scope.into(),
            ..Default::default()
        }
    }

    pub fn push(&mut self, finding: Finding) {
        self.findings.push(finding);
    }

    pub fn note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    pub fn detail(&mut self, line: impl Into<String>) {
        self.details.push(line.into());
    }

    /// Findings by severity, worst first — the order every renderer wants.
    pub fn ordered(&self) -> Vec<&Finding> {
        let mut findings: Vec<&Finding> = self.findings.iter().collect();
        // Worst first: `High` sorts above `Medium` by the enum's own order.
        findings.sort_by_key(|f| std::cmp::Reverse(f.severity));
        findings
    }

    pub fn counts(&self) -> (usize, usize) {
        let high = self
            .findings
            .iter()
            .filter(|f| f.severity == Severity::High)
            .count();
        (high, self.findings.len() - high)
    }

    /// True when nothing was found. Says nothing about what was *not* checked: the
    /// notes carry that, and a caller that wants to distinguish the two must read them.
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    /// The one-line summary a badge or a status bar shows.
    ///
    /// "clean" is only allowed when there was nothing to skip either — a review with no
    /// findings and no advisory database behind it is not clean, it is partial, and
    /// saying "clean" there is the exact lie the plan's §16.4 forbids.
    pub fn summary(&self) -> String {
        let (high, medium) = self.counts();
        if self.findings.is_empty() {
            if self.notes.is_empty() {
                format!("{}: clean", self.scope)
            } else {
                format!(
                    "{}: nothing found ({} unchecked)",
                    self.scope,
                    self.notes.len()
                )
            }
        } else {
            format!("{}: {high} high, {medium} medium", self.scope)
        }
    }

    /// The export the plan asks for (§4: `reportMarkdown()`), and the shape the CI
    /// action will comment with (§5#6).
    pub fn to_markdown(&self) -> String {
        let mut out = format!("# Review: {}\n\n", self.scope);
        out.push_str(&format!("{}\n\n", self.summary()));
        for line in &self.details {
            out.push_str(&format!("- {line}\n"));
        }
        if !self.details.is_empty() {
            out.push('\n');
        }
        if self.findings.is_empty() {
            out.push_str("No findings.\n");
        } else {
            for finding in self.ordered() {
                out.push_str(&format!(
                    "## {} {}\n\n",
                    finding.severity.mark(),
                    finding.title
                ));
                out.push_str(&format!(
                    "`{}` · {} · severity **{}**\n\n",
                    finding.category,
                    finding.file.as_deref().unwrap_or("—"),
                    finding.severity.label()
                ));
                out.push_str(&format!("{}\n\n", finding.description));
                if let Some(impact) = &finding.impact {
                    out.push_str(&format!("**Impact:** {impact}\n\n"));
                }
                if let Some(code) = &finding.code {
                    out.push_str(&format!("```\n{code}\n```\n\n"));
                }
                if !finding.steps.is_empty() {
                    out.push_str("Steps:\n");
                    for step in &finding.steps {
                        out.push_str(&format!("1. {step}\n"));
                    }
                    out.push('\n');
                }
                if let Some(fix) = &finding.fix {
                    out.push_str(&format!("**Fix:** {fix}\n\n"));
                }
            }
        }
        if !self.notes.is_empty() {
            out.push_str("## Not checked\n\n");
            for note in &self.notes {
                out.push_str(&format!("- {note}\n"));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(severity: Severity, title: &str) -> Finding {
        Finding::new("id", title, severity, "test", "d")
    }

    #[test]
    fn a_summary_with_unchecked_notes_is_not_called_clean() {
        // The distinction the whole module is built around: no findings because
        // everything was checked is a result; no findings because half the check could
        // not run is a caveat, and calling it "clean" is the lie §16.4 forbids.
        let mut checked = ReviewReport::new("deps");
        assert_eq!(checked.summary(), "deps: clean");

        checked.note("advisory database not installed");
        assert_eq!(checked.summary(), "deps: nothing found (1 unchecked)");
    }

    #[test]
    fn findings_are_ordered_worst_first() {
        let mut report = ReviewReport::new("deps");
        report.push(finding(Severity::Medium, "medium one"));
        report.push(finding(Severity::High, "high one"));
        report.push(finding(Severity::Medium, "medium two"));

        let titles: Vec<&str> = report.ordered().iter().map(|f| f.title.as_str()).collect();
        assert_eq!(titles, ["high one", "medium one", "medium two"]);
        assert_eq!(report.counts(), (1, 2));
        assert_eq!(report.summary(), "deps: 1 high, 2 medium");
    }

    #[test]
    fn the_markdown_export_carries_what_a_reader_needs_to_act() {
        let mut report = ReviewReport::new("dependencies");
        report.detail("licenses: MIT 12, Apache-2.0 8");
        report.push(
            Finding::new(
                "dep-1",
                "copyleft dependency: foo 1.2",
                Severity::High,
                "dependency",
                "Pulled in by bar.",
            )
            .with_file("Cargo.lock")
            .with_fix("Replace it, or confirm the licence is acceptable for this product."),
        );
        report.note("advisory check skipped: cargo-audit is not installed");

        let markdown = report.to_markdown();
        assert!(markdown.starts_with("# Review: dependencies\n"));
        assert!(markdown.contains("1 high, 0 medium"));
        assert!(markdown.contains("licenses: MIT 12, Apache-2.0 8"));
        assert!(markdown.contains("■ copyleft dependency: foo 1.2"));
        assert!(markdown.contains("`dependency` · Cargo.lock"));
        assert!(markdown.contains("**Fix:** Replace it"));
        // The unchecked section has to survive into the export, or the report a CI
        // comment carries looks more complete than the review was.
        assert!(markdown.contains("## Not checked"));
        assert!(markdown.contains("cargo-audit is not installed"));
    }
}
