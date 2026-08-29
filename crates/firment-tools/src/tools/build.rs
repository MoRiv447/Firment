use super::shell::dangerous_reason;
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub struct Build;

/// Well-known build manifests checked in priority order. The detected command
/// runs inside the manifest's directory (see `detect_build_command`).
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

/// A detected build system: the command string plus the directory it must run
/// in. The command is executed via `run_command` (cmd /C on Windows) with
/// `work_dir` as its working directory — the directory is deliberately NOT
/// spliced into the command string, so workspace paths containing `%`, `^` or
/// spaces survive cmd's quoting quirks untouched.
pub(crate) struct DetectedBuild {
    pub command: String,
    pub work_dir: PathBuf,
    pub note: String,
}

/// Auto-detect the project's build system by scanning the workspace and up to
/// 2 levels of subdirectories. Shallowest match wins. Shared with the hil
/// tool's build step.
pub(crate) fn detect_build_command(cwd: &Path) -> Option<DetectedBuild> {
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
                    inner.to_string()
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
                        // Plain interior quotes only: `"` cannot appear in a
                        // Windows file name, and cmd /C keeps quoted names
                        // intact — unlike `%%`/`^^` "doubling", which is
                        // batch-file syntax and corrupts a /C command line.
                        let command = format!("uv4 -j0 -b \"{name}\"");
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
    candidates.first().map(|(_, dir, command)| DetectedBuild {
        command: command.clone(),
        work_dir: dir.clone(),
        note: format!(
            "[auto-detected] build system in {}: {}",
            dir.display(),
            command
        ),
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
        let (command, work_dir, note) = match ctx.build_command.clone() {
            // A configured build_command runs in the workspace root, exactly
            // as it did before — only the auto-detected path gained a
            // separate work_dir.
            Some(cmd) => (cmd, ctx.cwd.clone(), String::new()),
            None => match detect_build_command(&ctx.cwd) {
                Some(d) => (d.command, d.work_dir, d.note),
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
            super::util::run_command(&command, &work_dir, timeout_ms, None, Some(&ctx.cancel))
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
        let d = detect_build_command(dir.path()).unwrap();
        assert_eq!(d.command, "pio run");
        assert_eq!(d.work_dir, dir.path());
        assert!(d.note.contains("auto-detected"), "got: {}", d.note);
    }

    #[test]
    fn detects_platformio_in_subdirectory() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("cubemx")).unwrap();
        std::fs::write(dir.path().join("cubemx").join("platformio.ini"), "[env]\n").unwrap();
        let d = detect_build_command(dir.path()).unwrap();
        assert_eq!(d.command, "pio run", "no cd prefix: {:?}", d.command);
        assert!(
            d.work_dir.ends_with("cubemx"),
            "command must run in the manifest dir: {:?}",
            d.work_dir
        );
    }

    #[test]
    fn special_chars_in_subdir_stay_out_of_command() {
        // Regression for the cmd.exe quoting bug: a directory named with
        // cmd-hostile characters must not leak into the command string (the
        // old `cd <dir> &&` prefix corrupted `%`/`^` via batch-style doubling).
        let dir = tempdir().unwrap();
        let weird = dir.path().join("rev^2_100%");
        std::fs::create_dir_all(&weird).unwrap();
        std::fs::write(weird.join("platformio.ini"), "[env]\n").unwrap();
        let d = detect_build_command(dir.path()).unwrap();
        assert_eq!(d.command, "pio run", "got: {:?}", d.command);
        assert!(d.work_dir.ends_with("rev^2_100%"));
    }

    #[test]
    fn detects_makefile() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("Makefile"), "all:\n").unwrap();
        let d = detect_build_command(dir.path()).unwrap();
        assert_eq!(d.command, "make");
        assert_eq!(d.work_dir, dir.path());
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
        let d = detect_build_command(dir.path()).unwrap();
        assert_eq!(d.command, "cmake --build build");
    }

    #[test]
    fn cmake_configures_when_build_dir_missing() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("CMakeLists.txt"),
            "cmake_minimum_required(VERSION 3)\n",
        )
        .unwrap();
        let d = detect_build_command(dir.path()).unwrap();
        assert_eq!(d.command, "cmake -B build && cmake --build build");
    }

    #[test]
    fn detects_keil_project() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("fw.uvprojx"), "{}").unwrap();
        let d = detect_build_command(dir.path()).unwrap();
        assert!(d.command.contains("uv4 -j0 -b"), "got: {}", d.command);
        assert!(d.command.contains("\"fw.uvprojx\""), "got: {}", d.command);
    }

    #[test]
    fn shallowest_match_wins() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("platformio.ini"), "[env]\n").unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("Makefile"), "all:\n").unwrap();
        let d = detect_build_command(dir.path()).unwrap();
        assert_eq!(d.command, "pio run", "workspace root must win over subdir");
        assert_eq!(d.work_dir, dir.path());
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
