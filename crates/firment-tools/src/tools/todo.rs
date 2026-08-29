use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

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

fn load_todos(path: &PathBuf) -> Vec<TodoItem> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_todos(path: &PathBuf, todos: &[TodoItem]) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| ToolError::new(format!("[Io] cannot create todo dir: {e}")))?;
    }
    // Atomic write (tmp + rename) so an interrupted save never leaves a
    // truncated/corrupt todos.json.
    let tmp = path.with_extension("json.tmp");
    fs::write(
        &tmp,
        serde_json::to_string_pretty(todos).unwrap_or_default(),
    )
    .map_err(|e| ToolError::new(format!("[Io] cannot write todos: {e}")))?;
    fs::rename(&tmp, path).map_err(|e| ToolError::new(format!("[Io] cannot write todos: {e}")))
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
        let mut todos = load_todos(&path);
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
            "clear" => {
                todos.clear();
                save_todos(&path, &todos)?;
                Ok(ToolOutput {
                    text: "Cleared the todo list".to_string(),
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
            allowed_roots: Vec::new(),
            cancel: firment_core::Cancellable::new(),
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
}
