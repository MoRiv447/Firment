pub mod assembly;
pub mod decode;
pub mod forensic;
pub mod hardware;
pub mod la_cmd;
pub mod la_measure;
pub mod plugin_tool;
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

/// The registry a session runs with: the built-in set for the mode, plus the configured plugins.
///
/// Plugins are added **last and through `register_plugin`**, so a name that collides with a
/// built-in is refused rather than winning — the plugin review's §5 step 1 finding, and the
/// reason this could not simply be a `register` loop. The refusals come back as messages instead
/// of being dropped: a plugin that silently did not load is the failure mode that wastes an
/// afternoon.
///
/// `base` is where relative plugin commands resolve from, and it is the **session's cwd** — the
/// same directory the rest of the merged config is read for, so one answer rather than two.
pub fn session_registry(
    plan: bool,
    plugins: &std::collections::HashMap<String, firment_core::plugin::PluginConfig>,
    base: &std::path::Path,
) -> (Arc<ToolRegistry>, Vec<String>) {
    let builtin = if plan {
        plan_registry()
    } else {
        default_registry()
    };
    let mut registry = ToolRegistry::new();
    registry.extend_from(&builtin);

    // The names a plugin may not take, and this is NOT the same set as the registry it is about
    // to join: plan mode's registry has no write tools, so checking only against it would let a
    // plugin called `write_file` register **because plan mode is read-only** — making plugins a
    // way around plan mode. The reserved set is every built-in tool, in every mode.
    let reserved: std::collections::HashSet<Arc<str>> =
        tools::all().iter().map(|tool| tool.owned_name()).collect();

    // The plugins the user vouched for. Nothing here is a sandbox: this is the difference between
    // "declared" and "enabled", which is why the report has two words for it.
    let trusted: std::collections::HashSet<&str> = plugins
        .iter()
        .filter(|(_, config)| config.trusted)
        .map(|(name, _)| name.as_str())
        .collect();

    let mut refusals = Vec::new();
    for tool in crate::plugin_tool::plugin_tools(plugins, base) {
        let name = tool.owned_name();
        if !trusted.contains(name.as_ref()) {
            // Declared, not enabled — and said out loud, because a plugin that is absent without
            // an explanation is a plugin the user believes is working.
            refusals.push(format!(
                "plugin {name:?} is declared but not enabled — a plugin runs unsandboxed, so it \
                 needs `trusted = true` in its `[plugins.{name}]` entry before it can be called"
            ));
            continue;
        }
        if reserved.contains(&name) {
            refusals.push(format!(
                "plugin tool {name:?} would take the name of a built-in tool — plugin names may \
                 not collide with built-ins in any mode (plan mode's read-only registry is a \
                 policy about built-ins, not a gap for plugins to fill)"
            ));
            continue;
        }
        if let Err(e) = registry.register_plugin(tool) {
            refusals.push(e);
        }
    }
    (Arc::new(registry), refusals)
}

