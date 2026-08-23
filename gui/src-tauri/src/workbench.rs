//! Workbench commands: project state for the workbench panel
//! (`.firment/workbench.toml` + session tree + git status).
//! See docs/gui-workbench.md.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::state::Shared;
use firment_core::WorkbenchConfig;

#[derive(Debug, Clone, Serialize)]
pub struct GitStatusDto {
    pub branch: String,
    pub dirty_files: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkbenchConfigDto {
    pub project_name: String,
    pub mainline_session: String,
    pub toml_raw: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkbenchStateDto {
    /// Parsed `.firment/workbench.toml` (empty default when absent).
    pub config: WorkbenchConfigDto,
    pub git: Option<GitStatusDto>,
    /// cwd the state was computed against.
    pub root: String,
}

async fn run_git(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn git_status(cwd: &Path) -> Option<GitStatusDto> {
    let branch = run_git(cwd, &["branch", "--show-current"]).await?;
    let status = run_git(cwd, &["status", "--porcelain"]).await?;
    Some(GitStatusDto {
        branch: branch.trim().to_string(),
        dirty_files: status.lines().count() as u32,
    })
}

fn load_config(cwd: &Path) -> WorkbenchConfigDto {
    let cfg = WorkbenchConfig::load(cwd).unwrap_or_default();
    let toml_raw = std::fs::read_to_string(WorkbenchConfig::path_for(cwd)).unwrap_or_default();
    WorkbenchConfigDto {
        project_name: cfg.project.name.clone(),
        mainline_session: cfg.workbench.mainline_session.clone(),
        toml_raw,
    }
}

#[tauri::command]
pub async fn workbench_state(cwd: String) -> Result<WorkbenchStateDto, String> {
    let root = PathBuf::from(&cwd);
    if !root.is_dir() {
        return Err(format!("cwd does not exist: {}", root.display()));
    }
    let git = git_status(&root).await;
    Ok(WorkbenchStateDto {
        config: load_config(&root),
        git,
        root: root.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn workbench_set_mainline(
    shared: tauri::State<'_, Arc<Shared>>,
    cwd: String,
    session_id: String,
) -> Result<(), String> {
    // The mainline must point at a real session: setting it to a deleted or
    // typo'd id would leave the workbench permanently "(unset)".
    shared
        .store
        .lock()
        .unwrap()
        .load(&session_id)
        .map_err(|e| format!("cannot set mainline: {e}"))?;
    let mut cfg = WorkbenchConfig::load(Path::new(&cwd))?;
    cfg.workbench.mainline_session = session_id;
    cfg.save(Path::new(&cwd))
}

/// Create a workbench branch session under `parent_id`. Returns the new
/// session id (also linked via parent in the session store).
#[tauri::command]
pub async fn workbench_branch_create(
    shared: tauri::State<'_, Arc<Shared>>,
    parent_id: String,
    title: String,
) -> Result<String, String> {
    let store = shared.store.lock().unwrap().clone();
    let branch = store
        .create_branch(&parent_id, &title)
        .map_err(|e| e.to_string())?;

    // Register it in the project's workbench.toml when the parent's cwd has
    // one (or create the file fresh).
    let cwd = branch.cwd.clone();
    let mut cfg = WorkbenchConfig::load(Path::new(&cwd)).unwrap_or_default();
    if cfg.project.name.is_empty() {
        cfg.project.name = cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    let short: String = branch.id.chars().take(8).collect();
    cfg.branch.insert(
        short,
        firment_core::workbench::BranchEntry {
            parent: "mainline".to_string(),
            title: title.clone(),
            status: "open".to_string(),
            created_at: branch.created_at,
            ..Default::default()
        },
    );
    cfg.save(Path::new(&cwd))?;

    Ok(branch.id)
}

// ---------- W1d: ELF budget card / verification badges / change timeline ----------

#[derive(Debug, Clone, Serialize)]
pub struct ElfCardDto {
    pub file: String,
    pub flash_bytes: u64,
    pub ram_bytes: u64,
    pub functions: usize,
    /// Gate thresholds from config [tools.elf], when configured.
    pub gate: Option<GateThresholdsDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GateThresholdsDto {
    pub stack_threshold: u32,
    pub flash_threshold_kib: u64,
    pub ram_threshold_kib: u64,
    pub strict: bool,
}

/// ELF stats for the budget card. `elf` overrides; otherwise the newest match
/// of the configured [tools.elf] glob inside `cwd` is used.
#[tauri::command]
pub async fn workbench_elf(
    shared: tauri::State<'_, Arc<Shared>>,
    cwd: String,
    elf: Option<String>,
) -> Result<ElfCardDto, String> {
    let root = PathBuf::from(&cwd);
    let resolved = match elf {
        Some(p) if !p.trim().is_empty() => {
            let p = PathBuf::from(p.trim());
            if p.is_absolute() {
                p
            } else {
                root.join(p)
            }
        }
        _ => {
            let glob = {
                let config = shared.config.lock().unwrap();
                config
                    .tools
                    .elf
                    .as_ref()
                    .and_then(|e| (!e.glob.is_empty()).then(|| e.glob.clone()))
            }
            .ok_or_else(|| {
                "no ELF configured: pass elf=<path> or set [tools.elf] glob in config.toml"
                    .to_string()
            })?;
            firment_core::agent::newest_elf_match(&root, &glob).ok_or_else(|| {
                format!(
                    "no file matches the configured ELF glob ({glob}) under {}",
                    root.display()
                )
            })?
        }
    };

    // Gate thresholds for context (they are DELTA thresholds, shown as
    // reference next to the absolute numbers).
    let gate = {
        let config = shared.config.lock().unwrap();
        config.tools.elf.as_ref().map(|e| GateThresholdsDto {
            stack_threshold: e.stack_threshold,
            flash_threshold_kib: e.flash_threshold_kib,
            ram_threshold_kib: e.ram_threshold_kib,
            strict: e.strict,
        })
    };

    let file_display = resolved.to_string_lossy().into_owned();
    let stats = tokio::task::spawn_blocking(move || firment_tools::analyze_elf_file(&resolved))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("[InvalidInput] {e}"))?;

    Ok(ElfCardDto {
        file: file_display,
        flash_bytes: stats.0,
        ram_bytes: stats.1,
        functions: stats.2,
        gate,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct QualityItemDto {
    pub tool: String,
    pub ok: bool,
    pub snippet: String,
}

/// Last build/verify/hil/flash/run outcome per tool, derived from the
/// session transcript (most recent tool result per name wins).
#[tauri::command]
pub async fn workbench_quality(
    shared: tauri::State<'_, Arc<Shared>>,
    session_id: String,
) -> Result<Vec<QualityItemDto>, String> {
    let store = shared.store.lock().unwrap().clone();
    let session = store.load(&session_id).map_err(|e| e.to_string())?;
    const WATCHED: [&str; 5] = ["build", "verify", "hil", "flash", "run"];
    const FAIL_TAGS: [&str; 6] = [
        "[Io]",
        "[Timeout]",
        "[InvalidInput]",
        "[Permission]",
        "[NotFound]",
        "FAILED",
    ];
    let mut last: std::collections::BTreeMap<String, (bool, String)> =
        std::collections::BTreeMap::new();
    for msg in session.messages.iter().rev() {
        // edition-2021 crate: no let-chains, so nest the guards.
        if let firment_core::ChatMessage::Tool { name, content, .. } = msg {
            if WATCHED.contains(&name.as_str()) && !last.contains_key(name) {
                let ok = !FAIL_TAGS.iter().any(|tag| content.contains(tag));
                let snippet: String = content.chars().take(120).collect();
                last.insert(name.clone(), (ok, snippet));
            }
        }
    }
    Ok(last
        .into_iter()
        .map(|(tool, (ok, snippet))| QualityItemDto { tool, ok, snippet })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineFileDto {
    pub path: String,
    pub old_lines: usize,
    pub new_lines: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineEntryDto {
    pub seq: u64,
    pub created_at: u64,
    pub files: Vec<TimelineFileDto>,
}

/// Committed-change timeline for a session (newest first), from the
/// per-session change ledger.
#[tauri::command]
pub async fn workbench_timeline(
    shared: tauri::State<'_, Arc<Shared>>,
    session_id: String,
    limit: Option<usize>,
) -> Result<Vec<TimelineEntryDto>, String> {
    let store = shared.store.lock().unwrap().clone();
    let ledger = firment_core::journal::Ledger::new(store.ledger_path(&session_id));
    let entries = ledger.entries();
    let limit = limit.unwrap_or(12);
    Ok(entries
        .into_iter()
        .rev()
        .take(limit)
        .map(|(seq, created_at, changes)| TimelineEntryDto {
            seq,
            created_at,
            files: changes
                .into_iter()
                .map(|c| TimelineFileDto {
                    path: c.path.to_string_lossy().into_owned(),
                    old_lines: c.old_lines,
                    new_lines: c.new_lines,
                })
                .collect(),
        })
        .collect())
}
