//! Change self-review (plan §4-A) — the agent reviewing what it just changed.
//!
//! The plan's own negative-optimisation review puts the constraint that shapes this
//! module first: **the automatic trigger defaults to off**, because "every edit costs an
//! extra LLM call" doubles both the wait and the bill for the user whose complaint was
//! the waiting (§16.2-1). So the pieces here are deliberately separable:
//!
//! * `diff_size` — what an edit did, which is what `on_large` decides on.
//! * `review_prompt` — the request, built from the diff and whatever context the caller
//!   has. Pure, so what the model is asked is testable and reviewable.
//! * `parse_findings` — the model's answer back into the shared [`Finding`] shape. Also
//!   pure, and strict about the one thing that matters: a malformed answer is a *note*
//!   ("the review could not be read"), never a finding, and never a silent empty report.
//! * `review_diff` — the one place that talks to a provider, so the CLI and the TUI
//!   cannot drift apart in how they ask.

use super::{Finding, ReviewReport, Severity};
use crate::config::Config;
use crate::types::{ChatMessage, ChatRequest, ThinkingLevel};

/// When an edit triggers a self-review (plan §4-A).
///
/// Lives in `config` next to the rest of the policy, but the *decision* is here so that
/// the threshold has one implementation:
///
/// * `Off` — the default, and a requirement rather than a taste: §16.2-1 forbids the
///   alternative because an extra model call per edit doubles both the wait and the bill
///   for the user whose complaint was the waiting. What makes `Off` usable rather than a
///   feature nobody can reach is the manual path (`/review-last`, `firm review last`).
/// * `OnLarge` — only edits big enough to be worth a second opinion.
/// * `On` — every edit. For a demonstration, or for someone who has decided the wait is
///   worth it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfterEdit {
    #[default]
    Off,
    OnLarge,
    On,
}

/// Whether this edit is worth a review under `policy`.
///
/// `OnLarge` is "more than `min_lines` changed, **or** more than one hunk": separate
/// places in one edit are harder to read as a single change than the same number of lines
/// in one place, so a small edit that touched three regions is large for this purpose.
pub fn should_review(policy: AfterEdit, size: DiffSize, min_lines: usize) -> bool {
    match policy {
        AfterEdit::Off => false,
        AfterEdit::On => true,
        AfterEdit::OnLarge => size.total() > min_lines || size.hunks > 1,
    }
}

/// Whether a tool's output carries a unified diff.
///
/// The edit tools prepend a one-line "Edited …" header, so an `@@ ` hunk header is the
/// signal. This lives in core rather than in the TUI that first needed it, because the
/// CLI and the agent's self-review need the same answer and three copies of a heuristic
/// is three chances to disagree about what counts as a reviewable change.
pub fn looks_like_diff(text: &str) -> bool {
    text.lines().any(|line| line.starts_with("@@ "))
}

/// The file a diff is about, from its `+++` header.
///
/// `+++ b/src/main.rs` → `src/main.rs`; the `a/`+`b/` prefixes are what `git diff`
/// writes and are not part of the path. A diff with no header (or with `/dev/null`, a
/// file that was deleted) answers `None`, and the caller labels the review with what it
/// does know rather than with a path it invented.
pub fn path_from_diff(diff: &str) -> Option<String> {
    let pick = |line: &str| -> Option<String> {
        let raw = line[4..].trim();
        let path = raw
            .strip_prefix("b/")
            .or_else(|| raw.strip_prefix("a/"))
            .unwrap_or(raw);
        // A trailing tab separates the path from a timestamp in some producers.
        let path = path.split('\t').next().unwrap_or(path);
        (!path.is_empty() && path != "/dev/null").then(|| path.to_string())
    };
    // `+++` first: for a normal edit that is the file as it now exists. For a deletion
    // it is `/dev/null`, and the `---` side then names the file that was removed —
    // still the right title for "review this change", and truer than the tool name.
    let new = diff.lines().find(|l| l.starts_with("+++ ")).and_then(pick);
    let old = diff.lines().find(|l| l.starts_with("--- ")).and_then(pick);
    new.or(old)
}

/// What an edit changed, as far as the trigger and the prompt care.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffSize {
    pub added: usize,
    pub removed: usize,
    /// `@@` markers: more than one means the edit touched separate places, which is
    /// harder to review as one change than the same number of lines in one hunk.
    pub hunks: usize,
}

impl DiffSize {
    /// The number the policy's threshold compares against: everything the diff did,
    /// counted once.
    pub fn total(&self) -> usize {
        self.added + self.removed
    }
}