/// Registry for a subagent that is allowed to **write** (`[tools] subagents_may_write`).
///
/// The full tool set minus the same two tools a read-only child must not have: `todo` needs a
/// session directory a nested agent does not have, and `ask_user` needs a user a nested agent
/// cannot reach. Everything else — including `task`, bounded by `max_subagent_depth` — is
/// available, and the workspace boundary still comes from `resolve_within`.
///
/// Opt-in, never the default: a child that writes is a child nobody watches, and the turn's
/// journal makes the edits undoable rather than reviewable.
pub fn write_capable_subagent_registry() -> Arc<ToolRegistry> {
    let full = default_registry();
    let mut registry = ToolRegistry::new();
    for tool in tools::all() {
        if full.get(tool.name()).is_none() {
            continue;
        }
        // `shell` is excluded from the write-capable set on purpose: a declared scope cannot
        // constrain a shell, and a scope that silently does not cover one is worse than no
        // scope at all. A child that needs a shell is a separate decision, not a default.
        if matches!(tool.name(), "todo" | "ask_user" | "shell") {
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
    fn a_plugin_tool_joins_the_session_registry_and_a_built_in_name_is_refused_in_every_mode() {
        // The wiring, asserted where it happens. The second half is the part worth reading: plan
        // mode's registry does not contain `write_file`, so checking "is this name taken?" against
        // *that* registry would let a plugin called `write_file` register — turning plugins into a
        // way around plan mode. The check has to be against every built-in tool, in every mode.
        let dir = tempfile::tempdir().unwrap();
        let mut plugins = std::collections::HashMap::new();
        plugins.insert(
            "sensor".to_string(),
            firment_core::plugin::PluginConfig {
                command: "sensor".to_string(),
                args: vec![],
                capabilities: vec!["fs.read".to_string()],
                trusted: true,
            },
        );
        plugins.insert(
            "write_file".to_string(),
            firment_core::plugin::PluginConfig {
                command: "impostor".to_string(),
                args: vec![],
                capabilities: vec!["fs.write".to_string()],
                trusted: true,
            },
        );
        // Declared and vouched for by nobody: in the config, absent from the session.
        plugins.insert(
            "unvouched".to_string(),
            firment_core::plugin::PluginConfig {
                command: "mystery".to_string(),
                args: vec![],
                capabilities: vec![],
                trusted: false,
            },
        );

        let (registry, refusals) = session_registry(false, &plugins, dir.path());
        assert!(
            registry.get("sensor").is_some(),
            "a plugin tool the session can call"
        );
        assert!(
            registry.get("unvouched").is_none(),
            "an untrusted plugin must not be callable"
        );
        assert_eq!(
            refusals.len(),
            2,
            "the shadow and the untrusted one are both refused: {refusals:?}"
        );
        assert!(
            refusals.iter().any(|r| r.contains("write_file")),
            "{refusals:?}"
        );
        // The refusal names the flag: "why is my plugin not working" should not need a second
        // lookup, and the answer is a decision the user has to make, not a bug to report.
        assert!(
            refusals.iter().any(|r| r.contains("trusted = true")),
            "{refusals:?}"
        );

        // Plan mode: the plugin is still there, and the impostor is still refused — even though
        // the read-only registry has no `write_file` for it to collide with.
        let (plan, plan_refusals) = session_registry(true, &plugins, dir.path());
        assert!(plan.get("sensor").is_some());
        assert!(
            plan.get("write_file").is_none(),
            "plan mode must not acquire a write tool through a plugin"
        );
        assert_eq!(plan_refusals.len(), 2, "{plan_refusals:?}");
    }

    #[test]
    fn the_write_capable_registry_is_the_full_set_minus_what_a_child_cannot_use() {
        // Even with writes on, the two structural exclusions stand: a child has no session
        // directory (`todo`) and no user to ask (`ask_user`) — and a third, `shell`, because a
        // declared scope cannot constrain a shell (asserted below).
        let writing = write_capable_subagent_registry();
        for name in ["write_file", "edit_file", "read_file", "grep", "task"] {
            assert!(
                writing.get(name).is_some(),
                "{name} should be available when writes are on"
            );
        }
        for name in ["todo", "ask_user"] {
            assert!(
                writing.get(name).is_none(),
                "{name} cannot work in a child either way"
            );
        }
        // `shell` is excluded on purpose: a declared scope cannot constrain a shell, and a
        // scope that silently does not cover one is worse than no scope at all.
        assert!(
            writing.get("shell").is_none(),
            "a shell would step around every declared scope"
        );
        // And the read-only table must not have grown a write tool by accident.
        let read_only = subagent_registry();
        for name in ["write_file", "edit_file", "shell"] {
            assert!(
                read_only.get(name).is_none(),
                "{name} must not reach a read-only child"
            );
        }
    }

    #[test]
    fn writes_from_a_subagent_are_off_unless_asked_for() {
        // The default is the feature: an unattended child that can edit is something a user
        // opts into, not something they discover.
        let config = firment_core::config::Config::default_config();
        assert!(!config.tools.subagents_may_write);
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
