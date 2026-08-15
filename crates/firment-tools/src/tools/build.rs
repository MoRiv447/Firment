use super::shell::dangerous_reason;
use super::util::shell_quote;
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub struct Build;

/// Well-known build manifests checked in priority order. The inner command
/// runs inside the manifest's directory (see `cmd_in`).
const BUILD_MANIFESTS: &[(&str, &str)] = &[
    ("platformio.ini", "pio run"),
    ("Makefile", "make"),
    ("makefile", "make"),
    ("CMakeLists.txt", ""), // special-cased in detect_build_command
];

/// Directories never scanned for build manifests (build artifacts, VCS,
/// hidden dirs).
fn is_skipped_dir(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "target" | "node_modules" | ".pio" | "build" | "obj" | "Debug" | "Release" | "dist"
        )
}

/// Prefix an inner command with `cd <relative-dir> &&` when the build manifest
/// lives in a subdirectory of the workspace; at the workspace root no cd is
/// needed (the shell runner already uses the workspace as its cwd).
fn cmd_in(dir: &Path, cwd: &Path, inner: &str) -> String {
    if dir == cwd {
        inner.to_string()
    } else {
        let rel = dir.strip_prefix(cwd).unwrap_or(dir);
        format!("cd {} && {inner}", shell_quote(&rel.to_string_lossy()))
    }
}

/// Auto-detect the project's build system by scanning the workspace and up to
/// 2 levels of subdirectories. Returns `(command, note)` or `None` when no
/// known build system is found. Shallowest match wins.
fn detect_build_command(cwd: &Path) -> Option<(String, String)> {
    let mut candidates: Vec<(usize, PathBuf, String)> = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(cwd.to_path_buf(), 0)];
    let mut visited: Vec<PathBuf> = Vec::new();

    while let Some((dir, depth)) = stack.pop() {
        if depth > 2 || visited.contains(&dir) {
            continue;
        }
        visited.push(dir.clone());

        // Highest-priority manifest in this directory wins.
        let mut found = false;
        for (manifest, inner) in BUILD_MANIFESTS {
            if dir.join(manifest).is_file() {
                let command = if *manifest == "CMakeLists.txt" {
                    if dir.join("build").is_dir() {
                        "cmake --build build".to_string()
                    } else {
                        "cmake -B build && cmake --build build".to_string()
                    }
                } else {
                    cmd_in(&dir, cwd, inner)
                };
                candidates.push((depth, dir.clone(), command));
                found = true;
                break;
            }
        }
        if !found {
            // Keil MDK project files (only when no other manifest was found here).
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.ends_with(".uvprojx") {
                        let command =
                            cmd_in(&dir, cwd, &format!("uv4 -j0 -b {}", shell_quote(&name)));
                        candidates.push((depth, dir.clone(), command));
                        found = true;
                        break;
                    }
                }
            }
        }
        if found {
            continue;
        }

        // Recurse into subdirectories.
        if depth < 2
            && let Ok(entries) = std::fs::read_dir(&dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir()
                    && let Some(name) = path.file_name().and_then(|n| n.to_str())
                    && !is_skipped_dir(name)
                {
                    stack.push((path, depth + 1));
                }
            }
        }
    }

    candidates.sort_by_key(|(depth, _, _)| *depth);
    candidates.first().map(|(_, dir, command)| {
        (
            command.clone(),
            format!(
                "[auto-detected] build system in {}: {}",
                dir.display(),
                command
            ),
        )
    })
}

