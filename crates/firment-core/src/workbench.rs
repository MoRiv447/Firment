//! Workbench project state: `.firment/workbench.toml` in the repo root.
//!
//! The file is the single source of truth for the workbench (see
//! `docs/gui-workbench.md` §2): project meta, the mainline session, branch
//! registry, member scopes, quick commands, the pin/resource map and ADRs.
//! Single-player works with just this file; teams sync it through git.
//!
//! Schema notes: unknown fields are KEPT (serde default tolerance) so future
//! versions can add sections without breaking older readers; every section
//! is optional so a fresh project can start from an empty file.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const WORKBENCH_FILE: &str = "workbench.toml";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ProjectMeta {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GuardConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Standby-watch cadence in minutes (small-model guard rounds).
    #[serde(default = "default_standby_minutes")]
    pub standby_minutes: u32,
    /// Severity at which the guard escalates to the big model.
    #[serde(default = "default_escalate_sev")]
    pub escalate_sev: String,
}

fn default_standby_minutes() -> u32 {
    30
}

fn default_escalate_sev() -> String {
    "warn".to_string()
}

impl Default for GuardConfig {
    /// Manual impl: the derived Default would give an EMPTY escalate_sev
    /// (String's default), and one GUI save with that value permanently
    /// overrode the serde field-default — the threshold then read as "".
    fn default() -> Self {
        Self {
            enabled: false,
            standby_minutes: default_standby_minutes(),
            escalate_sev: default_escalate_sev(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WorkbenchSection {
    /// Session id of the project's main line.
    #[serde(default)]
    pub mainline_session: String,
    #[serde(default)]
    pub guard: GuardConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BranchEntry {
    /// "mainline" or another branch's id — forms the session tree.
    #[serde(default)]
    pub parent: String,
    #[serde(default)]
    pub title: String,
    /// Optional git branch the workbench keeps this line on.
    #[serde(default)]
    pub git_branch: String,
    /// open | merged | archived
    #[serde(default = "default_branch_status")]
    pub status: String,
    #[serde(default)]
    pub created_at: u64,
}

fn default_branch_status() -> String {
    "open".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ScopeEntry {
    pub member: String,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct QuickCommand {
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub vars: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PinEntry {
    pub func: String,
    #[serde(default)]
    pub owner: String,
}

/// Per-board device registration ([devices] in workbench.toml). The key is
/// the node/board name and SHOULD match the MQTT node name so the devices
/// card, the pin registry and the physical board all cross-reference.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DeviceEntry {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub note: String,
    /// Optional command-prefix whitelist for `device_cmd` (e.g. ["rgb:"]).
    /// Empty = all commands allowed (still logged).
    #[serde(default)]
    pub allow: Vec<String>,
}

/// Board-scoped pin map: board -> pin -> entry. Serialized as
/// `[pinmap.<board>]` tables; legacy flat `[pinmap]` files (pin -> entry
/// directly) migrate to the board "default" on load and are rewritten in
/// the nested shape on the next save.
pub type Pinmap = std::collections::BTreeMap<String, std::collections::BTreeMap<String, PinEntry>>;

/// Board that owns pre-hierarchy flat pinmap entries.
pub const DEFAULT_BOARD: &str = "default";

mod pinmap_serde {
    use super::PinEntry;
    use serde::Deserialize;

    pub type Map = std::collections::BTreeMap<String, std::collections::BTreeMap<String, PinEntry>>;

    /// Accept BOTH the nested `[pinmap.<board>]` shape and the legacy flat
    /// `[pinmap]` shape (pin -> {func, owner}); legacy entries land on the
    /// "default" board.
    pub fn deserialize<'de, D>(d: D) -> Result<Map, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = toml::Value::deserialize(d)?;
        let Some(table) = value.as_table() else {
            return Err(serde::de::Error::custom("pinmap must be a table"));
        };
        let mut out: Map = Default::default();
        for (key, value) in table {
            let Some(t) = value.as_table() else {
                return Err(serde::de::Error::custom(format!(
                    "pinmap entry '{key}' must be a table"
                )));
            };
            if t.is_empty() {
                // Declared-but-empty board: keep it so a freshly created
                // board does not brick the whole file.
                out.insert(key.clone(), Default::default());
                continue;
            }
            let is_board = t
                .values()
                .all(|v| v.as_table().and_then(|v| v.get("func")).is_some());
            if is_board {
                let board: std::collections::BTreeMap<String, PinEntry> =
                    value.clone().try_into().map_err(serde::de::Error::custom)?;
                // MERGE into the board: a file may legally mix legacy flat
                // pins with an explicit board of the same name (e.g. both
                // `[pinmap.PA5]` and `[pinmap.default]`) — insert() here
                // silently amputated the migrated entries.
                out.entry(key.clone()).or_default().extend(board);
            } else {
                // Legacy flat entry -> default board.
                let entry: PinEntry = value.clone().try_into().map_err(|_| {
                    serde::de::Error::custom(format!(
                        "pinmap entry '{key}' is neither a board nor a valid pin (missing 'func')"
                    ))
                })?;
                out.entry(super::DEFAULT_BOARD.to_string())
                    .or_default()
                    .insert(key.clone(), entry);
            }
        }
        Ok(out)
    }

    /// Always write the nested (board-scoped) shape — legacy files are
    /// migrated on load and rewritten in the new spelling.
    pub fn serialize<S>(map: &Map, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serde::Serialize::serialize(map, s)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DecisionEntry {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub date: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WorkbenchConfig {
    #[serde(default)]
    pub project: ProjectMeta,
    #[serde(default)]
    pub workbench: WorkbenchSection,
    #[serde(default)]
    pub scope: std::collections::BTreeMap<String, ScopeEntry>,
    /// Branch registry keyed by short id (session id prefix).
    #[serde(default)]
    pub branch: std::collections::BTreeMap<String, BranchEntry>,
    #[serde(default)]
    pub quickcmd: std::collections::BTreeMap<String, QuickCommand>,
    /// Pin/resource map, scoped per board: `[pinmap.<board>]` with the board
    /// name matching the MQTT node name. Legacy flat files migrate to the
    /// "default" board on load.
    #[serde(default, with = "pinmap_serde")]
    pub pinmap: Pinmap,
    /// Per-project device/board registry: `[devices.<node>]`. The agent's
    /// `device_cmd` tool refuses nodes that are not registered here.
    #[serde(default)]
    pub devices: std::collections::BTreeMap<String, DeviceEntry>,
    #[serde(default)]
    pub decision: Vec<DecisionEntry>,
}

impl WorkbenchConfig {
    /// Path of the workbench file for a repo root: `<root>/.firment/workbench.toml`.
    pub fn path_for(root: &Path) -> PathBuf {
        root.join(".firment").join(WORKBENCH_FILE)
    }

    /// Load from a repo root. A missing file yields the empty default
    /// (fresh project); a corrupt file is an error — the workbench must not
    /// silently invent state.
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = Self::path_for(root);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut cfg: WorkbenchConfig =
            toml::from_str(&text).map_err(|e| format!("corrupt {}: {e}", path.display()))?;
        // Heal files written before the GuardConfig default fix: an empty
        // escalate_sev would make the escalation threshold rank as "info".
        if cfg.workbench.guard.escalate_sev.trim().is_empty() {
            cfg.workbench.guard.escalate_sev = default_escalate_sev();
        }
        Ok(cfg)
    }

    /// Persist atomically to the repo root (tmp + rename), creating
    /// `.firment/` when needed.
    pub fn save(&self, root: &Path) -> Result<(), String> {
        let path = Self::path_for(root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let body = toml::to_string_pretty(self).map_err(|e| format!("serialize workbench: {e}"))?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// The mainline session id, if the workbench declares one.
    pub fn mainline_session(&self) -> Option<&str> {
        let s = self.workbench.mainline_session.trim();
        (!s.is_empty()).then_some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn root_with(toml_text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join(".firment")).unwrap();
        let mut f = std::fs::File::create(WorkbenchConfig::path_for(&root)).unwrap();
        f.write_all(toml_text.as_bytes()).unwrap();
        (dir, root)
    }

    #[test]
    fn missing_file_yields_default() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = WorkbenchConfig::load(dir.path()).unwrap();
        assert_eq!(cfg, WorkbenchConfig::default());
    }

    #[test]
    fn documented_example_parses_and_roundtrips() {
        let example = r#"
[project]
name = "fw-thermostat"
created_at = 1755850000

[workbench]
mainline_session = "e5bd87a2-057c-4a3e-a87b-7cb5bf6b3335"
guard = { enabled = true, standby_minutes = 30, escalate_sev = "warn" }

[branch.a1b2c3d4]
parent = "mainline"
title = "传感器漂移排查"
git_branch = "exp/drift-hunt"
status = "open"
created_at = 1755851000

[scope.owner]
member = "alice"
paths = ["**"]

[quickcmd.flash-usb0]
steps = ["flash", "monitor"]
vars = { port = "COM14" }

[pinmap.PA5]
func = "LED status"
owner = "alice"

[pinmap.s3-node-1]
GPIO48 = { func = "WS2812 RGB", owner = "agent" }

[devices.s3-node-1]
role = "main mcu"
note = "RGB experiment"
allow = ["rgb:"]

[[decision]]
title = "传感器总线选 CAN 而非 RS485"
body = "节点数可能扩到 16"
date = "2026-08-22"
"#;
        let (dir, root) = root_with(example);
        let cfg = WorkbenchConfig::load(&root).unwrap();
        assert_eq!(cfg.project.name, "fw-thermostat");
        assert_eq!(
            cfg.workbench.mainline_session,
            "e5bd87a2-057c-4a3e-a87b-7cb5bf6b3335"
        );
        assert!(cfg.workbench.guard.enabled);
        assert_eq!(cfg.branch["a1b2c3d4"].git_branch, "exp/drift-hunt");
        assert_eq!(cfg.quickcmd["flash-usb0"].steps, vec!["flash", "monitor"]);
        // Legacy flat [pinmap.PA5] migrates to the "default" board; the
        // nested board keeps its own table.
        assert_eq!(cfg.pinmap["default"]["PA5"].func, "LED status");
        assert_eq!(cfg.pinmap["s3-node-1"]["GPIO48"].func, "WS2812 RGB");
        assert_eq!(cfg.devices["s3-node-1"].role, "main mcu");
        assert_eq!(cfg.devices["s3-node-1"].allow, vec!["rgb:".to_string()]);
        assert_eq!(cfg.decision.len(), 1);
        // Round-trip: save -> load keeps the same data (legacy entry is
        // rewritten nested under "default").
        cfg.save(&root).unwrap();
        let reloaded = WorkbenchConfig::load(&root).unwrap();
        assert_eq!(cfg, reloaded);
        drop(dir);
    }

    #[test]
    fn corrupt_file_is_an_error_not_silently_default() {
        let (dir, root) = root_with("[project\nname = broken");
        let err = WorkbenchConfig::load(&root).unwrap_err();
        assert!(err.contains("corrupt"), "got: {err}");
        drop(dir);
    }
}
