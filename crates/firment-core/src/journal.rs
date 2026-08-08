use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// One recorded file mutation inside a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EntryRecord {
    path: PathBuf,
    /// Backup file name inside the journal directory (empty when the file
    /// did not exist before the edit).
    backup: String,
    existed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexRecord {
    created_at: u64,
    entries: Vec<EntryRecord>,
}

/// Result of restoring a committed undo entry.
#[derive(Debug, Clone)]
pub struct UndoSummary {
    pub files: usize,
    pub restored: Vec<String>,
}

/// One file's change within a committed turn (change ledger).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerChange {
    pub path: PathBuf,
    pub old_lines: usize,
    pub new_lines: usize,
    /// Compact `-`/`+` hunk lines, capped in size.
    pub hunks: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LedgerLine {
    seq: u64,
    created_at: u64,
    changes: Vec<LedgerChange>,
}

/// Session-scoped change ledger: one JSONL line per committed turn.
#[derive(Debug, Clone)]
pub struct Ledger {
    path: PathBuf,
}

impl Ledger {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn append(&self, changes: &[LedgerChange]) -> Result<(), String> {
        if changes.is_empty() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let seq = if self.path.exists() {
            fs::read_to_string(&self.path)
                .map(|t| t.lines().count() as u64 + 1)
                .unwrap_or(1)
        } else {
            1
        };
        let line = LedgerLine {
            seq,
            created_at: now_secs(),
            changes: changes.to_vec(),
        };
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| e.to_string())?;
        use std::io::Write;
        writeln!(
            file,
            "{}",
            serde_json::to_string(&line).map_err(|e| e.to_string())?
        )
        .map_err(|e| e.to_string())
    }

    /// Most recent entries, formatted for injection into the system prompt.
    pub fn summary(&self, max_entries: usize, max_chars: usize) -> String {
        let Ok(text) = fs::read_to_string(&self.path) else {
            return String::new();
        };
        let entries: Vec<LedgerLine> = text
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        let start = entries.len().saturating_sub(max_entries);
        let mut out = String::new();
        for entry in &entries[start..] {
            for change in &entry.changes {
                out.push_str(&format!(
                    "- {}（{} 行 -> {} 行）\n{}",
                    change.path.display(),
                    change.old_lines,
                    change.new_lines,
                    change.hunks
                ));
            }
        }
        truncate_chars(&out, max_chars)
    }
}

/// Per-turn edit journal: backs up every file before the first mutation and
/// can roll the whole batch back. On a successful turn it commits the entry
/// so `/undo` can restore it later.
#[derive(Debug)]
pub struct EditJournal {
    dir: PathBuf,
    entries: Vec<EntryRecord>,
    next_seq: u64,
}

impl EditJournal {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            entries: Vec::new(),
            next_seq: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Record a path before it is mutated. The first call per path keeps the
    /// original bytes; later mutations to the same path reuse that backup.
    pub fn begin(&mut self, path: &Path) -> Result<(), String> {
        if self.entries.iter().any(|e| e.path == path) {
            return Ok(());
        }
        let existed = path.exists();
        let backup = if existed {
            fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
            let name = format!("{}.bak", self.next_seq);
            fs::copy(path, self.dir.join(&name))
                .map_err(|e| format!("backup {} failed: {e}", path.display()))?;
            self.next_seq += 1;
            name
        } else {
            String::new()
        };
        self.entries.push(EntryRecord {
            path: path.to_path_buf(),
            backup,
            existed,
        });
        Ok(())
    }

    /// Restore every recorded file to its pre-turn state and drop the batch.
    pub fn rollback(&mut self) -> Result<Vec<String>, String> {
        let mut restored = Vec::new();
        let mut errors = Vec::new();
        for entry in self.entries.iter().rev() {
            match restore_entry(&self.dir, entry) {
                Ok(()) => restored.push(entry.path.to_string_lossy().into_owned()),
                Err(e) => errors.push(e),
            }
        }
        for entry in &self.entries {
            if !entry.backup.is_empty() {
                let _ = fs::remove_file(self.dir.join(&entry.backup));
            }
        }
        self.entries.clear();
        if errors.is_empty() {
            Ok(restored)
        } else {
            Err(format!("回滚不完整: {}", errors.join("; ")))
        }
    }

