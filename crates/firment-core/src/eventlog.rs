//! The session event log (plan §5, item 1): the record `replay` reads.
//!
//! A session is a sequence of things that happened — turns started and finished, tools
//! started and finished, changes reviewed, sessions loaded. The transcript holds the
//! *conversation*; this holds the *operations*, which is what "go back to before that edit"
//! needs and what a bug report needs when the reporter cannot remember the order.
//!
//! Two constraints come from the plan's own review and shape the whole module:
//!
//! * **On disk, capped, rotated** (§16.2-11). A long session must not grow a file without
//!   limit, and the log must survive a crash — so it is appended as it happens, and when it
//!   passes the cap the oldest generation is dropped rather than the newest.
//! * **Significant events only.** Text and thinking deltas are the majority of the events
//!   and none of the meaning: they are the model's output, reconstructible from the
//!   transcript, and writing every token would make the log larger than the conversation it
//!   describes.
//!
//! It is a *sink decorator* rather than a flag on the agent: wrapping the sink in
//! [`LoggingSink`] at assembly means every surface gets the log for free, and the 42 places
//! inside the agent that emit directly do not each have to remember to log.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::agent::{AgentEvent, EventSink};

/// How large a log may grow before it rotates.
///
/// Generous enough that a long session rarely rotates (the lines are one per operation, not
/// per token) and small enough that the pair of generations stays a bug report rather than
/// an archive.
pub const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// How many generations are kept, including the current file.
///
/// Two: enough to still hold the beginning of a session that rotated, not enough to turn a
/// session directory into a log directory.
pub const GENERATIONS: u32 = 2;

/// One line of the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRecord {
    /// Seconds since the Unix epoch, so a reader can order records without trusting the
    /// file's own order to have survived a copy.
    pub at: u64,
    /// What happened: `turn_start`, `tool_end`, `review`, `session_loaded`, …
    pub kind: String,
    /// The one-line description a human reads.
    pub summary: String,
}

impl LogRecord {
    /// The line as it appears in the log's text form, for a reader that wants the file
    /// itself rather than a parser.
    pub fn display(&self) -> String {
        format!("{:<14} {}", self.kind, self.summary)
    }
}

/// Seconds since the Unix epoch.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Whether an event is worth a line.
///
/// The exclusions are the point: a delta is not an operation, the liveness heartbeat (which
/// stays at the provider level — `ProviderEvent::Activity` never becomes an `AgentEvent`, so
/// there is nothing to exclude here) keeps a socket honest without describing anything a
/// person did, and a subagent's own start/end are already described by the `task` tool's
/// `ToolStart`/`ToolEnd` pair that wraps them.
pub fn is_significant(event: &AgentEvent) -> bool {
    !matches!(
        event,
        AgentEvent::TextDelta(_)
            | AgentEvent::Thinking(_)
            | AgentEvent::SubagentStart { .. }
            | AgentEvent::SubagentEnd { .. }
    )
}

/// The record an event becomes, or `None` when it is not worth logging.
pub fn record_of(event: &AgentEvent, at: u64) -> Option<LogRecord> {
    if !is_significant(event) {
        return None;
    }
    let (kind, summary) = match event {
        AgentEvent::TurnStart => ("turn_start".to_string(), "turn started".to_string()),
        AgentEvent::TurnEnd { text } => (
            "turn_end".to_string(),
            format!("turn ended ({} chars)", text.chars().count()),
        ),
        AgentEvent::ToolStart {
            name, seq, owner, ..
        } => (
            "tool_start".to_string(),
            format!("{} {name}", card_ref(*seq, owner)),
        ),
        AgentEvent::ToolEnd {
            name,
            ok,
            summary,
            seq,
            owner,
            ..
        } => (
            "tool_end".to_string(),
            format!(
                "{} {name} {} — {summary}",
                card_ref(*seq, owner),
                if *ok { "ok" } else { "FAILED" }
            ),
        ),
        AgentEvent::Review {
            seq,
            owner,
            findings,
        } => (
            "review".to_string(),
            format!("{} {} finding(s)", card_ref(*seq, owner), findings.len()),
        ),
        AgentEvent::Info(message) => ("info".to_string(), one_line(message)),
        AgentEvent::Error(message) => ("error".to_string(), one_line(message)),
        AgentEvent::SessionLoaded(session) => (
            "session_loaded".to_string(),
            format!("{} ({} messages)", session.id, session.messages.len()),
        ),
        AgentEvent::Settings { .. } => ("settings".to_string(), "settings changed".to_string()),
        other => (
            // Anything new lands here rather than being dropped: an unlogged event is a
            // hole in the record, and a hole is worse than a line whose wording is rough.
            "event".to_string(),
            format!("{other:?}"),
        ),
    };
    Some(LogRecord {
        at,
        kind,
        summary: truncate(&summary, 200),
    })
}

