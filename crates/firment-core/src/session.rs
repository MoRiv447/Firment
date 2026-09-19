use crate::config::config_dir;
use crate::{ChatMessage, SessionMode, ThinkingLevel};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Kind of a session in the workbench model:
/// - `Normal` — a plain standalone conversation (default; every
///   pre-workbench session loads as this)
/// - `Mainline` — the long-lived project line registered in
///   `.firment/workbench.toml`
/// - `Branch` — an experiment/subtask spawned from a mainline or branch
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    /// Sessions written before the Normal/Mainline/Branch triple existed
    /// persisted `"main"` for EVERY chat (the old enum had no Normal). The
    /// alias reads those files as plain Normal conversations; whichever one
    /// a project registers as its mainline is re-promoted by the workbench
    /// state self-heal.
    #[default]
    #[serde(alias = "main")]
    Normal,
    Mainline,
    Branch,
}

impl SessionKind {
    pub fn label(&self) -> &'static str {
        match self {
            SessionKind::Normal => "normal",
            SessionKind::Mainline => "mainline",
            SessionKind::Branch => "branch",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub cwd: PathBuf,
    pub provider: String,
    pub model: String,
    pub thinking: ThinkingLevel,
    pub mode: SessionMode,
    /// Per-session context budget in chars (compaction threshold). `0` means
    /// "unset" — the agent's built-in default applies. Adjustable from the
    /// GUI/TUI without touching global config. Persisted via MetaLine.
    pub context_budget_chars: usize,
    /// Workbench tree linkage: `Some(parent id)` marks this as a branch of
    /// another session. `None` on main-line sessions.
    pub parent_session: Option<String>,
    pub kind: SessionKind,
    pub created_at: u64,
    pub updated_at: u64,
    pub messages: Vec<ChatMessage>,
    /// What the user called this session, if they called it anything.
    ///
    /// `None` is the normal state: the name shown is derived from the first
    /// message, so a session is never nameless and is never named by whoever
    /// happened to render it. `Some` is an explicit rename, and clearing it
    /// (setting whitespace) hands the row back to the derived name.
    pub title: Option<String>,
}

impl Session {
    pub fn new(cwd: PathBuf, provider: impl Into<String>, model: impl Into<String>) -> Self {
        let now = now_secs();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            cwd,
            provider: provider.into(),
            model: model.into(),
            thinking: ThinkingLevel::Off,
            mode: SessionMode::Agent,
            context_budget_chars: 0,
            parent_session: None,
            kind: SessionKind::Normal,
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            title: None,
        }
    }

    pub fn push(&mut self, message: ChatMessage) {
        self.updated_at = now_secs();
        self.messages.push(message);
    }

    /// Rewind to just before the last request, and hand that request back to be sent
    /// again.
    ///
    /// Everything the turn recorded after the last user message goes with it. A retry
    /// that kept the failed assistant tail would send the model its own error output
    /// and then the question a second time -- a different, and worse, prompt than the
    /// one that failed. The user message goes too, because the caller re-sends it
    /// through the ordinary path: keeping it here would mean either two copies of the
    /// question or a merge nobody asked for.
    ///
    /// `None` when there is no user message to repeat.
    pub fn retry_last(&mut self) -> Option<String> {
        let at = self
            .messages
            .iter()
            .rposition(|m| matches!(m, ChatMessage::User { .. }))?;
        let mut tail = self.messages.split_off(at).into_iter();
        let prompt = match tail.next() {
            // `at` is the index of a user message, so this arm is unreachable. `None`
            // rather than a panic: a mistake here must not be able to take the process
            // down in the middle of a retry.
            Some(ChatMessage::User { content }) => content,
            _ => return None,
        };
        // `tail` drops here. The marked time moves with the transcript, or the stored
        // file would keep a timestamp from the turn that was thrown away.
        self.updated_at = now_secs();
        Some(prompt)
    }

    /// The newest tool output that carries a unified diff, with the tool's name.
    ///
    /// What `firm review last` and the TUI's `/review-last` review: the change as it was
    /// actually written, taken from the transcript rather than reconstructed from the
    /// journal or the working tree (which may have moved on since).
    pub fn last_change(&self) -> Option<(String, String)> {
        self.last_change_at(self.messages.len())
    }

    /// The newest change **as of** a point in the transcript (plan §5, item 1).
    ///
    /// The transcript is the conversation's timeline: it is written in order as the session
    /// happens, so "the change I made before that mess" is a question about message indices.
    /// The event log is the *operations* timeline and answers the same question in its own
    /// units; the two are deliberately separate records, and neither is derived from the
    /// other.
    ///
    /// `at` is exclusive, exactly like a slice bound, so
    /// `last_change_at(self.messages.len())` is `last_change()`.
    pub fn last_change_at(&self, at: usize) -> Option<(String, String)> {
        self.messages[..at.min(self.messages.len())]
            .iter()
            .rev()
            .find_map(|message| match message {
                ChatMessage::Tool { name, content, .. }
                    if crate::review::self_review::looks_like_diff(content) =>
                {
                    Some((name.clone(), content.clone()))
                }
                _ => None,
            })
    }

