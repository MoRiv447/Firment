//! The left rail: the sessions in this workspace, and the files under it.
//!
//! Two lists that answer the two questions a terminal workspace has to answer
//! without a click: *which conversation am I in* and *what is in front of me*.
//! Both are read-only here -- the session switcher already exists as a picker,
//! and the file list is a map, not a file manager.
//!
//! The walk is deliberately shallow and capped. A rail that has to read a whole
//! tree before the first frame is a rail that makes startup worse, and the
//! point of the panel is orientation, not completeness: two levels and a couple
//! of hundred entries is what fits on screen anyway.

use std::collections::HashSet;
use std::path::Path;

/// How deep the walk goes. `src/main.c` is depth 2, which is what the workspace
/// mockup shows; deeper trees are what the agent's own tools are for.
const MAX_DEPTH: usize = 2;

/// Upper bound on rows. Reached on real projects, so it is a real limit rather
/// than a safety net.
///
/// Both sections of the rail stop here, and both say so: the FILES list used to be capped
/// while the SESSIONS list was not, so the section that reads as "the workspace" bounded its
/// memory by accident and the one that reads as "my chats" did not -- 200 open sessions is
/// unusual, several thousand is not impossible, and a rail row costs a label and an id per
/// agent-event repaint of the whole list.
pub(crate) const MAX_ROWS: usize = 200;

/// What a capped list had to leave out, and how much of that is knowable.
///
/// The distinction is the point. `Exact(n)` is a claim; `Unknown` says only that the producer
/// stopped early, which is all the file walk can honestly report -- it abandons the directory
/// at the cap, so the remainder was never counted. Printing "… 3 more" for a list that simply
/// stopped at 200 would be a number the code cannot support, and a silent truncation is the
/// failure this replaced: a file that is not on the rail looks like a file that is not there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shed {
    /// Nothing was left out.
    None,
    /// Exactly this many rows were left out.
    Exact(usize),
    /// The list hit the cap and the remainder was not counted.
    Unknown,
}

impl Shed {
    /// The rail's line for this, or `None` when there is nothing to say.
    pub(crate) fn line(&self) -> Option<String> {
        match self {
            Shed::None => None,
            Shed::Exact(n) => Some(format!("… {n} more")),
            Shed::Unknown => Some(format!("… more (the list stops at {MAX_ROWS})")),
        }
    }
}

/// Directories that are never worth a rail row: build output and dependency
/// trees are large, generated, and not what anyone is navigating by hand.
const SKIP_DIRS: [&str; 8] = [
    ".git",
    "target",
    "node_modules",
    ".next",
    "dist",
    "build",
    ".firment",
    "__pycache__",
];

/// One row of the FILES list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileRow {
    pub(crate) name: String,
    /// 0 for a top-level entry, 1 for a child. Drives the indent.
    pub(crate) depth: usize,
    pub(crate) is_dir: bool,
    /// Whether git reports this path as changed, which is what the dot marks.
    pub(crate) modified: bool,
}

/// One row of the SESSIONS list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionRow {
    pub(crate) id: String,
    /// What to show. The preview, falling back to the id, because a session
    /// that has not been used yet has no preview and an empty row reads as a
    /// rendering bug.
    pub(crate) label: String,
    pub(crate) current: bool,
}

impl SessionRow {
    /// Build the rail's session rows, marking `current`, and report what the cap
    /// left out. The newest sessions are kept: the rail is a switcher, and the
    /// sessions a person is cycling between are the recent ones.
    pub(crate) fn list(
        sessions: &[firment_core::SessionSummary],
        current: &str,
    ) -> (Vec<Self>, Shed) {
        let hidden = sessions.len().saturating_sub(MAX_ROWS);
        let rows = sessions
            .iter()
            .take(MAX_ROWS)
            .map(|s| SessionRow {
                id: s.id.clone(),
                label: label_for(&s.preview, &s.id),
                current: s.id == current,
            })
            .collect();
        (
            rows,
            if hidden == 0 {
                Shed::None
            } else {
                Shed::Exact(hidden)
            },
        )
    }
}

