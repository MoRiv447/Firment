//! Finding the files a static review should read.
//!
//! One implementation for every caller. The CLI and the TUI each had their own copy of
//! this walk, which is two chances to disagree about what counts as reviewable source —
//! and they did: the CLI skipped `target/` and the TUI skipped `target/` and `dist/`, so
//! the same command from two surfaces read different files.

use std::path::{Path, PathBuf};

/// Extensions the static rules can read. Deliberately short: a rule that pretends to
/// understand a language it cannot parse is worse than no rule.
pub const REVIEWABLE_EXTENSIONS: [&str; 4] = ["rs", "c", "h", "s"];

/// Directories that are never source: build output, dependencies, version control.
const SKIPPED_DIRECTORIES: [&str; 5] = ["target", "node_modules", ".git", "dist", "build"];

/// How many files one run will read. A review of `crates/` should not become a build.
pub const FILE_LIMIT: usize = 200;

/// What a review run would read, and how many files it left.
pub struct ReviewFiles {
    pub files: Vec<PathBuf>,
    /// Reviewable files beyond [`FILE_LIMIT`]. Reported rather than dropped in silence:
    /// a review of a prefix that says it reviewed everything is the kind of claim this
    /// whole module exists to avoid.
    pub skipped: usize,
}

/// Whether this path is source rather than test code.
///
/// Integration tests live in `tests/`, not in `#[cfg(test)]` modules, so the line-level
/// skip in `rules` cannot see them. Running the rules over this repository reported three
/// `env::set_var` findings in `crates/firment-core/tests/` — legitimate in a test, and
/// noise in a review of production code.
fn is_test_path(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some("tests") | Some("benches") | Some("examples")
        )
    })
}

/// Every reviewable file under `root`, with `root` itself when it is a file.
pub fn collect(root: &Path) -> ReviewFiles {
    if root.is_file() {
        return ReviewFiles {
            files: vec![root.to_path_buf()],
            skipped: 0,
        };
    }
    let mut files = Vec::new();
    let mut skipped = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        children.sort();
        for path in children {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if SKIPPED_DIRECTORIES.contains(&name) || is_test_path(&path) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let reviewable = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| REVIEWABLE_EXTENSIONS.contains(&e));
            if !reviewable || is_test_path(&path) {
                continue;
            }
            if files.len() >= FILE_LIMIT {
                skipped += 1;
                continue;
            }
            files.push(path);
        }
    }
    files.sort();
    ReviewFiles { files, skipped }
}

/// A file's label in a report: the path relative to what was reviewed.
///
/// A single file has no relative form — stripping the root from itself leaves an empty
/// label, and an empty label silently turns off every path-dependent rule (the `unsafe`
/// rule only runs on `.rs`, and it could no longer tell). Measured: reviewing one file
/// reported nothing while reviewing its directory reported two findings in it.
pub fn label_for(root: &Path, file: &Path) -> String {
    let relative = file
        .strip_prefix(root)
        .ok()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new(file.file_name().unwrap_or(file.as_os_str())));
    relative.display().to_string().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_reviewable_sources_are_collected_and_build_output_is_skipped() {
        // The rule is about *not* reviewing what the tools cannot read: a `.txt` beside a
        // `.rs` is not source, and `target/` holds thousands of generated files that would
        // make the file limit meaningless.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        std::fs::create_dir_all(dir.path().join("tests")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(dir.path().join("src/b.c"), "int b(void) { return 0; }\n").unwrap();
        std::fs::write(dir.path().join("src/notes.txt"), "hi\n").unwrap();
        std::fs::write(dir.path().join("target/debug/generated.rs"), "fn g() {}\n").unwrap();
        std::fs::write(
            dir.path().join("tests/integration.rs"),
            "unsafe { env::set_var(\"A\", \"b\") }\n",
        )
        .unwrap();

        let found = collect(dir.path());
        assert_eq!(found.skipped, 0);
        let names: Vec<String> = found
            .files
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"a.rs".to_string()), "{names:?}");
        assert!(names.contains(&"b.c".to_string()), "{names:?}");
        assert!(!names.iter().any(|n| n.ends_with(".txt")), "{names:?}");
        assert!(
            !names.iter().any(|n| n == "generated.rs"),
            "target/ must be skipped: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n == "integration.rs"),
            "integration tests are test code, not source: {names:?}"
        );
    }

    #[test]
    fn a_single_file_keeps_its_own_name_as_its_label() {
        // The defect this pins: stripping a file from itself leaves an empty label, and an
        // empty label turns off every path-dependent rule without a word.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("install.rs");
        std::fs::write(&file, "fn f() {}\n").unwrap();

        let found = collect(&file);
        assert_eq!(found.files.len(), 1);
        assert_eq!(label_for(&file, &file), "install.rs");

        let nested = dir.path().join("src");
        std::fs::create_dir_all(&nested).unwrap();
        let inside = nested.join("main.rs");
        assert_eq!(label_for(dir.path(), &inside), "src/main.rs");
    }
}
