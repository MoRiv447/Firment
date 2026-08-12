use super::util::resolve_within;
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::collections::HashMap;

pub struct Shell;

/// Statically detectable shell metaprogramming / sensitive-access patterns.
/// These cannot be verified by a token blacklist alone — e.g. `d$(echo el)`
/// rebuilds `del` at expansion time — so any hit is flagged outright.
const METAPROGRAMMING_PATTERNS: &[(&str, &str)] = &[
    ("$(", "command substitution $()"),
    ("${", "parameter substitution ${}"),
    ("$[", "arithmetic expansion $[]"),
    ("<(", "process substitution <()"),
    (">(", "process substitution >()"),
    ("=(", "zsh process substitution =()"),
    ("IFS=", "IFS injection"),
    (
        "%",
        "cmd-style env expansion %VAR% (cannot be verified statically)",
    ),
    ("/proc/self/", "sensitive /proc access"),
    ("/etc/passwd", "sensitive file access"),
    ("/etc/shadow", "sensitive file access"),
    ("~/.ssh", "credential access"),
    ("id_rsa", "credential access"),
    ("id_ed25519", "credential access"),
    ("id_dsa", "credential access"),
    ("authorized_keys", "credential access"),
];

/// Returns a short reason when the shell command looks destructive. Used to
/// warn in interactive approval popups and to hard-block in one-shot mode
/// unless `--allow-dangerous` was passed.
pub fn dangerous_reason(command: &str) -> Option<&'static str> {
    let lower = command.to_lowercase();
    // Shell metaprogramming cannot be verified statically: `$(...)` and
    // backticks run arbitrary commands at expansion time, and `eval` /
    // `source` re-interpret text as code. A blacklist can be bypassed by
    // e.g. `d$(echo el) f.txt`, so flag these patterns outright.
    if lower.contains('`') {
        return Some("shell metaprogramming (backticks; cannot be verified statically)");
    }
    for (pattern, why) in METAPROGRAMMING_PATTERNS {
        if lower.contains(pattern) {
            return Some(why);
        }
    }
    let normalized = lower.replace('\\', "/").replace(['"', '\''], " ");
    let tokens: Vec<&str> = normalized
        .split([' ', '\t', '|', '&', ';', '>', '<', '\n', '\r'])
        .filter(|t| !t.is_empty())
        .collect();

    // Scan every token, not just the first: `cmd /c del ...`,
    // `cd . && del ...`, `powershell -Command Remove-Item ...`, and other
    // wrapper shapes must not slip past.
    for token in &tokens {
        let base = token
            .trim_start_matches(['.', '/', '\\'])
            .rsplit('/')
            .next()
            .unwrap_or(token);
        match base {
            "del" | "erase" | "rm" | "rmdir" | "rd" | "deltree" | "remove-item" | "ri"
            | "shred" | "unlink" => {
                return Some("delete/erase files or directories");
            }
            "eval" | "source" => {
                return Some("shell metaprogramming (eval/source re-interprets text as commands)");
            }
            "mv" | "move" | "move-item" | "mi" | "ren" | "rename" | "rename-item" | "rni" => {
                return Some("move/rename files (changes workspace layout)");
            }
            "format" | "diskpart" | "shutdown" | "taskkill" | "wipe" => {
                return Some("destructive system operation");
            }
            "reg" if tokens.contains(&"delete") => {
                return Some("delete registry keys");
            }
            "mkfs" | "mkfs.ext2" | "mkfs.ext3" | "mkfs.ext4" | "mkfs.xfs" | "mkfs.btrfs" => {
                return Some("format a filesystem");
            }
            _ => {}
        }
    }

    // Scripting-API deletions: `python -c "os.remove(...)"` / `Path.unlink()`
    // / `shutil.rmtree(...)` etc. Quoting was normalized above, so tokens
    // still carry the call shape.
    for token in &tokens {
        if token.contains("os.remove")
            || token.contains("os.unlink")
            || token.contains("shutil.rmtree")
            || token.contains("shutil.move")
            || token.contains("os.rename")
            || token.contains("os.replace")
            || token.contains("unlink(")
            || token.contains("rmtree(")
            || token.contains("remove(")
            || token.contains("rename(")
            || token.contains("move(")
        {
            return Some("delete/move via scripting API");
        }
    }

    let joined = tokens.join(" ");
    if joined.contains("git clean") {
        Some("git clean (discard untracked files)")
    } else if joined.contains("git reset") && joined.contains("--hard") {
        Some("git reset --hard (discard local changes)")
    } else if joined.contains("git push")
        && (joined.contains("--force") || joined.contains(" -f ") || joined.ends_with(" -f"))
    {
        Some("git force push (overwrite remote history)")
    } else {
        None
    }
}

