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

/// Display form for an unset owner. Kept as a helper (not an inline
/// if/else inside format! args) so rustfmt versions agree on the layout.
fn owner_display(owner: &str) -> &str {
    if owner.is_empty() { "?" } else { owner }
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

#[async_trait]
impl Tool for Pinmap {
    fn name(&self) -> &'static str {
        "pinmap"
    }

    fn description(&self) -> &'static str {
        "Project pin/resource allocation registry (.firment/workbench.toml [pinmap]). \
         ALWAYS check here before wiring up any peripheral, and claim the pins you \
         take so other branches (and humans) don't double-allocate them. \
         Actions: list (show every claim), check (pins+func -> free / taken-by), \
         claim (register pins for a function; refuses on conflict unless force), \
         release (free pins). `pins` accepts comma/slash separated names like PA9,PA10."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "check", "claim", "release"], "description": "What to do"},
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
            "list" => {
                if cfg.pinmap.is_empty() {
                    return Ok(ToolOutput {
                        text: "pinmap is empty — no pins claimed yet in this project.".into(),
                    });
                }
                let mut text = String::from("| pin | func | owner |\n|---|---|---|\n");
                for (pin, entry) in &cfg.pinmap {
                    text.push_str(&format!("| {pin} | {} | {} |\n", entry.func, entry.owner));
                }
                Ok(ToolOutput { text })
            }

            "check" => {
                let func = required_str(&args, "func")?;
                let pins = pins_arg(&args)?;
                let mut free = Vec::new();
                let mut conflicts = Vec::new();
                let mut same = Vec::new();
                for pin in pins {
                    match cfg.pinmap.get(&pin) {
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
                    text.push_str(&format!("⚠ CONFLICT:\n - {}\n", conflicts.join("\n - ")));
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
                let func = required_str(&args, "func")?;
                let pins = pins_arg(&args)?;
                let force = args.get("force").and_then(|f| f.as_bool()).unwrap_or(false);
                let owner = args
                    .get("owner")
                    .and_then(|o| o.as_str())
                    .unwrap_or("agent")
                    .to_string();
                // Conflict guard: refuse when ANY requested pin is claimed by
                // a DIFFERENT function and force was not set. Same-func
                // re-claims are idempotent (owner may be refreshed).
                let mut conflicts = Vec::new();
                for pin in &pins {
                    if let Some(entry) = cfg.pinmap.get(pin)
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
                for pin in &pins {
                    cfg.pinmap.insert(
                        pin.clone(),
                        firment_core::PinEntry {
                            func: func.clone(),
                            owner: owner.clone(),
                        },
                    );
                }
                cfg.save(&ctx.cwd)
                    .map_err(|e| ToolError::new(format!("[Pinmap] {e}")))?;
                Ok(ToolOutput {
                    text: format!(
                        "claimed {}: {} ({} total pins registered)",
                        pins.join(", "),
                        func,
                        cfg.pinmap.len()
                    ),
                })
            }

            "release" => {
                let pins = pins_arg(&args)?;
                let mut released = Vec::new();
                for pin in &pins {
                    if cfg.pinmap.remove(pin).is_some() {
                        released.push(pin.clone());
                    }
                }
                cfg.save(&ctx.cwd)
                    .map_err(|e| ToolError::new(format!("[Pinmap] {e}")))?;
                Ok(ToolOutput {
                    text: format!(
                        "released: {} ({} total pins registered)",
                        if released.is_empty() {
                            "(none of those were claimed)".into()
                        } else {
                            released.join(", ")
                        },
                        cfg.pinmap.len()
                    ),
                })
            }

            other => Err(ToolError::new(format!(
                "[InvalidInput] unknown action '{other}' (list/check/claim/release)"
            ))),
        }
    }
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
    async fn claim_check_conflict_force_release_roundtrip() {
        let dir = tempdir().unwrap();

        // Empty project lists clean.
        let out = Pinmap
            .run(json!({"action": "list"}), &ctx(dir.path()))
            .await
            .unwrap();
        assert!(out.text.contains("empty"), "got: {}", out.text);

        // Claim two pins (slash + space separators, lowercase input).
        let out = Pinmap
            .run(
                json!({"action": "claim", "pins": "pa9/pa10", "func": "USART1"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(
            out.text.contains("PA9, PA10"),
            "normalized claim, got: {}",
            out.text
        );

        // Check: same func is idempotent-friendly, different func conflicts.
        let out = Pinmap
            .run(
                json!({"action": "check", "pins": "PA9", "func": "USART1"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(out.text.contains("already registered"), "got: {}", out.text);
        let out = Pinmap
            .run(
                json!({"action": "check", "pins": "PA9 PB7", "func": "PWM"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(
            out.text.contains("CONFLICT") && out.text.contains("free: PB7"),
            "got: {}",
            out.text
        );

        // Claiming a conflicting pin without force must FAIL.
        let err = Pinmap
            .run(
                json!({"action": "claim", "pins": "PA9", "func": "PWM"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("[Conflict]"), "got: {}", err.message);

        // force=true overwrites.
        let out = Pinmap
            .run(
                json!({"action": "claim", "pins": "PA9", "func": "PWM", "force": true}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(out.text.contains("claimed PA9"), "got: {}", out.text);

        // Persisted to disk with normalized keys.
        let cfg = WorkbenchConfig::load(dir.path()).unwrap();
        assert_eq!(cfg.pinmap["PA9"].func, "PWM");
        assert_eq!(cfg.pinmap["PA10"].func, "USART1");

        // Release frees them again.
        let out = Pinmap
            .run(
                json!({"action": "release", "pins": "pa9, pa10"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(
            out.text.contains("released: PA9, PA10"),
            "got: {}",
            out.text
        );
        let cfg = WorkbenchConfig::load(dir.path()).unwrap();
        assert!(cfg.pinmap.is_empty());
    }

    #[tokio::test]
    async fn missing_args_are_rejected() {
        let dir = tempdir().unwrap();
        let err = Pinmap
            .run(json!({"action": "claim", "pins": "PA5"}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("missing 'func'"),
            "got: {}",
            err.message
        );

        let err = Pinmap
            .run(json!({"action": "check", "func": "x"}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(err.message.contains("'pins'"), "got: {}", err.message);
    }
}
