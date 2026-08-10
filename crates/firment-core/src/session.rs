use crate::config::config_dir;
use crate::{ChatMessage, SessionMode, ThinkingLevel};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub cwd: PathBuf,
    pub provider: String,
    pub model: String,
    pub thinking: ThinkingLevel,
    pub mode: SessionMode,
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
        let mut messages = Vec::new();
        for line in std::io::BufReader::new(file).lines() {
            let Ok(line) = line else { continue };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
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
        let model = migrate_legacy_model(&meta.model);
        let session = Session {
            id: meta.id,
            cwd: meta.cwd,
            provider: meta.provider,
            model: model.clone(),
            thinking: meta.thinking,
            mode: meta.mode,
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
            });
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        Ok(out)
    }

    pub fn latest(&self) -> Result<Option<SessionSummary>, SessionError> {
        Ok(self.list()?.into_iter().next())
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
    tmp.persist(path)
        .map_err(|e| SessionError::Io(std::io::Error::other(e.to_string())))?;
    Ok(())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
