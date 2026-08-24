use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::path::Path;

use firment_core::WorkbenchConfig;

pub struct Pinmap;

/// Normalize a pin name for map keys: trim, uppercase, collapse inner
/// whitespace — "pa5", "PA5" and "PA5 " all land on one entry.
fn normalize_pin(raw: &str) -> String {
    raw.trim().to_uppercase()
}

/// Board names are MQTT node names — case-sensitive by convention, but we
/// trim stray whitespace so copy-paste doesn't create phantom boards.
fn normalize_board(raw: &str) -> String {
    raw.trim().to_string()
}

fn parse_pins(raw: &str) -> Vec<String> {
    raw.split([',', '/', ';', ' '])
        .map(normalize_pin)
        .filter(|p| !p.is_empty())
        .collect()
}

fn load(root: &Path) -> Result<WorkbenchConfig, ToolError> {
    WorkbenchConfig::load(root).map_err(|e| ToolError::new(format!("[Pinmap] {e}")))
}

fn save(cfg: &WorkbenchConfig, root: &Path) -> Result<(), ToolError> {
    cfg.save(root)
        .map_err(|e| ToolError::new(format!("[Pinmap] {e}")))
}

/// Display form for an unset owner. Kept as a helper (not an inline
/// if/else inside format! args) so rustfmt versions agree on the layout.
fn owner_display(owner: &str) -> &str {
    if owner.is_empty() { "?" } else { owner }
}

fn required_str(args: &Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ToolError::new(format!("[InvalidInput] missing '{key}'")))
}

fn pins_arg(args: &Value) -> Result<Vec<String>, ToolError> {
    let raw = required_str(args, "pins")?;
    let pins = parse_pins(&raw);
    if pins.is_empty() {
        return Err(ToolError::new(
            "[InvalidInput] no usable pin names in 'pins'",
        ));
    }
    Ok(pins)
}

/// `board` is required wherever a pin is addressed: with multiple boards a
/// bare pin name is ambiguous, and the board name doubles as the MQTT node
/// name so claims stay attributable to physical hardware.
fn board_arg(args: &Value) -> Result<String, ToolError> {
    let board = normalize_board(&required_str(args, "board")?);
    if board.is_empty() {
        return Err(ToolError::new("[InvalidInput] 'board' must not be blank"));
    }
    Ok(board)
}

fn render_board(
    name: &str,
    pins: &std::collections::BTreeMap<String, firment_core::PinEntry>,
) -> String {
    let mut text = format!("## board: {name}\n| pin | func | owner |\n|---|---|---|\n");
    for (pin, entry) in pins {
        text.push_str(&format!(
            "| {pin} | {} | {} |\n",
            entry.func,
            owner_display(&entry.owner)
        ));
    }
    text
}