/// A session's display name: its preview, trimmed to one line.
fn label_for(preview: &str, id: &str) -> String {
    let first = preview
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if first.is_empty() {
        // Not a placeholder like "untitled": the id is genuinely more useful,
        // because it is what the user typed to resume this session.
        id.chars().take(8).collect()
    } else {
        first.chars().take(40).collect()
    }
}

/// Whether git reports `name` (relative, `/`-separated) as changed.
///
/// Git's paths are relative to the repository root while the rail's are
/// relative to the working directory, so a suffix match is the honest
/// comparison: the rail can only mark a file it is actually showing.
pub(crate) fn is_modified(name: &str, changed: &HashSet<String>) -> bool {
    changed.contains(name)
        || changed
            .iter()
            .any(|c| c.ends_with(&format!("/{name}")) || c == name)
}

/// The FILES rows under `root`: directories first, then files, alphabetically,
/// depth-limited and capped.
///
/// `Shed::Unknown` when the cap was reached: the walk abandons the tree at that point, so the
/// remainder was never counted, and "… 3 more" would be a number this function cannot know.
pub(crate) fn file_rows(root: &Path, changed: &HashSet<String>) -> (Vec<FileRow>, Shed) {
    let mut rows = Vec::new();
    walk(root, root, 1, changed, &mut rows);
    let shed = if rows.len() >= MAX_ROWS {
        Shed::Unknown
    } else {
        Shed::None
    };
    rows.truncate(MAX_ROWS);
    (rows, shed)
}

