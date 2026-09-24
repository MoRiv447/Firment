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
    /// The session's highest tool-call number when this turn closed (0 when it is not known).
    ///
    /// The counter is the session's, so this column must increase from one turn to the next: a
    /// turn's range is *"above the previous turn's number, up to mine"*, which is what lets
    /// "rewind to before the step where the review found a problem" be answered by a number
    /// instead of a mapping table. A store where the column does not increase is one written
    /// while the counter restarted — per turn in the GUI, per process on reopen — and
    /// `turns_before_seq` refuses to guess at those rather than undoing the wrong amount of
    /// work. `serde(default)` keeps every entry written before the field existed readable;
    /// those read back as 0, and the caller is told the answer is unknown.
    #[serde(default)]
    max_seq: u64,
}

/// How [`EditJournal::turns_before_seq`] answered a "rewind to before call N".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rewind {
    /// Undo this many turns; the turn that made the call is among them.
    Turns(usize),
    /// No recorded turn reaches that number, so this session never made that call. Nothing to do.
    Unrecorded,
    /// The recorded numbers cannot answer the question: a turn written before the call number was
    /// kept, or a store whose numbering restarted so the column is no longer cumulative. Guessing
    /// here would restore the wrong files.
    Unknown,
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
    /// Standard unified-diff hunk body: `@@ -a,b +c,d @@` headers plus context
    /// lines, capped in size. No `--- `/`+++ ` file header (see `line_diff`).
    pub hunks: String,
    /// SHA-256 of the file before the change (content anchoring).
    pub old_sha256: String,
    /// SHA-256 of the file after the change (content anchoring).
    pub new_sha256: String,
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

    /// Entries with `seq > since_seq` (capped), plus the latest seq seen.
    /// Used to merge only the new change-ledger delta into the next turn.
    pub fn delta_text(&self, since_seq: u64, max_entries: usize) -> (String, u64) {
        let Ok(text) = fs::read_to_string(&self.path) else {
            return (String::new(), since_seq);
        };
        let entries: Vec<LedgerLine> = text
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        let last_seq = entries.iter().map(|e| e.seq).max().unwrap_or(since_seq);
        let new_entries: Vec<&LedgerLine> = entries.iter().filter(|e| e.seq > since_seq).collect();
        let start = new_entries.len().saturating_sub(max_entries);
        let mut out = String::new();
        for entry in &new_entries[start..] {
            for change in &entry.changes {
                out.push_str(&format!(
                    "- {} ({} lines -> {} lines)\n{}",
                    change.path.display(),
                    change.old_lines,
                    change.new_lines,
                    change.hunks
                ));
            }
        }
        (truncate_chars(&out, 3000), last_seq)
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
                    "- {} ({} lines -> {} lines)\n{}",
                    change.path.display(),
                    change.old_lines,
                    change.new_lines,
                    change.hunks
                ));
            }
        }
        truncate_chars(&out, max_chars)
    }

    /// Structured ledger entries in file order (oldest first), for embedders
    /// (GUI workbench timeline). Missing file yields an empty vec.
    pub fn entries(&self) -> Vec<(u64, u64, Vec<LedgerChange>)> {
        fs::read_to_string(&self.path)
            .map(|text| {
                text.lines()
                    .filter_map(|l| serde_json::from_str::<LedgerLine>(l).ok())
                    .map(|line| (line.seq, line.created_at, line.changes))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every change the session has committed, as one unified-diff document.
    ///
    /// `root` relativises the paths so the report is readable (and, for a small
    /// change, directly `git apply`-able); pass the session cwd.
    ///
    /// # This is a report, not a guaranteed patch
    ///
    /// Each hunk body was capped at [`LEDGER_HUNK_MAX_CHARS`] when the turn
    /// committed, so an edit larger than that was truncated BEFORE it reached
    /// the ledger. The marker the capping leaves behind (`… diff truncated`) is
    /// preserved verbatim, and [`Self::export_truncated`] lists the files it
    /// affects -- a caller that wants to apply the result must check that list
    /// first, because applying a truncated patch does not fail loudly, it
    /// applies a PARTIAL change.
    pub fn export_unified_diff(&self, root: &Path) -> String {
        let mut out = String::new();
        for (_, _, changes) in self.entries() {
            for change in changes {
                out.push_str(&change.as_unified_diff(root));
            }
        }
        out
    }

    /// Paths whose exported diff is incomplete because the hunk body was capped.
    ///
    /// Empty for a session whose every change fits the budget, which is the
    /// case that can be applied as a patch.
    pub fn export_truncated(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for (_, _, changes) in self.entries() {
            for change in changes {
                if change.hunks.contains(TRUNCATION_MARKER) {
                    out.push(change.path.clone());
                }
            }
        }
        out
    }
}

/// Appended by `line_diff` when it has to stop before printing every hunk.
pub const TRUNCATION_MARKER: &str = "… diff truncated";

/// Hunk budget the ledger commits with. Kept here next to the exporter that has
/// to reason about it. See `ledger_change_for`.
pub const LEDGER_HUNK_MAX_CHARS: usize = 1600;

impl LedgerChange {
    /// One file's entry in a unified-diff document: the `--- `/`+++ ` header
    /// plus the stored hunk body.
    ///
    /// The hunk body already carries `@@ -a,b +c,d @@` headers and context
    /// lines (it is produced by `line_diff`), so only the file header is added
    /// here. The body is NOT re-generated from the current file: the ledger
    /// stores the change as it was committed, and re-diffing against today's
    /// content would quietly report a different change than the one that
    /// happened.
    pub fn as_unified_diff(&self, root: &Path) -> String {
        let shown = self
            .path
            .strip_prefix(root)
            .unwrap_or(&self.path)
            .to_string_lossy()
            .replace('\\', "/");
        format!("--- {shown}\n+++ {shown}\n{}", self.hunks)
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
    /// Nanoseconds since epoch at construction; fixed-width hex so backup and
    /// undo file names are globally unique across turns and sort chronologically.
    stamp: u128,
}

/// Lexical `.`/`..` folding, for paths whose file does not exist yet and so
/// cannot be canonicalized.
fn lexical_key(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                let at_root = matches!(
                    out.components().next_back(),
                    Some(std::path::Component::RootDir) | Some(std::path::Component::Prefix(_))
                );
                if !at_root {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Do these two spellings name the same file? `PathEq` alone said no for
/// `src\a.rs` and `src/./a.rs`, which recorded the same path twice in one
/// turn — and the second backup captured content the first edit had already
/// changed.
fn same_path(a: &Path, b: &Path) -> bool {
    a == b
        || lexical_key(a) == lexical_key(b)
        || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

impl EditJournal {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            entries: Vec::new(),
            next_seq: 0,
            stamp: now_nanos(),
        }
    }

    fn backup_name(&self, seq: u64) -> String {
        format!("{:032x}-{:04x}.bak", self.stamp, seq)
    }

    fn index_name(&self, seq: u64) -> String {
        format!("undo-{:032x}-{:04x}.json", self.stamp, seq)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Record a path before it is mutated. The first call per path keeps the
    /// original bytes; later mutations to the same path reuse that backup.
    pub fn begin(&mut self, path: &Path) -> Result<(), String> {
        if self.entries.iter().any(|e| same_path(&e.path, path)) {
            return Ok(());
        }
        let existed = path.exists();
        let backup = if existed {
            fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
            let name = self.backup_name(self.next_seq);
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
    /// Entries whose restore failed are RETAINED (with their backups) so a
    /// later rollback/undo can retry — deleting their backups would leave the
    /// mutated content in place forever with no recovery path (e.g. a file
    /// locked by an IDE or flash tool on Windows).
    pub fn rollback(&mut self) -> Result<Vec<String>, String> {
        let mut restored = Vec::new();
        let mut errors = Vec::new();
        let mut restored_entries: Vec<EntryRecord> = Vec::new();
        let mut kept: Vec<EntryRecord> = Vec::new();
        for entry in self.entries.drain(..).rev() {
            match restore_entry(&self.dir, &entry) {
                Ok(()) => {
                    restored.push(entry.path.to_string_lossy().into_owned());
                    restored_entries.push(entry);
                }
                Err(e) => {
                    errors.push(e);
                    kept.push(entry);
                }
            }
        }
        // Entries restored successfully no longer need their backups;
        // failed ones RETAIN theirs so a later rollback/undo can retry.
        for entry in &restored_entries {
            if !entry.backup.is_empty() {
                let _ = fs::remove_file(self.dir.join(&entry.backup));
            }
        }
        self.entries = kept.into_iter().rev().collect();
        if errors.is_empty() {
            Ok(restored)
        } else {
            Err(format!("rollback incomplete: {}", errors.join("; ")))
        }
    }

    pub fn commit(&mut self) -> Result<Vec<LedgerChange>, String> {
        self.commit_at_seq(0)
    }

    /// `commit`, recording which tool calls the turn reached.
    ///
    /// `max_seq` is the turn's highest seq, and it is what makes `/undo --before <seq>` possible:
    /// with it, "which turn contained call 7" is arithmetic; without it, it would need a mapping
    /// kept in step with the journal, which is one more thing to get wrong.
    pub fn commit_at_seq(&mut self, max_seq: u64) -> Result<Vec<LedgerChange>, String> {
        if self.entries.is_empty() {
            return Ok(Vec::new());
        }
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let created = now_secs();
        let record = IndexRecord {
            created_at: created,
            entries: self.entries.clone(),
            max_seq,
        };
        // The slot must be this commit's own: `next_seq` only moves when a backup is taken, so a
        // turn that edited brand-new files would reuse the previous turn's name and overwrite its
        // undo entry.
        //
        // And the *name* must be free, not merely derived from the clock. `now_nanos` has
        // millisecond resolution on Windows, so two turns committed in the same tick used to
        // write to the same file — the second silently replacing the first's undo entry and
        // losing a turn of history. The stamp still orders entries; this walks the sequence up
        // until the name is unused, which keeps that ordering and makes the collision impossible.
        self.next_seq += 1;
        let mut name = self.index_name(self.next_seq);
        while self.dir.join(&name).exists() {
            self.next_seq += 1;
            name = self.index_name(self.next_seq);
        }
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
    /// Undo up to `turns` committed turns, newest first.
    ///
    /// Returns how many turns were actually undone *and* what they restored, because those are
    /// different facts: asking to go back three turns when the session only has one is not an
    /// error, and the caller should be able to say "one, and here is why" instead of reporting a
    /// silent success. Running out of turns stops the walk — it never fails on it.
    ///
    /// **Files only, and that is the whole of what this command does.** The transcript is not
    /// rewritten: the turns still happened, and the user who asked to go back can see what they
    /// are going back from. Rewinding the conversation is a separate decision with its own
    /// question (what does the model see afterwards?), and this function does not make it by
    /// accident.
    pub fn undo_turns(dir: &Path, turns: usize) -> Result<(usize, UndoSummary), String> {
        // Counted first, so an empty session is "zero turns" rather than an error: walking back
        // three turns when only one is recorded should say "one", not fail.
        let available = EditJournal::pending_turns(dir);
        let mut undone = 0;
        let mut restored: Vec<String> = Vec::new();
        for _ in 0..turns.min(available) {
            let summary = EditJournal::undo_latest(dir)?;
            undone += 1;
            restored.extend(summary.restored);
        }
        Ok((
            undone,
            UndoSummary {
                files: restored.len(),
                restored,
            },
        ))
    }

    /// How many turns to undo so that the turn **containing** `seq` is undone — the count a
    /// "rewind to before the step where this was found" needs.
    ///
    /// Three answers, not one `Option`, because the caller has to say something true: undoing the
    /// wrong number of turns destroys work, and "I cannot tell" and "that call is not in this
    /// session" are different instructions for what to type next.
    pub fn turns_before_seq(dir: &Path, seq: u64) -> Rewind {
        let Ok(candidates) = undo_candidates(dir) else {
            return Rewind::Unknown;
        };
        // Newest first, and read the whole column before answering: the walk below stops as soon
        // as a turn ends below `seq`, so validating it *during* the walk would happily answer from
        // a store whose numbering restarted further down — the very case that makes a count wrong.
        let mut columns: Vec<u64> = Vec::with_capacity(candidates.len());
        for path in candidates.iter().rev() {
            let Ok(text) = fs::read_to_string(path) else {
                return Rewind::Unknown;
            };
            let Ok(record) = serde_json::from_str::<IndexRecord>(&text) else {
                return Rewind::Unknown;
            };
            // A turn from before `max_seq` was recorded cannot answer the question, and neither can
            // the ones past it.
            if record.max_seq == 0 {
                return Rewind::Unknown;
            }
            columns.push(record.max_seq);
        }
        // Walking younger to older the column has to fall; anywhere it does not, this is a store
        // whose counter restarted and "the oldest turn reaching `seq`" stops meaning anything.
        if columns.windows(2).any(|pair| pair[1] >= pair[0]) {
            return Rewind::Unknown;
        }
        let mut counted = 0;
        let mut answer: Option<usize> = None;
        // The walk continues while turns *reach* `seq` — because "reaches" is not "contains". A
        // turn reaching seq 5 also reaches seq 2 if an older turn made it, and the turn whose range
        // actually contains the call is the **oldest** one that reaches it: its range starts after
        // the turn below it ended.
        //
        // Getting this wrong is not academic: returning the newest turn that reached the call would
        // rewind one turn for a finding from three turns ago, leaving the very edit the user asked
        // to undo in place. This test caught exactly that.
        for max_seq in &columns {
            counted += 1;
            if *max_seq >= seq {
                answer = Some(counted);
            } else {
                // This turn ended before the call, so the one above it in age is the owner.
                break;
            }
        }
        match answer {
            Some(turns) => Rewind::Turns(turns),
            // No turn reached it: the call is not in this session's recorded turns.
            None => Rewind::Unrecorded,
        }
    }

    /// How many committed turns are available to undo. Zero is a normal state, not an error —
    /// which is the whole reason this exists: `undo_latest` reports "nothing to undo" as an
    /// error, so a caller walking back several turns cannot tell "the session is empty" from
    /// "something went wrong" without asking first.
    pub fn pending_turns(dir: &Path) -> usize {
        undo_candidates(dir).map(|c| c.len()).unwrap_or(0)
    }

    pub fn undo_latest(dir: &Path) -> Result<UndoSummary, String> {
        let candidates = undo_candidates(dir)?;
        let latest = candidates.last().ok_or_else(|| {
            "nothing to undo (no file changes committed in this session)".to_string()
        })?;
        let text = fs::read_to_string(latest).map_err(|e| e.to_string())?;
        let record: IndexRecord = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let mut restored = Vec::new();
        let mut errors = Vec::new();
        // Reverse order, like `rollback`: the last mutation of the turn is
        // the first one unwound.
        for entry in record.entries.iter().rev() {
            match restore_entry(dir, entry) {
                Ok(()) => restored.push(entry.path.to_string_lossy().into_owned()),
                Err(e) => errors.push(e),
            }
        }
        if !errors.is_empty() {
            return Err(format!("undo incomplete: {}", errors.join("; ")));
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
        hunks: line_diff(&old, &new, LEDGER_HUNK_MAX_CHARS),
        old_sha256: crate::hash::sha256_hex(&old_bytes),
        new_sha256: crate::hash::sha256_hex(&new_bytes),
    })
}

/// Standard unified diff of `old` against `new`, capped at `max_chars`.
///
/// Emits `@@ -a,b +c,d @@` hunk headers plus up to [`CONTEXT_LINES`] unchanged
/// context lines around each change, so a reader can see WHERE in the file an
/// edit landed instead of a bare run of `-`/`+` lines (the old shape reported
/// an interior move as a whole-region rewrite: there was no line matching).
///
/// The `--- `/`+++ ` file header is deliberately NOT emitted here. This is the
/// shared hunk BODY: the ledger stores it as `LedgerChange::hunks`, and the
/// tool-layer permission previews prepend the two file-header lines
/// themselves (`firment_tools::tools::util::simple_diff`).
///
/// Identical inputs yield an empty string.
pub fn line_diff(old: &str, new: &str, max_chars: usize) -> String {
    /// Unchanged lines kept on each side of a change.
    const CONTEXT_LINES: usize = 3;

    let old_lines = split_diff_lines(old);
    let new_lines = split_diff_lines(new);
    let ops = diff_ops(&old_lines, &new_lines);

    // `pos[i]` is how many old/new lines the first `i` ops consumed, with a
    // final entry for the end of the list. Hunk headers are then arithmetic
    // rather than a second counting pass over the same ops.
    let mut pos = Vec::with_capacity(ops.len() + 1);
    let (mut o, mut n) = (0usize, 0usize);
    for op in &ops {
        pos.push((o, n));
        match *op {
            DiffOp::Keep(..) => {
                o += 1;
                n += 1;
            }
            DiffOp::Del(_) => o += 1,
            DiffOp::Ins(_) => n += 1,
        }
    }
    pos.push((o, n));

    // Changed ops closer together than two context blocks share one hunk, so
    // two nearby edits do not print overlapping context under two headers.
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        if matches!(op, DiffOp::Keep(..)) {
            continue;
        }
        let start = i.saturating_sub(CONTEXT_LINES);
        let end = (i + CONTEXT_LINES + 1).min(ops.len());
        match hunks.last_mut() {
            Some(last) if start <= last.1 => last.1 = end,
            _ => hunks.push((start, end)),
        }
    }

    let mut out = String::new();
    for (s, e) in hunks {
        let (old_start, new_start) = pos[s];
        let old_count = pos[e].0 - old_start;
        let new_count = pos[e].1 - new_start;
        let mut hunk = format!(
            "@@ -{} +{} @@\n",
            range_header(old_start, old_count),
            range_header(new_start, new_count)
        );
        for op in &ops[s..e] {
            match *op {
                DiffOp::Keep(line, _) => hunk.push_str(&format!(" {}\n", old_lines[line])),
                DiffOp::Del(line) => hunk.push_str(&format!("-{}\n", old_lines[line])),
                DiffOp::Ins(line) => hunk.push_str(&format!("+{}\n", new_lines[line])),
            }
        }
        // Whole hunks only. A diff cut mid-hunk leaves a `@@` header that does
        // not describe the lines under it, which is worse than showing less:
        // stop before the first hunk that would blow the budget.
        if !out.is_empty() && out.chars().count() + hunk.chars().count() > max_chars {
            out.push_str("… diff truncated\n");
            break;
        }
        out.push_str(&hunk);
    }
    if out.chars().count() > max_chars {
        return truncate_chars(&out, max_chars);
    }
    out
}

/// One line-level edit decision, in output order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DiffOp {
    /// Unchanged line: (0-based old index, 0-based new index).
    Keep(usize, usize),
    /// Removed line (0-based old index).
    Del(usize),
    /// Added line (0-based new index).
    Ins(usize),
}

/// The `-a,b` / `+c,d` half of a hunk header, following unified-diff
/// conventions: one line prints just its 1-based number, and an EMPTY range
/// prints the 0-based position (`@@ -0,0 +1,3 @@` for an insert at the top).
fn range_header(start: usize, count: usize) -> String {
    match count {
        0 => format!("{start},0"),
        1 => format!("{}", start + 1),
        _ => format!("{},{}", start + 1, count),
    }
}

/// Line-level LCS diff. The common prefix and suffix are trimmed FIRST so the
/// quadratic table only covers the region that actually changed, and a region
/// too large for that table degrades to delete-then-insert instead of
/// allocating gigabytes (`line_diff` runs over whole files on every commit).
fn diff_ops(old: &[&str], new: &[&str]) -> Vec<DiffOp> {
    /// Ceiling for the LCS table, in cells; 1M cells = 4 MB of `u32`.
    const MAX_CELLS: usize = 1_000_000;

    let mut prefix = 0;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old.len().saturating_sub(prefix)
        && suffix < new.len().saturating_sub(prefix)
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let a = &old[prefix..old.len() - suffix];
    let b = &new[prefix..new.len() - suffix];
    let mut ops: Vec<DiffOp> = (0..prefix).map(|k| DiffOp::Keep(k, k)).collect();
    let (n, m) = (a.len(), b.len());
    if n.saturating_mul(m) > MAX_CELLS {
        ops.extend((0..n).map(|i| DiffOp::Del(prefix + i)));
        ops.extend((0..m).map(|j| DiffOp::Ins(prefix + j)));
    } else {
        let stride = m + 1;
        // lcs[i * stride + j] = length of the LCS of a[i..] and b[j..].
        let mut lcs = vec![0u32; (n + 1) * stride];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i * stride + j] = if a[i] == b[j] {
                    lcs[(i + 1) * stride + j + 1] + 1
                } else {
                    lcs[(i + 1) * stride + j].max(lcs[i * stride + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0usize, 0usize);
        while i < n && j < m {
            if a[i] == b[j] {
                ops.push(DiffOp::Keep(prefix + i, prefix + j));
                i += 1;
                j += 1;
            } else if lcs[(i + 1) * stride + j] >= lcs[i * stride + j + 1] {
                ops.push(DiffOp::Del(prefix + i));
                i += 1;
            } else {
                ops.push(DiffOp::Ins(prefix + j));
                j += 1;
            }
        }
        ops.extend((i..n).map(|k| DiffOp::Del(prefix + k)));
        ops.extend((j..m).map(|k| DiffOp::Ins(prefix + k)));
    }
    ops.extend((0..suffix).map(|k| DiffOp::Keep(old.len() - suffix + k, new.len() - suffix + k)));
    ops
}

/// Lines to diff, with `str::lines()` semantics: a trailing `\n` does NOT
/// yield a phantom empty line. That keeps hunk ranges consistent with the
/// `old_lines` / `new_lines` counts printed beside the diff in the ledger and
/// in the system prompt (both use `lines()` too).
fn split_diff_lines(text: &str) -> Vec<&str> {
    text.lines().collect()
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

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// The `undo-*.json` index files of a session, oldest first. Factored out so `undo_latest` and
/// `pending_turns` cannot disagree about what counts as a turn — the two answers have to be the
/// same list or the count means nothing.
#[allow(dead_code)]
fn undo_candidates(dir: &Path) -> Result<Vec<PathBuf>, String> {
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
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    #[test]
    fn turns_before_seq_refuses_a_store_whose_counter_restarted() {
        // What the GUI wrote while the call counter lived on the Agent and the Agent was rebuilt
        // every turn: three turns closing at 5, 2 and 3. The column is no longer cumulative, so
        // "the oldest turn reaching call 2" answers three — undoing a turn that had already ended
        // before that call was made. Refusing is the only honest answer; `/undo <n>` still works.
        let dir = tempfile::tempdir().unwrap();
        let undo_dir = dir.path().join("undo");
        for (i, max_seq) in [5u64, 2, 3].iter().enumerate() {
            let file = dir.path().join(format!("f{i}.rs"));
            std::fs::write(&file, "original\n").unwrap();
            let mut journal = EditJournal::new(undo_dir.clone());
            journal.begin(&file).unwrap();
            std::fs::write(&file, "changed\n").unwrap();
            journal.commit_at_seq(*max_seq).unwrap();
        }

        for seq in [1u64, 2, 3, 5, 9] {
            assert_eq!(
                EditJournal::turns_before_seq(&undo_dir, seq),
                Rewind::Unknown,
                "call #{seq}: a restarted column must not be walked as if it were cumulative"
            );
        }
        // The store is otherwise healthy: an ordinary undo of the newest turn still works.
        assert_eq!(EditJournal::pending_turns(&undo_dir), 3);
    }

    #[test]
    fn turns_before_seq_counts_back_to_the_turn_that_contained_a_call() {
        // The review linkage: a finding names a tool call (`seq`), and the rewind has to include
        // the turn that made it. Three turns reaching seqs 1, 3 and 5 — rewinding past call 1
        // takes one turn, past 2 takes two (it was in the turn that reached 3), past 5 takes all
        // three.
        let dir = tempfile::tempdir().unwrap();
        let undo_dir = dir.path().join("undo");
        for (i, max_seq) in [1u64, 3, 5].iter().enumerate() {
            let file = dir.path().join(format!("f{i}.rs"));
            std::fs::write(&file, "original\n").unwrap();
            let mut journal = EditJournal::new(undo_dir.clone());
            journal.begin(&file).unwrap();
            std::fs::write(&file, "changed\n").unwrap();
            journal.commit_at_seq(*max_seq).unwrap();
        }

        // Call 5 was in the newest turn; call 3 in the middle; call 1 in the oldest — so
        // rewinding *past* it takes one, two and three turns respectively.
        assert_eq!(
            EditJournal::turns_before_seq(&undo_dir, 5),
            Rewind::Turns(1)
        );
        assert_eq!(
            EditJournal::turns_before_seq(&undo_dir, 3),
            Rewind::Turns(2)
        );
        assert_eq!(
            EditJournal::turns_before_seq(&undo_dir, 1),
            Rewind::Turns(3)
        );
        // Call 2 was made by the middle turn (its range is 2..=3), so it takes two as well.
        assert_eq!(
            EditJournal::turns_before_seq(&undo_dir, 2),
            Rewind::Turns(2)
        );
        // A call this session never made: nothing to rewind, and no guess.
        assert_eq!(
            EditJournal::turns_before_seq(&undo_dir, 9),
            Rewind::Unrecorded
        );
    }

    #[test]
    fn a_turn_without_a_recorded_seq_says_so_instead_of_guessing() {
        // Entries written before `max_seq` existed still parse (serde default) and read as 0. The
        // count is then unknowable, and `None` is the only honest answer — undoing the wrong
        // number of turns loses work, and `/undo <n>` is right there.
        let dir = tempfile::tempdir().unwrap();
        let undo_dir = dir.path().join("undo");
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "original\n").unwrap();
        let mut journal = EditJournal::new(undo_dir.clone());
        journal.begin(&file).unwrap();
        std::fs::write(&file, "changed\n").unwrap();
        journal.commit().unwrap();

        assert_eq!(EditJournal::turns_before_seq(&undo_dir, 1), Rewind::Unknown);
    }

    #[test]
    fn undo_turns_walks_back_several_turns_and_stops_at_the_end() {
        // The rewind, at the level where it can be checked: three committed turns, walk back two,
        // then ask for more than is left. "Asked for five and got one" is not an error — it is a
        // smaller number, which is why the count comes back alongside the files.
        let dir = tempfile::tempdir().unwrap();
        let undo_dir = dir.path().join("undo");
        let files: Vec<PathBuf> = (0..3)
            .map(|i| dir.path().join(format!("f{i}.rs")))
            .collect();

        for (i, file) in files.iter().enumerate() {
            std::fs::write(file, "original\n").unwrap();
            let mut journal = EditJournal::new(undo_dir.clone());
            journal.begin(file).unwrap();
            std::fs::write(file, format!("changed {i}\n")).unwrap();
            journal.commit().unwrap();
        }

        let (undone, summary) = EditJournal::undo_turns(&undo_dir, 2).unwrap();
        assert_eq!(undone, 2);
        assert_eq!(summary.files, 2, "two turns, one file each: {summary:?}");
        assert_eq!(std::fs::read_to_string(&files[2]).unwrap(), "original\n");
        assert_eq!(std::fs::read_to_string(&files[1]).unwrap(), "original\n");
        assert_eq!(
            std::fs::read_to_string(&files[0]).unwrap(),
            "changed 0\n",
            "the oldest turn is still standing — the walk goes newest first and stops"
        );

        // Asking for more turns than the session has takes what is there and stops.
        let (undone, summary) = EditJournal::undo_turns(&undo_dir, 5).unwrap();
        assert_eq!(undone, 1);
        assert_eq!(summary.files, 1);
        assert_eq!(std::fs::read_to_string(&files[0]).unwrap(), "original\n");

        // And with nothing left, still not an error.
        let (undone, summary) = EditJournal::undo_turns(&undo_dir, 3).unwrap();
        assert_eq!(undone, 0);
        assert_eq!(summary.files, 0);
        assert!(summary.restored.is_empty());
    }

    #[test]
    fn undo_turns_restores_every_file_of_one_turn() {
        // One turn, several files: `/undo 1` is the existing behaviour and has to stay exactly
        // that after the count arrived.
        let dir = tempfile::tempdir().unwrap();
        let undo_dir = dir.path().join("undo");
        let a = dir.path().join("a.rs");
        let b = dir.path().join("b.rs");
        std::fs::write(&a, "a original\n").unwrap();
        std::fs::write(&b, "b original\n").unwrap();

        let mut journal = EditJournal::new(undo_dir.clone());
        journal.begin(&a).unwrap();
        journal.begin(&b).unwrap();
        std::fs::write(&a, "a changed\n").unwrap();
        std::fs::write(&b, "b changed\n").unwrap();
        journal.commit().unwrap();

        let (undone, summary) = EditJournal::undo_turns(&undo_dir, 1).unwrap();
        assert_eq!(undone, 1);
        assert_eq!(summary.files, 2);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "a original\n");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b original\n");
    }

    use super::*;
    use tempfile::tempdir;

    /// `EditJournal::commit` does NOT write the ledger itself: it returns the
    /// changes and `Agent` appends them to `store.ledger_path(session)` (see
    /// `agent.rs`). So an export test has to do the same two steps -- commit,
    /// then append -- or it reads a file nothing ever created.
    fn committed_ledger(undo: &std::path::Path, journal: &mut EditJournal) -> Ledger {
        let changes = journal.commit().unwrap();
        let ledger = Ledger::new(undo.join("ledger.jsonl"));
        ledger.append(&changes).unwrap();
        ledger
    }

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
    fn begin_dedupes_the_same_file_named_two_ways() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "v1").unwrap();
        let mut journal = EditJournal::new(dir.path().join("undo"));
        journal.begin(&file).unwrap();
        // A roundabout spelling of the same file: a second entry would back up
        // the content the FIRST edit already wrote and "restore" that instead.
        let roundabout = dir.path().join("sub").join("..").join("a.txt");
        journal.begin(&roundabout).unwrap();
        assert_eq!(journal.entries.len(), 1, "one file, one backup entry");
        fs::write(&file, "v2").unwrap();
        journal.rollback().unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "v1");
    }

    #[test]
    fn same_path_folds_dot_components() {
        assert!(same_path(
            Path::new("/x/y/a.rs"),
            Path::new("/x/./y/../y/a.rs")
        ));
        assert!(!same_path(Path::new("/x/a.rs"), Path::new("/x/b.rs")));
    }

    #[test]
    fn each_committed_turn_keeps_its_own_undo_entry() {
        let dir = tempdir().unwrap();
        let undo = dir.path().join("undo");
        let mut journal = EditJournal::new(undo.clone());
        let a = dir.path().join("a.txt");
        fs::write(&a, "v1").unwrap();
        journal.begin(&a).unwrap();
        fs::write(&a, "v2").unwrap();
        journal.commit().unwrap();
        // Second turn touches only a NEW file, so no backup was taken and the
        // sample counter never moved — the index name used to collide with the
        // previous turn's and overwrite it.
        let b = dir.path().join("b.txt");
        journal.begin(&b).unwrap();
        fs::write(&b, "new").unwrap();
        journal.commit().unwrap();

        let indexes: Vec<String> = fs::read_dir(&undo)
            .unwrap()
            .filter_map(|e| {
                let name = e.unwrap().file_name().to_string_lossy().into_owned();
                name.starts_with("undo-").then_some(name)
            })
            .collect();
        assert_eq!(indexes.len(), 2, "two turns, two undo files: {indexes:?}");

        EditJournal::undo_latest(&undo).unwrap();
        assert!(
            !b.exists(),
            "undo of the newest turn removes the file it created"
        );
        EditJournal::undo_latest(&undo).unwrap();
        assert_eq!(fs::read_to_string(&a).unwrap(), "v1");
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
            changes[0].hunks.starts_with("@@ -1 +1 @@\n"),
            "got: {}",
            changes[0].hunks
        );
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
    fn ledger_delta_text_returns_only_new_entries() {
        let dir = tempdir().unwrap();
        let ledger = Ledger::new(dir.path().join("ledger.jsonl"));
        ledger
            .append(&[LedgerChange {
                path: PathBuf::from("a.txt"),
                old_lines: 0,
                new_lines: 1,
                hunks: "+a\n".to_string(),
                old_sha256: "old-a".to_string(),
                new_sha256: "new-a".to_string(),
            }])
            .unwrap();
        ledger
            .append(&[LedgerChange {
                path: PathBuf::from("b.txt"),
                old_lines: 0,
                new_lines: 1,
                hunks: "+b\n".to_string(),
                old_sha256: "old-b".to_string(),
                new_sha256: "new-b".to_string(),
            }])
            .unwrap();
        let (delta, last) = ledger.delta_text(1, 5);
        assert_eq!(last, 2);
        assert!(delta.contains("b.txt"), "got: {delta}");
        assert!(!delta.contains("a.txt"), "got: {delta}");
        let (delta2, _) = ledger.delta_text(2, 5);
        assert!(delta2.is_empty());
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
                old_sha256: "old-a".to_string(),
                new_sha256: "new-a".to_string(),
            }])
            .unwrap();
        ledger
            .append(&[LedgerChange {
                path: PathBuf::from("b.txt"),
                old_lines: 0,
                new_lines: 1,
                hunks: "+hello\n".to_string(),
                old_sha256: "old-b".to_string(),
                new_sha256: "new-b".to_string(),
            }])
            .unwrap();
        let summary = ledger.summary(10, 1000);
        assert!(summary.contains("a.txt"), "got: {summary}");
        assert!(summary.contains("b.txt"), "got: {summary}");
        assert!(summary.contains("+hello"), "got: {summary}");
        assert!(summary.contains("1 lines -> 2 lines"), "got: {summary}");
    }

    #[test]
    fn line_diff_single_line_change_reports_a_hunk_header() {
        let diff = line_diff("hello\nworld\n", "hi\nworld\n", 4000);
        assert_eq!(diff, "@@ -1,2 +1,2 @@\n-hello\n+hi\n world\n");
    }

    #[test]
    fn line_diff_groups_distant_changes_and_merges_nearby_ones() {
        // Context is 3 lines, so two changes whose context blocks do not touch
        // get their own hunks, each with its own `@@` header.
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\n";
        let new = "A\nb\nc\nd\ne\nf\ng\nh\ni\nj\nK\n";
        let diff = line_diff(old, new, 4000);
        assert_eq!(diff.matches("@@ -").count(), 2, "got: {diff}");
        assert!(diff.contains("-a\n+A\n"), "got: {diff}");
        assert!(diff.contains("-k\n+K\n"), "got: {diff}");

        // Changes whose context DOES touch share one hunk: printing the shared
        // context once is cheaper than printing it twice under two headers.
        let old = "a\nb\nc\nd\ne\n";
        let new = "A\nb\nc\nd\nE\n";
        let diff = line_diff(old, new, 4000);
        assert_eq!(diff.matches("@@ -").count(), 1, "got: {diff}");
        assert!(
            diff.contains(" b\n c\n d\n"),
            "shared context printed once: {diff}"
        );
    }

    #[test]
    fn line_diff_range_header_follows_unified_conventions() {
        // A whole-file insertion: the old side is empty, so it prints the
        // 0-based position instead of a 1-based number.
        assert_eq!(line_diff("", "x\ny\n", 4000), "@@ -0,0 +1,2 @@\n+x\n+y\n");
        // A single-line insertion in the middle prints a bare `+2`, not `+2,1`.
        assert_eq!(
            line_diff("a\nc\n", "a\nb\nc\n", 4000),
            "@@ -1,2 +1,3 @@\n a\n+b\n c\n"
        );
    }

    #[test]
    fn line_diff_identical_input_is_empty() {
        assert_eq!(line_diff("a\nb\n", "a\nb\n", 4000), "");
        assert_eq!(line_diff("", "", 4000), "");
    }

    #[test]
    fn line_diff_truncates_on_a_hunk_boundary() {
        // Two changes far enough apart to need two hunks; the first hunk is
        // "…@@ -1,4 +1,4 @@\n-a\n+A\n b\n c\n d\n" = 45 chars.
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let new = "A\nb\nc\nd\ne\nf\ng\nh\ni\nJ\n";
        let diff = line_diff(old, new, 60);
        // Exactly the first hunk is printed: the cut landed on a boundary
        // rather than inside a hunk, so no header is left describing a body
        // that was cut away.
        assert_eq!(diff.matches("@@ -").count(), 1, "got: {diff}");
        assert!(diff.contains("-a\n+A\n"), "got: {diff}");
        assert!(
            !diff.contains("-j\n+J\n"),
            "second hunk must be cut: {diff}"
        );
        assert!(diff.contains("… diff truncated"), "got: {diff}");
    }

    #[test]
    fn export_writes_a_unified_diff_per_changed_file() {
        let dir = tempdir().unwrap();
        let undo = dir.path().join("undo");
        let mut journal = EditJournal::new(undo.clone());
        let file = dir.path().join("src").join("main.c");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "int main(void) {\n  return 0;\n}\n").unwrap();

        journal.begin(&file).unwrap();
        fs::write(&file, "int main(void) {\n  return 1;\n}\n").unwrap();
        let ledger = committed_ledger(&undo, &mut journal);
        // The export reads the changes back from the JSONL the commit appended
        // to, the same way an embedder (or `/ledger --export`) would.
        let text = ledger.export_unified_diff(dir.path());
        assert!(text.contains("--- src/main.c\n"), "got: {text}");
        assert!(text.contains("+++ src/main.c\n"), "got: {text}");
        assert!(text.contains("@@ -1,3 +1,3 @@"), "got: {text}");
        assert!(text.contains("-  return 0;"), "got: {text}");
        assert!(text.contains("+  return 1;"), "got: {text}");
        // Forward slashes on every platform: a report copied between machines
        // (or into a patch) must not carry `src\\main.c`.
        assert!(!text.contains('\\'), "got: {text}");
    }

    /// The invariant a caller must be able to check: a truncated export is
    /// reported as truncated, because applying a partial patch does not fail
    /// loudly -- it applies a PARTIAL change.
    #[test]
    fn export_reports_which_files_were_truncated() {
        let dir = tempdir().unwrap();
        let undo = dir.path().join("undo");
        let mut journal = EditJournal::new(undo.clone());
        let file = dir.path().join("big.txt");
        // Two blocks of long lines, far enough apart to need two hunks and long
        // enough together to blow the 1600-char ledger budget. Short lines would
        // both fit and the test would silently stop covering truncation.
        let filler = "x".repeat(80);
        let old: String = (0..120).map(|i| format!("line {i} {filler}\n")).collect();
        let mut new_lines: Vec<String> = (0..120).map(|i| format!("line {i} {filler}\n")).collect();
        for line in new_lines.iter_mut().take(10) {
            line.push_str("ADDED\n");
        }
        for line in new_lines.iter_mut().skip(100) {
            line.push_str("ADDED\n");
        }
        let new: String = new_lines.concat();
        fs::write(&file, &old).unwrap();

        journal.begin(&file).unwrap();
        fs::write(&file, &new).unwrap();
        let ledger = committed_ledger(&undo, &mut journal);
        let exported = ledger.export_unified_diff(dir.path());
        assert!(
            exported.contains(TRUNCATION_MARKER),
            "a body past the budget must stay visibly truncated: {}",
            &exported[..exported.len().min(200)]
        );
        assert_eq!(
            ledger.export_truncated(),
            vec![file.clone()],
            "the affected file must be listed"
        );
    }

    #[test]
    fn export_of_a_small_change_lists_nothing_truncated() {
        let dir = tempdir().unwrap();
        let undo = dir.path().join("undo");
        let mut journal = EditJournal::new(undo.clone());
        let file = dir.path().join("a.txt");
        fs::write(&file, "one\ntwo\n").unwrap();
        journal.begin(&file).unwrap();
        fs::write(&file, "one\nTWO\n").unwrap();
        let ledger = committed_ledger(&undo, &mut journal);
        assert!(ledger.export_truncated().is_empty());
        assert!(
            !ledger
                .export_unified_diff(dir.path())
                .contains(TRUNCATION_MARKER)
        );
    }
}