fn one_line(text: &str) -> String {
    truncate(&text.split_whitespace().collect::<Vec<_>>().join(" "), 200)
}

/// A reference to the tool card an event belongs to.
///
/// A bare `#7` stopped identifying one thing: a delegated call is numbered from the subagent's
/// own session, so the parent turn and a `task` child can each show `#7`. The log line says which
/// one it was, because a reader who cannot find the card cannot use the entry.
fn card_ref(seq: u64, owner: &Option<String>) -> String {
    match owner {
        Some(_) => format!("#{seq} (subagent)"),
        None => format!("#{seq}"),
    }
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let cut: String = text.chars().take(limit).collect();
    format!("{cut}…")
}

/// One session's log file.
#[derive(Debug, Clone)]
pub struct EventLog {
    path: PathBuf,
    max_bytes: u64,
    generations: u32,
}

impl EventLog {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            max_bytes: MAX_BYTES,
            generations: GENERATIONS,
        }
    }

    /// With a smaller cap, for a test that must not write four megabytes.
    pub fn with_limits(mut self, max_bytes: u64, generations: u32) -> Self {
        self.max_bytes = max_bytes;
        self.generations = generations;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one record, rotating first if the file is already over the cap.
    ///
    /// Rotation happens *before* the write rather than after: the rule is "the file stays
    /// under the cap", and a write that pushes it over only to rotate later would break the
    /// promise for exactly the line that mattered.
    pub fn append(&self, record: &LogRecord) -> std::io::Result<()> {
        if let Ok(line) = serde_json::to_string(record) {
            self.append_line(&line)?;
        }
        Ok(())
    }

    fn append_line(&self, line: &str) -> std::io::Result<()> {
        self.rotate_if_needed(line.len() as u64)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{line}")
    }

    /// Move the current file aside when it is full, dropping the oldest generation.
    fn rotate_if_needed(&self, incoming: u64) -> std::io::Result<()> {
        let size = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if size + incoming <= self.max_bytes {
            return Ok(());
        }
        // `events.jsonl` → `events.1.jsonl` → drop what was there. Oldest first, so a
        // crash in the middle loses the oldest generation rather than the current one.
        for index in (1..self.generations).rev() {
            let from = self.generation_path(index);
            let to = self.generation_path(index + 1);
            if from.exists() {
                if index + 1 >= self.generations {
                    let _ = std::fs::remove_file(&from);
                } else {
                    let _ = std::fs::rename(&from, &to);
                }
            }
        }
        if self.generations > 1 && self.path.exists() {
            let _ = std::fs::rename(&self.path, self.generation_path(1));
        } else if self.path.exists() {
            let _ = std::fs::remove_file(&self.path);
        }
        Ok(())
    }

    /// `events.jsonl` → `events.1.jsonl`, `events.2.jsonl`, …
    pub fn generation_path(&self, index: u32) -> PathBuf {
        let stem = self
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("events");
        let extension = self
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jsonl");
        self.path
            .with_file_name(format!("{stem}.{index}.{extension}"))
    }

    /// Every record still on disk, oldest generation first.
    ///
    /// Rotated generations are read in order, so a session that outgrew the cap still reads
    /// as one sequence rather than as the tail of itself.
    pub fn read(&self) -> Vec<LogRecord> {
        let mut records = Vec::new();
        for index in (1..self.generations).rev() {
            records.extend(self.read_file(&self.generation_path(index)));
        }
        records.extend(self.read_file(&self.path));
        records
    }

    fn read_file(&self, path: &Path) -> Vec<LogRecord> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str::<LogRecord>(line).ok())
            .collect()
    }
}

/// An [`EventSink`] that writes the significant events to a log and passes everything on.
///
/// The decorator exists so the log is a property of *the session* rather than of each
/// surface: the TUI, the GUI and a one-shot run all go through the assembly, so all three
/// get the same record without any of them knowing about it.
pub struct LoggingSink {
    inner: std::sync::Arc<dyn EventSink>,
    log: Mutex<EventLog>,
}

impl LoggingSink {
    pub fn new(inner: std::sync::Arc<dyn EventSink>, log: EventLog) -> Self {
        Self {
            inner,
            log: Mutex::new(log),
        }
    }
}

