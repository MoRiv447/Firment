//! Architecture decision records (plan §5, item 4).
//!
//! A project's decisions are the part of its history that is **not in the code**: why the
//! interrupt handler owns that buffer, why the build script generates that table, why the
//! obvious library is not used. An agent that cannot see them re-litigates them every
//! session — or worse, quietly contradicts one — and the person reviewing the diff is the
//! one who pays.
//!
//! Two rules shape everything here:
//!
//! * **A digest, not the text.** The prompt gets the list — number, status, title, one line,
//!   and the path to read. Pasting every ADR into every request would spend the context
//!   window on decisions this turn does not need, and would grow with the project.
//! * **Silence costs nothing.** A project with no `docs/adr/` gets no section at all, so
//!   nothing changes for the many projects that keep their decisions elsewhere. (The
//!   project's `AGENTS.md`/`FIRMENT.md` are loaded by `context::load_project_instructions`
//!   and are a different thing: conventions for *how to work*, not decisions about *what
//!   was built*.)
//!
//! The format is Nygard's, which is what the ecosystem writes and what a person who has
//! never seen this tool will produce:
//!
//! ```text
//! # 1. Use Rust for the core
//! Date: 2026-09-20
//! Status: Accepted
//!
//! ## Context
//! …
//! ```

use std::path::{Path, PathBuf};

/// Where the records live, relative to a project root.
pub const DIR: &str = "docs/adr";

/// How many records the digest names before it summarises the rest.
///
/// Twelve lines of a prompt is a reminder; a hundred is a document nobody asked for. The
/// count that is left out is stated, so the section never looks complete when it is not.
pub const MAX_ADRS: usize = 12;

/// How much of one decision's opening paragraph the digest carries.
pub const MAX_SUMMARY: usize = 140;

/// One decision record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adr {
    /// From the filename (`0007-…`), which is the number people cite.
    pub number: u32,
    pub title: String,
    /// `Accepted`, `Proposed`, `Superseded by 0012`, … `None` when the file does not say.
    pub status: Option<String>,
    pub path: PathBuf,
    /// The opening paragraph, truncated. Never the whole document.
    pub summary: String,
}

impl Adr {
    /// One line for the digest.
    pub fn line(&self) -> String {
        let status = match &self.status {
            Some(status) => format!(" [{status}]"),
            None => String::new(),
        };
        let summary = if self.summary.is_empty() {
            String::new()
        } else {
            format!(" — {}", self.summary)
        };
        format!("- ADR-{:04}{status}: {}{summary}", self.number, self.title)
    }
}

/// `<root>/docs/adr`, whether or not it exists.
pub fn dir(root: &Path) -> PathBuf {
    root.join(DIR)
}

/// Every record under `<root>/docs/adr`, ordered by number.
///
/// Unreadable files are skipped rather than failing the scan: this runs while building a
/// system prompt, and a prompt that cannot be built because one ADR has odd permissions is
/// worse than a prompt missing one line.
pub fn scan(root: &Path) -> Vec<Adr> {
    let Ok(entries) = std::fs::read_dir(dir(root)) else {
        return Vec::new();
    };
    let mut records: Vec<Adr> = entries
        .flatten()
        .filter(|entry| {
            entry.path().extension().and_then(|e| e.to_str()) == Some("md")
                && entry.path().is_file()
        })
        .filter_map(|entry| {
            let path = entry.path();
            let number = number_from_path(&path)?;
            let text = std::fs::read_to_string(&path).ok()?;
            Some(parse(&text, number, path))
        })
        .collect();
    records.sort_by_key(|record| record.number);
    records
}