fn walk(root: &Path, dir: &Path, depth: usize, changed: &HashSet<String>, out: &mut Vec<FileRow>) {
    if depth > MAX_DEPTH || out.len() >= MAX_ROWS {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            if SKIP_DIRS.contains(&name.as_str()) || name.starts_with('.') {
                continue;
            }
            dirs.push(name);
        } else {
            files.push(name);
        }
    }
    dirs.sort();
    files.sort();
    for name in dirs.iter().chain(files.iter()) {
        let is_dir = dirs.contains(name);
        let path = dir.join(name);
        // Compared against the walk's own root rather than the repository root:
        // git reports `crates/x/y.rs` where the rail shows `src/y.rs`, and a
        // prefix that does not line up would silently mark nothing at all.
        let rel = path
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| name.clone());
        out.push(FileRow {
            name: name.clone(),
            depth: depth - 1,
            is_dir,
            modified: !is_dir && is_modified(&rel, changed),
        });
        if is_dir {
            walk(root, &path, depth + 1, changed, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_session_with_no_preview_falls_back_to_its_id() {
        // An empty row reads as a rendering bug; the id is what the user types
        // to resume it, so it is the useful fallback rather than a placeholder.
        let (rows, shed) = SessionRow::list(
            &[firment_core::SessionSummary {
                id: "abc12345".to_string(),
                updated_at: 0,
                model: String::new(),
                cwd: std::path::PathBuf::from("."),
                preview: "   \n  ".to_string(),
                kind: firment_core::SessionKind::Normal,
                parent_session: None,
            }],
            "abc12345",
        );
        assert_eq!(rows[0].label, "abc12345");
        assert!(rows[0].current);
        assert_eq!(shed, Shed::None, "one session sheds nothing");
    }

    #[test]
    fn a_session_label_is_the_first_line_of_its_preview() {
        let label = label_for("\n\nfix the I2C timeout\nand then flash", "deadbeef");
        assert_eq!(label, "fix the I2C timeout");
    }

    #[test]
    fn a_long_preview_is_clipped() {
        let label = label_for(&"x".repeat(200), "id");
        assert_eq!(label.chars().count(), 40);
    }

    #[test]
    fn only_the_current_session_is_marked() {
        let sessions: Vec<firment_core::SessionSummary> = ["a", "b"]
            .iter()
            .map(|id| firment_core::SessionSummary {
                id: id.to_string(),
                updated_at: 0,
                model: String::new(),
                cwd: std::path::PathBuf::from("."),
                preview: format!("chat {id}"),
                kind: firment_core::SessionKind::Normal,
                parent_session: None,
            })
            .collect();
        let (rows, shed) = SessionRow::list(&sessions, "b");
        assert_eq!(rows.iter().filter(|r| r.current).count(), 1);
        assert!(rows[1].current);
        assert_eq!(shed, Shed::None);
    }

    #[test]
    fn the_session_list_is_capped_and_says_what_it_left_out() {
        // FILES was capped at 200 rows and SESSIONS was not -- the section that reads as
        // "the workspace" bounded its memory by accident, and the one that reads as "my
        // chats" grew with however many sessions exist, rebuilt in full on every
        // agent-event repaint. Same cap, and the shed count is shown rather than implied.
        let sessions: Vec<firment_core::SessionSummary> = (0..=MAX_ROWS)
            .map(|i| firment_core::SessionSummary {
                id: format!("s{i:04}"),
                updated_at: 0,
                model: String::new(),
                cwd: std::path::PathBuf::from("."),
                preview: format!("chat {i}"),
                kind: firment_core::SessionKind::Normal,
                parent_session: None,
            })
            .collect();
        let (rows, shed) = SessionRow::list(&sessions, "s0000");
        assert_eq!(rows.len(), MAX_ROWS, "the rail stops at the cap");
        assert_eq!(
            shed,
            Shed::Exact(1),
            "the remainder IS knowable here, unlike the file walk"
        );
        assert_eq!(shed.line().as_deref(), Some("… 1 more"));
    }

    #[test]
    fn the_files_cap_is_reported_as_unknown_rather_than_as_a_number() {
        // The walk abandons the tree at the cap, so it cannot know how many files it did not
        // look at. Claiming a count would be a number the producer cannot support.
        let root = std::env::temp_dir().join(format!("firment-rail-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for i in 0..(MAX_ROWS + 40) {
            std::fs::write(root.join(format!("f{i:04}.c")), "").unwrap();
        }

        let (rows, shed) = file_rows(&root, &HashSet::new());
        assert_eq!(rows.len(), MAX_ROWS);
        assert_eq!(shed, Shed::Unknown);
        assert_eq!(
            shed.line().as_deref(),
            Some("… more (the list stops at 200)"),
            "the capped form must not read like a count"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_changed_file_is_marked_even_though_git_reports_a_repo_relative_path() {
        // The trap: git says `crates/firment-tui/src/view.rs`, the rail shows
        // `view.rs`. A plain equality check would mark nothing and look fine.
        assert!(is_modified(
            "view.rs",
            &changed(&["crates/firment-tui/src/view.rs"])
        ));
        assert!(is_modified("src/main.c", &changed(&["src/main.c"])));
        assert!(!is_modified("pwm.c", &changed(&["src/main.c"])));
    }

    #[test]
    fn the_walk_lists_directories_before_files_and_skips_build_output() {
        let root = std::env::temp_dir().join(format!("firment-rail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("include")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("src/main.c"), "").unwrap();
        std::fs::write(root.join("src/pwm.c"), "").unwrap();
        std::fs::write(root.join("include/pwm.h"), "").unwrap();
        std::fs::write(root.join("target/junk"), "").unwrap();
        std::fs::write(root.join("Makefile"), "").unwrap();

        let (rows, shed) = file_rows(&root, &changed(&["src/main.c"]));
        assert_eq!(shed, Shed::None, "a small tree must not claim a cap");

        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        // Directories first, and never the build output.
        assert_eq!(
            names,
            vec!["include", "pwm.h", "src", "main.c", "pwm.c", "Makefile"]
        );
        assert!(rows[0].is_dir);
        assert!(!rows[1].is_dir);
        assert_eq!(rows[1].depth, 1, "a child is indented");
        assert!(rows[3].modified, "src/main.c is the changed one");
        assert!(!rows[4].modified);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_directory_is_empty_rather_than_a_panic() {
        let (rows, shed) = file_rows(Path::new("no-such-dir-firment-xyz"), &HashSet::new());
        assert!(rows.is_empty());
        assert_eq!(
            shed,
            Shed::None,
            "an unreadable root is an empty list, not a list that hit its cap"
        );
    }
}
