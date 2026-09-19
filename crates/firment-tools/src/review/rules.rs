//! Deterministic static review rules (plan §4-C).
//!
//! The plan splits this capability in two: **rules that cost nothing and can be tested**
//! here, and an LLM pass that adds what rules cannot see. This module is the first half,
//! and it is deliberately small: a rule earns its place by catching something a reviewer
//! would ask to change, not by being an opinion.
//!
//! Three rules, chosen because each one is a real defect class in *this* project's world
//! (Rust on a host, C on a target, and a serial/ISR boundary):
//!
//! * **`unsafe` with no reason given.** The language's own convention is a `SAFETY:` note
//!   saying why the block is sound. A missing one is a question the next reader has to
//!   answer from scratch — and the answer is not in the code.
//! * **A blocking call inside an interrupt handler.** An ISR that sleeps, prints over
//!   UART or waits on a lock hangs the system it was meant to serve. This is a heuristic
//!   and says so: it names the call it found so the reader can disagree in one look.
//! * **`cargo clippy`'s own diagnostics**, parsed from `--message-format=json`, so the
//!   report can carry what the compiler already knows instead of asking a model to guess
//!   it again.

use firment_core::review::{Finding, Severity};

/// A line of the file with its **own** line number. The number travels with the text so a
/// finding can say "line 300" and mean line 300 of the file, whatever the rules chose to
/// skip along the way.
type Line<'a> = (usize, &'a str);

/// One file's review: the findings, and what the rules deliberately did not look at.
#[derive(Debug, Clone, Default)]
pub struct SourceReview {
    pub findings: Vec<Finding>,
    /// Lines skipped because they sit inside a `#[cfg(test)]` module.
    ///
    /// Counted rather than silently dropped: "we did not look at the tests" is a fact the
    /// reader is entitled to, and a report that hid it would imply the whole file was
    /// checked.
    pub skipped_test_lines: usize,
}

/// Every rule that applies to one file's text.
///
/// `#[cfg(test)]` modules are skipped, and running the rules on this repository is why:
/// they reported twelve findings in a single file, every one of them a *fixture* — a test
/// demonstrating an `unsafe` block, a C snippet in a string. A test that shows the pattern
/// is not a defect, and neither is `env::set_var` in a single-threaded test, which is
/// unsafe by declaration and sound in practice.
pub fn review_source(path: &str, text: &str) -> SourceReview {
    let all: Vec<&str> = text.lines().collect();
    let skipped = test_module_ranges(&all);
    let is_skipped = |index: usize| {
        skipped
            .iter()
            .any(|(from, to)| (*from..=*to).contains(&index))
    };
    let skipped_test_lines = (0..all.len()).filter(|index| is_skipped(*index)).count();
    let visible: Vec<Line<'_>> = all
        .iter()
        .enumerate()
        .filter(|(index, _)| !is_skipped(*index))
        .map(|(index, line)| (index, *line))
        .collect();

    let mut findings = Vec::new();
    if path.ends_with(".rs") {
        findings.extend(unsafe_without_reason(path, &visible));
    }
    findings.extend(blocking_call_in_isr(path, &visible));
    SourceReview {
        findings,
        skipped_test_lines,
    }
}

/// The line ranges of `#[cfg(test)]` modules, found by brace counting.
fn test_module_ranges(lines: &[&str]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        if !lines[index].trim_start().starts_with("#[cfg(test)]") {
            index += 1;
            continue;
        }
        let open = match (index..lines.len()).find(|i| lines[*i].contains('{')) {
            Some(open) => open,
            None => break,
        };
        let end = block_end(lines, open);
        ranges.push((index, end));
        index = end + 1;
    }
    ranges
}

/// The last line of the block that opens at or after `start`, by brace counting.
fn block_end(lines: &[&str], start: usize) -> usize {
    let mut depth = 0i32;
    let mut opened = false;
    for (offset, line) in lines.iter().enumerate().skip(start) {
        for ch in code_only(line).chars() {
            match ch {
                '{' => {
                    depth += 1;
                    opened = true;
                }
                '}' => depth -= 1,
                _ => {}
            }
        }
        if opened && depth <= 0 {
            return offset;
        }
    }
    lines.len().saturating_sub(1)
}

/// The line with any trailing `//` comment removed, so braces and calls inside prose do
/// not count.
fn code_only(line: &str) -> &str {
    match line.split_once("//") {
        Some((before, _)) => before,
        None => line,
    }
}

/// Whether the first occurrence of `needle` in `line` sits inside a string literal.
///
/// A quote count is a heuristic and is treated as one: it is right about `"unsafe {"` in
/// code and about `let s = "x"`, and it is wrong about raw strings with quotes in them —
/// where it fails *open* (the line is treated as code), which is the direction that keeps
/// a real finding rather than losing one.
fn pattern_is_quoted(line: &str, needle: &str) -> bool {
    let Some(at) = line.find(needle) else {
        return false;
    };
    line[..at].matches('"').count() % 2 == 1
}

