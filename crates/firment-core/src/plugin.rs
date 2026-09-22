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
    /// Whether the user vouches for this plugin.
    ///
    /// **Off by default, and the reason is not caution for its own sake.** A plugin is a program
    /// this agent starts, and the host does **not** sandbox it: `capabilities` is a contract the
    /// plugin is trusted to honour, not a wall around it. A plugin without `fs.write` can still
    /// write anywhere the OS lets it, because nothing stops it — so a plugin that declares
    /// nothing and runs anyway would be telling the user a story about a boundary that does not
    /// exist yet.
    ///
    /// Setting this to `true` says: *I know what this program is, and I accept that it runs with
    /// my authority.* Until the OS-level sandbox in the review's §3B exists, that is what
    /// running a plugin means.
    #[serde(default)]
    pub trusted: bool,
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
            trusted: config.trusted,
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
    /// See [`PluginConfig::trusted`]: false means it is declared but will not be registered.
    pub trusted: bool,
    /// The resolved command path.
    pub path: PathBuf,
    pub args: Vec<String>,
    /// `Err` when a declared capability name is not in the vocabulary.
    pub capabilities: Result<Vec<Capability>, String>,
    /// The raw strings, so a report can show what was written even when it did not parse.
    pub declared: Vec<String>,
}

// ---------------------------------------------------------------------------
// The wire: one MCP-shaped call over stdio (plugin review §5 step 3)
// ---------------------------------------------------------------------------

/// The request a plugin receives on stdin.
///
/// The **shape** of an MCP `tools/call`, not a whole MCP session: no initialize handshake, no
/// `tools/list`, one request and one response per invocation. That is a deliberate limit of the
/// first slice — a plugin that must keep state between calls (a held serial port, say) would
/// need the session, and the shape is what a future session would still use.
pub fn request_json(id: u64, tool: &str, arguments: &serde_json::Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": tool, "arguments": arguments },
    })
    .to_string()
}

/// The text out of an MCP-shaped response, or why the output is not one.
///
/// The child's stdout is **untrusted** (review §4 invariant 2): it may contain the plugin's own
/// logging, and it may contain nothing usable at all. So: scan for the last line that parses as
/// a JSON object carrying our id, take `result.content[]` entries of type `text`, and when
/// there is none say so with a bounded excerpt — the reader needs to see what came back, and
/// the prompt must not receive an unbounded blob of somebody else's output.
pub fn parse_result(stdout: &str, id: u64) -> Result<String, String> {
    let mut found: Option<serde_json::Value> = None;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        // A guard rather than nested `if let`: clippy is right that the nested form reads as
        // two conditions when it is one question — "is this line an answer to our call".
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(value) if value.get("id").and_then(|v| v.as_u64()) == Some(id) => {
                found = Some(value);
            }
            _ => {}
        }
    }

    let Some(value) = found else {
        return Err(format!(
            "the plugin produced no response for call {id}; its output was: {}",
            excerpt(stdout)
        ));
    };

    if let Some(error) = value.get("error") {
        return Err(format!(
            "the plugin reported an error: {}",
            excerpt(&error.to_string())
        ));
    }

    let text: Vec<String> = value
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    if text.is_empty() {
        return Err(format!(
            "the plugin's response carried no text content: {}",
            excerpt(&value.to_string())
        ));
    }
    Ok(text.join("\n"))
}

fn excerpt(text: &str) -> String {
    const MAX: usize = 400;
    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(MAX).collect();
    format!("{head}… ({} chars total)", trimmed.chars().count())
}

