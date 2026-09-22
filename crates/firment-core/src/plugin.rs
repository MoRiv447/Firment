//! Plugin declarations and the capability vocabulary (plugin review §5 step 2).
//!
//! A plugin is declared in `config.toml` under `[plugins.<name>]`, with a command to run and
//! the capabilities it is allowed. The review's phrase is "declared, not discovered": nothing
//! here inspects what a plugin *could* do, and the host's job (step 3) is to honour exactly
//! this list. Everything in this module exists so a declaration can be checked, printed and
//! compared *before* anything runs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What a plugin is allowed to do, as a closed vocabulary.
///
/// Closed on purpose: an open set of strings would mean a plugin can ask for a capability the
/// host has never heard of and have it silently do nothing — the declaration would stop being
/// a promise. [`Capability::parse`] rejects anything not on this list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Read files inside the workspace.
    FsRead,
    /// Write files inside the workspace. Writes are additionally subject to the turn's
    /// transaction and to any declared scope, exactly like a built-in tool's.
    FsWrite,
    /// Open network connections.
    Net,
    /// Execute other programs.
    Exec,
    /// Touch attached hardware (serial ports, probes). Separate from `exec` because a plugin
    /// that reads a sensor is not the same request as one that can flash a board.
    Hardware,
}

impl Capability {
    /// Every capability, in the order they are printed. The parser and the error message both
    /// read from here, so a new variant cannot be forgotten in one place.
    pub const ALL: [Capability; 5] = [
        Capability::FsRead,
        Capability::FsWrite,
        Capability::Net,
        Capability::Exec,
        Capability::Hardware,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Capability::FsRead => "fs.read",
            Capability::FsWrite => "fs.write",
            Capability::Net => "net",
            Capability::Exec => "exec",
            Capability::Hardware => "hardware",
        }
    }

    /// Parse one declared capability, or say what the valid ones are.
    pub fn parse(raw: &str) -> Result<Capability, String> {
        Capability::ALL
            .into_iter()
            .find(|c| c.as_str() == raw)
            .ok_or_else(|| {
                format!(
                    "unknown capability {raw:?} — valid ones are {}",
                    Capability::ALL
                        .iter()
                        .map(|c| c.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }
}

/// A `[plugins.<name>]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginConfig {
    /// The program to run. A relative path is resolved against the directory the config was
    /// loaded for (see [`resolve_command`]), because a plugin committed with a project is
    /// written relative to that project, not to wherever the user happens to be.
    pub command: String,
    /// Arguments passed to `command`, verbatim.
    #[serde(default)]
    pub args: Vec<String>,
    /// The capability names, checked by [`PluginConfig::capabilities`]. An **empty list is a
    /// valid declaration** and means the plugin gets nothing but its own stdio: the baseline is
    /// nothing, so `fs.read` has to be asked for.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl PluginConfig {
    /// The declared capabilities, or the reason the declaration is not usable.
    ///
    /// Called by whatever is about to *act* on the declaration (a doctor report, the host in
    /// step 3), so an unknown name surfaces as an error naming the valid set rather than as a
    /// plugin that quietly has fewer powers than its author believes.
    pub fn capabilities(&self) -> Result<Vec<Capability>, String> {
        self.capabilities
            .iter()
            .map(|raw| Capability::parse(raw))
            .collect()
    }
}

/// Resolve a plugin's command against the directory the config was loaded for.
///
/// Absolute commands are returned unchanged. A relative one is joined to `base`, so the same
/// `command = "./plugins/sensor.sh"` means the same file whether the user ran `firm` from the
/// project root or from a subdirectory. `firm doctor` prints the result, which is what makes a
/// change to it visible (review §5 step 2).
pub fn resolve_command(base: &Path, command: &str) -> PathBuf {
    let path = PathBuf::from(command);
    let resolved = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    drop_cur_dir_components(&resolved)
}

/// Remove `.` components, and nothing else.
///
/// `./plugins/x.sh` and `plugins/x.sh` are the same file, and a report that printed them
/// differently would show a change where none happened — which is the one job the printed path
/// has. `..` is deliberately **not** touched: collapsing it is a claim about the filesystem
/// (symlinks make it false), and a wrong claim is worse than a long path.
fn drop_cur_dir_components(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        if component == std::path::Component::CurDir {
            continue;
        }
        out.push(component);
    }
    if out.as_os_str().is_empty() {
        path.to_path_buf()
    } else {
        out
    }
}

/// Every declared plugin, resolved and checked, ready to print.
///
/// Returns them sorted by name so a doctor report is stable across runs — a diff of two
/// reports should show what changed, not what happened to be iterated first.
pub fn declared_plugins(
    plugins: &HashMap<String, PluginConfig>,
    base: &Path,
) -> Vec<DeclaredPlugin> {
    let mut declared: Vec<DeclaredPlugin> = plugins
        .iter()
        .map(|(name, config)| DeclaredPlugin {
            name: name.clone(),
            path: resolve_command(base, &config.command),
            args: config.args.clone(),
            capabilities: config.capabilities(),
            declared: config.capabilities.clone(),
        })
        .collect();
    declared.sort_by(|a, b| a.name.cmp(&b.name));
    declared
}

/// One plugin as the doctor report needs it: resolved, and either usable or explained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredPlugin {
    pub name: String,
    /// The resolved command path.
    pub path: PathBuf,
    pub args: Vec<String>,
    /// `Err` when a declared capability name is not in the vocabulary.
    pub capabilities: Result<Vec<Capability>, String>,
    /// The raw strings, so a report can show what was written even when it did not parse.
    pub declared: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vocabulary_is_closed_and_the_error_names_it() {
        // "Declared, not discovered" only means something if a declaration is checked. An
        // unknown name has to fail loudly: ignored, it would leave a plugin with fewer powers
        // than its author believes, and a *silent* capability grant is worse still.
        assert_eq!(Capability::parse("fs.read"), Ok(Capability::FsRead));
        assert_eq!(Capability::parse("fs.write"), Ok(Capability::FsWrite));
        assert_eq!(Capability::parse("net"), Ok(Capability::Net));

        let err = Capability::parse("fs.raed").unwrap_err();
        assert!(err.contains("fs.raed"), "{err}");
        assert!(
            err.contains("fs.read"),
            "the error should list the valid ones: {err}"
        );
        // Every variant is in `ALL`, so a new one cannot be forgotten in the error message.
        for capability in Capability::ALL {
            assert_eq!(Capability::parse(capability.as_str()), Ok(capability));
        }
    }

    #[test]
    fn a_declaration_reports_which_capability_it_could_not_use() {
        let config: PluginConfig = toml::from_str(
            r#"
            command = "./plugins/sensor.sh"
            capabilities = ["fs.read", "nope"]
            "#,
        )
        .unwrap();
        let err = config.capabilities().unwrap_err();
        assert!(err.contains("nope"), "{err}");

        let ok: PluginConfig = toml::from_str(
            r#"
            command = "sensor"
            capabilities = ["fs.read", "hardware"]
            "#,
        )
        .unwrap();
        assert_eq!(
            ok.capabilities().unwrap(),
            vec![Capability::FsRead, Capability::Hardware]
        );
        assert_eq!(ok.args, Vec::<String>::new());
    }

    #[test]
    fn an_empty_declaration_is_valid_and_means_nothing_was_granted() {
        // The baseline is nothing: `fs.read` has to be asked for. A plugin that declares no
        // capabilities is not an error, it is a plugin that can do nothing but talk on stdio.
        let config: PluginConfig = toml::from_str(r#"command = "noop""#).unwrap();
        assert!(config.capabilities().unwrap().is_empty());
    }

    #[test]
    fn a_relative_command_resolves_against_the_configs_directory_not_the_cwd() {
        // The same declaration has to mean the same file whether `firm` was run from the
        // project root or three directories down; that is the whole reason the path is resolved
        // and printed rather than passed through.
        let base = Path::new("/project");
        // Two spellings of one file must print as one path — that is what makes the printed
        // path able to show a *change* rather than a reformatting.
        assert_eq!(
            resolve_command(base, "./plugins/sensor.sh"),
            resolve_command(base, "plugins/sensor.sh"),
        );
        assert_eq!(
            resolve_command(base, "./plugins/sensor.sh"),
            base.join("plugins/sensor.sh")
        );
        // `..` is left alone on purpose: collapsing it would be a claim about the filesystem.
        assert_eq!(
            resolve_command(base, "../sibling/x.sh"),
            base.join("../sibling/x.sh")
        );
        assert_eq!(
            resolve_command(base, "/usr/local/bin/sensor"),
            PathBuf::from("/usr/local/bin/sensor")
        );
    }

    #[test]
    fn the_report_is_sorted_and_carries_the_parse_result() {
        // A doctor report is diffed by humans: stable order, and a broken declaration shows up
        // as a broken line rather than as a plugin that is quietly absent.
        let mut plugins = HashMap::new();
        plugins.insert(
            "zebra".to_string(),
            PluginConfig {
                command: "z".to_string(),
                args: vec![],
                capabilities: vec!["net".to_string()],
            },
        );
        plugins.insert(
            "alpha".to_string(),
            PluginConfig {
                command: "a".to_string(),
                args: vec!["--flag".to_string()],
                capabilities: vec!["bogus".to_string()],
            },
        );

        let declared = declared_plugins(&plugins, Path::new("/project"));
        assert_eq!(declared.len(), 2);
        assert_eq!(declared[0].name, "alpha", "sorted by name");
        assert_eq!(declared[1].name, "zebra");
        assert!(declared[0].capabilities.is_err());
        assert_eq!(declared[0].declared, vec!["bogus".to_string()]);
        assert_eq!(declared[0].path, Path::new("/project").join("a"));
        assert_eq!(
            declared[1].capabilities.as_ref().unwrap(),
            &vec![Capability::Net]
        );
    }
}