    /// Every change the session had made by `at`, oldest first, with the message index that
    /// made it.
    pub fn changes_at(&self, at: usize) -> Vec<(usize, String, String)> {
        self.messages[..at.min(self.messages.len())]
            .iter()
            .enumerate()
            .filter_map(|(index, message)| match message {
                ChatMessage::Tool { name, content, .. }
                    if crate::review::self_review::looks_like_diff(content) =>
                {
                    Some((index, name.clone(), content.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// The session as of `at` (plan §5, item 1's time travel).
    ///
    /// Deliberately not a re-execution: replaying tool calls would re-flash hardware and
    /// re-write files, which is the opposite of what someone looking back wants. What this
    /// shows is what the session *knew* at that point.
    pub fn replay_at(&self, at: usize) -> ReplayView {
        let at = at.min(self.messages.len());
        ReplayView {
            at,
            messages: self.messages[..at].to_vec(),
            changes: self.changes_at(at),
            tool_calls: self.messages[..at]
                .iter()
                .filter(|message| matches!(message, ChatMessage::Tool { .. }))
                .count(),
        }
    }

    /// One line per message, for scanning for the point to go back to.
    pub fn timeline(&self) -> Vec<(usize, String)> {
        self.messages
            .iter()
            .enumerate()
            .map(|(index, message)| (index, timeline_label(message)))
            .collect()
    }

    pub fn title(&self) -> String {
        self.messages
            .iter()
            .find_map(|m| match m {
                ChatMessage::User { content } => Some(content.trim().to_string()),
                _ => None,
            })
            .map(|t| {
                let t: String = t.chars().take(48).collect();
                if t.chars().count() >= 48 {
                    format!("{t}…")
                } else {
                    t
                }
            })
            .unwrap_or_else(|| "(empty)".to_string())
    }

    /// The name to show: what the user called it, else what it says, else that
    /// it is new.
    ///
    /// The last case is the one that was being answered four different ways --
    /// the rail fell back to an id, the CLI to nothing, the GUI to a literal.
    /// A session with nothing in it is a new session, and that is a fact about
    /// the session rather than about the client.
    pub fn display_name(&self) -> String {
        if let Some(title) = self
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            return title.to_string();
        }
        let derived = self.title();
        if self.messages.is_empty() || derived == "(empty)" {
            "New session".to_string()
        } else {
            derived
        }
    }

    /// Rename, or clear the name with an empty one.
    pub fn set_title(&mut self, title: Option<String>) {
        self.title = title
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        self.updated_at = now_secs();
    }
}

/// A session as of a point in its transcript (plan §5, item 1).
#[derive(Debug, Clone)]
pub struct ReplayView {
    /// How many messages were considered (exclusive bound, like a slice).
    pub at: usize,
    pub messages: Vec<ChatMessage>,
    /// `(message index, tool, diff)` for every change made by then, oldest first.
    pub changes: Vec<(usize, String, String)>,
    pub tool_calls: usize,
}

impl ReplayView {
    /// The header: where this view stops and what the session had done by then.
    pub fn summary_line(&self) -> String {
        format!(
            "as of message {} — {} change(s) made, {} tool call(s) run",
            self.at,
            self.changes.len(),
            self.tool_calls
        )
    }
}

/// One line naming a message, for the timeline list.
fn timeline_label(message: &ChatMessage) -> String {
    let shorten = |text: &str| -> String {
        let first = text
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("");
        let cut: String = first.chars().take(70).collect();
        if first.chars().count() > 70 {
            format!("{cut}…")
        } else {
            cut
        }
    };
    match message {
        ChatMessage::User { content } => format!("user: {}", shorten(content)),
        ChatMessage::Assistant { content, .. } => format!("assistant: {}", shorten(content)),
        ChatMessage::Tool { name, content, .. } => {
            // A tool that changed a file says so; the diff itself would drown the list.
            if crate::review::self_review::looks_like_diff(content) {
                let path = crate::review::self_review::path_from_diff(content)
                    .unwrap_or_else(|| name.clone());
                format!("{name} → changed {path}")
            } else {
                format!("{name}: {}", shorten(content))
            }
        }
        ChatMessage::System { content } => format!("system: {}", shorten(content)),
    }
}

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub id: String,
    pub updated_at: u64,
    pub model: String,
    pub cwd: PathBuf,
    pub preview: String,
    /// Workbench tree linkage (see `Session::parent_session`).
    pub kind: SessionKind,
    pub parent_session: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("session not found: {0}")]
    NotFound(String),
    #[error("corrupt session file {0}: {1}")]
    Corrupt(PathBuf, String),
}

#[derive(Serialize, Deserialize)]
#[allow(dead_code)]
struct MetaLine {
    #[serde(rename = "type")]
    kind: String,
    id: String,
    cwd: PathBuf,
    provider: String,
    model: String,
    #[serde(default)]
    thinking: ThinkingLevel,
    #[serde(default)]
    mode: SessionMode,
    /// Per-session compaction budget in chars; 0 = agent default. Defaults
    /// keep pre-budget JSONL files loadable unchanged.
    #[serde(default)]
    context_budget_chars: usize,
    // Workbench tree linkage. Defaults keep pre-workbench JSONL files
    // loadable unchanged (they are all main-line sessions).
    #[serde(default)]
    parent_session: Option<String>,
    #[serde(default)]
    session_kind: SessionKind,
    /// An explicit rename. Absent in every file written before this existed,
    /// which is what `default` is for.
    #[serde(default)]
    title: Option<String>,
    /// The name derived from the first message at write time, so `list()` can
    /// show a row without opening the session. Kept separate from `title` so a
    /// rename does not bake itself into the derivation.
    #[serde(default)]
    preview: String,
    created_at: u64,
    updated_at: u64,
}

#[derive(Serialize, Deserialize)]
struct MessageLine {
    #[serde(rename = "type")]
    kind: String,
    message: ChatMessage,
}

#[derive(Clone)]
pub struct SessionStore {
    pub dir: PathBuf,
}

impl SessionStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.jsonl", sanitize_id(id)))
    }

    /// Directory holding undo backups for a session (next to its JSONL file).
    pub fn undo_dir(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.undo", sanitize_id(id)))
    }

    /// Directory holding spilled (out-of-band) tool outputs for a session.
    pub fn spill_dir(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.spill", sanitize_id(id)))
    }

    /// Directory of per-session tool scratch: the todo list, HIL artifacts,
    /// debug snapshots, the ELF-analysis cache.
    ///
    /// This is what `ToolContext::session_dir` points at. It used to be a single
    /// shared `sessions/work`, which made the `todo` tool's own contract -- "keep
    /// a session-scoped todo list" -- false: two parallel chats wrote the same
    /// `todos.json` and clobbered each other, and a GUI pane reading it would
    /// have shown another session's items.
    pub fn work_dir(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.work", sanitize_id(id)))
    }

    /// Path of the session's change ledger (JSONL, one committed turn per line).
    pub fn ledger_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.ledger.jsonl", sanitize_id(id)))
    }