/// Quote one word for the platform's shell.
///
/// A plugin's command and args come from the user's own config, and the *model's* arguments go
/// through stdin as JSON — never onto the command line — so this is not the only thing standing
/// between a model and a shell. It is still needed, because a plugin path with a space in it
/// (`C:\Program Files\…`) has to survive being turned into a command string.
pub fn quote_for_shell(word: &str, windows: bool) -> String {
    if windows {
        // cmd.exe has no escape for a double quote inside a quoted argument; refusing the word
        // is honest, and a plugin path containing `"` is not a case worth guessing at.
        format!("\"{}\"", word.replace('"', ""))
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// The environment a plugin gets, given what it declared.
///
/// **Allow-listed, never inherited** (review §5 step 3): `parent` is read for the few variables
/// below and nothing else, so a plugin cannot see the user's API keys, their `FIRMENT_*`
/// settings or anything else — not because they are filtered out, but because nothing is passed
/// unless it is named here.
///
/// `PATH` is always passed, because a child that cannot be found cannot run at all;
/// `SystemRoot`, `TEMP` and `TMP` come with it on Windows because programs there refuse to
/// start without them. The proxy variables come **only** with the `net` capability: a plugin
/// that did not ask for the network does not get a route to it either.
pub fn plugin_env(
    parent: &std::collections::HashMap<String, String>,
    capabilities: &[Capability],
    windows: bool,
) -> std::collections::HashMap<String, String> {
    let mut names: Vec<&str> = vec!["PATH"];
    if windows {
        names.extend(["SystemRoot", "TEMP", "TMP"]);
    }
    if capabilities.contains(&Capability::Net) {
        names.extend(["HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY", "ALL_PROXY"]);
    }

    names
        .into_iter()
        .filter_map(|name| {
            parent
                .get(name)
                .map(|value| (name.to_string(), value.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_the_mcp_call_shape_and_nothing_else() {
        let request = request_json(7, "sensor", &serde_json::json!({"bus": "i2c"}));
        let value: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["id"], 7);
        assert_eq!(value["method"], "tools/call");
        assert_eq!(value["params"]["name"], "sensor");
        assert_eq!(value["params"]["arguments"]["bus"], "i2c");
    }

    #[test]
    fn a_result_is_taken_from_the_line_that_answers_our_call() {
        // The child's stdout is untrusted and may carry its own logging, so the parser looks for
        // the response rather than trusting the first thing it sees — and it must not accept a
        // response to somebody else's call id.
        let stdout = format!(
            "plugin: starting\n{}\n{}\n",
            r#"{"jsonrpc":"2.0","id":99,"result":{"content":[{"type":"text","text":"not ours"}]}}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"ours"},{"type":"text","text":"second"}]}}"#,
        );
        assert_eq!(parse_result(&stdout, 1).unwrap(), "ours\nsecond");
    }

    #[test]
    fn unusable_output_says_what_came_back() {
        // Three ways a plugin's output is not a result, and the reader needs the excerpt in
        // both of them: "it printed nothing useful" is not diagnosable, "it printed this" is.
        let nothing = parse_result("plugin: oops\n", 1).unwrap_err();
        assert!(nothing.contains("no response"), "{nothing}");
        assert!(
            nothing.contains("oops"),
            "the excerpt should be there: {nothing}"
        );

        let err = parse_result(
            r#"{"jsonrpc":"2.0","id":1,"error":{"message":"no bus"}}"#,
            1,
        )
        .unwrap_err();
        assert!(err.contains("no bus"), "{err}");

        let empty =
            parse_result(r#"{"jsonrpc":"2.0","id":1,"result":{"content":[]}}"#, 1).unwrap_err();
        assert!(empty.contains("no text"), "{empty}");
    }

    #[test]
    fn an_excerpt_is_bounded() {
        let long = "x".repeat(2_000);
        let err = parse_result(&long, 1).unwrap_err();
        assert!(err.contains("chars total"), "{err}");
        assert!(
            err.len() < 600,
            "the excerpt must stay bounded: {}",
            err.len()
        );
    }

    #[test]
    fn quoting_keeps_a_path_with_a_space_in_one_piece() {
        assert_eq!(quote_for_shell("sensor", false), "'sensor'");
        assert_eq!(
            quote_for_shell("C:\\Program Files\\sensor.exe", true),
            "\"C:\\Program Files\\sensor.exe\""
        );
        // A single quote inside a unix word must survive as data, not end the quoting.
        assert_eq!(quote_for_shell("it's", false), r"'it'\''s'");
    }

    #[test]
    fn the_environment_is_a_list_and_never_the_parents() {
        // The invariant the review asks for, as an assertion: a plugin sees the handful of
        // variables named here and nothing else. Checked with a parent environment that looks
        // like a real one, keys included.
        let mut parent = HashMap::new();
        for (k, v) in [
            ("PATH", "/usr/bin"),
            ("SystemRoot", "C:\\Windows"),
            ("TEMP", "C:\\Temp"),
            ("TMP", "C:\\Temp"),
            ("HTTPS_PROXY", "http://127.0.0.1:7890"),
            ("NO_PROXY", "localhost"),
            ("ANTHROPIC_API_KEY", "sk-secret"),
            ("OPENAI_API_KEY", "sk-secret"),
            ("FIRMENT_SOMETHING", "private"),
            ("AWS_SECRET_ACCESS_KEY", "secret"),
        ] {
            parent.insert(k.to_string(), v.to_string());
        }

        let read_only = plugin_env(&parent, &[Capability::FsRead], false);
        assert_eq!(read_only.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert!(!read_only.contains_key("ANTHROPIC_API_KEY"));
        assert!(!read_only.contains_key("OPENAI_API_KEY"));
        assert!(!read_only.contains_key("FIRMENT_SOMETHING"));
        assert!(!read_only.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert!(
            !read_only.contains_key("HTTPS_PROXY"),
            "without `net` a plugin does not even get a route out"
        );

        // The network capability adds the proxy variables — and nothing else.
        let networked = plugin_env(&parent, &[Capability::FsRead, Capability::Net], false);
        assert_eq!(
            networked.get("HTTPS_PROXY").map(String::as_str),
            Some("http://127.0.0.1:7890")
        );
        assert!(!networked.contains_key("ANTHROPIC_API_KEY"));
        assert_eq!(
            networked.len(),
            read_only.len() + 2,
            "net adds proxies, not the parent's environment"
        );

        // Windows needs a few more to start at all; unix must not be given them.
        let windows = plugin_env(&parent, &[Capability::FsRead], true);
        assert!(windows.contains_key("SystemRoot"));
        assert!(!read_only.contains_key("SystemRoot"));
    }

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
                trusted: true,
            },
        );
        plugins.insert(
            "alpha".to_string(),
            PluginConfig {
                command: "a".to_string(),
                args: vec!["--flag".to_string()],
                capabilities: vec!["bogus".to_string()],
                trusted: true,
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
