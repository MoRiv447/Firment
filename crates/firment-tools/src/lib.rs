pub mod assembly;
pub mod decode;
pub mod forensic;
pub mod hardware;
pub mod la_cmd;
pub mod la_measure;
pub mod redteam;
pub mod review;
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

/// Registry for a **research subagent**: `plan_registry` minus the two tools that cannot
/// work in a subagent.
///
/// Found by comparing with opencode's `deriveSubagentSessionPermission`, which denies
/// `todowrite` and `task` in a child unless the child's own ruleset permits them. Firment
/// does the equivalent by swapping the whole registry — mutating tools are never even
/// advertised — but reusing plan mode's list left two tools advertised that are *structurally*
/// broken in a child, which is worse than either:
///
/// * `todo` needs `ToolContext::session_dir`, and a nested agent's is `None` — it fails the
///   moment it is called (`tools/src/tools/todo.rs`);
/// * `ask_user` needs an asker, and the runner deliberately gives a child none.
///
/// A tool that fails on every call is a wasted round trip and a misleading error. Plan mode
/// keeps both, because a plan-mode session has a session directory and a user to ask — which
/// is why this is a separate function rather than a change to that one.
pub fn subagent_registry() -> Arc<ToolRegistry> {
    let plan = plan_registry();
    let mut registry = ToolRegistry::new();
    for tool in tools::all() {
        if plan.get(tool.name()).is_none() {
            continue;
        }
        if matches!(tool.name(), "todo" | "ask_user") {
            continue;
        }
        registry.register(tool);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_research_subagent_is_not_advertised_tools_that_cannot_work_for_it() {
        // The comparison with opencode's `deriveSubagentSessionPermission` is what surfaced
        // this: it denies `todowrite` in a child, and Firment's equivalent — reusing plan
        // mode's registry — advertised `todo` and `ask_user`, both of which fail on every
        // call in a child (`session_dir` is `None`, and the runner gives no asker).
        let subagent = subagent_registry();
        assert!(
            subagent.get("todo").is_none(),
            "todo cannot work in a subagent"
        );
        assert!(
            subagent.get("ask_user").is_none(),
            "a subagent has no user to ask"
        );

        // What it must keep: the research tools, and `task` (nested research is intentional,
        // bounded by max_subagent_depth).
        for name in ["read_file", "grep", "glob", "symbols", "web_search", "task"] {
            assert!(subagent.get(name).is_some(), "{name} should stay available");
        }
        // And nothing that writes.
        for name in ["write_file", "edit_file", "shell", "flash", "rename_symbol"] {
            assert!(
                subagent.get(name).is_none(),
                "{name} must not reach a subagent"
            );
        }
    }

    #[test]
    fn plan_mode_still_has_the_tools_a_subagent_must_not() {
        // The contrast that makes the function above a separate one rather than a change to
        // `plan_registry`: a plan-mode session has a session directory and a human to ask, so
        // dropping these there would break plan mode to fix subagents.
        let plan = plan_registry();
        assert!(plan.get("todo").is_some());
        assert!(plan.get("ask_user").is_some());
    }
}
