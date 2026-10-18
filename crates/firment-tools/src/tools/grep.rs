use super::util::{read_text, rel_str, resolve_within};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use globset::Glob;
use ignore::WalkBuilder;
use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};

pub struct Grep;

#[async_trait]
impl Tool for Grep {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn description(&self) -> &'static str {
        "Search file contents with a regular expression. `path` may be a directory (walked \
         recursively, hidden entries skipped unless include_hidden) or a single file — use the \
         file form to find a specific line in one authoritative file (a BSP driver, a generated \
         init, an index) instead of guessing a window with read_file. Optionally filter by glob \
         and line count; results are `path:line:text`."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string"},
                "path": {"type": "string", "default": "."},
                "glob": {"type": "string", "description": "Optional file glob filter, e.g. **/*.c"},
                "case_sensitive": {"type": "boolean", "default": false},
                "max_results": {"type": "integer", "minimum": 1, "default": 100},
                "include_hidden": {"type": "boolean", "default": false}
            },
            "required": ["pattern"]
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let pattern = args
            .get("pattern")
            .and_then(|p| p.as_str())
            .ok_or_else(|| ToolError::new("missing 'pattern'"))?;
        let path = args.get("path").and_then(|p| p.as_str()).unwrap_or(".");
        let case_sensitive = args
            .get("case_sensitive")
            .and_then(|c| c.as_bool())
            .unwrap_or(false);
        let max_results = args
            .get("max_results")
            .and_then(|m| m.as_u64())
            .unwrap_or(100) as usize;
        let include_hidden = args
            .get("include_hidden")
            .and_then(|h| h.as_bool())
            .unwrap_or(false);
        let glob_filter = args
            .get("glob")
            .and_then(|g| g.as_str())
            .map(Glob::new)
            .transpose()
            .map_err(|e| ToolError::new(format!("[InvalidInput] bad glob pattern: {e}")))?
            .map(|g| g.compile_matcher());
        let regex = RegexBuilder::new(pattern)
            .case_insensitive(!case_sensitive)
            .build()
            .map_err(|e| ToolError::new(format!("[InvalidInput] bad regex: {e}")))?;
        let resolved =
            resolve_within(&ctx.cwd, path, &ctx.allowed_roots).map_err(ToolError::new)?;
        if !resolved.is_dir() && !resolved.is_file() {
            return Err(ToolError::new(format!(
                "[NotFound] {} does not exist",
                resolved.display()
            )));
        }

        let mut out = Vec::new();
        if resolved.is_file() {
            // **搜索根可以是一个文件。** 基准里这条失败 20 次、8 道题全中，而且失败对象
            // 恰好是权威资料本身（BSP 的 `.c`/`.h`、`.ioc` 旁边的 `.cproject`、知识库的
            // `vendor-index.toml`）：模型想"在这个文件里找那一行"，工具回它
            // "is not a directory"，它就只能改用 read_file 的固定窗口去猜 —— 极性那种
            // 只差 5 行的证据就是这么漏掉的。目录语义不是唯一有价值的语义。
            let rel = rel_str(&ctx.cwd, &resolved);
            let wanted = glob_filter
                .as_ref()
                .is_none_or(|matcher| matcher.is_match(&rel));
            if wanted && let Ok(content) = read_text(&resolved) {
                collect_matches(&regex, &rel, &content, max_results, &mut out);
            }
        } else {
            for entry in WalkBuilder::new(&resolved).hidden(!include_hidden).build() {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    continue;
                }
                let rel = rel_str(&resolved, entry.path());
                if let Some(matcher) = &glob_filter
                    && !matcher.is_match(&rel)
                {
                    continue;
                }
                let Ok(content) = read_text(entry.path()) else {
                    continue;
                };
                collect_matches(&regex, &rel, &content, max_results, &mut out);
                if out.len() >= max_results {
                    break;
                }
            }
        }
        if out.len() >= max_results {
            out.push(format!("... stopped at {max_results} results"));
        }
        // 命中定义 ≠ 读到实现。2026-10-18 基准里，agent 靠一次 grep 拿到
        // `BSP_PB_Init` 的签名就下了"按键低有效"的结论，而 `Pull = GPIO_PULLDOWN`
        // 在同一个函数的第 11 行。这里不替它读，只提醒"这几行是定义"。
        let defs = out.iter().filter(|l| looks_like_definition(l)).count();
        if defs > 0 {
            out.push(format!(
                "hint: {defs} of these lines define something — a hit is not a read. Open the \
                 definition body before concluding: a signature or a macro name does not tell \
                 you the value, the pull or the edge."
            ));
        }
        Ok(ToolOutput {
            text: out.join("\n"),
        })
    }
}