/// The leading number of a record's filename (`0007-title.md` → 7).
fn number_from_path(path: &Path) -> Option<u32> {
    let stem = path.file_stem()?.to_str()?;
    let digits: String = stem.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Parse one record's text.
///
/// Forgiving on purpose: the heading may be `# 1. Title`, `# ADR-0001: Title` or just
/// `# Title`, and the metadata may be `Status:` or `**Status**:`. A parser that demanded one
/// spelling would make the feature work only in the project that wrote the parser.
pub fn parse(text: &str, number: u32, path: PathBuf) -> Adr {
    let mut title = String::new();
    let mut status = None;
    let mut metadata_done = false;
    let mut summary_lines: Vec<String> = Vec::new();

    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if index < 40 {
            let lowered = trimmed.to_ascii_lowercase();
            if let Some(rest) = lowered
                .strip_prefix("status:")
                .or_else(|| lowered.strip_prefix("**status**:"))
            {
                let _ = rest;
                // Take the value from the *original* line, so the case survives.
                if let Some((_, value)) = trimmed.split_once(':') {
                    let value = value.trim().trim_matches('*').trim();
                    if !value.is_empty() {
                        status = Some(value.to_string());
                    }
                }
            }
        }
        if title.is_empty() {
            if let Some(heading) = trimmed.strip_prefix("# ") {
                title = clean_title(heading, number);
            }
            continue;
        }
        if trimmed.starts_with("## ") {
            // Before any prose a heading is structure to look past — Nygard's format puts
            // the opening paragraph under `## Context`, so treating a heading as the end of
            // the summary would leave every such record with an empty one. After the prose
            // has started, a heading is where the section ended.
            if summary_lines.is_empty() {
                continue;
            }
            break;
        }
        if !metadata_done {
            // The metadata block sits between the heading and the first prose.
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.contains(':') && !trimmed.starts_with('-') {
                continue;
            }
            metadata_done = true;
        }
        if trimmed.is_empty() {
            if !summary_lines.is_empty() {
                break;
            }
            continue;
        }
        summary_lines.push(trimmed.to_string());
    }

    let summary = truncate(&summary_lines.join(" "), MAX_SUMMARY);
    Adr {
        number,
        title: if title.is_empty() {
            format!("(untitled {number:04})")
        } else {
            title
        },
        status,
        path,
        summary,
    }
}

/// Strip the number prefix a heading may carry, so the digest does not say it twice.
fn clean_title(heading: &str, number: u32) -> String {
    let trimmed = heading.trim();
    let without_adr = trimmed
        .strip_prefix(&format!("ADR-{number:04}"))
        .or_else(|| trimmed.strip_prefix(&format!("ADR-{number}")))
        .unwrap_or(trimmed)
        .trim_start_matches([':', '.', '-', ' ']);
    let without_ordinal = without_adr
        .strip_prefix(&format!("{number}."))
        .or_else(|| without_adr.strip_prefix(&format!("{number})")))
        .unwrap_or(without_adr)
        .trim_start();
    without_ordinal.to_string()
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let cut: String = text.chars().take(limit).collect();
    format!("{}…", cut.trim_end())
}

/// The section for the system prompt, or `None` when the project has no records.
pub fn prompt_section(root: &Path) -> Option<String> {
    let records = scan(root);
    if records.is_empty() {
        return None;
    }
    let mut out = String::from(
        "# Architecture decisions (docs/adr)\n\
         The project's decisions live here. Read the record with read_file before changing \
         something it covers — and if the change you are asked for contradicts one, say so \
         instead of quietly overriding it.\n",
    );
    for record in records.iter().take(MAX_ADRS) {
        out.push_str(&record.line());
        out.push('\n');
    }
    if records.len() > MAX_ADRS {
        out.push_str(&format!(
            "- … and {} more record(s) in {}/\n",
            records.len() - MAX_ADRS,
            DIR
        ));
    }
    Some(out)
}

/// The number the next record should get.
pub fn next_number(root: &Path) -> u32 {
    scan(root)
        .iter()
        .map(|record| record.number)
        .max()
        .map(|highest| highest + 1)
        .unwrap_or(1)
}