#[async_trait]
impl Tool for Pinmap {
    fn name(&self) -> &'static str {
        "pinmap"
    }

    fn description(&self) -> &'static str {
        "Project pin/resource allocation registry (.firment/workbench.toml [pinmap.<board>]), \
         scoped PER BOARD. The board name should match the device's MQTT node name \
         (e.g. s3-node-1) so claims are attributable to physical hardware. \
         ALWAYS check here before wiring up any peripheral, and claim the pins you \
         take so other branches (and humans) don't double-allocate them. \
         Actions: boards (list known boards), list [board], check (board+pins+func -> \
         free / taken-by), claim (board+pins+func; refuses on conflict unless force), \
         release (board+pins). `pins` accepts comma/slash separated names like PA9,PA10."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["boards", "list", "check", "claim", "release"], "description": "What to do"},
                "board": {"type": "string", "description": "Board name (= MQTT node name), e.g. s3-node-1. Required for list/check/claim/release."},
                "pins": {"type": "string", "description": "Pin names for check/claim/release, e.g. \"PA9/PA10\" or \"PB6,PB7\""},
                "func": {"type": "string", "description": "Intended function, e.g. \"USART1_TX\" or \"user LED\" (required for check/claim)"},
                "force": {"type": "boolean", "default": false, "description": "claim: overwrite an existing different-func claim instead of refusing"}
            },
            "required": ["action"]
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = args
            .get("action")
            .and_then(|a| a.as_str())
            .unwrap_or("list")
            .to_lowercase();
        let mut cfg = load(&ctx.cwd)?;

        match action.as_str() {
            "boards" => {
                if cfg.pinmap.is_empty() {
                    return Ok(ToolOutput {
                        text: "no boards registered in the pinmap yet.".into(),
                    });
                }
                let boards: Vec<String> = cfg
                    .pinmap
                    .iter()
                    .map(|(b, pins)| format!("{b} ({} pins)", pins.len()))
                    .collect();
                Ok(ToolOutput {
                    text: format!("boards:\n - {}", boards.join("\n - ")),
                })
            }

            "list" => {
                let board = args
                    .get("board")
                    .and_then(|b| b.as_str())
                    .map(normalize_board)
                    .filter(|b| !b.is_empty());
                match board {
                    Some(name) => match cfg.pinmap.get(&name) {
                        Some(pins) if !pins.is_empty() => Ok(ToolOutput {
                            text: render_board(&name, pins),
                        }),
                        _ => Ok(ToolOutput {
                            text: format!("board '{name}' has no claimed pins."),
                        }),
                    },
                    None => {
                        if cfg.pinmap.is_empty() {
                            return Ok(ToolOutput {
                                text: "pinmap is empty — no boards/claims yet.".into(),
                            });
                        }
                        let mut text = String::new();
                        for (name, pins) in &cfg.pinmap {
                            text.push_str(&render_board(name, pins));
                        }
                        Ok(ToolOutput { text })
                    }
                }
            }

            "check" => {
                let board = board_arg(&args)?;
                let func = required_str(&args, "func")?;
                let pins = pins_arg(&args)?;
                let board_pins = cfg.pinmap.get(&board);
                let mut free = Vec::new();
                let mut conflicts = Vec::new();
                let mut same = Vec::new();
                for pin in pins {
                    match board_pins.and_then(|p| p.get(&pin)) {
                        None => free.push(pin),
                        Some(entry) => {
                            if entry.func == func {
                                same.push(pin);
                            } else {
                                conflicts.push(format!(
                                    "{pin}: already claimed as '{}' (owner: {})",
                                    entry.func,
                                    owner_display(&entry.owner)
                                ));
                            }
                        }
                    }
                }
                let mut text = String::new();
                if !conflicts.is_empty() {
                    text.push_str(&format!(
                        "⚠ CONFLICT on board '{board}':\n - {}\n",
                        conflicts.join("\n - ")
                    ));
                }
                if !same.is_empty() {
                    text.push_str(&format!(
                        "same func already registered: {}\n",
                        same.join(", ")
                    ));
                }
                if !free.is_empty() {
                    text.push_str(&format!("free: {}\n", free.join(", ")));
                }
                Ok(ToolOutput { text })
            }

            "claim" => {
                let board = board_arg(&args)?;
                let func = required_str(&args, "func")?;
                let pins = pins_arg(&args)?;
                let force = args.get("force").and_then(|f| f.as_bool()).unwrap_or(false);
                let owner = args
                    .get("owner")
                    .and_then(|o| o.as_str())
                    .unwrap_or("agent")
                    .to_string();
                // Conflict guard within THIS board: refuse when any requested
                // pin is claimed by a DIFFERENT function and force was not
                // set. Same-func re-claims are idempotent.
                let board_pins = cfg.pinmap.get(&board);
                let mut conflicts = Vec::new();
                for pin in &pins {
                    if let Some(entry) = board_pins.and_then(|p| p.get(pin))
                        && entry.func != func
                        && !force
                    {
                        conflicts.push(format!(
                            "{pin} is already '{}' (owner: {}); pass force=true to overwrite",
                            entry.func,
                            owner_display(&entry.owner)
                        ));
                    }
                }
                if !conflicts.is_empty() {
                    return Err(ToolError::new(format!(
                        "[Conflict] {}\nRun pinmap list first; pick free pins or use force deliberately.",
                        conflicts.join("; ")
                    )));
                }
                let board_pins = cfg.pinmap.entry(board.clone()).or_default();
                for pin in &pins {
                    board_pins.insert(
                        pin.clone(),
                        firment_core::PinEntry {
                            func: func.clone(),
                            owner: owner.clone(),
                        },
                    );
                }
                let total: usize = cfg
                    .pinmap
                    .values()
                    .map(std::collections::BTreeMap::len)
                    .sum();
                save(&cfg, &ctx.cwd)?;
                Ok(ToolOutput {
                    text: format!(
                        "claimed on '{board}': {} -> {} ({} pins total across boards)",
                        pins.join(", "),
                        func,
                        total
                    ),
                })
            }

            "release" => {
                let board = board_arg(&args)?;
                let pins = pins_arg(&args)?;
                let mut released = Vec::new();
                if let Some(board_pins) = cfg.pinmap.get_mut(&board) {
                    for pin in &pins {
                        if board_pins.remove(pin).is_some() {
                            released.push(pin.clone());
                        }
                    }
                    if board_pins.is_empty() {
                        cfg.pinmap.remove(&board);
                    }
                }
                save(&cfg, &ctx.cwd)?;
                Ok(ToolOutput {
                    text: format!(
                        "released on '{board}': {}",
                        if released.is_empty() {
                            "(none of those were claimed)".into()
                        } else {
                            released.join(", ")
                        }
                    ),
                })
            }

            other => Err(ToolError::new(format!(
                "[InvalidInput] unknown action '{other}' (boards/list/check/claim/release)"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal};
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    fn ctx(dir: &Path) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(EditJournal::new(dir.join("undo")))),
            verify_command: None,
            symbols_backend: None,
            build_command: None,
            default_chip: None,
            monitor_port: None,
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        }
    }

    #[tokio::test]
    async fn board_scoped_claim_conflict_force_release_roundtrip() {
        let dir = tempdir().unwrap();

        // claim on board A and board B: same pin name, different funcs —
        // the whole point of the hierarchy.
        Pinmap.run(json!({"action": "claim", "board": "s3-node-1", "pins": "pa9/pa10", "func": "USART1"}), &ctx(dir.path())).await.unwrap();
        Pinmap
            .run(
                json!({"action": "claim", "board": "stm32-main", "pins": "PA9", "func": "PWM"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        // boards action lists both.
        let out = Pinmap
            .run(json!({"action": "boards"}), &ctx(dir.path()))
            .await
            .unwrap();
        assert!(
            out.text.contains("s3-node-1") && out.text.contains("stm32-main"),
            "got: {}",
            out.text
        );

        // check is board-scoped: PA9 is PWM on stm32-main but USART1 on s3.
        let out = Pinmap
            .run(
                json!({"action": "check", "board": "s3-node-1", "pins": "PA9", "func": "USART1"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(out.text.contains("already registered"), "got: {}", out.text);
        let out = Pinmap
            .run(
                json!({"action": "check", "board": "stm32-main", "pins": "PA9", "func": "USART1"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(out.text.contains("CONFLICT"), "got: {}", out.text);

        // conflict claim refused without force; force overwrites.
        let err = Pinmap
            .run(
                json!({"action": "claim", "board": "stm32-main", "pins": "PA9", "func": "ADC"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("[Conflict]"), "got: {}", err.message);
        Pinmap.run(json!({"action": "claim", "board": "stm32-main", "pins": "PA9", "func": "ADC", "force": true}), &ctx(dir.path())).await.unwrap();

        // persisted nested under the right boards.
        let cfg = WorkbenchConfig::load(dir.path()).unwrap();
        assert_eq!(cfg.pinmap["s3-node-1"]["PA9"].func, "USART1");
        assert_eq!(cfg.pinmap["stm32-main"]["PA9"].func, "ADC");

        // release empties the board and removes it.
        Pinmap
            .run(
                json!({"action": "release", "board": "s3-node-1", "pins": "pa9, pa10"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        let cfg = WorkbenchConfig::load(dir.path()).unwrap();
        assert!(!cfg.pinmap.contains_key("s3-node-1"));
        assert!(cfg.pinmap.contains_key("stm32-main"));
    }

    #[tokio::test]
    async fn board_is_required_for_pin_actions() {
        let dir = tempdir().unwrap();
        let err = Pinmap
            .run(
                json!({"action": "claim", "pins": "PA5", "func": "LED"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("missing 'board'"),
            "got: {}",
            err.message
        );

        let err = Pinmap
            .run(
                json!({"action": "claim", "board": "  ", "pins": "PA5", "func": "LED"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("'board'"), "got: {}", err.message);
    }

    /// Regression for the schema migration: legacy flat [pinmap] files
    /// (pin -> entry) must land on the "default" board and keep working.
    #[tokio::test]
    async fn legacy_flat_pinmap_migrates_to_default_board() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".firment")).unwrap();
        std::fs::write(
            dir.path().join(".firment/workbench.toml"),
            "[pinmap]\nPA5 = { func = \"LED\", owner = \"user\" }\n",
        )
        .unwrap();

        let out = Pinmap
            .run(json!({"action": "list"}), &ctx(dir.path()))
            .await
            .unwrap();
        assert!(
            out.text.contains("default"),
            "legacy entries land on default board, got: {}",
            out.text
        );
        assert!(out.text.contains("PA5"), "got: {}", out.text);

        // Claiming more pins on the default board coexists with the legacy entry.
        Pinmap
            .run(
                json!({"action": "claim", "board": "default", "pins": "PB7", "func": "SCL"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        let cfg = WorkbenchConfig::load(dir.path()).unwrap();
        assert_eq!(cfg.pinmap["default"]["PA5"].func, "LED");
        assert_eq!(cfg.pinmap["default"]["PB7"].func, "SCL");
    }
}