/// 这行像不像"定义"（而不是使用点）。只做粗判：多一个提醒，不会改变匹配结果。
fn looks_like_definition(line: &str) -> bool {
    let body = line.splitn(3, ':').nth(2).unwrap_or(line).trim();
    let keyword = [
        "fn ",
        "def ",
        "function ",
        "struct ",
        "enum ",
        "impl ",
        "trait ",
        "#define",
        "typedef",
    ]
    .iter()
    .any(|k| body.contains(k));
    if keyword || body.ends_with('{') {
        return true;
    }
    // C 签名：`int32_t BSP_PB_Init(Button_TypeDef Button)`。**调用**以分号结尾，
    // 用"有括号 + 不以分号结尾"把两者分开。
    body.contains('(') && !body.ends_with(';')
}

/// 一个文件里的全部匹配行，`path:line:text` 形式追加进 `out`（到 `max_results` 为止）。
fn collect_matches(
    regex: &Regex,
    rel: &str,
    content: &str,
    max_results: usize,
    out: &mut Vec<String>,
) {
    for (idx, line) in content.lines().enumerate() {
        if regex.is_match(line) {
            out.push(format!(
                "{}:{}:{}",
                rel,
                idx + 1,
                super::util::truncate(line, 500)
            ));
            if out.len() >= max_results {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_lines_are_recognised_so_the_hint_can_fire() {
        assert!(looks_like_definition(
            "a/b.c:12:int32_t BSP_PB_Init(Button_TypeDef B)"
        ));
        assert!(looks_like_definition(
            "a/b.c:12:#define BTN_PIN GPIO_PIN_13"
        ));
        assert!(looks_like_definition("a/b.c:12:static void init(void) {"));
        assert!(!looks_like_definition(
            "a/b.c:12:  gpio.Pull = GPIO_PULLDOWN;"
        ));
    }
    use firment_core::ToolContext;
    use std::path::Path;

    async fn grep(dir: &Path, args: Value) -> Result<ToolOutput, ToolError> {
        Grep.run(args, &ToolContext::with_cwd(dir.to_path_buf()))
            .await
    }

    /// 搜索根是文件时必须能用：基准里这一条失败 20 次，失败对象正是权威资料本身，
    /// 模型退回用 read_file 的固定窗口去猜，极性那种只差几行的证据就漏了。
    #[tokio::test]
    async fn a_file_path_is_a_valid_search_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("driver.c"),
            "line one\n  .Pull = GPIO_PULLDOWN\nline three\n",
        )
        .unwrap();
        let out = grep(
            dir.path(),
            json!({"pattern": "PULLDOWN", "path": "driver.c"}),
        )
        .await
        .expect("grep must accept a file as its search root");
        assert!(out.text.contains("driver.c:2:"), "got: {}", out.text);
    }

    #[tokio::test]
    async fn a_directory_root_still_walks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/a.c"), "PIN_13 here\n").unwrap();
        std::fs::write(dir.path().join("b.c"), "no match\n").unwrap();
        let out = grep(dir.path(), json!({"pattern": "PIN_13"}))
            .await
            .unwrap();
        assert!(out.text.contains("sub/a.c:1:"), "got: {}", out.text);
    }

    /// 不存在的路径要说"找不到"，不能再把"它不是目录"当成拒绝理由 ——
    /// 那句错误信息把模型引向"换个目录再试"，而真相是"这个文件不存在"。
    #[tokio::test]
    async fn a_missing_path_says_it_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let err = grep(dir.path(), json!({"pattern": "x", "path": "nope.c"}))
            .await
            .expect_err("absent path must fail");
        assert!(
            err.message.contains("does not exist"),
            "got: {}",
            err.message
        );
    }
}
