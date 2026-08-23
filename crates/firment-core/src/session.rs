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
    #[default]
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
    /// Workbench tree linkage: `Some(parent id)` marks this as a branch of
    /// another session. `None` on main-line sessions.
    pub parent_session: Option<String>,
    pub kind: SessionKind,
    pub created_at: u64,
    pub updated_at: u64,
    pub messages: Vec<ChatMessage>,
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
            parent_session: None,
            kind: SessionKind::Normal,
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
        }
    }

    pub fn push(&mut self, message: ChatMessage) {
        self.updated_at = now_secs();
        self.messages.push(message);
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
    // Workbench tree linkage. Defaults keep pre-workbench JSONL files
    // loadable unchanged (they are all main-line sessions).
    #[serde(default)]
    parent_session: Option<String>,
    #[serde(default)]
    session_kind: SessionKind,
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

    /// Path of the session's change ledger (JSONL, one committed turn per line).
    pub fn ledger_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.ledger.jsonl", sanitize_id(id)))
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
            let Ok(line) = line else { continue };
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
                    }
                }
                "message" => {
                    if let Ok(m) = serde_json::from_value::<MessageLine>(value) {
                        messages.push(m.message);
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
        let model = migrate_legacy_model(&meta.model);
        let session = Session {
            id: meta.id,
            cwd: meta.cwd,
            provider: meta.provider,
            model: model.clone(),
            thinking: meta.thinking,
            mode: meta.mode,
            parent_session: meta.parent_session,
            kind: meta.session_kind,
            created_at: meta.created_at,
            updated_at: meta.updated_at,
            messages,
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
                preview: String::new(),
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

fn serialize_session(session: &Session) -> Result<String, SessionError> {
    let meta = MetaLine {
        kind: "meta".to_string(),
        id: session.id.clone(),
        cwd: session.cwd.clone(),
        provider: session.provider.clone(),
        model: session.model.clone(),
        thinking: session.thinking,
        mode: session.mode,
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
    // temp file back so it can be retried.
    let mut last_err = None;
    let mut pending = Some(tmp);
    while pending.is_some() {
        let file = pending.take().unwrap();
        match file.persist(path) {
            Ok(_) => return Ok(()),
            Err(e) => {
                last_err = Some(SessionError::Io(std::io::Error::other(e.to_string())));
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
