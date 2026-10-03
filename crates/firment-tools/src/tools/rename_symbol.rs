//! Cross-file symbol rename (plan §5, item 9): the whole rename, or none of it.
//!
//! The plan asks for two things and they are not the same size. The first is mechanical:
//! rename an identifier across files, matching **whole words** — `foo` inside `foobar` is a
//! different identifier, and a tool that replaced it would be a find-and-replace pretending
//! to be a rename. The second is the one that matters: **all of it or none of it**. A
//! rename that lands in three files and fails on the fourth leaves a project that does not
//! compile, in a state nobody asked for, with the failure explained in a tool result the
//! user may never read.
//!
//! So the transaction is the feature, and it reuses the machinery that already exists
//! rather than inventing a second one: `EditJournal::begin` backs a file up before it is
//! touched, and `rollback_paths` gives back the backups this call took. One `begin` per
//! file, one rollback to that set on the first failure, and **no commit** — the turn's
//! epilogue closes the shared transaction, because that is the only place which knows the
//! call number the turn reached.
//!
//! `dry_run` reports the plan. It is off by default because the caller asked for a rename,
//! not for a plan — but the tool is built so that running it twice is safe: the second run
//! finds no matches and refuses, which is the same answer as "already done".

use super::util::{resolve_write_scope, simple_diff};
use async_trait::async_trait;
use firment_core::{EditJournal, Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::path::PathBuf;

pub struct RenameSymbol;

/// One file's part of the plan.
#[derive(Debug)]
struct FilePlan {
    resolved: PathBuf,
    label: String,
    count: usize,
    /// The file as it was read, kept for the diff in the result: without it the "reviewable
    /// change" is computed against nothing and renders the whole file as additions.
    original: String,
    updated: String,
}

#[async_trait]
impl Tool for RenameSymbol {
    fn name(&self) -> &'static str {
        "rename_symbol"
    }

    fn description(&self) -> &'static str {
        "Rename an identifier across several files as one transaction: every file is backed \
         up before it is written, and a failure on any file rolls all of them back. Matches \
         whole words only — `foo` inside `foobar` is left alone — and it rewrites matches in \
         comments and strings too, which is what a rename means. Use dry_run to see the plan \
         first."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "from": {"type": "string", "description": "The identifier to rename"},
                "to": {"type": "string", "description": "Its new name"},
                "paths": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "The files to rewrite. Glob or grep first to build this list; a file in it with no match is reported rather than treated as an error."
                },
                "dry_run": {
                    "type": "boolean",
                    "default": false,
                    "description": "Report the plan without writing anything"
                }
            },
            "required": ["from", "to", "paths"]
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        let from = args.get("from").and_then(|v| v.as_str()).unwrap_or("");
        let to = args.get("to").and_then(|v| v.as_str()).unwrap_or("");
        let files = args
            .get("paths")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let dry = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Some(format!(
            "rename `{from}` to `{to}` across up to {files} file(s){}",
            if dry { " (dry run)" } else { "" }
        ))
    }

    fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<String> {
        let from = args.get("from").and_then(|v| v.as_str())?;
        let to = args.get("to").and_then(|v| v.as_str())?;
        let paths = args.get("paths").and_then(|v| v.as_array())?;
        let mut preview = String::new();
        for path in paths.iter().filter_map(|value| value.as_str()) {
            let Ok(resolved) = resolve_write_scope(ctx, path) else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&resolved) else {
                continue;
            };
            let (updated, count) = replace_whole_word(&text, from, to);
            if count > 0 {
                preview.push_str(&simple_diff(&resolved, &text, &updated, 4000));
            }
        }
        (!preview.is_empty()).then_some(preview)
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let from = require_identifier(&args, "from")?;
        let to = require_identifier(&args, "to")?;
        if from == to {
            return Err(ToolError::new(
                "[InvalidInput] `from` and `to` are the same identifier — nothing to rename",
            ));
        }
        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let paths: Vec<String> = args
            .get("paths")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|value| value.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if paths.is_empty() {
            return Err(ToolError::new(
                "[InvalidInput] `paths` is empty — list the files to rename in (glob or grep \
                 first). A rename with no files is a rename that did nothing.",
            ));
        }

        // Everything is resolved and read BEFORE anything is written: a path outside the
        // workspace, or a file that is not UTF-8 text, is a reason to stop — not a reason
        // to have already rewritten three other files.
        let mut plans: Vec<FilePlan> = Vec::new();
        let mut unmatched: Vec<String> = Vec::new();
        for path in &paths {
            let resolved = resolve_write_scope(ctx, path).map_err(ToolError::new)?;
            let text = std::fs::read_to_string(&resolved).map_err(|e| {
                ToolError::new(format!(
                    "[Io] {} could not be read as text: {e} — remove it from `paths` if it is \
                     not source",
                    resolved.display()
                ))
            })?;
            let (updated, count) = replace_whole_word(&text, &from, &to);
            let label = resolved.display().to_string();
            if count == 0 {
                unmatched.push(label);
                continue;
            }
            plans.push(FilePlan {
                resolved,
                label,
                count,
                original: text,
                updated,
            });
        }

        let total: usize = plans.iter().map(|plan| plan.count).sum();
        if total == 0 {
            return Err(ToolError::new(format!(
                "[NotFound] no whole-word match for `{from}` in {} file(s). Either the name is \
                 spelled differently, or the rename already happened.",
                paths.len()
            )));
        }

        let summary = |plans: &[FilePlan], unmatched: &[String], verb: &str| -> String {
            let mut out = format!(
                "{verb} {total} occurrence(s) across {} file(s):\n",
                plans.len()
            );
            for plan in plans {
                out.push_str(&format!("  {} — {}\n", plan.count, plan.label));
            }
            if !unmatched.is_empty() {
                out.push_str(&format!(
                    "  no match in {} file(s): {}\n",
                    unmatched.len(),
                    unmatched.join(", ")
                ));
            }
            out
        };

        if dry_run {
            return Ok(ToolOutput {
                text: summary(&plans, &unmatched, "dry run:"),
            });
        }

        // The transaction. One `begin` per file before any write, and **no commit here**: the
        // turn's own epilogue closes the shared journal (`agent.rs`, `commit_at_seq`) with the
        // turn's highest call number, which is the number `/undo --before` measures against. A
        // tool that closed it itself wrote `max_seq = 0`, and `turns_before_seq` answers
        // "unknown" for every turn in a session that holds one — so `/undo --before` would stop
        // working for the rest of the session's life. Any failure rolls back only the backups
        // this call took, which is what makes the failure a no-op rather than a half-finished
        // rename *and* keeps another tool's work in the same turn from being handed back.
        {
            let mut journal = ctx
                .journal
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // Which of these files this call is the first to record. A path already in the
            // batch was written by another tool this turn: its backup is that tool's state, not
            // the state before this rename, so this call has no business restoring it.
            let mut ours: Vec<bool> = Vec::with_capacity(plans.len());
            for plan in &plans {
                ours.push(journal.begin(&plan.resolved).map_err(ToolError::new)?);
            }
            for (i, plan) in plans.iter().enumerate() {
                // Re-read and compare before writing: a file that changed since we read it
                // means someone else is editing, and the right answer is to stop rather
                // than to overwrite their work. (Same CAS rule as `edit_file`.)
                let current = std::fs::read_to_string(&plan.resolved).map_err(|e| {
                    ToolError::new(format!("[Io] re-read failed for {}: {e}", plan.label))
                });
                let current = match current {
                    Ok(text) => text,
                    Err(e) => return Err(aborted(&mut journal, &plans, &ours, i, e.message)),
                };
                let (fresh, _) = replace_whole_word(&current, &from, &to);
                if fresh != plan.updated {
                    return Err(aborted(
                        &mut journal,
                        &plans,
                        &ours,
                        i,
                        format!(
                            "[ConcurrentChange] {} changed while the rename was being prepared",
                            plan.label
                        ),
                    ));
                }
                if let Err(e) = firment_core::session::write_atomic(&plan.resolved, &plan.updated) {
                    return Err(aborted(
                        &mut journal,
                        &plans,
                        &ours,
                        i,
                        format!("[Io] writing {} failed: {e}", plan.label),
                    ));
                }
            }
        }

        let mut text = summary(&plans, &unmatched, "renamed");
        // The diffs, so the change is reviewable from the result rather than only in the
        // transcript's tool card.
        for plan in &plans {
            text.push_str(&format!(
                "\n{}",
                simple_diff(&plan.resolved, &plan.original, &plan.updated, 400)
            ));
        }
        Ok(ToolOutput { text })
    }
}

