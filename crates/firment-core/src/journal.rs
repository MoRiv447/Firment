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

    /// Seal the turn's mutations as an undo entry. Empty batches are skipped.
    pub fn commit(&mut self) -> Result<(), String> {
        if self.entries.is_empty() {
            return Ok(());
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
        self.entries.clear();
        Ok(())
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
        journal.commit().unwrap();
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
}