/// How far a `SAFETY:` note may sit from its block and still count.
///
/// Three lines above is enough for the conventional `// SAFETY: …` immediately before the
/// block plus a blank line and an attribute; one line below covers the note written inside
/// the block, which is just as good. Wider than this and the note stops being *about* this
/// block.
const SAFETY_WINDOW: usize = 3;

fn unsafe_without_reason(path: &str, lines: &[Line<'_>]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (slot, (file_line, line)) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        // `unsafe fn`/`unsafe impl` declare a contract, they do not perform an operation;
        // `unsafe {` is where the unchecked operation actually happens.
        if !trimmed.contains("unsafe {") {
            continue;
        }
        // A comment that *mentions* unsafe is not a reason for it, and neither is a
        // string that contains the pattern — including this rule's own source, which the
        // review of this repository found by reporting itself.
        if trimmed.starts_with("//") || pattern_is_quoted(line, "unsafe {") {
            continue;
        }
        let from = slot.saturating_sub(SAFETY_WINDOW);
        let to = (slot + 1).min(lines.len().saturating_sub(1));
        let explained = lines[from..=to].iter().any(|(_, near)| {
            let lower = near.to_ascii_lowercase();
            lower.contains("safety") || lower.contains("sound because")
        });
        if explained {
            continue;
        }
        findings.push(
            Finding::new(
                format!("unsafe-no-safety-{}", file_line + 1),
                format!("unsafe block with no stated reason (line {})", file_line + 1),
                Severity::Medium,
                "static",
                "Nothing in the surrounding lines says why this block is sound, so every reader has to re-derive it — and the next reader has less context than the author did.",
            )
            .with_file(path)
            .with_impact("An unjustified unsafe block is the one place a memory-safety bug can hide without any rule being able to see it.")
            .with_code(trimmed.to_string())
            .with_fix("Add a `// SAFETY:` line above the block saying what makes it sound, or take the operation out of `unsafe`.")
            .with_tags(&["unsafe", "rust"]),
        );
    }
    findings
}

/// Calls that block the caller. Named explicitly rather than matched by a pattern, so a
/// false positive can be removed by deleting one entry.
const BLOCKING_CALLS: [&str; 12] = [
    "delay",
    "sleep",
    "printf",
    "println!",
    "print!",
    "uart_write",
    "hal_delay",
    "vTaskDelay",
    "malloc",
    "wait_for",
    ".lock()",
    ".await",
];

/// How far to look when a header has no body at all (a declaration, or a macro the
/// preprocessor runs before this sees it).
const ISR_BODY_FALLBACK_LINES: usize = 40;

fn blocking_call_in_isr(path: &str, lines: &[Line<'_>]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (slot, (_, line)) in lines.iter().enumerate() {
        if !looks_like_isr_header(line) {
            continue;
        }
        // The body ends at the handler's own closing brace; `None` means the header has no
        // body the rules can see, and the fallback window applies.
        let end = match handler_end(lines, slot) {
            Some(end) => end,
            None => (slot + ISR_BODY_FALLBACK_LINES).min(lines.len().saturating_sub(1)),
        };
        for (file_line, body_line) in lines[slot..=end].iter() {
            let code = code_only(body_line);
            // Case-insensitive: C naming conventions spell these HAL_Delay, vTaskDelay,
            // UART_Write, and a rule that only matched the lowercase spelling would miss
            // most of the code it is for.
            let lowered = code.to_ascii_lowercase();
            let Some(call) = BLOCKING_CALLS
                .iter()
                .find(|call| lowered.contains(&call.to_ascii_lowercase()))
            else {
                continue;
            };
            let at = file_line + 1;
            findings.push(
                Finding::new(
                    format!("isr-blocks-{at}"),
                    format!("interrupt handler calls `{call}` (line {at})"),
                    Severity::Medium,
                    "static",
                    format!("`{call}` blocks the caller. Inside an interrupt handler that means every other interrupt — and any code waiting on this one — waits too."),
                )
                .with_file(path)
                .with_impact("A handler that blocks turns a short interrupt into a system-wide stall, and on a target it is a hang nobody can debug after the fact.")
                .with_code(code.trim().to_string())
                .with_steps(vec![
                    format!("Check line {at}: is the call non-blocking in this configuration?"),
                    "If it blocks, move the work out of the handler and signal a task instead.".to_string(),
                ])
                .with_fix("Set a flag or push to a queue in the handler; do the work where waiting is allowed.")
                .with_tags(&["isr", "embedded", "heuristic"]),
            );
            // One finding per handler is enough to send someone to the right place.
            break;
        }
    }
    findings
}