/// Give back what an aborted rename wrote, and say by how much.
///
/// `done` is how many files had already been written when the rename stopped: the write loop runs
/// file by file, so failing on the fourth means three changed, and a message claiming "nothing
/// was written" would be the one thing the reader cannot act on.
///
/// Two routes, because there are two states to go back to. A file this call recorded is restored
/// from its backup and leaves the batch, so the turn's commit does not record an edit that no
/// longer stands. A file another tool already recorded stays in that tool's batch — its backup is
/// the state before *that* tool ran, and restoring it would lose someone else's work — so this
/// call writes back the bytes it read, which is that tool's text with the rename taken out.
fn aborted(
    journal: &mut EditJournal,
    plans: &[FilePlan],
    ours: &[bool],
    done: usize,
    reason: String,
) -> ToolError {
    let mine: Vec<PathBuf> = plans
        .iter()
        .zip(ours)
        .filter(|(_, recorded)| **recorded)
        .map(|(plan, _)| plan.resolved.clone())
        .collect();
    // Both routes land on the same state — the text this rename read — so one sentence covers
    // them: a backup this call took holds exactly `plan.original`.
    let mut reverted: Vec<String> = Vec::new();
    let mut stranded: Vec<String> = Vec::new();
    for (plan, recorded) in plans[..done].iter().zip(&ours[..done]) {
        if *recorded {
            continue;
        }
        match firment_core::session::write_atomic(&plan.resolved, &plan.original) {
            Ok(()) => reverted.push(plan.label.clone()),
            Err(e) => stranded.push(format!("{} ({e})", plan.label)),
        }
    }
    let incomplete = match journal.rollback_paths(&mine) {
        Ok(files) => {
            reverted.extend(files);
            String::new()
        }
        Err(e) => format!("; {e}"),
    };
    let mut out = reason;
    if reverted.is_empty() && stranded.is_empty() && incomplete.is_empty() {
        out.push_str(" — nothing had been written");
    } else {
        out.push_str(&format!(
            " — {} file(s) were put back as they were read: {reverted}{incomplete}",
            reverted.len(),
            reverted = reverted.join(", ")
        ));
    }
    if !stranded.is_empty() {
        out.push_str(&format!(
            "; {} file(s) still carry the rename: {}",
            stranded.len(),
            stranded.join(", ")
        ));
    }
    ToolError::new(out)
}

