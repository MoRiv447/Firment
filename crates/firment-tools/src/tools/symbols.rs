use super::util::{read_text, rel_str, resolve_within, truncate};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use ignore::WalkBuilder;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub struct Symbols {
    cache: Mutex<HashMap<PathBuf, (Instant, Vec<TagEntry>)>>,
}

impl Symbols {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for Symbols {
    fn default() -> Self {
        Self::new()
    }
}

/// (extensions, kind, pattern) — ctags-level, line-based definitions.
const PATTERNS: &[(&str, &str, &str)] = &[
    (
        "rs",
        "fn",
        r#"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+|unsafe\s+|extern\s*"[^"]*"\s+)*fn\s+(?P<name>[A-Za-z_]\w*)"#,
    ),
    (
        "rs",
        "type",
        r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|trait|union|mod|type)\s+(?P<name>[A-Za-z_]\w*)",
    ),
    (
        "rs",
        "const",
        r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+(?P<name>[A-Za-z_]\w*)",
    ),
    (
        "c|cpp|h|hpp|cc|cxx|hh",
        "type",
        r"^\s*(?:typedef\s+)?(?:static\s+|inline\s+|extern\s+|const\s+|volatile\s+|unsigned\s+|signed\s+|virtual\s+)*(?:struct|class|enum|union)\s+(?P<name>[A-Za-z_]\w*)",
    ),
    (
        "c|cpp|h|hpp|cc|cxx|hh",
        "macro",
        r"^\s*#\s*define\s+(?P<name>[A-Za-z_]\w*)",
    ),
    (
        "c|cpp|h|hpp|cc|cxx|hh",
        "fn",
        r"^\s*(?:static\s+|inline\s+|extern\s+|const\s+|volatile\s+|virtual\s+|unsigned\s+|signed\s+)*[A-Za-z_][\w:<>,*& ]*?\b(?P<name>[A-Za-z_]\w*)\s*\([^;{}]*\)\s*\{",
    ),
    (
        "py",
        "def",
        r"^\s*(?:async\s+)?def\s+(?P<name>[A-Za-z_]\w*)",
    ),
    ("py", "class", r"^\s*class\s+(?P<name>[A-Za-z_]\w*)"),
    (
        "js|ts|jsx|tsx|mjs|cjs",
        "function",
        r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+(?P<name>[A-Za-z_$][\w$]*)",
    ),
    (
        "js|ts|jsx|tsx|mjs|cjs",
        "class",
        r"^\s*(?:export\s+)?class\s+(?P<name>[A-Za-z_$][\w$]*)",
    ),
    (
        "js|ts|jsx|tsx|mjs|cjs",
        "const fn",
        r"^\s*(?:export\s+)?const\s+(?P<name>[A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?(?:\([^)]*\)|[A-Za-z_$][\w$]*)\s*=>",
    ),
    (
        "go",
        "func",
        r"^\s*func\s+(?:\([^)]*\)\s*)?(?P<name>[A-Za-z_]\w*)",
    ),
    ("go", "type", r"^\s*type\s+(?P<name>[A-Za-z_]\w*)"),
    (
        "java",
        "type",
        r"^\s*(?:(?:public|private|protected|static|final|abstract|native|synchronized)\s+)*(?:class|interface|enum)\s+(?P<name>[A-Za-z_]\w*)",
    ),
    (
        "java",
        "method",
        r"^\s*(?:(?:public|private|protected|static|final|abstract|synchronized)\s+)*[\w<>,.?\[\] ]+\s+(?P<name>[A-Za-z_]\w*)\s*\(",
    ),
];

fn compiled_patterns() -> &'static Vec<(String, String, Regex)> {
    static CACHE: OnceLock<Vec<(String, String, Regex)>> = OnceLock::new();
    CACHE.get_or_init(|| {
        PATTERNS
            .iter()
            .map(|(exts, kind, pattern)| {
                (
                    exts.to_string(),
                    kind.to_string(),
                    Regex::new(pattern).expect("built-in symbols pattern must compile"),
                )
            })
            .collect()
    })
}

