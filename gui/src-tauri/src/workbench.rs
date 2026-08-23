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
pub async fn workbench_set_mainline(cwd: String, session_id: String) -> Result<(), String> {
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
