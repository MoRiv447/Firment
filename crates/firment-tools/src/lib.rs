pub mod assembly;
pub mod decode;
pub mod hardware;
pub mod tools;

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
                | "web_search"
                | "web_fetch"
                | "task"
                | "todo"
                | "ask_user"
                | "elf_analyze"
                | "periph_init"
        ) {
            registry.register(tool);
        }
    }
    Arc::new(registry)
}
