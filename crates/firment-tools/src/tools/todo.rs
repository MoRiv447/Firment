use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

pub struct Todo;

#[derive(Serialize, Deserialize, Clone)]
struct TodoItem {
    text: String,
    done: bool,
}

fn todos_path(ctx: &ToolContext) -> Result<PathBuf, ToolError> {
    let dir = ctx.session_dir.as_ref().ok_or_else(|| {
        ToolError::new(
            "[NoSession] no session directory in this context (direct tool run or tests)",
        )
    })?;
    Ok(dir.join("todos.json"))
}

/// The list as it sits on disk, or why it could not be read.
///
/// A file that exists and does not parse is an error rather than an empty list. Every writing op
/// puts the whole vector back, so reading damage as "no todos" and then answering `op=add` would
/// delete the items the file still holds — and `op=list` saying "the list is empty" would send
/// the model off to rebuild a list it never lost.
fn load_todos(path: &Path) -> Result<Vec<TodoItem>, ToolError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        // No file is not a failure: the first `op=add` of a session has nothing to read.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(ToolError::new(format!(
                "[Io] {} could not be read: {e}",
                path.display()
            )));
        }
    };
    if text.trim().is_empty() {
        // The signature of a write that died after truncating. `op=clear` is how to say "an empty
        // list is what I want", and it does not read first.
        return Err(ToolError::new(format!(
            "[Corrupt] {} is empty — the todo list was truncated, not cleared",
            path.display()
        )));
    }
    serde_json::from_str(&text)
        .map_err(|e| ToolError::new(format!("[Corrupt] {} does not parse: {e}", path.display())))
}

fn save_todos(path: &Path, todos: &[TodoItem]) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| ToolError::new(format!("[Io] cannot create todo dir: {e}")))?;
    }
    let text = serde_json::to_string_pretty(todos).map_err(|e| {
        ToolError::new(format!(
            "[Io] the todo list was not saved, and nothing on disk changed: {e}"
        ))
    })?;
    // tmp + rename with a name no other writer is using. The old code wrote one shared
    // `todos.json.tmp` and renamed it, so two todo calls in the same tool wave could rename each
    // other's temp file; and `unwrap_or_default()` on the serialise step would have saved an
    // empty list over a good one had it ever failed.
    firment_core::session::write_atomic(path, &text)
        .map_err(|e| ToolError::new(format!("[Io] cannot write todos: {e}")))
}

/// Resolve a `1`-based item number, falling back to an exact text match.
fn resolve_target(todos: &[TodoItem], target: &str) -> Option<usize> {
    if let Ok(n) = target.trim().parse::<usize>()
        && n >= 1
        && n <= todos.len()
    {
        return Some(n - 1);
    }
    todos.iter().position(|item| item.text == target)
}