#[async_trait]
impl Tool for Symbols {
    fn name(&self) -> &'static str {
        "symbols"
    }

    fn description(&self) -> &'static str {
        "Find symbol definitions (ctags-level) or references in the workspace. Supports C/C++, Rust, Python, JS/TS, Go, Java."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "references": {"type": "boolean", "default": false, "description": "find all occurrences instead of definitions"},
                "path": {"type": "string", "default": "."},
                "max_results": {"type": "integer", "minimum": 1, "default": 200}
            },
            "required": ["query"]
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let query = args
            .get("query")
            .and_then(|q| q.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing 'query'"))?;
        let references = args
            .get("references")
            .and_then(|r| r.as_bool())
            .unwrap_or(false);
        let path = args.get("path").and_then(|p| p.as_str()).unwrap_or(".");
        let max_results = args
            .get("max_results")
            .and_then(|m| m.as_u64())
            .unwrap_or(200) as usize;
        let resolved =
            resolve_within(&ctx.cwd, path, &ctx.allowed_roots).map_err(ToolError::new)?;
        if !resolved.is_dir() {
            return Err(ToolError::new(format!(
                "[NotFound] {} is not a directory",
                resolved.display()
            )));
        }

        let backend = ctx.symbols_backend.as_deref().unwrap_or("auto");
        let use_ctags =
            !references && (backend == "ctags" || (backend == "auto" && ctags_available()));
        if use_ctags && let Some(entries) = self.ctags_entries(&resolved) {
            let lower = query.to_lowercase();
            let mut out = Vec::new();
            for entry in entries
                .iter()
                .filter(|e| e.name.to_lowercase().contains(&lower))
            {
                out.push(format!(
                    "{}:{}: {} {} — {}",
                    rel_str(&resolved, &entry.path),
                    entry.line,
                    entry.kind,
                    entry.name,
                    truncate(&entry.snippet, 100)
                ));
                if out.len() >= max_results {
                    out.push(format!("... stopped at {max_results} results"));
                    break;
                }
            }
            if out.is_empty() {
                return Ok(ToolOutput {
                    text: format!("no symbols found for {query:?}"),
                });
            }
            return Ok(ToolOutput {
                text: out.join("\n"),
            });
        }

        let mut out = Vec::new();
        for entry in WalkBuilder::new(&resolved).hidden(true).build() {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) else {
                continue;
            };
            let Ok(content) = read_text(entry.path()) else {
                continue;
            };
            if references {
                scan_references(
                    &content,
                    query,
                    &resolved,
                    entry.path(),
                    &mut out,
                    max_results,
                );
            } else {
                scan_definitions(
                    ext,
                    &content,
                    query,
                    &resolved,
                    entry.path(),
                    &mut out,
                    max_results,
                );
            }
            if out.len() >= max_results {
                break;
            }
        }
        if out.len() >= max_results {
            out.push(format!("... stopped at {max_results} results"));
        }
        if out.is_empty() {
            return Ok(ToolOutput {
                text: format!("no symbols found for {query:?}"),
            });
        }
        Ok(ToolOutput {
            text: out.join("\n"),
        })
    }
}

fn ctags_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        std::process::Command::new("ctags")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

#[derive(Clone)]
struct TagEntry {
    path: PathBuf,
    line: usize,
    kind: String,
    name: String,
    snippet: String,
}