    /// Path of the session's event log (plan §5, item 1): what `replay` reads, one JSON
    /// line per significant event, capped and rotated by `core::eventlog`.
    pub fn event_log_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.events.jsonl", sanitize_id(id)))
    }

    /// Path of the session's pinned-file list (JSON array of paths).
    pub fn pins_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.pins.json", sanitize_id(id)))
    }

    pub fn load_pins(&self, id: &str) -> Vec<PathBuf> {
        let Ok(text) = fs::read_to_string(self.pins_path(id)) else {
            return Vec::new();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save_pins(&self, id: &str, pins: &[PathBuf]) -> Result<(), SessionError> {
        fs::create_dir_all(&self.dir)?;
        fs::write(self.pins_path(id), serde_json::to_string_pretty(pins)?)?;
        Ok(())
    }

    pub fn save(&self, session: &Session) -> Result<(), SessionError> {
        fs::create_dir_all(&self.dir)?;
        let content = serialize_session(session)?;
        atomic_write(&self.path_for(&session.id), &content)?;
        Ok(())
    }

    /// Load a session. Corrupt lines are skipped so a single bad line (e.g.
    /// from an interrupted write) does not discard the whole transcript.
    pub fn load(&self, id: &str) -> Result<Session, SessionError> {
        let path = self.path_for(id);
        let file = fs::File::open(&path).map_err(|_| SessionError::NotFound(id.to_string()))?;
        let mut meta: Option<MetaLine> = None;
        let mut messages: Vec<ChatMessage> = Vec::new();
        let mut corrupt_lines = 0usize;
        for line in std::io::BufReader::new(file).lines() {
            let Ok(line) = line else {
                // Not every lost record is bad JSON: a non-UTF-8 byte (a
                // hand-edited or half-flushed transcript) surfaces here as an
                // I/O error. It is still a skipped line, so it must arm the
                // dangling-tool-call repair below like any other corruption.
                corrupt_lines += 1;
                continue;
            };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                corrupt_lines += 1;
                continue;
            };
            let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match kind {
                "meta" => {
                    if let Ok(m) = serde_json::from_value::<MetaLine>(value) {
                        meta = Some(m);
                    } else {
                        // Schema-drifted meta is as dangerous as a skipped
                        // tool result: count it so dangling-tool-call repair
                        // still runs below.
                        corrupt_lines += 1;
                    }
                }
                "message" => {
                    if let Ok(m) = serde_json::from_value::<MessageLine>(value) {
                        messages.push(m.message);
                    } else {
                        // A valid-JSON line that fails MessageLine
                        // deserialization (schema drift, hand-edited or
                        // partially rewritten transcript) is exactly the
                        // shape that strands an assistant tool_call without
                        // its result and 400s the next provider request —
                        // count it so the repair pass below triggers.
                        corrupt_lines += 1;
                    }
                }
                _ => {}
            }
        }
        let meta =
            meta.ok_or_else(|| SessionError::Corrupt(path.clone(), "missing meta line".into()))?;
        // A skipped line may have been a tool result, leaving an assistant
        // tool_call dangling — which both providers reject with HTTP 400 on
        // the next request. Only repair in that case: an intact transcript
        // must round-trip byte-for-byte (an assistant may legitimately carry
        // un-answered tool_calls in stored-but-not-yet-run states).
        if corrupt_lines > 0 {
            repair_dangling_tool_calls(&mut messages);
        }
        // Ungated on purpose: a consecutive-user transcript is made of valid
        // JSON lines, so `corrupt_lines` never counts it, yet it 400s every
        // request. Sessions written before this rule healed are on disk now.
        normalize_role_alternation(&mut messages);
        let model = migrate_legacy_model(&meta.model);
        let session = Session {
            id: meta.id,
            cwd: meta.cwd,
            provider: meta.provider,
            model: model.clone(),
            thinking: meta.thinking,
            mode: meta.mode,
            context_budget_chars: meta.context_budget_chars,
            parent_session: meta.parent_session,
            kind: meta.session_kind,
            created_at: meta.created_at,
            updated_at: meta.updated_at,
            messages,
            title: meta.title,
        };
        if model != meta.model {
            // deepseek-chat / deepseek-reasoner were deprecated on 2026-07-24;
            // persist the migration so later loads don't redo it.
            fs::create_dir_all(&self.dir)?;
            let content = serialize_session(&session)?;
            atomic_write(&self.path_for(&session.id), &content)?;
        }
        Ok(session)
    }

    pub fn list(&self) -> Result<Vec<SessionSummary>, SessionError> {
        let mut out = Vec::new();
        if !self.dir.exists() {
            return Ok(out);
        }
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(meta) = read_meta_line(&path) else {
                continue;
            };
            out.push(SessionSummary {
                id: meta.id,
                updated_at: meta.updated_at,
                model: migrate_legacy_model(&meta.model),
                cwd: meta.cwd,
                // What the row shows: the rename if there is one, else the name
                // derived when the file was written, else that it is new. The
                // core answers this so that the three clients cannot answer it
                // three ways.
                preview: meta
                    .title
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| {
                        if meta.preview.is_empty() {
                            "New session".to_string()
                        } else {
                            meta.preview.clone()
                        }
                    }),
                kind: meta.session_kind,
                parent_session: meta.parent_session,
            });
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        Ok(out)
    }

    pub fn latest(&self) -> Result<Option<SessionSummary>, SessionError> {
        Ok(self.list()?.into_iter().next())
    }

    /// Spawn a workbench BRANCH session from an existing one: same cwd,
    /// provider and model, fresh (empty) message history, linked to its
    /// parent via `parent_session`. The branch starts with a synthetic user
    /// note carrying `title` so the transcript preview is meaningful.
    ///
    /// ADR-lite inheritance (per docs/gui-workbench.md §已决 1): decisions
    /// whose title overlaps the branch title get injected as context, so an
    /// "i2c driver" branch automatically learns about the recorded "I2C
    /// pull-up" decision. Token-overlap matching, capped at 3 entries — the
    /// full registry stays one `decision` tool call away.
    ///
    /// Deleting a parent does NOT cascade: orphaned branches keep running
    /// and render as roots in the tree view.
    pub fn create_branch(&self, parent_id: &str, title: &str) -> Result<Session, SessionError> {
        let parent = self.load(parent_id)?;
        let mut child = Session::new(parent.cwd.clone(), &parent.provider, &parent.model);
        child.thinking = parent.thinking;
        child.mode = parent.mode;
        child.kind = SessionKind::Branch;
        child.parent_session = Some(parent.id.clone());
        if !title.trim().is_empty() {
            let title = title.trim();
            let t: String = title.chars().take(48).collect();
            child.push(ChatMessage::User {
                content: format!("[branch] {t}"),
            });
            // Relevant-decision injection. Best-effort: a project without a
            // workbench file simply inherits nothing.
            if let Ok(cfg) = crate::workbench::WorkbenchConfig::load(&parent.cwd) {
                let injected = relevant_decisions(&cfg.decision, title);
                if !injected.is_empty() {
                    let mut text =
                        String::from("[inherited project decisions relevant to this branch]\n");
                    for d in &injected {
                        text.push_str(&format!(
                            "- {}{}{}\n",
                            d.title,
                            if d.date.is_empty() { "" } else { " (" },
                            if d.date.is_empty() {
                                String::new()
                            } else {
                                format!("{})", d.date)
                            }
                        ));
                        if !d.body.is_empty() {
                            let body: String = d.body.chars().take(300).collect();
                            text.push_str(&format!("  {body}\n"));
                        }
                    }
                    child.push(ChatMessage::User { content: text });
                }
            }
        }
        self.save(&child)?;
        Ok(child)
    }

    /// Promote a session to the project MAINLINE: the session itself becomes
    /// `Mainline`, and any OTHER Mainline session sharing its cwd is demoted
    /// back to Normal (one mainline per project). Errors if the target does
    /// not exist.
    pub fn mark_mainline(&self, session_id: &str) -> Result<(), SessionError> {
        let mut target = self.load(session_id)?;
        for summary in self.list()? {
            if summary.id != target.id
                && summary.kind == SessionKind::Mainline
                && summary.cwd == target.cwd
            {
                let mut demoted = self.load(&summary.id)?;
                demoted.kind = SessionKind::Normal;
                self.save(&demoted)?;
            }
        }
        target.kind = SessionKind::Mainline;
        self.save(&target)?;
        Ok(())
    }

    /// Delete a session and all of its sidecar data (undo backups, spilled
    /// tool outputs, change ledger, pinned-file list).
    pub fn delete(&self, id: &str) -> Result<(), SessionError> {
        let id = sanitize_id(id);
        let mut removed = false;
        for candidate in [
            self.dir.join(format!("{id}.jsonl")),
            self.dir.join(format!("{id}.undo")),
            self.dir.join(format!("{id}.spill")),
            self.dir.join(format!("{id}.ledger.jsonl")),
            self.dir.join(format!("{id}.pins.json")),
        ] {
            if candidate.is_dir() {
                fs::remove_dir_all(&candidate)?;
                removed = true;
            } else if candidate.is_file() {
                fs::remove_file(&candidate)?;
                removed = true;
            }
        }
        if !removed {
            return Err(SessionError::NotFound(id.to_string()));
        }
        Ok(())
    }
}