/// Count a unified diff.
///
/// `+++`/`---` headers are not changes — counting them would make every diff look two
/// lines bigger and, worse, would push a pure-rename edit over a threshold it does not
/// belong over.
pub fn diff_size(diff: &str) -> DiffSize {
    let mut size = DiffSize::default();
    for line in diff.lines() {
        if line.starts_with("@@") {
            size.hunks += 1;
        } else if line.starts_with("+++") || line.starts_with("---") {
            continue;
        } else if line.starts_with('+') {
            size.added += 1;
        } else if line.starts_with('-') {
            size.removed += 1;
        }
    }
    size
}

/// The question the model is asked.
///
/// It asks for JSON because the answer has to become [`Finding`]s, and it says what to
/// leave out as clearly as what to look for: a self-review that reports style
/// preferences as findings trains the reader to ignore the badge.
pub fn review_prompt(path: &str, diff: &str, context: Option<&str>) -> String {
    let size = diff_size(diff);
    let mut prompt = String::new();
    prompt.push_str(
        "You are reviewing a change just made to a firmware project. Judge the change \
         itself, not the file it lives in: report only defects a reviewer would ask to \
         fix before this is committed — wrong behaviour, a missed edge case, a broken \
         boundary between layers, a resource that leaks, a claim in a comment the code \
         does not support. Do not report style, naming or formatting.\n\n",
    );
    prompt.push_str(&format!(
        "File: {path}\nChange: +{} -{} lines across {} hunk(s)\n\n",
        size.added,
        size.removed,
        size.hunks.max(1)
    ));
    if let Some(context) = context.filter(|c| !c.trim().is_empty()) {
        prompt.push_str("Surrounding context:\n```\n");
        prompt.push_str(context);
        prompt.push_str("\n```\n\n");
    }
    prompt.push_str("The change:\n```diff\n");
    prompt.push_str(diff.trim_end());
    prompt.push_str(
        "\n```\n\nAnswer with a JSON array and nothing else. Each element:\n\
         {\"title\": \"one line, imperative\", \"severity\": \"high\"|\"medium\", \
         \"description\": \"what is wrong and where\", \"impact\": \"what it costs if \
         shipped\", \"fix\": \"what to do\", \"steps\": [\"how to check it\"]}\n\
         An empty array (`[]`) is the right answer when there is nothing to fix.",
    );
    prompt
}

/// The model's answer as findings.
///
/// Lenient about the wrapping (models like a ```json fence) and strict about the shape:
/// anything that is not a JSON array of objects becomes a note on the report rather than
/// a finding, because "the review could not be read" and "the change is fine" must not
/// look the same.
pub fn parse_findings(answer: &str, path: &str) -> (Vec<Finding>, Option<String>) {
    let trimmed = answer.trim();
    let body = strip_code_fence(trimmed);
    // An answer with no  never had an array; one with  and no  was cut off
    // mid-JSON. Those need different words, because the second one has an obvious
    // remedy (the answer hit the token limit) and the first one does not.
    let Some(start) = body.find('[') else {
        return (
            Vec::new(),
            Some("the review's answer carried no JSON array".to_string()),
        );
    };
    let Some(end) = body.rfind(']').filter(|end| *end > start) else {
        return (
            Vec::new(),
            Some(format!(
                "the review's answer was cut off before the JSON array closed: {}",
                truncate(&body[start..], 80)
            )),
        );
    };
    let slice = &body[start..=end];
    let parsed: Result<Vec<serde_json::Value>, _> = serde_json::from_str(slice);
    let Ok(items) = parsed else {
        return (
            Vec::new(),
            Some(format!(
                "the review's answer was not valid JSON: {}",
                truncate(slice, 120)
            )),
        );
    };

    let mut findings = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let Some(title) = item["title"].as_str().filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        let severity = match item["severity"]
            .as_str()
            .unwrap_or("medium")
            .to_lowercase()
            .as_str()
        {
            "high" | "critical" => Severity::High,
            _ => Severity::Medium,
        };
        let mut finding = Finding::new(
            format!("self-review-{index}"),
            title.to_string(),
            severity,
            "self-review",
            item["description"]
                .as_str()
                .unwrap_or("(no description given)")
                .to_string(),
        )
        .with_file(path)
        .with_tags(&["self-review"]);
        if let Some(impact) = item["impact"].as_str().filter(|i| !i.trim().is_empty()) {
            finding = finding.with_impact(impact);
        }
        if let Some(fix) = item["fix"].as_str().filter(|f| !f.trim().is_empty()) {
            finding = finding.with_fix(fix);
        }
        for step in item["steps"].as_array().into_iter().flatten() {
            if let Some(step) = step.as_str() {
                finding.steps.push(step.to_string());
            }
        }
        findings.push(finding);
    }
    (findings, None)
}

