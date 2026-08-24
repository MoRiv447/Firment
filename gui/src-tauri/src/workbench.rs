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
    let mut command = tokio::process::Command::new("git");
    command.args(args).current_dir(cwd);
    // GUI has no console: spawning git without this flag flashes a black
    // terminal window on every workbench refresh.
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().await.ok().filter(|o| o.status.success())?;
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
pub async fn workbench_state(
    shared: tauri::State<'_, Arc<Shared>>,
    cwd: String,
) -> Result<WorkbenchStateDto, String> {
    let root = PathBuf::from(&cwd);
    if !root.is_dir() {
        return Err(format!("cwd does not exist: {}", root.display()));
    }
    let git = git_status(&root).await;

    // One-time self-heal for sessions created before the Normal/Mainline/
    // Branch triple existed: the registered mainline gets promoted (and any
    // sibling Mainline in the same project demoted to Normal) so the sidebar
    // tags are truthful without user action.
    let cfg = load_config(&root);
    if !cfg.mainline_session.is_empty() {
        let store = shared.store.lock().unwrap().clone();
        let _ = store.mark_mainline(&cfg.mainline_session);
    }

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
    // mark_mainline promotes the target to Mainline and demotes any other
    // Mainline session sharing its cwd back to Normal.
    shared
        .store
        .lock()
        .unwrap()
        .mark_mainline(&session_id)
        .map_err(|e| format!("cannot set mainline: {e}"))?;
    let mut cfg = WorkbenchConfig::load(Path::new(&cwd))?;
    cfg.workbench.mainline_session = session_id;
    cfg.save(Path::new(&cwd))
}

// ---------- pin/resource registry ([pinmap] in workbench.toml) ----------

#[derive(Debug, Clone, Serialize)]
pub struct PinEntryDto {
    pub pin: String,
    pub func: String,
    pub owner: String,
}

#[tauri::command]
pub async fn workbench_pinmap_list(cwd: String) -> Result<Vec<PinEntryDto>, String> {
    let cfg = WorkbenchConfig::load(Path::new(&cwd))?;
    Ok(cfg
        .pinmap
        .into_iter()
        .map(|(pin, e)| PinEntryDto {
            pin,
            func: e.func,
            owner: e.owner,
        })
        .collect())
}

#[tauri::command]
pub async fn workbench_pinmap_set(
    cwd: String,
    pin: String,
    func: String,
    owner: String,
) -> Result<Vec<PinEntryDto>, String> {
    // Same normalization as the agent-side pinmap tool so GUI edits and
    // agent claims always land on the same key.
    let key = pin.trim().to_uppercase();
    if key.is_empty() || func.trim().is_empty() {
        return Err("pin and func are required".into());
    }
    let root = PathBuf::from(&cwd);
    let mut cfg = WorkbenchConfig::load(&root)?;
    cfg.pinmap.insert(
        key,
        firment_core::PinEntry {
            func: func.trim().to_string(),
            owner: owner.trim().to_string(),
        },
    );
    cfg.save(&root)?;
    workbench_pinmap_list(cwd).await
}

#[tauri::command]
pub async fn workbench_pinmap_remove(cwd: String, pin: String) -> Result<Vec<PinEntryDto>, String> {
    let root = PathBuf::from(&cwd);
    let mut cfg = WorkbenchConfig::load(&root)?;
    cfg.pinmap.remove(pin.trim().to_uppercase().as_str());
    cfg.save(&root)?;
    workbench_pinmap_list(cwd).await
}

// ---------- ADR-lite decision log ([[decision]] in workbench.toml) -------

#[derive(Debug, Clone, Serialize)]
pub struct DecisionEntryDto {
    pub title: String,
    pub body: String,
    pub date: String,
}

#[tauri::command]
pub async fn workbench_decision_list(cwd: String) -> Result<Vec<DecisionEntryDto>, String> {
    let cfg = WorkbenchConfig::load(Path::new(&cwd))?;
    Ok(cfg
        .decision
        .iter()
        .map(|d| DecisionEntryDto {
            title: d.title.clone(),
            body: d.body.clone(),
            date: d.date.clone(),
        })
        .collect())
}

#[tauri::command]
pub async fn workbench_decision_add(
    cwd: String,
    title: String,
    body: String,
) -> Result<Vec<DecisionEntryDto>, String> {
    if title.trim().is_empty() {
        return Err("title is required".into());
    }
    let root = PathBuf::from(&cwd);
    let mut cfg = WorkbenchConfig::load(&root)?;
    cfg.decision.push(firment_core::DecisionEntry {
        title: title.trim().to_string(),
        body: body.trim().to_string(),
        // Same stamp the agent-side `decision` tool uses.
        date: chrono_like_today(),
    });
    cfg.save(&root)?;
    workbench_decision_list(cwd).await
}