/// After a corrupt-line skip, an assistant message may have lost its tool
/// results (a single damaged `Tool` line). Both providers reject a
/// `tool_calls`-bearing assistant without the matching results (HTTP 400),
/// and the next successful save would persist the amputated transcript
/// forever. Strip call ids that have no following result; an assistant left
/// with zero ids degrades to a plain content-only message.
fn repair_dangling_tool_calls(messages: &mut [ChatMessage]) {
    // Collect ids that DO have a result.
    let mut answered: std::collections::HashSet<String> = std::collections::HashSet::new();
    for m in messages.iter() {
        if let ChatMessage::Tool { tool_call_id, .. } = m {
            answered.insert(tool_call_id.clone());
        }
    }
    for m in messages.iter_mut() {
        if let ChatMessage::Assistant { tool_calls, .. } = m
            && !tool_calls.is_empty()
        {
            let before = tool_calls.len();
            tool_calls.retain(|call| answered.contains(&call.id));
            if tool_calls.len() != before {
                tracing::warn!(
                    "session repair: dropped {before} dangling tool_call(s) after corrupt-line skip"
                );
            }
        }
    }
}

fn is_empty_assistant(message: &ChatMessage) -> bool {
    matches!(
        message,
        ChatMessage::Assistant {
            content,
            tool_calls,
            thinking_blocks,
        } if content.is_empty() && tool_calls.is_empty() && thinking_blocks.is_empty()
    )
}