    /// Seal the turn's mutations as an undo entry and return the change
    /// ledger entries. Empty batches are skipped.
    pub fn commit(&mut self) -> Result<Vec<LedgerChange>, String> {
        if self.entries.is_empty() {
            return Ok(Vec::new());
        }
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let created = now_secs();
        let record = IndexRecord {
            created_at: created,
            entries: self.entries.clone(),
        };
        let name = format!("undo-{created}-{}.json", self.next_seq);
        let text = serde_json::to_string_pretty(&record).map_err(|e| e.to_string())?;
        fs::write(self.dir.join(name), text).map_err(|e| e.to_string())?;

        let mut changes = Vec::new();
        for entry in &self.entries {
            changes.push(ledger_change_for(&self.dir, entry)?);
        }
        self.entries.clear();
        Ok(changes)
    }

    /// Restore the most recently committed undo entry for a session.
    pub fn undo_latest(dir: &Path) -> Result<UndoSummary, String> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if dir.is_dir() {
            for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("undo-") && name.ends_with(".json") {
                    candidates.push(entry.path());
                }
            }
        }
        candidates.sort();
        let latest = candidates
            .last()
            .ok_or_else(|| "没有可撤销的编辑（本会话还没有提交过文件改动）".to_string())?;
        let text = fs::read_to_string(latest).map_err(|e| e.to_string())?;
        let record: IndexRecord = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let mut restored = Vec::new();
        let mut errors = Vec::new();
        for entry in &record.entries {
            match restore_entry(dir, entry) {
                Ok(()) => restored.push(entry.path.to_string_lossy().into_owned()),
                Err(e) => errors.push(e),
            }
        }
        if !errors.is_empty() {
            return Err(format!("撤销不完整: {}", errors.join("; ")));
        }
        fs::remove_file(latest).map_err(|e| e.to_string())?;
        for entry in &record.entries {
            if !entry.backup.is_empty() {
                let _ = fs::remove_file(dir.join(&entry.backup));
            }
        }
        Ok(UndoSummary {
            files: record.entries.len(),
            restored,
        })
    }
}

fn ledger_change_for(dir: &Path, entry: &EntryRecord) -> Result<LedgerChange, String> {
    let old_bytes = if entry.existed {
        fs::read(dir.join(&entry.backup)).unwrap_or_default()
    } else {
        Vec::new()
    };
    let new_bytes = fs::read(&entry.path).unwrap_or_default();
    let old = String::from_utf8_lossy(&old_bytes).into_owned();
    let new = String::from_utf8_lossy(&new_bytes).into_owned();
    Ok(LedgerChange {
        path: entry.path.clone(),
        old_lines: old.lines().count(),
        new_lines: new.lines().count(),
        hunks: diff_capped(&old, &new, 1600),
    })
}

/// Compact `-`/`+` line diff (common prefix/suffix trimmed), capped in size.
fn diff_capped(old: &str, new: &str, max_chars: usize) -> String {
    let old_lines: Vec<&str> = old.split('\n').collect();
    let new_lines: Vec<&str> = new.split('\n').collect();
    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_lines.len().saturating_sub(prefix)
        && suffix < new_lines.len().saturating_sub(prefix)
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let mut out = String::new();
    for line in &old_lines[prefix..old_lines.len() - suffix] {
        out.push_str(&format!("-{line}\n"));
    }
    for line in &new_lines[prefix..new_lines.len() - suffix] {
        out.push_str(&format!("+{line}\n"));
    }
    truncate_chars(&out, max_chars)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    if chars.len() > max_chars {
        chars.truncate(max_chars);
        chars.push('…');
    }
    chars.into_iter().collect()
}