#[async_trait::async_trait]
impl EventSink for LoggingSink {
    async fn event(&self, event: AgentEvent) {
        if let Some(record) = record_of(&event, now_secs()) {
            // A poisoned or failing log must not take the session with it: the log is a
            // convenience, and the event it failed to record is still on its way to the UI.
            if let Ok(log) = self.log.lock() {
                let _ = log.append(&record);
            }
        }
        self.inner.event(event).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(kind: &str, summary: &str) -> LogRecord {
        LogRecord {
            at: 1000,
            kind: kind.to_string(),
            summary: summary.to_string(),
        }
    }

    #[test]
    fn records_round_trip_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::new(dir.path().join("events.jsonl"));
        log.append(&record("turn_start", "turn started")).unwrap();
        log.append(&record("tool_end", "#1 edit_file ok")).unwrap();

        let read = log.read();
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].kind, "turn_start");
        assert_eq!(read[1].summary, "#1 edit_file ok");
    }

    #[test]
    fn the_log_rotates_and_keeps_the_newest_lines() {
        // The cap is a promise: the file stays under it. Incoming lines are counted before
        // they are written, or the line that mattered would be the one to break it.
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::new(dir.path().join("events.jsonl")).with_limits(400, 2);
        for index in 0..40 {
            log.append(&record("tool_end", &format!("step {index}")))
                .unwrap();
        }

        let current = std::fs::metadata(log.path()).unwrap().len();
        assert!(current <= 400, "current generation is {current} bytes");

        let read = log.read();
        assert!(read.len() >= 2, "rotation lost everything: {read:?}");
        // The newest line survives, and the oldest is the one that went.
        assert!(read.iter().any(|r| r.summary == "step 39"), "{read:?}");
        assert!(!read.iter().any(|r| r.summary == "step 0"), "{read:?}");
    }

    #[test]
    fn one_generation_keeps_only_the_current_file() {
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::new(dir.path().join("events.jsonl")).with_limits(200, 1);
        for index in 0..20 {
            log.append(&record("tool_end", &format!("step {index}")))
                .unwrap();
        }
        assert!(log.path().exists());
        assert!(!log.generation_path(1).exists());
        assert!(log.read().iter().any(|r| r.summary == "step 19"));
    }

    #[test]
    fn a_missing_log_reads_as_empty_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::new(dir.path().join("nothing.jsonl"));
        assert!(log.read().is_empty());
    }

    #[test]
    fn deltas_are_not_operations() {
        // The filter that keeps the log a record of what happened rather than a transcript
        // written twice.
        assert!(!is_significant(&AgentEvent::TextDelta("hello".into())));
        assert!(!is_significant(&AgentEvent::Thinking("hmm".into())));
        assert!(!is_significant(&AgentEvent::TextDelta("tok".to_string())));
        assert!(is_significant(&AgentEvent::TurnStart));
        assert!(record_of(&AgentEvent::TextDelta("hello".into()), 1).is_none());
    }

    #[test]
    fn every_significant_event_becomes_a_line_including_ones_that_come_later() {
        // The catch-all arm: a new event variant must appear in the log rather than
        // vanish, because a hole in the record is worse than a rough line.
        let events = [
            AgentEvent::TurnStart,
            AgentEvent::TurnEnd {
                text: "done".to_string(),
            },
            AgentEvent::ToolStart {
                name: "build".to_string(),
                args: serde_json::json!({}),
                seq: 3,
                owner: None,
            },
            AgentEvent::ToolEnd {
                name: "build".to_string(),
                ok: false,
                summary: "exit 1".to_string(),
                detail: None,
                seq: 3,
                owner: None,
                waited_ms: None,
            },
            AgentEvent::Review {
                seq: 3,
                owner: None,
                findings: Vec::new(),
            },
            AgentEvent::Info("note".to_string()),
            AgentEvent::Error("boom".to_string()),
        ];
        for event in &events {
            let record = record_of(event, 5).unwrap_or_else(|| panic!("{event:?} was dropped"));
            assert_eq!(record.at, 5);
            assert!(!record.summary.is_empty());
        }
        // The parent turn and a delegated call can both be number 3, since each agent numbers from
        // its own session. A log line that cannot tell them apart cannot be followed back to the
        // card it describes.
        let delegated = record_of(
            &AgentEvent::ToolEnd {
                name: "build".to_string(),
                ok: true,
                summary: "exit 0".to_string(),
                detail: None,
                seq: 3,
                owner: Some("sub-1".to_string()),
                waited_ms: None,
            },
            5,
        )
        .unwrap();
        let own = record_of(
            &AgentEvent::ToolStart {
                name: "build".to_string(),
                args: serde_json::json!({}),
                seq: 3,
                owner: None,
            },
            5,
        )
        .unwrap();
        assert!(
            delegated.summary.contains("(subagent)"),
            "a delegated call must read as one: {}",
            delegated.summary
        );
        assert!(
            !own.summary.contains("(subagent)"),
            "the session's own turn must not: {}",
            own.summary
        );
        // A failure says so: a log that erased the difference would be a log that lies.
        let failed = record_of(
            &AgentEvent::ToolEnd {
                name: "build".to_string(),
                ok: false,
                summary: "exit 1".to_string(),
                detail: None,
                seq: 3,
                owner: None,
                waited_ms: None,
            },
            5,
        )
        .unwrap();
        assert!(failed.summary.contains("FAILED"), "{}", failed.summary);
    }
}