/// Collapse consecutive same-role messages the provider APIs reject.
///
/// A turn that ends before the model ever answers (interrupted, stream timed
/// out, stream creation failed) has still persisted its `user` prompt — the
/// UI already showed it, so deleting it would lose what the user typed. The
/// NEXT turn then appends a second user message, and both APIs reject
/// `[user, user]` with HTTP 400, which leaves the session broken even after a
/// successful retry. Merging adjacent user messages keeps every character the
/// user typed and restores the alternation invariant.
///
/// Deliberately narrow: only `User` + `User` merges, an `Assistant` and its
/// `Tool` results are never reordered or dropped (that is
/// [`repair_dangling_tool_calls`]'s job), and no assistant message is
/// invented — an empty content block is itself rejected. A fully empty
/// assistant carries no information and is dropped.
pub(crate) fn normalize_role_alternation(messages: &mut Vec<ChatMessage>) {
    let mut merged = 0usize;
    let mut dropped = 0usize;
    let mut kept: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    for message in std::mem::take(messages) {
        if let ChatMessage::User { content } = &message {
            if let Some(ChatMessage::User { content: prev }) = kept.last_mut() {
                prev.push_str("\n\n");
                prev.push_str(content);
                merged += 1;
                continue;
            }
        } else if is_empty_assistant(&message) {
            dropped += 1;
            continue;
        }
        kept.push(message);
    }
    if merged > 0 || dropped > 0 {
        tracing::warn!(
            "session repair: merged {merged} consecutive user message(s), \
             dropped {dropped} empty assistant message(s)"
        );
    }
    *messages = kept;
}

fn serialize_session(session: &Session) -> Result<String, SessionError> {
    let meta = MetaLine {
        kind: "meta".to_string(),
        title: session.title.clone(),
        preview: session.title(),
        id: session.id.clone(),
        cwd: session.cwd.clone(),
        provider: session.provider.clone(),
        model: session.model.clone(),
        thinking: session.thinking,
        mode: session.mode,
        context_budget_chars: session.context_budget_chars,
        parent_session: session.parent_session.clone(),
        session_kind: session.kind,
        created_at: session.created_at,
        updated_at: session.updated_at,
    };
    let mut content = serde_json::to_string(&meta)? + "\n";
    for message in &session.messages {
        content.push_str(&serde_json::to_string(&MessageLine {
            kind: "message".to_string(),
            message: message.clone(),
        })?);
        content.push('\n');
    }
    Ok(content)
}

impl Default for SessionStore {
    fn default() -> Self {
        Self {
            dir: config_dir().join("sessions"),
        }
    }
}

fn read_meta_line(path: &Path) -> Result<MetaLine, SessionError> {
    let file = fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&line)?;
        if value.get("type").and_then(|v| v.as_str()) == Some("meta") {
            return Ok(serde_json::from_value(value)?);
        }
    }
    Err(SessionError::Corrupt(
        path.to_path_buf(),
        "no meta line".into(),
    ))
}

/// Session ids are interpolated into file names; allow only safe characters so
/// a crafted id (e.g. `../x`) cannot redirect sidecar files outside the store.
fn sanitize_id(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect::<String>()
}

fn migrate_legacy_model(model: &str) -> String {
    match model {
        "deepseek-chat" | "deepseek-reasoner" => "deepseek-v4-flash".to_string(),
        other => other.to_string(),
    }
}