fn restore_entry(dir: &Path, entry: &EntryRecord) -> Result<(), String> {
    if entry.existed {
        if let Some(parent) = entry.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create parent {}: {e}", parent.display()))?;
        }
        fs::copy(dir.join(&entry.backup), &entry.path)
            .map(|_| ())
            .map_err(|e| format!("restore {}: {e}", entry.path.display()))
    } else if entry.path.exists() {
        fs::remove_file(&entry.path)
            .map_err(|e| format!("remove created {}: {e}", entry.path.display()))
    } else {
        Ok(())
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rollback_restores_modified_and_removes_created() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("sub");
        fs::create_dir_all(&target).unwrap();
        let file = target.join("a.txt");
        fs::write(&file, "original").unwrap();
        let created = target.join("b.txt");
        let mut journal = EditJournal::new(dir.path().join("undo"));
        journal.begin(&file).unwrap();
        journal.begin(&created).unwrap();
        fs::write(&file, "modified").unwrap();
        fs::write(&created, "new").unwrap();
        let restored = journal.rollback().unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(fs::read_to_string(&file).unwrap(), "original");
        assert!(!created.exists());
        assert!(journal.is_empty());
    }

    #[test]
    fn begin_dedupes_same_path() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "v1").unwrap();
        let mut journal = EditJournal::new(dir.path().join("undo"));
        journal.begin(&file).unwrap();
        journal.begin(&file).unwrap();
        fs::write(&file, "v2").unwrap();
        let _ = journal.rollback().unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "v1");
    }

    #[test]
    fn commit_then_undo_restores_files() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "original").unwrap();
        let undo_dir = dir.path().join("undo");
        let mut journal = EditJournal::new(undo_dir.clone());
        journal.begin(&file).unwrap();
        fs::write(&file, "changed").unwrap();
        let changes = journal.commit().unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].old_lines, 1);
        assert_eq!(changes[0].new_lines, 1);
        assert!(
            changes[0].hunks.contains("-original"),
            "got: {}",
            changes[0].hunks
        );
        assert!(
            changes[0].hunks.contains("+changed"),
            "got: {}",
            changes[0].hunks
        );
        fs::write(&file, "changed-again").unwrap();
        let summary = EditJournal::undo_latest(&undo_dir).unwrap();
        assert_eq!(summary.files, 1);
        assert_eq!(fs::read_to_string(&file).unwrap(), "original");
        assert!(EditJournal::undo_latest(&undo_dir).is_err());
    }

    #[test]
    fn empty_journal_commit_is_noop() {
        let dir = tempdir().unwrap();
        let undo_dir = dir.path().join("undo");
        let mut journal = EditJournal::new(undo_dir.clone());
        journal.commit().unwrap();
        assert!(EditJournal::undo_latest(&undo_dir).is_err());
    }

    #[test]
    fn ledger_appends_and_summarizes() {
        let dir = tempdir().unwrap();
        let ledger = Ledger::new(dir.path().join("ledger.jsonl"));
        ledger
            .append(&[LedgerChange {
                path: PathBuf::from("a.txt"),
                old_lines: 1,
                new_lines: 2,
                hunks: "-old\n+new\n".to_string(),
            }])
            .unwrap();
        ledger
            .append(&[LedgerChange {
                path: PathBuf::from("b.txt"),
                old_lines: 0,
                new_lines: 1,
                hunks: "+hello\n".to_string(),
            }])
            .unwrap();
        let summary = ledger.summary(10, 1000);
        assert!(summary.contains("a.txt"), "got: {summary}");
        assert!(summary.contains("b.txt"), "got: {summary}");
        assert!(summary.contains("+hello"), "got: {summary}");
        assert!(summary.contains("1 行 -> 2 行"), "got: {summary}");
    }
}