/// An argument that must be a bare identifier: a rename of a phrase is a different tool.
fn require_identifier(args: &Value, key: &str) -> Result<String, ToolError> {
    let value = args
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if value.is_empty() {
        return Err(ToolError::new(format!(
            "[InvalidInput] `{key}` is empty — a rename needs an identifier"
        )));
    }
    if !value.chars().all(is_identifier_char) {
        return Err(ToolError::new(format!(
            "[InvalidInput] `{key}` = `{value}` is not an identifier (letters, digits and \
             underscores). Renaming a phrase or a path is a different job; use edit_file for \
             that."
        )));
    }
    Ok(value)
}

/// Whether a character can be part of an identifier.
///
/// Letters, digits and `_`, for every language this project touches. A rename that also
/// wanted to respect a language's own rules (`$` in JS, `-` in CSS) would need a parser per
/// language, and the honest trade is the one the tool's description makes: whole words by
/// this definition, and the plan visible before it runs.
fn is_identifier_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Replace whole-word occurrences of `from`, returning the new text and the count.
///
/// Occurrences inside comments and strings **are** replaced, both because that is what a
/// rename means and because the alternative (a lexer per language) is a much larger promise
/// than this tool makes.
pub fn replace_whole_word(text: &str, from: &str, to: &str) -> (String, usize) {
    if from.is_empty() {
        return (text.to_string(), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut count = 0usize;
    let mut at = 0usize;
    while let Some(offset) = text[at..].find(from) {
        let start = at + offset;
        let end = start + from.len();
        let before_ok = text[..start]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_identifier_char(ch));
        let after_ok = text[end..]
            .chars()
            .next()
            .is_none_or(|ch| !is_identifier_char(ch));
        if before_ok && after_ok {
            out.push_str(&text[at..start]);
            out.push_str(to);
            count += 1;
        } else {
            out.push_str(&text[at..end]);
        }
        at = end;
    }
    out.push_str(&text[at..]);
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal, journal::Rewind};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    fn ctx(dir: &Path) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            journal: Arc::new(Mutex::new(EditJournal::new(dir.join("undo")))),
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        }
    }

    #[test]
    fn a_rename_matches_whole_words_and_not_parts_of_other_identifiers() {
        // The difference between a rename and a find-and-replace, which is the whole point.
        let text = "let foo = 1; let foobar = 2; foo(); my_foo(x);";
        let (updated, count) = replace_whole_word(text, "foo", "bar");
        assert_eq!(count, 2, "{updated}");
        assert_eq!(updated, "let bar = 1; let foobar = 2; bar(); my_foo(x);");
    }

    #[test]
    fn a_rename_reaches_the_edges_of_the_file() {
        let (updated, count) = replace_whole_word("foo a foo", "foo", "b");
        assert_eq!(count, 2);
        assert_eq!(updated, "b a b");
        // A name that is not there, and the empty case, change nothing.
        assert_eq!(replace_whole_word("x", "foo", "b"), ("x".to_string(), 0));
        assert_eq!(replace_whole_word("", "foo", "b"), (String::new(), 0));
    }

    #[tokio::test]
    async fn a_rename_touches_every_file_it_names_and_reports_the_ones_it_did_not() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn foo() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "foo();\nfoo();\n").unwrap();
        std::fs::write(dir.path().join("c.rs"), "fn other() {}\n").unwrap();
        let ctx = ctx(dir.path());

        let result = RenameSymbol
            .run(
                json!({"from": "foo", "to": "bar", "paths": ["a.rs", "b.rs", "c.rs"]}),
                &ctx,
            )
            .await
            .unwrap();

        // The result is supposed to be reviewable without re-reading the files: an empty
        // "before" rendered every touched file as `+` lines and hid the rename itself.
        assert!(
            result
                .text
                .lines()
                .any(|l| l.starts_with('-') && l.contains("foo")),
            "no removed line naming the old symbol: {}",
            result.text
        );
        assert!(
            result
                .text
                .lines()
                .any(|l| l.starts_with('+') && l.contains("bar")),
            "no added line naming the new symbol: {}",
            result.text
        );

        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
            "fn bar() {}\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("b.rs")).unwrap(),
            "bar();\nbar();\n"
        );
        // The file with no match is reported, not silently rewritten and not an error.
        assert!(
            result.text.contains("no match in 1 file(s)"),
            "{}",
            result.text
        );
        assert!(result.text.contains("3 occurrence(s)"), "{}", result.text);
    }

    #[tokio::test]
    async fn a_dry_run_reports_the_plan_and_writes_nothing() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "foo\n").unwrap();
        let ctx = ctx(dir.path());

        let result = RenameSymbol
            .run(
                json!({"from": "foo", "to": "bar", "paths": ["a.rs"], "dry_run": true}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.text.starts_with("dry run:"), "{}", result.text);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
            "foo\n",
            "a dry run must not write"
        );
    }

    #[tokio::test]
    async fn a_rename_with_nothing_to_rename_refuses_instead_of_doing_half_of_it() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "nothing here\n").unwrap();
        let ctx = ctx(dir.path());

        let error = RenameSymbol
            .run(json!({"from": "foo", "to": "bar", "paths": ["a.rs"]}), &ctx)
            .await
            .unwrap_err();
        assert!(error.message.contains("[NotFound]"), "{}", error.message);
    }

    #[tokio::test]
    async fn a_rename_of_a_phrase_is_refused_with_the_reason() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "foo\n").unwrap();
        let ctx = ctx(dir.path());
        let error = RenameSymbol
            .run(
                json!({"from": "foo bar", "to": "baz", "paths": ["a.rs"]}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(
            error.message.contains("not an identifier"),
            "{}",
            error.message
        );
    }

    #[tokio::test]
    async fn a_file_the_rename_cannot_write_puts_every_other_file_back() {
        // The transaction, tested the only way that means anything: force a real failure and
        // check the earlier writes are gone.
        //
        // The failure has to be real on both platforms, and a read-only file stopped being
        // one of them: the write goes through a temporary created next to the target and a
        // rename over it, and a rename needs write permission on the **directory**, not on
        // the file it covers. POSIX honours that and ignores the file attribute; Windows
        // notices the attribute and refuses. So the second file carries both restrictions —
        // read-only, inside a directory that cannot be written into — and each system is
        // stopped by whichever half applies to it.
        let dir = tempdir().unwrap();
        let nested = dir.path().join("sub");
        std::fs::create_dir(&nested).unwrap();
        let first = dir.path().join("a.rs");
        let second = nested.join("b.rs");
        std::fs::write(&first, "foo\n").unwrap();
        std::fs::write(&second, "foo\n").unwrap();
        // The originals are kept so the cleanup can put back exactly what was
        // there. `set_readonly(false)` is not a restore: on Unix it widens the
        // mode to world-writable, which is the footgun clippy names here.
        let mut restricted = Vec::new();
        for path in [&second, &nested] {
            let original = std::fs::metadata(path).unwrap().permissions();
            let mut locked = original.clone();
            locked.set_readonly(true);
            std::fs::set_permissions(path, locked).unwrap();
            restricted.push((path, original));
        }

        let ctx = ctx(dir.path());
        let result = RenameSymbol
            .run(
                json!({"from": "foo", "to": "bar", "paths": ["a.rs", "sub/b.rs"]}),
                &ctx,
            )
            .await;

        // Restore before asserting anything: removing an entry needs write permission on
        // its directory and Windows will not unlink a read-only file, so a test that left
        // the restrictions in place could not clean up after itself — and a leaked
        // read-only tree under the temp dir makes the next run fail for a reason this one
        // did not intend.
        for (path, original) in &restricted {
            std::fs::set_permissions(path, original.clone()).unwrap();
        }

        assert!(result.is_err(), "the rename should have failed: {result:?}");
        assert_eq!(
            std::fs::read_to_string(&first).unwrap(),
            "foo\n",
            "the first file must be rolled back"
        );
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "foo\n");
    }

    #[tokio::test]
    async fn a_rename_leaves_the_turns_transaction_open_for_the_turn_to_close() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn foo() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "foo();\n").unwrap();
        let ctx = ctx(dir.path());

        RenameSymbol
            .run(
                json!({"from": "foo", "to": "bar", "paths": ["a.rs", "b.rs"]}),
                &ctx,
            )
            .await
            .unwrap();

        // The tool used to `commit()` the journal it was handed, which closed the *turn's*
        // transaction in the middle of the turn: every edit that came after it landed in a batch
        // nobody would ever undo. Only the turn knows where it stops.
        assert!(
            !ctx.journal.lock().unwrap().is_empty(),
            "a tool that commits the shared journal has ended the turn's transaction"
        );

        // Closing it the way the epilogue does has to leave a store `/undo --before` can answer from.
        // The tool-level commit wrote `max_seq = 0`, and `turns_before_seq` gives up on any entry that
        // holds one — for every turn in the session, permanently.
        ctx.journal.lock().unwrap().commit_at_seq(4).unwrap();
        assert_eq!(
            EditJournal::turns_before_seq(&dir.path().join("undo"), 2),
            Rewind::Turns(1),
            "the rename reached call 2, so one turn is what goes back"
        );
    }

    #[tokio::test]
    async fn a_rename_failure_does_not_take_back_another_tools_file() {
        // The batch can already hold this turn's earlier work, and its backup is the state before
        // *that* tool ran. Rolling the whole batch back would delete the change the user asked
        // for two tools ago, because a later tool failed.
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        let shared = dir.path().join("a.rs");
        let blocked = dir.path().join("sub").join("b.rs");
        std::fs::write(&shared, "foo();\n").unwrap();
        std::fs::write(&blocked, "foo();\n").unwrap();

        let ctx = ctx(dir.path());
        ctx.journal.lock().unwrap().begin(&shared).unwrap();
        const OTHER_TOOL: &str = "the earlier tool wrote foo();\n";
        std::fs::write(&shared, OTHER_TOOL).unwrap();

        // The rename reads that text as its starting point, then cannot write the second file.
        let original = std::fs::metadata(&blocked).unwrap().permissions();
        let mut locked = original.clone();
        locked.set_readonly(true);
        std::fs::set_permissions(&blocked, locked).unwrap();
        let result = RenameSymbol
            .run(
                json!({"from": "foo", "to": "bar", "paths": ["a.rs", "sub/b.rs"]}),
                &ctx,
            )
            .await;
        std::fs::set_permissions(&blocked, original).unwrap();

        let err = result.expect_err("the read-only file should have stopped the rename");
        assert_eq!(
            std::fs::read_to_string(&shared).unwrap(),
            OTHER_TOOL,
            "the other tool's change must survive, with the rename taken out of it: {}",
            err.message
        );
        assert_eq!(std::fs::read_to_string(&blocked).unwrap(), "foo();\n");
        // And the foreign entry is still in the batch for the turn to close — the rename neither
        // committed it nor dropped it.
        assert!(!ctx.journal.lock().unwrap().is_empty(), "{}", err.message);
    }
}