fn atomic_write(path: &Path, content: &str) -> Result<(), SessionError> {
    let parent = path.parent().ok_or_else(|| {
        SessionError::Io(std::io::Error::other(format!(
            "no parent directory for {}",
            path.display()
        )))
    })?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    tmp.write_all(content.as_bytes())?;
    // Flush to disk before the rename: without sync_all a power loss can
    // persist the (empty) directory entry while losing the data.
    tmp.flush()?;
    tmp.as_file().sync_all()?;
    // On Windows persist fails if another handle holds the target open
    // without FILE_SHARE_DELETE (AV scanner, indexer, a second firm
    // instance) — retry briefly before giving up. PersistError hands the
    // temp file back so it can be retried. The cap keeps a long-lived
    // external handle from spinning this loop (and the blocking sleep
    // inside an async turn) forever; saves are best-effort, so giving up
    // lets the turn proceed and the next checkpoint retry.
    let mut last_err = None;
    let mut pending = Some(tmp);
    let mut attempts = 0u32;
    while let Some(file) = pending.take() {
        match file.persist(path) {
            Ok(_) => return Ok(()),
            Err(e) => {
                last_err = Some(SessionError::Io(std::io::Error::other(e.to_string())));
                attempts += 1;
                if attempts >= 20 {
                    // Dropping the NamedTempFile removes it from disk.
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
                pending = Some(e.file);
            }
        }
    }
    Err(last_err.unwrap())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Token-overlap relevance between a branch title and decision records:
/// split both into lowercase alphanumeric tokens (>=2 chars), score each
/// decision by how many of its title tokens appear in the branch title
/// (and vice versa), keep the hits, newest first, capped at 3.
fn relevant_decisions(
    decisions: &[crate::workbench::DecisionEntry],
    branch_title: &str,
) -> Vec<crate::workbench::DecisionEntry> {
    const STOP: [&str; 4] = ["the", "and", "for", "with"];
    let tokenize = |s: &str| -> Vec<String> {
        let mut out = Vec::new();
        // Unicode runs (is_alphanumeric keeps CJK); ASCII runs become whole
        // tokens, CJK runs additionally become 2-char sliding windows —
        // Chinese has no spaces, so whole-run tokens would never overlap
        // and decision inheritance silently no-opped for Chinese titles.
        for run in s.to_lowercase().split(|c: char| !c.is_alphanumeric()) {
            if run.is_empty() {
                continue;
            }
            let has_cjk = !run.is_ascii();
            if has_cjk {
                let chars: Vec<char> = run.chars().collect();
                if chars.len() < 2 {
                    continue;
                }
                for w in chars.windows(2) {
                    out.push(w.iter().collect::<String>());
                }
            } else if run.len() >= 2 && !STOP.contains(&run) {
                out.push(run.to_string());
            }
        }
        out
    };
    let title_tokens = tokenize(branch_title);
    if title_tokens.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, &crate::workbench::DecisionEntry)> = decisions
        .iter()
        .map(|d| {
            let dt = tokenize(&d.title);
            let hit = dt.iter().filter(|t| title_tokens.contains(t)).count();
            (hit, d)
        })
        .filter(|(hit, _)| *hit > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.date.cmp(&a.1.date)));
    scored.into_iter().take(3).map(|(_, d)| d.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolCall;

    /// `work_dir` is what `ToolContext::session_dir` points at, so it decides
    /// where the `todo` tool keeps `todos.json`.
    ///
    /// It used to be a single shared `sessions/work` for every session, which
    /// made the tool's own contract -- "keeps a session-scoped todo list" --
    /// false: two parallel chats wrote the same file. The assertion that matters
    /// is the second one: distinct ids must not collide, and neither may be the
    /// old shared path.
    #[test]
    fn work_dir_is_per_session_and_not_the_shared_scratch() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().join("sessions"));
        let a = store.work_dir("session-a");
        let b = store.work_dir("session-b");
        assert_ne!(a, b, "two sessions must not share a todo list");
        assert_ne!(a, store.dir.join("work"), "the shared scratch dir is gone");
        assert!(
            a.starts_with(&store.dir),
            "scratch lives beside the transcript"
        );
    }

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage::User {
            content: text.to_string(),
        }
    }

    fn assistant_msg(text: &str) -> ChatMessage {
        ChatMessage::Assistant {
            content: text.to_string(),
            tool_calls: Vec::new(),
            thinking_blocks: Vec::new(),
        }
    }

    /// The rewind behind `/retry-last`, which is the whole substance of the command.
    #[test]
    fn retry_last_drops_the_failed_tail_and_hands_the_request_back() {
        let mut s = Session::new(PathBuf::from("."), "p", "m");
        s.push(user_msg("first"));
        s.push(assistant_msg("answered"));
        s.push(user_msg("second"));
        s.push(assistant_msg("boom"));

        assert_eq!(s.retry_last().as_deref(), Some("second"));
        // The earlier exchange is untouched, and the failed one is gone rather than
        // left in the transcript for the model to read back at itself.
        assert_eq!(s.messages.len(), 2);
        assert!(
            matches!(&s.messages[1], ChatMessage::Assistant { content, .. } if content == "answered")
        );
    }

    /// A turn that failed before it said anything, and a turn that never ran: both
    /// rewind to the same place, and the request still comes back.
    #[test]
    fn retry_last_works_when_nothing_was_recorded_after_the_request() {
        let mut s = Session::new(PathBuf::from("."), "p", "m");
        s.push(user_msg("only question"));
        assert_eq!(s.retry_last().as_deref(), Some("only question"));
        assert!(s.messages.is_empty(), "the caller re-sends it");
    }

    /// A tool call whose result never arrived -- an interrupted turn -- is part of the
    /// tail, and it has to go too: a `Tool` message with no matching `tool_call_id` in
    /// the request is rejected outright by both provider APIs.
    #[test]
    fn retry_last_takes_the_dangling_tool_call_with_it() {
        let mut s = Session::new(PathBuf::from("."), "p", "m");
        s.push(user_msg("flash it"));
        s.push(ChatMessage::Assistant {
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: "c1".to_string(),
                name: "flash".to_string(),
                arguments: serde_json::json!({}),
            }],
            thinking_blocks: Vec::new(),
        });
        s.push(ChatMessage::Tool {
            tool_call_id: "c1".to_string(),
            name: "flash".to_string(),
            content: "cancelled".to_string(),
        });

        assert_eq!(s.retry_last().as_deref(), Some("flash it"));
        assert!(s.messages.is_empty());
    }

    #[test]
    fn retry_last_has_nothing_to_repeat_in_a_fresh_session() {
        let mut s = Session::new(PathBuf::from("."), "p", "m");
        assert_eq!(s.retry_last(), None);
        s.push(assistant_msg("a greeting nobody asked for"));
        assert_eq!(s.retry_last(), None);
    }

    #[test]
    fn the_last_change_is_the_newest_tool_output_that_carries_a_diff() {
        let mut s = Session::new(PathBuf::from("."), "p", "m");
        s.push(user_msg("edit it"));
        // A tool call whose output is not a diff is not a change to review — its text is
        // a summary, and reviewing a summary reviews nothing.
        s.push(ChatMessage::Tool {
            tool_call_id: "c1".to_string(),
            name: "read_file".to_string(),
            content: "fn main() {}\n".to_string(),
        });
        s.push(ChatMessage::Tool {
            tool_call_id: "c2".to_string(),
            name: "edit_file".to_string(),
            content: "Edited main.c (1 lines -> 2 lines)\n@@ -1 +1,2 @@\n-old\n+new\n".to_string(),
        });

        let (tool, diff) = s.last_change().expect("the edit is a change");
        assert_eq!(tool, "edit_file");
        assert!(diff.contains("@@ -1 +1,2 @@"));
        assert_eq!(
            s.last_change().map(|(_, d)| d.contains("main.c")),
            Some(true)
        );
    }

    #[test]
    fn a_session_that_only_read_files_has_no_change_to_review() {
        let mut s = Session::new(PathBuf::from("."), "p", "m");
        s.push(user_msg("look at it"));
        s.push(ChatMessage::Tool {
            tool_call_id: "c1".to_string(),
            name: "read_file".to_string(),
            content: "fn main() {}\n".to_string(),
        });
        assert!(s.last_change().is_none());
    }

    /// A session id is used in a path, so it must not be able to escape the
    /// store directory.
    #[test]
    fn work_dir_sanitises_the_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().join("sessions"));
        let escaped = store.work_dir("../../etc/passwd");
        assert!(escaped.starts_with(&store.dir));
        assert!(!escaped.to_string_lossy().contains(".."));
    }

    /// An undecodable byte surfaces as an I/O error from `Lines::next`, not as
    /// bad JSON. It used to be skipped without counting, so the tool result it
    /// hid left the assistant tool_call dangling — and every later request 400d.
    #[test]
    fn unreadable_line_still_arms_dangling_tool_call_repair() {
        let dir = tempfile::tempdir().unwrap();
        let store_dir = dir.path().join("sessions");
        fs::create_dir_all(&store_dir).unwrap();
        let store = SessionStore::new(store_dir);
        let mut session = Session::new(dir.path().to_path_buf(), "p", "m");
        session.id = "garbage-line".into();
        session.messages.push(ChatMessage::User {
            content: "hi".into(),
        });
        session.messages.push(ChatMessage::Assistant {
            content: "calling".into(),
            tool_calls: vec![ToolCall {
                id: "call-1".into(),
                name: "shell".into(),
                arguments: serde_json::json!({}),
            }],
            thinking_blocks: Vec::new(),
        });
        session.messages.push(ChatMessage::Tool {
            tool_call_id: "call-1".into(),
            name: "shell".into(),
            content: "ok".into(),
        });
        store.save(&session).unwrap();

        let path = store.path_for("garbage-line");
        let text = fs::read_to_string(&path).unwrap();
        let mut lines: Vec<&str> = text.lines().collect();
        assert!(lines.len() >= 4, "meta + 3 messages: {text}");
        lines.pop(); // the tool-result line
        let mut bytes = lines.join("\n").into_bytes();
        bytes.extend_from_slice(b"\n\xff\xfe this is not valid utf-8\n");
        fs::write(&path, &bytes).unwrap();

        let loaded = store.load("garbage-line").unwrap();
        let dangling = loaded.messages.iter().any(
            |m| matches!(m, ChatMessage::Assistant { tool_calls, .. } if !tool_calls.is_empty()),
        );
        assert!(
            !dangling,
            "the unreadable line must count as corruption and repair it"
        );
    }

    /// Regression: sessions persisted before the Normal/Mainline/Branch
    /// triple carry `"session_kind": "main"` (the old enum had no Normal,
    /// so every saved chat wrote it). Without the serde alias the meta line
    /// failed to parse, `list()` silently skipped the file and whole
    /// projects went blank in every UI. Legacy "main" maps to Normal; the
    /// registered mainline is re-promoted by workbench_state's self-heal.
    #[test]
    fn legacy_main_kind_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let store_dir = dir.path().join("sessions");
        fs::create_dir_all(&store_dir).unwrap();
        let id = "legacy-main-session";
        // Hand-written JSONL exactly as the pre-refactor binary wrote it.
        let meta = format!(
            r#"{{"type":"meta","id":"{id}","cwd":"{}","provider":"openrouter","model":"test-model","thinking":"off","mode":"agent","parent_session":null,"session_kind":"main","created_at":100,"updated_at":200}}"#,
            dir.path().to_string_lossy().replace('\\', "\\\\")
        );
        let path = store_dir.join(format!("{id}.jsonl"));
        fs::write(&path, format!("{meta}\n")).unwrap();

        let store = SessionStore::new(store_dir.clone());

        // list() must not skip the file.
        let summaries = store.list().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].kind, SessionKind::Normal);

        // load() must succeed and map to Normal.
        let loaded = store.load(id).unwrap();
        assert_eq!(loaded.kind, SessionKind::Normal);

        // Re-saving rewrites the kind in the new spelling.
        store.save(&loaded).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"session_kind\":\"normal\""));
        assert!(!raw.contains("\"session_kind\":\"main\""));
    }

    /// ADR-lite inheritance: creating a branch whose title overlaps a
    /// recorded decision's title injects that decision as context; unrelated
    /// branches inherit nothing.
    #[test]
    fn branch_creation_injects_relevant_decisions() {
        let dir = tempfile::tempdir().unwrap();
        let store_dir = dir.path().join("sessions");
        fs::create_dir_all(&store_dir).unwrap();

        let mut cfg = crate::workbench::WorkbenchConfig::default();
        cfg.decision.push(crate::workbench::DecisionEntry {
            title: "I2C bus runs at 400k".into(),
            body: "sensor max clock; PA9/PA10 reserved".into(),
            date: "2026-08-24".into(),
        });
        cfg.decision.push(crate::workbench::DecisionEntry {
            title: "UART bootloader stays at 115200".into(),
            body: String::new(),
            date: "2026-08-20".into(),
        });
        cfg.save(dir.path()).unwrap();

        let store = SessionStore::new(store_dir);
        let main = store.create_branch("nonexistent", "");
        // create_branch needs a real parent: save one directly.
        drop(main);
        let mut parent = Session::new(dir.path().to_path_buf(), "p", "m");
        parent.id = "parent-1".into();
        store.save(&parent).unwrap();

        // Related title -> inherits the I2C decision, not the UART one.
        let child = store
            .create_branch("parent-1", "rewrite i2c sensor driver")
            .unwrap();
        let text: String = child
            .messages
            .iter()
            .filter_map(|m| match m {
                ChatMessage::User { content } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert!(text.contains("inherited project decisions"), "got: {text}");
        assert!(text.contains("I2C bus runs at 400k"), "got: {text}");
        assert!(
            !text.contains("bootloader"),
            "unrelated decision must not leak: {text}"
        );

        // Unrelated title -> no injection.
        let child2 = store.create_branch("parent-1", "led blinking").unwrap();
        let has_inherit = child2
            .messages
            .iter()
            .any(|m| matches!(m, ChatMessage::User { content } if content.contains("inherited")));
        assert!(!has_inherit, "no relevant decisions for 'led blinking'");

        // CJK titles: bigram matching must work for Chinese decision +
        // Chinese branch titles (the tokenizer used to be ASCII-only, which
        // silently no-opped inheritance for the primary user language).
        cfg.decision.push(crate::workbench::DecisionEntry {
            title: "传感器总线选 CAN 而非 RS485".into(),
            body: "节点数可能扩到 16".into(),
            date: "2026-08-25".into(),
        });
        cfg.save(dir.path()).unwrap();
        let child3 = store.create_branch("parent-1", "重写传感器驱动").unwrap();
        let has_cjk_inherit = child3.messages.iter().any(
            |m| matches!(m, ChatMessage::User { content } if content.contains("传感器总线选")),
        );
        assert!(has_cjk_inherit, "CJK decision inheritance failed");
    }

    /// The name is decided here, so three clients cannot decide it three ways.
    #[test]
    fn a_session_with_nothing_said_in_it_is_a_new_session() {
        let s = Session::new(PathBuf::from("C:/work"), "p", "m");
        assert_eq!(s.title, None);
        assert_eq!(s.display_name(), "New session");
    }

    #[test]
    fn the_name_comes_from_the_first_user_message() {
        let mut s = Session::new(PathBuf::from("C:/work"), "p", "m");
        s.push(ChatMessage::User {
            content: "  flash the board and tell me what happens  ".to_string(),
        });
        assert_eq!(s.display_name(), "flash the board and tell me what happens");
    }

    /// A rename wins, and clearing it hands the row back rather than blanking it.
    #[test]
    fn a_rename_wins_and_a_blank_one_gives_the_name_back() {
        let mut s = Session::new(PathBuf::from("C:/work"), "p", "m");
        s.push(ChatMessage::User {
            content: "first".to_string(),
        });
        s.set_title(Some("  JTAG bring-up  ".to_string()));
        assert_eq!(s.display_name(), "JTAG bring-up");
        s.set_title(Some("   ".to_string()));
        assert_eq!(s.display_name(), "first");
        assert_eq!(s.title, None, "whitespace is not a name");
    }

    /// The row reads it without opening the session, so it has to be on the meta
    /// line -- and a load has to bring the rename back with it.
    #[test]
    fn the_name_survives_a_save_and_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().join("sessions"));
        let mut s = Session::new(PathBuf::from("C:/work"), "p", "m");
        s.push(ChatMessage::User {
            content: "first".to_string(),
        });
        s.set_title(Some("JTAG bring-up".to_string()));
        store.save(&s).unwrap();

        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].preview, "JTAG bring-up");

        let reloaded = store.load(&s.id).unwrap();
        assert_eq!(reloaded.title.as_deref(), Some("JTAG bring-up"));
        assert_eq!(reloaded.display_name(), "JTAG bring-up");
    }

    /// An old file has neither field, and must still name itself from its content.
    #[test]
    fn a_file_written_before_the_fields_existed_still_names_itself() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().join("sessions"));
        let mut s = Session::new(PathBuf::from("C:/work"), "p", "m");
        s.push(ChatMessage::User {
            content: "an older session".to_string(),
        });
        store.save(&s).unwrap();

        // strip the two new keys, as a file from before them would be
        let file = store.path_for(&s.id);
        let text = fs::read_to_string(&file).unwrap();
        let stripped: String = text
            .lines()
            .map(|line| {
                if line.starts_with("{\"type\":\"meta\"") {
                    let mut value: serde_json::Value = serde_json::from_str(line).unwrap();
                    if let Some(obj) = value.as_object_mut() {
                        obj.remove("title");
                        obj.remove("preview");
                    }
                    serde_json::to_string(&value).unwrap()
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&file, stripped).unwrap();

        assert_eq!(store.list().unwrap()[0].preview, "New session");
        assert_eq!(
            store.load(&s.id).unwrap().display_name(),
            "an older session"
        );
    }
}