#[tauri::command]
pub async fn workbench_decision_remove(
    cwd: String,
    index: u64,
) -> Result<Vec<DecisionEntryDto>, String> {
    let root = PathBuf::from(&cwd);
    let mut cfg = WorkbenchConfig::load(&root)?;
    let idx = index as usize;
    if idx == 0 || idx > cfg.decision.len() {
        return Err(format!(
            "index {index} out of range (1..={})",
            cfg.decision.len()
        ));
    }
    cfg.decision.remove(idx - 1);
    cfg.save(&root)?;
    workbench_decision_list(cwd).await
}

/// YYYY-MM-DD for GUI-added entries; mirrors the tool's civil-from-days
/// conversion without pulling a date crate into the GUI layer.
fn chrono_like_today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        / 86_400;
    let z = secs as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

// ---------- project knowledge base entry (W2-3) ---------------------------
//
// The agent already reads three knowledge sources per project: AGENTS.md
// (project memory), vendor-index.toml (hardware KB index, root or docs/),
// and any files that index points at. This section exposes them to the GUI
// through a strict whitelist — no arbitrary path reads/writes.

const KB_AGENTS: &str = "AGENTS.md";
const KB_VENDOR: &str = "vendor-index.toml";

/// Resolve a whitelisted KB key to an absolute path. Anything not in the
/// whitelist (or containing path separators beyond the known shapes) is
/// rejected, so the frontend can never touch unrelated files.
fn kb_path(cwd: &Path, key: &str) -> Option<PathBuf> {
    match key {
        KB_AGENTS => Some(cwd.join(KB_AGENTS)),
        KB_VENDOR => {
            let docs = cwd.join("docs").join(KB_VENDOR);
            if docs.is_file() {
                Some(docs)
            } else {
                Some(cwd.join("docs").join(KB_VENDOR))
            }
        }
        other => {
            // Project-private cheatsheet: "cheatsheet:<name>.toml".
            let name = other.strip_prefix("cheatsheet:")?;
            if name.contains(['/', '\\'])
                || name.contains("..")
                || !name.ends_with(".toml")
                || name.len() > 80
            {
                return None;
            }
            Some(cwd.join(".firment").join("cheatsheets").join(name))
        }
    }
}

fn read_or_empty(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
pub struct KbEntryDto {
    /// Whitelist key ("AGENTS.md", "vendor-index.toml", "cheatsheet:x.toml").
    pub key: String,
    pub exists: bool,
    pub content: String,
}

#[tauri::command]
pub async fn workbench_kb_list(cwd: String) -> Result<Vec<KbEntryDto>, String> {
    let root = PathBuf::from(&cwd);
    if !root.is_dir() {
        return Err(format!("cwd does not exist: {}", root.display()));
    }
    let mut entries = vec![
        KbEntryDto {
            key: KB_AGENTS.into(),
            exists: root.join(KB_AGENTS).is_file(),
            content: read_or_empty(&root.join(KB_AGENTS)),
        },
        KbEntryDto {
            key: format!("docs/{KB_VENDOR}"),
            exists: root.join("docs").join(KB_VENDOR).is_file(),
            content: read_or_empty(&root.join("docs").join(KB_VENDOR)),
        },
    ];
    let cheat_dir = root.join(".firment").join("cheatsheets");
    if let Ok(read) = std::fs::read_dir(&cheat_dir) {
        let mut names: Vec<String> = read
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("toml"))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for name in names {
            let p = cheat_dir.join(&name);
            entries.push(KbEntryDto {
                key: format!("cheatsheet:{name}"),
                exists: true,
                content: read_or_empty(&p),
            });
        }
    }
    Ok(entries)
}

#[tauri::command]
pub async fn workbench_kb_save(cwd: String, key: String, content: String) -> Result<(), String> {
    let root = PathBuf::from(&cwd);
    let path = kb_path(&root, &key)
        .ok_or_else(|| format!("unknown or disallowed knowledge file '{key}'"))?;
    // The vendor-index key always writes under docs/ (the location the
    // auto-detector checks first).
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, content).map_err(|e| format!("write {}: {e}", path.display()))
}

#[tauri::command]
pub async fn workbench_kb_delete(cwd: String, key: String) -> Result<(), String> {
    let root = PathBuf::from(&cwd);
    // Only project-private cheatsheets are deletable; AGENTS.md and the
    // vendor index are cleared by saving empty content instead.
    if !key.starts_with("cheatsheet:") {
        return Err("only cheatsheet:* entries can be deleted".into());
    }
    let path = kb_path(&root, &key)
        .ok_or_else(|| format!("unknown or disallowed knowledge file '{key}'"))?;
    if path.is_file() {
        std::fs::remove_file(&path).map_err(|e| format!("delete {}: {e}", path.display()))?;
    }
    Ok(())
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