/// The last visible line of an interrupt handler's body.
///
/// A fixed window is not good enough, and a test found why: the next function in the file
/// is usually where the blocking call lives — `main()` delaying, `printf` in a helper —
/// and a window that reached into it would report that call as if it were in the handler.
/// `None` when the header has no braces at all, which the caller turns into the fallback
/// window rather than into a scan to the end of the file.
fn handler_end(lines: &[Line<'_>], header: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut opened = false;
    for (slot, (_, line)) in lines.iter().enumerate().skip(header) {
        for ch in code_only(line).chars() {
            match ch {
                '{' => {
                    depth += 1;
                    opened = true;
                }
                '}' => depth -= 1,
                _ => {}
            }
        }
        if opened && depth <= 0 {
            return Some(slot);
        }
    }
    None
}

/// Whether a line opens an interrupt handler.
///
/// The two ecosystems spell it differently and both are common in this project: Cortex-M
/// C (`void TIM2_IRQHandler(void)`, `__attribute__((interrupt))`, `ISR(TIMER2_vect)`) and
/// embedded Rust (`#[interrupt]`, `#[exception]`).
fn looks_like_isr_header(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return false;
    }
    let patterns = [
        "#[interrupt]",
        "#[exception]",
        "__attribute__((interrupt",
        "ISR(",
        "_IRQHandler(",
        "Handler(void)",
    ];
    patterns.iter().any(|p| line.contains(p))
}

/// `cargo clippy --message-format=json` diagnostics as findings.
///
/// The JSON is newline-delimited, with one object per diagnostic plus a few objects that
/// are not diagnostics at all (`build-finished`, `compiler-artifact`). Those are skipped
/// silently; a broken line is counted, because a clippy run we cannot read is a part of the
/// review that did not happen.
pub fn findings_from_clippy(json: &str) -> (Vec<Finding>, usize) {
    let mut findings = Vec::new();
    let mut unread = 0usize;
    for line in json.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            unread += 1;
            continue;
        };
        if value["reason"].as_str() != Some("compiler-message") {
            continue;
        }
        let message = &value["message"];
        let level = message["level"].as_str().unwrap_or("warning");
        if !matches!(level, "warning" | "error") {
            continue;
        }
        let text = message["message"].as_str().unwrap_or("(no message)");
        let code = message["code"]["code"].as_str().unwrap_or("");
        let span = &message["spans"][0];
        let file = span["file_name"].as_str().unwrap_or("");
        let at = span["line_start"].as_u64().unwrap_or(0);
        let severity = if level == "error" {
            Severity::High
        } else {
            Severity::Medium
        };
        let mut finding = Finding::new(
            format!("clippy-{code}-{file}-{at}"),
            format!("clippy: {text}"),
            severity,
            "static",
            format!(
                "{} {file}:{at}",
                if code.is_empty() {
                    "diagnostic at"
                } else {
                    code
                }
            ),
        )
        .with_file(file)
        .with_tags(&["clippy"]);
        if let Some(rendered) = message["rendered"].as_str() {
            finding.steps.push(rendered.trim().to_string());
        }
        findings.push(finding);
    }
    (findings, unread)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unsafe_block_with_a_safety_note_is_left_alone() {
        let source = "\
fn f() {
    // SAFETY: the pointer comes from the allocator we just called, and nothing else
    // holds it.
    unsafe { *p = 1 }
}
";
        assert!(review_source("a.rs", source).findings.is_empty());
    }

    #[test]
    fn an_unsafe_block_without_a_reason_is_a_finding_that_quotes_it() {
        let source = "\
fn f() {
    let ok = unsafe { std::env::set_var(\"A\", \"b\") };
}
";
        let findings = review_source("a.rs", source).findings;
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Medium);
        assert!(
            findings[0].title.contains("line 2"),
            "{:?}",
            findings[0].title
        );
        assert_eq!(findings[0].file.as_deref(), Some("a.rs"));
        // The reader needs the line itself, not a description of it.
        assert!(findings[0].code.as_deref().unwrap().contains("set_var"));
    }

    #[test]
    fn a_comment_about_unsafe_is_not_an_unsafe_block() {
        // Prose mentioning `unsafe {` must not fire: the rule is about code.
        let source = "// we could write unsafe { here } but we do not\nfn f() {}\n";
        assert!(review_source("a.rs", source).findings.is_empty());
    }

    #[test]
    fn unsafe_fn_declarations_are_not_unsafe_operations() {
        // `unsafe fn` states a contract on the caller; there is nothing here to justify.
        let source = "unsafe fn f() {}\nunsafe impl Send for X {}\n";
        assert!(review_source("a.rs", source).findings.is_empty());
    }

    #[test]
    fn a_blocking_call_in_an_interrupt_handler_is_named() {
        let source = "\
void TIM2_IRQHandler(void) {
    HAL_GPIO_TogglePin(LED_PORT, LED_PIN);
    HAL_Delay(1);
}
";
        let findings = review_source("main.c", source).findings;
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0].title.contains("`delay`"),
            "{:?}",
            findings[0].title
        );
        assert!(
            findings[0].title.contains("line 3"),
            "{:?}",
            findings[0].title
        );
        assert_eq!(findings[0].tags, ["isr", "embedded", "heuristic"]);
    }

    #[test]
    fn a_handler_that_does_not_block_is_quiet() {
        // The delay in `main` is right after the handler and must not count: the scan ends
        // at the handler's closing brace, not at a fixed number of lines. A fixed window
        // reported this — the test that caught it is the reason `handler_end` exists.
        let source = "\
void TIM2_IRQHandler(void) {
    flag = 1;
}
void main(void) { HAL_Delay(100); }
";
        assert!(review_source("main.c", source).findings.is_empty());
    }

    #[test]
    fn a_handler_whose_brace_is_on_the_next_line_still_bounds_its_scan() {
        let source = "\
ISR(TIMER2_vect)
{
    flag = 1;
}
void main(void) { printf(\"hi\"); }
";
        assert!(review_source("main.c", source).findings.is_empty());
    }

    #[test]
    fn a_rust_interrupt_handler_awaiting_is_caught_too() {
        let source = "\
#[interrupt]
fn TIM2() {
    let _ = tx.send(1).await;
}
";
        let findings = review_source("a.rs", source).findings;
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].title.contains("`.await`"));
    }

    #[test]
    fn a_cfg_test_module_is_skipped_and_counted() {
        // The defect this test exists for: running the rules over this repository reported
        // twelve findings in this very file, all of them fixtures in the test module below
        // — a test that demonstrates an `unsafe` block is not a defect. The count comes
        // back so a report can say what it did not look at.
        let source = "\
fn production() {
    unsafe { *p = 1 }
}