fn strip_code_fence(text: &str) -> &str {
    let text = text.trim();
    let Some(rest) = text.strip_prefix("```") else {
        return text;
    };
    // Drop the language tag on the fence line, then the closing fence.
    let rest = rest.split_once('\n').map(|(_, body)| body).unwrap_or("");
    rest.trim_end().trim_end_matches("```").trim()
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let cut: String = text.chars().take(limit).collect();
    format!("{cut}…")
}

/// Review one diff, through the user's own provider.
///
/// The single place that talks to a model, so the CLI and the TUI cannot drift in how
/// they ask. A provider failure is an `Err` — a review that could not run at all is a
/// failure of the command, not a clean report about the code.
pub async fn review_diff(
    config: &Config,
    provider_name: &str,
    path: &str,
    diff: &str,
    context: Option<&str>,
) -> Result<ReviewReport, String> {
    use futures::StreamExt as _;

    let provider = config
        .build_provider(Some(provider_name), None)
        .map_err(|e| format!("could not build provider '{provider_name}': {e}"))?;
    let request = ChatRequest {
        model: provider.model().to_string(),
        messages: vec![ChatMessage::User {
            content: review_prompt(path, diff, context),
        }],
        // No tools: a reviewer that can edit is a reviewer that can hide its own
        // findings, and this call must not change anything.
        tools: Vec::new(),
        max_tokens: config.max_output_tokens,
        temperature: None,
        // Thinking off: the answer is a small JSON document, and a reasoning pass here
        // is exactly the extra wait §16.2-1 is protecting.
        thinking: Some(ThinkingLevel::Off),
    };

    let mut stream = provider
        .stream(request)
        .await
        .map_err(|e| format!("review request failed: {e}"))?;
    let mut answer = String::new();
    while let Some(event) = stream.next().await {
        let event = event.map_err(|e| format!("review stream failed: {e}"))?;
        // Only text matters here: thinking, stop reasons and liveness heartbeats have
        // nothing to add to a JSON document, and no tool was offered, so no tool call
        // can arrive.
        if let crate::provider::ProviderEvent::Text(delta) = event {
            answer.push_str(&delta);
        }
    }

    let mut report = ReviewReport::new(format!("self-review of {path}"));
    let size = diff_size(diff);
    report.detail(format!(
        "change: +{} -{} lines, {} hunk(s)",
        size.added,
        size.removed,
        size.hunks.max(1)
    ));
    let (findings, note) = parse_findings(&answer, path);
    for finding in findings {
        report.push(finding);
    }
    if let Some(note) = note {
        report.note(note);
        // The raw answer goes into the details, so a malformed reply is inspectable
        // rather than lost.
        report.detail(format!("answer: {}", truncate(answer.trim(), 400)));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "--- a/a.c\n+++ b/a.c\n@@ -1,3 +1,4 @@\n int main(void) {\n-    boot();\n+    boot();\n+    loop_forever();\n }\n";

    #[test]
    fn the_default_policy_reviews_nothing() {
        // §16.2-1 as an assertion: if someone flips the default, this fails and the
        // commit that does it has to say why.
        assert_eq!(AfterEdit::default(), AfterEdit::Off);
        let size = DiffSize {
            added: 900,
            removed: 900,
            hunks: 9,
        };
        assert!(!should_review(AfterEdit::Off, size, 20));
    }

    #[test]
    fn on_large_uses_both_the_size_and_the_number_of_places() {
        let small = DiffSize {
            added: 3,
            removed: 1,
            hunks: 1,
        };
        assert!(!should_review(AfterEdit::OnLarge, small, 20));
        // A small edit in three separate places is large for this purpose.
        let scattered = DiffSize {
            added: 3,
            removed: 1,
            hunks: 3,
        };
        assert!(should_review(AfterEdit::OnLarge, scattered, 20));
        let big = DiffSize {
            added: 21,
            removed: 0,
            hunks: 1,
        };
        assert!(should_review(AfterEdit::OnLarge, big, 20));
        // The threshold is exclusive: exactly twenty lines is not "more than twenty".
        let exactly = DiffSize {
            added: 20,
            removed: 0,
            hunks: 1,
        };
        assert!(!should_review(AfterEdit::OnLarge, exactly, 20));
        // `On` does not consult either number.
        assert!(should_review(AfterEdit::On, small, 20));
    }

    #[test]
    fn a_diff_is_recognised_by_its_hunk_header() {
        assert!(looks_like_diff(DIFF));
        // The one-line "Edited …" header alone is not a diff: there is nothing to
        // review in a summary.
        assert!(!looks_like_diff("Edited a.c (1 lines -> 3 lines)"));
    }

    #[test]
    fn the_path_comes_from_the_header_without_the_git_prefix() {
        assert_eq!(path_from_diff(DIFF).as_deref(), Some("a.c"));
        assert_eq!(
            path_from_diff("--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n").as_deref(),
            Some("src/main.rs")
        );
        // A deletion: `+++` is /dev/null, so the `---` side names what was removed —
        // still the right title for reviewing the change.
        assert_eq!(
            path_from_diff("--- a/x.c\n+++ /dev/null\n@@ -1 +0,0 @@\n").as_deref(),
            Some("x.c")
        );
        // Nothing usable at all: the caller labels the review with the tool name.
        assert_eq!(path_from_diff("--- /dev/null\n+++ /dev/null\n"), None);
        assert_eq!(path_from_diff("no header here"), None);
    }

    #[test]
    fn headers_are_not_counted_as_changes() {
        let size = diff_size(DIFF);
        assert_eq!(size.added, 2);
        assert_eq!(size.removed, 1);
        assert_eq!(size.hunks, 1);
        assert_eq!(size.total(), 3);
        // The `---`/`+++` header lines are not a change each: a diff of one line that
        // added nothing would otherwise report two.
        assert_eq!(diff_size("--- a/f\n+++ b/f\n").total(), 0);
    }

    #[test]
    fn separate_hunks_are_counted_separately() {
        let diff = "@@ -1 +1 @@\n-a\n+b\n@@ -20 +20 @@\n-c\n+d\n";
        let size = diff_size(diff);
        assert_eq!(size.hunks, 2);
        assert_eq!(size.total(), 4);
    }

    #[test]
    fn the_prompt_carries_the_size_the_diff_and_the_rules() {
        let prompt = review_prompt("src/main.rs", DIFF, None);
        assert!(prompt.contains("File: src/main.rs"));
        assert!(prompt.contains("+2 -1 lines across 1 hunk(s)"));
        assert!(prompt.contains("+    loop_forever();"));
        // The instruction that keeps the badge believable.
        assert!(prompt.contains("Do not report style, naming or formatting."));
        assert!(prompt.contains("An empty array (`[]`) is the right answer"));
    }

    #[test]
    fn an_answer_in_a_code_fence_is_read() {
        let answer = "```json\n[{\"title\": \"leaks the port\", \"severity\": \"high\", \
                      \"description\": \"the handle is never closed\", \"fix\": \"close it\", \
                      \"steps\": [\"run it twice\"]}]\n```";
        let (findings, note) = parse_findings(answer, "a.c");
        assert!(note.is_none(), "got: {note:?}");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::High);
        assert_eq!(findings[0].title, "leaks the port");
        assert_eq!(findings[0].file.as_deref(), Some("a.c"));
        assert_eq!(findings[0].fix.as_deref(), Some("close it"));
        assert_eq!(findings[0].steps, ["run it twice"]);
    }

    #[test]
    fn an_empty_array_is_a_clean_review() {
        let (findings, note) = parse_findings("[]", "a.c");
        assert!(findings.is_empty());
        assert!(note.is_none(), "no findings and no note is a real result");
    }

    #[test]
    fn prose_instead_of_json_is_a_note_not_a_finding() {
        // The distinction the module exists to keep: a review nobody could read is not
        // a review that found nothing.
        let (findings, note) = parse_findings("I could not review this change.", "a.c");
        assert!(findings.is_empty());
        assert!(note.is_some(), "an unreadable answer must leave a note");

        // Truncated mid-array: the note says which of the two failures this is, because
        // one of them has a remedy (the answer hit the token limit).
        let (findings, note) = parse_findings("[{\"title\": ", "a.c");
        assert!(findings.is_empty());
        assert!(note.unwrap().contains("cut off"), "got the wrong note");

        let (findings, note) = parse_findings("[{\"title\": }]", "a.c");
        assert!(findings.is_empty());
        assert!(note.unwrap().contains("not valid JSON"));
    }

    #[test]
    fn an_item_without_a_title_is_dropped_rather_than_rendered_blank() {
        let answer = r#"[{"severity": "high", "description": "no title"}, {"title": "kept", "description": "d"}]"#;
        let (findings, note) = parse_findings(answer, "a.c");
        assert!(note.is_none());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].title, "kept");
    }
}