#[async_trait]
impl Tool for Shell {
    fn name(&self) -> &'static str {
        "shell"
    }

    fn description(&self) -> &'static str {
        "Run a shell command in the workspace. Supports pipes and conditionals via the platform shell."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "cwd": {"type": "string", "description": "Working directory, defaults to workspace root"},
                "timeout_ms": {"type": "integer", "minimum": 1, "default": 120000},
                "env": {"type": "object", "additionalProperties": {"type": "string"}}
            },
            "required": ["command"]
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        let command = args.get("command").and_then(|c| c.as_str())?;
        Some(match dangerous_reason(command) {
            Some(reason) => format!("⚠ dangerous command ({reason}): {command}"),
            None => format!("run shell command: {command}"),
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let command = args
            .get("command")
            .and_then(|c| c.as_str())
            .ok_or_else(|| ToolError::new("missing 'command'"))?;
        if let Some(reason) = dangerous_reason(command)
            && !ctx.allow_dangerous
        {
            return Err(ToolError::new(format!(
                "[Permission] Dangerous command blocked by the safety guard ({reason}): \
                 {command}\n\
                 You are in one-shot/auto-approve mode, which disallows destructive or \
                 layout-changing operations by default.\n\
                 Use a safer operation instead; if it is truly required, ask the user to \
                 rerun with --allow-dangerous or confirm it in the interactive TUI.\n\
                 Before reporting, verify the actual workspace state with git status / \
                 list_dir; never claim an operation was fully blocked if it was not."
            )));
        }
        let cwd = match args.get("cwd").and_then(|c| c.as_str()) {
            // Enforce the workspace boundary like every other tool: a
            // relative cwd is resolved against the workspace root and must
            // stay inside it (or one of the allowed extra roots).
            Some(c) => resolve_within(&ctx.cwd, c, &ctx.allowed_roots).map_err(ToolError::new)?,
            None => ctx.cwd.clone(),
        };
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|t| t.as_u64())
            .unwrap_or(120_000);
        let env = args.get("env").and_then(|e| e.as_object()).map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect::<HashMap<String, String>>()
        });
        let (text, _code) =
            super::util::run_command(command, &cwd, timeout_ms, env.as_ref(), Some(&ctx.cancel))
                .await
                .map_err(ToolError::new)?;
        Ok(ToolOutput { text })
    }
}