#[async_trait]
impl Tool for Build {
    fn name(&self) -> &'static str {
        "build"
    }

    fn description(&self) -> &'static str {
        "Build the project — use THIS tool for building, never run pio/cmake/make/uv4 via the shell tool. Uses [tools] build_command if configured, otherwise auto-detects the build system in the workspace (and up to 2 levels of subdirectories): platformio.ini -> pio run, Makefile -> make, CMakeLists.txt -> cmake --build, *.uvprojx -> uv4, cd-ing into the manifest's directory automatically. A non-zero exit means the build failed."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "timeout_ms": {"type": "integer", "minimum": 1, "default": 600000}
            }
        })
    }

    fn approval(&self, _args: &Value) -> Option<String> {
        Some("run build command".to_string())
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let (command, note) = match ctx.build_command.clone() {
            Some(cmd) => (cmd, String::new()),
            None => match detect_build_command(&ctx.cwd) {
                Some((cmd, note)) => (cmd, note),
                None => {
                    return Err(ToolError::new(
                        "[InvalidInput] build tool is not configured and no build system was \
                         auto-detected (looked for platformio.ini / Makefile / CMakeLists.txt / \
                         *.uvprojx in the workspace and up to 2 levels of subdirectories). Set \
                         build_command in [tools] of config.toml, or create a project \
                         .firment.toml with build_command for this project.",
                    ));
                }
            },
        };
        if let Some(reason) = dangerous_reason(&command)
            && !ctx.allow_dangerous
        {
            return Err(ToolError::new(format!(
                "[Permission] build command was blocked by the dangerous-command guard \
                 ({reason}); refusing to run: {command}"
            )));
        }
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|t| t.as_u64())
            .unwrap_or(600_000);
        let (text, code) =
            super::util::run_command(&command, &ctx.cwd, timeout_ms, None, Some(&ctx.cancel))
                .await
                .map_err(ToolError::new)?;
        match code {
            Some(0) => Ok(ToolOutput {
                text: format!("{note}build passed (exit 0)\n{text}"),
            }),
            Some(code) => Err(ToolError::new(format!(
                "[CompileError] build failed (exit {code})\n{note}{text}"
            ))),
            None => Err(ToolError::new(format!(
                "[Timeout] build timed out\n{note}{text}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal};
    use serde_json::json;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    fn ctx(dir: &Path, build_command: Option<&str>) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(EditJournal::new(dir.join("undo")))),
            verify_command: None,
            symbols_backend: None,
            build_command: build_command.map(|s| s.to_string()),
            default_chip: None,
            monitor_port: None,
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        }
    }

    #[test]
    fn detects_platformio_at_workspace_root() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("platformio.ini"), "[env]\n").unwrap();
        let (cmd, note) = detect_build_command(dir.path()).unwrap();
        assert_eq!(cmd, "pio run");
        assert!(note.contains("auto-detected"), "got: {note}");
    }

    #[test]
    fn detects_platformio_in_subdirectory_with_cd() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("cubemx")).unwrap();
        std::fs::write(dir.path().join("cubemx").join("platformio.ini"), "[env]\n").unwrap();
        let (cmd, _) = detect_build_command(dir.path()).unwrap();
        assert!(cmd.contains("cd") && cmd.contains("cubemx"), "got: {cmd}");
        assert!(cmd.contains("pio run"), "got: {cmd}");
    }

    #[test]
    fn detects_makefile() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("Makefile"), "all:\n").unwrap();
        let (cmd, _) = detect_build_command(dir.path()).unwrap();
        assert_eq!(cmd, "make");
    }

    #[test]
    fn cmake_uses_existing_build_dir() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("CMakeLists.txt"),
            "cmake_minimum_required(VERSION 3)\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("build")).unwrap();
        let (cmd, _) = detect_build_command(dir.path()).unwrap();
        assert_eq!(cmd, "cmake --build build");
    }

    #[test]
    fn cmake_configures_when_build_dir_missing() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("CMakeLists.txt"),
            "cmake_minimum_required(VERSION 3)\n",
        )
        .unwrap();
        let (cmd, _) = detect_build_command(dir.path()).unwrap();
        assert_eq!(cmd, "cmake -B build && cmake --build build");
    }

    #[test]
    fn detects_keil_project() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("fw.uvprojx"), "{}").unwrap();
        let (cmd, _) = detect_build_command(dir.path()).unwrap();
        assert!(cmd.contains("uv4 -j0 -b"), "got: {cmd}");
    }

    #[test]
    fn shallowest_match_wins() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("platformio.ini"), "[env]\n").unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("Makefile"), "all:\n").unwrap();
        let (cmd, _) = detect_build_command(dir.path()).unwrap();
        assert_eq!(cmd, "pio run", "workspace root must win over subdir");
    }

    #[test]
    fn skips_heavy_dirs_when_scanning() {
        let dir = tempdir().unwrap();
        let heavy = dir.path().join("node_modules").join("pkg");
        std::fs::create_dir_all(&heavy).unwrap();
        std::fs::write(heavy.join("platformio.ini"), "[env]\n").unwrap();
        assert!(detect_build_command(dir.path()).is_none());
    }

    #[test]
    fn empty_dir_detects_nothing() {
        let dir = tempdir().unwrap();
        assert!(detect_build_command(dir.path()).is_none());
    }

    #[tokio::test]
    async fn unconfigured_and_undetected_build_is_an_error() {
        let dir = tempdir().unwrap();
        let err = Build
            .run(json!({}), &ctx(dir.path(), None))
            .await
            .unwrap_err();
        assert!(err.message.contains("auto-detected"), "got: {err}");
    }

    #[tokio::test]
    async fn passing_build_returns_success() {
        let cmd = if cfg!(windows) {
            "cmd /c echo ok"
        } else {
            "echo ok"
        };
        let dir = tempdir().unwrap();
        let out = Build
            .run(json!({}), &ctx(dir.path(), Some(cmd)))
            .await
            .unwrap();
        assert!(out.text.contains("build passed (exit 0)"));
    }

    #[tokio::test]
    async fn failing_build_returns_compile_error() {
        let cmd = if cfg!(windows) {
            "cmd /c exit 2"
        } else {
            "exit 2"
        };
        let dir = tempdir().unwrap();
        let err = Build
            .run(json!({}), &ctx(dir.path(), Some(cmd)))
            .await
            .unwrap_err();
        assert!(err.message.contains("[CompileError]"));
    }
}