/// The skeleton `firm adr new` writes.
///
/// A template rather than an empty file: the three things every ADR needs are the three
/// things a person writing one forgets, and a file that asks the question is a file that
/// gets answered.
pub fn template(number: u32, title: &str, date: &str) -> String {
    format!(
        "# {number}. {title}\n\
         Date: {date}\n\
         Status: Proposed\n\
         \n\
         ## Context\n\
         \n\
         What is the situation that calls for a decision? Include the constraints that are\n\
         not obvious from the code — hardware, time, people, an earlier attempt that failed.\n\
         \n\
         ## Decision\n\
         \n\
         What we will do, in the active voice: \"We will …\". One decision per record.\n\
         \n\
         ## Consequences\n\
         \n\
         What becomes easier, and what becomes harder or impossible. What a future reader\n\
         will have to know to not undo this by accident.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# 7. Keep the ISR short

Date: 2026-09-20
Status: Accepted

## Context

The IMU driver used to parse frames inside the interrupt handler.

## Decision

We will copy the raw frame into a queue inside the handler and parse it in the main loop.

## Consequences

The handler is a few microseconds. Parsing latency grows by one loop iteration.
";

    fn parsed() -> Adr {
        parse(
            SAMPLE,
            7,
            PathBuf::from("docs/adr/0007-keep-the-isr-short.md"),
        )
    }

    #[test]
    fn a_record_reads_back_with_its_number_status_and_first_paragraph() {
        let record = parsed();
        assert_eq!(record.number, 7);
        assert_eq!(record.title, "Keep the ISR short");
        assert_eq!(record.status.as_deref(), Some("Accepted"));
        // The opening paragraph of the *body* — which in this format sits under
        // `## Context`, and is the one sentence a reader of the digest needs.
        assert!(
            record.summary.starts_with("The IMU driver"),
            "{}",
            record.summary
        );
        assert!(
            !record.summary.contains("## Decision"),
            "{}",
            record.summary
        );
    }

    #[test]
    fn the_heading_may_be_written_any_of_the_ways_people_write_it() {
        // The number must not appear twice, whatever spelling the author chose.
        for heading in [
            "# 7. Keep it short",
            "# ADR-0007: Keep it short",
            "# Keep it short",
        ] {
            let record = parse(
                &format!("{heading}\n\nStatus: Accepted\n"),
                7,
                PathBuf::new(),
            );
            assert_eq!(record.title, "Keep it short", "{heading}");
        }
    }

    #[test]
    fn a_record_that_does_not_state_a_status_still_reads() {
        let record = parse("# 3. Something\n\nWe will do it.\n", 3, PathBuf::new());
        assert_eq!(record.status, None);
        assert!(record.line().starts_with("- ADR-0003: Something"));
        assert!(!record.line().contains("["), "no status means no brackets");
    }

    #[test]
    fn a_long_opening_paragraph_is_truncated_in_the_digest() {
        let long = "word ".repeat(80);
        let record = parse(&format!("# 1. Long\n\n{long}\n"), 1, PathBuf::new());
        assert!(
            record.summary.chars().count() <= MAX_SUMMARY + 1,
            "{}",
            record.summary
        );
        assert!(record.summary.ends_with('…'));
    }

    #[test]
    fn the_digest_is_bounded_and_says_what_it_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let adr_dir = dir.path().join(DIR);
        std::fs::create_dir_all(&adr_dir).unwrap();
        for number in 1..=(MAX_ADRS + 3) {
            std::fs::write(
                adr_dir.join(format!("{number:04}-record.md")),
                format!("# {number}. Record {number}\n\nStatus: Accepted\n\nWhy.\n"),
            )
            .unwrap();
        }
        // A non-markdown file in the folder is not a record.
        std::fs::write(adr_dir.join("notes.txt"), "not a record").unwrap();

        let section = prompt_section(dir.path()).unwrap();
        assert!(section.contains("ADR-0001"), "{section}");
        assert!(section.contains("ADR-0012"), "{section}");
        assert!(!section.contains("ADR-0013"), "the digest stops at the cap");
        assert!(section.contains("and 3 more record(s)"), "{section}");
        // The instruction that makes the digest useful rather than decorative.
        assert!(
            section.contains("say so instead of quietly overriding it"),
            "{section}"
        );
    }

    #[test]
    fn a_project_without_records_gets_no_section_at_all() {
        // The common case: nothing is injected, so nothing costs anything.
        let dir = tempfile::tempdir().unwrap();
        assert!(prompt_section(dir.path()).is_none());
        assert_eq!(next_number(dir.path()), 1);
    }

    #[test]
    fn the_next_number_follows_the_highest_one_and_a_new_record_gets_a_usable_skeleton() {
        let dir = tempfile::tempdir().unwrap();
        let adr_dir = dir.path().join(DIR);
        std::fs::create_dir_all(&adr_dir).unwrap();
        // Out of order on disk, and with a gap: the next number is the highest plus one.
        std::fs::write(adr_dir.join("0003-c.md"), "# 3. C\n").unwrap();
        std::fs::write(adr_dir.join("0001-a.md"), "# 1. A\n").unwrap();
        assert_eq!(next_number(dir.path()), 4);

        let text = template(4, "Use the journal", "2026-09-20");
        assert!(text.starts_with("# 4. Use the journal\n"), "{text}");
        assert!(text.contains("Status: Proposed"), "{text}");
        // The three questions a person writing an ADR forgets to answer.
        for section in ["## Context", "## Decision", "## Consequences"] {
            assert!(text.contains(section), "{text}");
        }
        // And the skeleton parses back into a record, so the digest sees it immediately.
        let record = parse(&text, 4, adr_dir.join("0004-use-the-journal.md"));
        assert_eq!(record.number, 4);
        assert_eq!(record.status.as_deref(), Some("Proposed"));
    }
}