#[cfg(test)]
mod tests {
    use super::dangerous_reason;
    use crate::tools::shell::Shell;
    use firment_core::{AutoApprove, Tool, ToolContext};
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    #[test]
    fn detects_destructive_commands() {
        for cmd in [
            "del todo.py test_todo.py",
            "del /q /f *.py",
            "rm -rf src",
            "rm todo.py",
            "Remove-Item -Recurse -Force .",
            "rd /s /q build",
            "git clean -fdx",
            "git reset --hard HEAD",
            "git push --force origin main",
            "format C:",
            "taskkill /f /im app.exe",
            "cmd /c del todo.py test_todo.py",
            "cd . && del todo.py",
            "powershell -Command Remove-Item -Recurse -Force .",
            "pwsh -c 'rm -rf src'",
            "echo y | del /q *.py",
            "python -c \"import os; os.remove('todo.py')\"",
            "python -c \"import shutil; shutil.rmtree('src')\"",
            "git rm todo.py",
            "shred -u todo.py",
            "mkdir _backup && move todo.py _backup\\todo.py",
            "move *.py _backup\\",
            "mv todo.py test_todo.py _backup/",
            "Move-Item -Path *.py -Destination _backup",
            "ren todo.py old.py",
            "git mv todo.py src/todo.py",
            "python -c \"import shutil; shutil.move('todo.py', '_backup/')\"",
            "d$(echo el) todo.py",
            "`rm -rf src`",
            "eval \"$(echo rm) -rf src\"",
            "source /tmp/evil.sh",
        ] {
            assert!(
                dangerous_reason(cmd).is_some(),
                "should detect dangerous: {cmd}"
            );
        }
    }

    #[test]
    fn metaprogramming_is_detected_even_when_blacklisted_words_are_split() {
        for cmd in [
            "d$(echo el) f.txt",
            "r$(echo m) -rf src",
            "e$(echo val) ls",
            "`rm -rf src`",
            "echo x `cat /etc/passwd`",
            "cat <(echo evil)",
            "cat /etc/passwd",
            "cat /etc/shadow",
            "cat ~/.ssh/id_rsa",
            "IFS=;rm -rf src",
        ] {
            assert!(
                dangerous_reason(cmd).is_some(),
                "should detect metaprogramming: {cmd}"
            );
        }
    }

    #[test]
    fn allows_safe_commands() {
        for cmd in [
            "python -m pytest test_todo.py -q",
            "git status",
            "git diff",
            "cargo test",
            "pip install pytest",
            "cat todo.py",
            "cargo remove serde",
            "python -c \"print('hello')\"",
            "Get-ChildItem *.py",
            "copy todo.py todo.bak",
            "mkdir build",
        ] {
            assert!(dangerous_reason(cmd).is_none(), "should allow safe: {cmd}");
        }
    }

    #[tokio::test]
    async fn hard_guard_blocks_destructive_command_in_one_shot() {
        let tool = Shell;
        let ctx = ToolContext {
            cwd: PathBuf::from("."),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(firment_core::EditJournal::new(PathBuf::from(
                ".",
            )))),
            verify_command: None,
            symbols_backend: None,
            build_command: None,
            default_chip: None,
            monitor_port: None,
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        };
        let result = tool
            .run(json!({"command": "del todo.py test_todo.py"}), &ctx)
            .await;
        let err = result.unwrap_err();
        assert!(err.message.contains("Dangerous command blocked"));
    }

    #[tokio::test]
    async fn cwd_outside_workspace_is_rejected() {
        let tool = Shell;
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolContext {
            cwd: dir.path().to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(firment_core::EditJournal::new(PathBuf::from(
                ".",
            )))),
            verify_command: None,
            symbols_backend: None,
            build_command: None,
            default_chip: None,
            monitor_port: None,
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        };
        let outside = dir
            .path()
            .parent()
            .and_then(|p| p.to_str())
            .map(|p| p.replace('\\', "/"))
            .unwrap_or_else(|| "..".to_string());
        let err = tool
            .run(json!({"command": "echo hi", "cwd": outside}), &ctx)
            .await
            .unwrap_err();
        assert!(err.message.contains("outside the workspace"), "got: {err}");
    }

    #[tokio::test]
    async fn approval_labels_dangerous_commands() {
        let tool = Shell;
        let reason = tool
            .approval(&json!({"command": "git clean -fdx"}))
            .unwrap();
        assert!(reason.contains("⚠ dangerous command"));
        let safe = tool.approval(&json!({"command": "git status"})).unwrap();
        assert!(!safe.contains("⚠"));
    }
}