#[async_trait]
impl Tool for Todo {
    fn name(&self) -> &'static str {
        "todo"
    }

    fn description(&self) -> &'static str {
        "Keep a session-scoped todo list (persists for the whole session, survives context compaction). Operations: list (op=list), add (op=add, text=...), mark done (op=done, text=item number or text), remove (op=rm, text=item number or text), clear (op=clear). Use it to track multi-step tasks and to hand off remaining steps."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "op": {"type": "string", "enum": ["list", "add", "done", "rm", "clear"], "description": "Operation to perform"},
                "text": {"type": "string", "description": "Item text (op=add) or item number/text to target (op=done / op=rm)"}
            },
            "required": ["op"]
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let op = args
            .get("op")
            .and_then(|o| o.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing op"))?;
        let path = todos_path(ctx)?;
        if op == "clear" {
            // Nothing is read first: writing an empty list cannot destroy what this call did not
            // look at, and this is the one command that recovers a truncated or corrupt file.
            save_todos(&path, &[])?;
            return Ok(ToolOutput {
                text: "Cleared the todo list".to_string(),
            });
        }
        let mut todos = load_todos(&path)?;
        let text = args
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        match op {
            "list" => {
                if todos.is_empty() {
                    return Ok(ToolOutput {
                        text: "Todo list is empty. Add items with op=add.".to_string(),
                    });
                }
                let mut out = String::from("Todo:");
                for (i, item) in todos.iter().enumerate() {
                    let mark = if item.done { "[x]" } else { "[ ]" };
                    out.push_str(&format!("\n{}. {mark} {}", i + 1, item.text));
                }
                let remaining = todos.iter().filter(|t| !t.done).count();
                out.push_str(&format!(
                    "\n({remaining} remaining / {} total)",
                    todos.len()
                ));
                Ok(ToolOutput { text: out })
            }
            "add" => {
                if text.is_empty() {
                    return Err(ToolError::new("[InvalidInput] op=add needs text"));
                }
                todos.push(TodoItem {
                    text: text.clone(),
                    done: false,
                });
                save_todos(&path, &todos)?;
                Ok(ToolOutput {
                    text: format!("Added todo #{n}: {text}", n = todos.len()),
                })
            }
            "done" => {
                if text.is_empty() {
                    return Err(ToolError::new(
                        "[InvalidInput] op=done needs text (item number or text)",
                    ));
                }
                let idx = resolve_target(&todos, &text).ok_or_else(|| {
                    ToolError::new(format!("[InvalidInput] no todo matches '{text}'"))
                })?;
                todos[idx].done = true;
                save_todos(&path, &todos)?;
                Ok(ToolOutput {
                    text: format!("Marked done: {}", todos[idx].text),
                })
            }
            "rm" => {
                if text.is_empty() {
                    return Err(ToolError::new(
                        "[InvalidInput] op=rm needs text (item number or text)",
                    ));
                }
                let idx = resolve_target(&todos, &text).ok_or_else(|| {
                    ToolError::new(format!("[InvalidInput] no todo matches '{text}'"))
                })?;
                let removed = todos.remove(idx).text;
                save_todos(&path, &todos)?;
                Ok(ToolOutput {
                    text: format!("Removed todo: {removed}"),
                })
            }
            other => Err(ToolError::new(format!(
                "[InvalidInput] unknown op '{other}' (list / add / done / rm / clear)"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    fn ctx(dir: &Path) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(EditJournal::new(dir.join("undo")))),
            ledger_path: None,
            verify_command: None,
            symbols_backend: None,
            build_command: None,
            default_chip: None,
            monitor_port: None,
            monitor_baud: 115_200,
            subagent: None,
            subagent_depth: 0,
            max_subagent_depth: 2,
            asker: None,
            device_log_dir: None,
            web_search_provider: None,
            web_search_api_key: None,
            session_dir: Some(dir.join("session")),
            providers: Vec::new(),
            la: None,
            attacker: None,
            allowed_roots: Vec::new(),
            cancel: firment_core::Cancellable::new(),
            ..ToolContext::default()
        }
    }

    fn tool() -> Todo {
        Todo
    }

    #[tokio::test]
    async fn add_list_done_rm_clear_round_trip() {
        let dir = tempdir().unwrap();
        let c = ctx(dir.path());

        let out = tool()
            .run(json!({"op": "add", "text": "write bootloader"}), &c)
            .await
            .unwrap();
        assert!(out.text.contains("1"), "got: {}", out.text);

        tool()
            .run(json!({"op": "add", "text": "test on hardware"}), &c)
            .await
            .unwrap();
        let out = tool().run(json!({"op": "list"}), &c).await.unwrap();
        assert!(out.text.contains("write bootloader"), "got: {}", out.text);
        assert!(out.text.contains("[ ]"), "got: {}", out.text);
        assert!(out.text.contains("2 remaining"), "got: {}", out.text);

        tool()
            .run(json!({"op": "done", "text": "1"}), &c)
            .await
            .unwrap();
        let out = tool().run(json!({"op": "list"}), &c).await.unwrap();
        assert!(
            out.text.contains("[x] write bootloader"),
            "got: {}",
            out.text
        );
        assert!(out.text.contains("1 remaining"), "got: {}", out.text);

        tool()
            .run(json!({"op": "rm", "text": "test on hardware"}), &c)
            .await
            .unwrap();
        let out = tool().run(json!({"op": "list"}), &c).await.unwrap();
        assert!(!out.text.contains("test on hardware"), "got: {}", out.text);

        tool().run(json!({"op": "clear"}), &c).await.unwrap();
        let out = tool().run(json!({"op": "list"}), &c).await.unwrap();
        assert!(out.text.contains("empty"), "got: {}", out.text);
    }

    #[tokio::test]
    async fn done_on_missing_item_is_an_error() {
        let dir = tempdir().unwrap();
        let err = tool()
            .run(json!({"op": "done", "text": "99"}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(err.message.contains("[InvalidInput]"), "got: {err}");
    }

    #[tokio::test]
    async fn unknown_op_is_an_error() {
        let dir = tempdir().unwrap();
        let err = tool()
            .run(json!({"op": "delete-all"}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(err.message.contains("[InvalidInput]"), "got: {err}");
    }

    #[tokio::test]
    async fn without_session_dir_is_an_error() {
        let dir = tempdir().unwrap();
        let mut c = ctx(dir.path());
        c.session_dir = None;
        let err = tool().run(json!({"op": "list"}), &c).await.unwrap_err();
        assert!(err.message.contains("[NoSession]"), "got: {err}");
    }

    #[tokio::test]
    async fn a_damaged_todo_file_is_never_answered_as_an_empty_list() {
        let dir = tempdir().unwrap();
        let c = ctx(dir.path());
        std::fs::create_dir_all(dir.path().join("session")).unwrap();
        let store = dir.path().join("session").join("todos.json");
        std::fs::write(&store, "[{\"text\":\"keep me\",\"done\":false}").unwrap();
        let before = std::fs::read_to_string(&store).unwrap();

        // The list has one item in it and the file is merely truncated. Every writing op saves
        // the whole vector, so an op that read this as "no todos" would replace the file with
        // whatever it was about to add.
        let err = tool()
            .run(json!({"op": "list"}), &c)
            .await
            .expect_err("a broken file is not an empty list");
        assert!(err.message.contains("does not parse"), "got: {err}");

        let err = tool()
            .run(json!({"op": "add", "text": "one more"}), &c)
            .await
            .expect_err("adding must not rewrite the list from a read that failed");
        assert!(err.message.contains("does not parse"), "got: {err}");
        assert_eq!(
            std::fs::read_to_string(&store).unwrap(),
            before,
            "the item still in the file must not have been overwritten"
        );

        // An empty file is the same story: it is what a truncated write leaves, not a cleared list.
        std::fs::write(&store, "").unwrap();
        let err = tool()
            .run(json!({"op": "list"}), &c)
            .await
            .expect_err("an empty file is a loss, not an empty list");
        assert!(err.message.contains("truncated"), "got: {err}");

        // `clear` is the way out, and it needs nothing readable to do its job.
        let out = tool().run(json!({"op": "clear"}), &c).await.unwrap();
        assert!(out.text.contains("Cleared"), "got: {}", out.text);
        let out = tool().run(json!({"op": "list"}), &c).await.unwrap();
        assert!(out.text.contains("empty"), "got: {}", out.text);
        assert_eq!(
            std::fs::read_dir(dir.path().join("session"))
                .unwrap()
                .count(),
            1,
            "no temp file left beside the saved one"
        );
    }
}
