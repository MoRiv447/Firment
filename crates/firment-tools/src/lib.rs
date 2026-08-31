pub mod assembly;
pub mod decode;
pub mod forensic;
pub mod hardware;
pub mod la_cmd;
pub mod la_measure;
pub mod redteam;
pub mod tools;
pub mod utf8;

pub use tools::elf_analyze::analyze_elf_file;

use firment_core::{Tool, ToolRegistry};
use std::sync::Arc;

pub fn default_tools() -> Vec<Arc<dyn Tool>> {
    tools::all()
}

pub fn default_registry() -> Arc<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    for tool in tools::all() {
        registry.register(tool);
    }
    Arc::new(registry)
}

/// Read-only registry used in PLAN mode and for research subagents:
/// investigation tools only, plus the non-mutating research/planning tools.
/// Mutating tools are not even advertised to the model.
pub fn plan_registry() -> Arc<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    for tool in tools::all() {
        if matches!(
            tool.name(),
            "read_file"
                | "list_dir"
                | "glob"
                | "grep"
                | "symbols"
                | "models"
                | "web_search"
                | "web_fetch"
                | "task"
                | "todo"
                | "ask_user"
                | "elf_analyze"
                | "periph_init"
                | "device_log"
                | "observe"
        ) {
            registry.register(tool);
        }
    }
    Arc::new(registry)
}

/// Attacker-profile registry for the `redteam` campaign subagent: the
/// hardware-facing observation/probing tools, but NOT the ones that could
/// brick the host or self-replicate — no shell, no write_file/edit_file, no
/// flash/run (recovery is the suite's job, not the agent's), no task (no
/// nested nesting). TargetLockPermission restricts which port/node the
/// campaign may touch; the suite's approval covered the rest.
pub fn attacker_registry() -> Arc<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    for tool in tools::all() {
        if matches!(
            tool.name(),
            "monitor"
                | "debug"
                | "elf_analyze"
                | "la"
                | "observe"
                | "device_cmd"
                | "device_log"
                | "read_file"
                | "grep"
                | "glob"
                | "list_dir"
        ) {
            registry.register(tool);
        }
    }
    Arc::new(registry)
}