#[cfg(test)]
mod tests {
    #[test]
    fn demonstrates() {
        unsafe { *q = 2 }
    }
}
";
        let review = review_source("a.rs", source);
        assert_eq!(review.findings.len(), 1, "{:?}", review.findings);
        assert!(review.findings[0].title.contains("line 2"));
        assert!(
            review.skipped_test_lines >= 6,
            "{}",
            review.skipped_test_lines
        );
    }

    #[test]
    fn a_finding_after_a_test_module_keeps_the_files_own_line_number() {
        // Skipping lines must not renumber the rest: a finding that says "line 8" has to
        // mean line 8 of the file, or the reader opens the wrong line every time.
        let source = "\
#[cfg(test)]
mod tests {
    fn shows() {
        let _ = 1;
    }
}
fn production() {
    unsafe { *p = 1 }
}
";
        let review = review_source("a.rs", source);
        assert_eq!(review.findings.len(), 1);
        assert!(
            review.findings[0].title.contains("line 8"),
            "{:?}",
            review.findings[0].title
        );
    }

    #[test]
    fn a_string_holding_the_pattern_is_not_an_unsafe_block() {
        // Found by running the rules on this repository: the rule reported its own source,
        // because the guard below looks for the pattern in a string.
        let source =
            "fn f() {\n    if !trimmed.contains(\"unsafe {\") {\n        return;\n    }\n}\n";
        assert!(review_source("a.rs", source).findings.is_empty());
    }

    #[test]
    fn clippy_diagnostics_become_findings_and_other_objects_are_skipped() {
        let json = r#"{"reason":"compiler-artifact","package_id":"x"}
{"reason":"compiler-message","message":{"level":"warning","message":"unused variable: `x`","code":{"code":"unused_variables"},"spans":[{"file_name":"src/main.rs","line_start":12}]}}
{"reason":"compiler-message","message":{"level":"error","message":"cannot borrow as mutable","code":{"code":"E0596"},"spans":[{"file_name":"src/lib.rs","line_start":4}],"rendered":"error[E0596]: ...\n"}}
{"reason":"build-finished","success":true}"#;
        let (findings, unread) = findings_from_clippy(json);
        assert_eq!(unread, 0);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].severity, Severity::Medium);
        assert_eq!(findings[1].severity, Severity::High);
        assert!(findings[1].title.contains("cannot borrow as mutable"));
        assert_eq!(findings[1].file.as_deref(), Some("src/lib.rs"));
    }

    #[test]
    fn a_clippy_line_that_is_not_json_is_counted() {
        // Half a clippy run is not a clean clippy run.
        let (findings, unread) = findings_from_clippy("{\"reason\":\"compiler-mess\n");
        assert!(findings.is_empty());
        assert_eq!(unread, 1);
    }
}