impl Symbols {
    /// Run universal-ctags in JSON mode over the root, with a 60s cache.
    /// Returns None when ctags is missing or fails (caller falls back).
    fn ctags_entries(&self, root: &Path) -> Option<Vec<TagEntry>> {
        if let Some((at, entries)) = self.cache.lock().ok()?.get(root)
            && at.elapsed() < Duration::from_secs(60)
        {
            return Some(entries.clone());
        }
        let output = std::process::Command::new("ctags")
            .args(["-R", "--output-format=json", "--fields=+n"])
            .arg("--languages=C,C++,Rust,Python,JavaScript,TypeScript,Go,Java")
            .arg(".")
            .current_dir(root)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let mut entries = Vec::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let Some(name) = value.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            let Some(path) = value.get("path").and_then(|p| p.as_str()) else {
                continue;
            };
            let line_no = value.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as usize;
            let kind = value
                .get("kind")
                .and_then(|k| k.as_str())
                .unwrap_or("symbol")
                .to_string();
            let snippet = value
                .get("pattern")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string();
            let rel = Path::new(path);
            let rel = rel.strip_prefix(".").unwrap_or(rel);
            entries.push(TagEntry {
                path: root.join(rel),
                line: line_no,
                kind,
                name: name.to_string(),
                snippet,
            });
        }
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(root.to_path_buf(), (Instant::now(), entries.clone()));
        }
        Some(entries)
    }
}

fn scan_definitions(
    ext: &str,
    content: &str,
    query: &str,
    root: &Path,
    file: &Path,
    out: &mut Vec<String>,
    max: usize,
) {
    let lower = query.to_lowercase();
    for (exts, kind, regex) in compiled_patterns() {
        if !exts.split('|').any(|e| e == ext) {
            continue;
        }
        for (idx, line) in content.lines().enumerate() {
            if out.len() >= max {
                return;
            }
            if let Some(caps) = regex.captures(line)
                && let Some(name) = caps.name("name")
                && name.as_str().to_lowercase().contains(&lower)
            {
                out.push(format!(
                    "{}:{}: {kind} {} — {}",
                    rel_str(root, file),
                    idx + 1,
                    name.as_str(),
                    truncate(line.trim(), 100)
                ));
            }
        }
    }
}

fn scan_references(
    content: &str,
    query: &str,
    root: &Path,
    file: &Path,
    out: &mut Vec<String>,
    max: usize,
) {
    let Ok(regex) = Regex::new(&format!(r"\b{}\b", regex::escape(query))) else {
        return;
    };
    for (idx, line) in content.lines().enumerate() {
        if out.len() >= max {
            return;
        }
        if regex.is_match(line) {
            out.push(format!(
                "{}:{}: {}",
                rel_str(root, file),
                idx + 1,
                truncate(line.trim(), 160)
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal};
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    fn ctx(dir: &Path) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(EditJournal::new(dir.join("undo")))),
            verify_command: None,
            symbols_backend: None,
            build_command: None,
            default_chip: None,
            monitor_port: None,
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
        }
    }

    #[tokio::test]
    async fn finds_definitions_across_languages() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "pub fn greet(name: &str) -> String { format!(\"hi {name}\") }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/lib.c"),
            "int add(int a, int b) { return a + b; }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/use.py"),
            "def greet():\n    return 'hi'\n",
        )
        .unwrap();

        let tool = Symbols::new();
        let out = tool
            .run(json!({"query": "greet"}), &ctx(dir.path()))
            .await
            .unwrap();
        assert!(out.text.contains("greet"), "got: {}", out.text);

        let out = tool
            .run(json!({"query": "add"}), &ctx(dir.path()))
            .await
            .unwrap();
        assert!(out.text.contains("add"), "got: {}", out.text);
    }

    #[tokio::test]
    async fn ctags_backend_finds_definitions_when_available() {
        if !ctags_available() {
            return;
        }
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.rs"),
            "pub fn greet(name: &str) -> String { name.into() }\n",
        )
        .unwrap();
        let mut tool_ctx = ctx(dir.path());
        tool_ctx.symbols_backend = Some("ctags".to_string());
        let tool = Symbols::new();
        let out = tool
            .run(json!({"query": "greet"}), &tool_ctx)
            .await
            .unwrap();
        assert!(out.text.contains("greet"), "got: {}", out.text);
        assert!(!out.text.contains("no symbols"), "got: {}", out.text);
    }

    #[tokio::test]
    async fn finds_references() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("use.c"),
            "int call_add(void) { return add(1, 2); }\n",
        )
        .unwrap();
        let tool = Symbols::new();
        let out = tool
            .run(
                json!({"query": "add", "references": true}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(out.text.contains("call_add"), "got: {}", out.text);
    }
}
