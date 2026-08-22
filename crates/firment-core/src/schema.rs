use serde_json::Value;

/// Validate tool call arguments against the tool's JSON Schema
/// (`type: object`, `properties`, `required`, `minimum`/`maximum`, `items`,
/// nested objects, `oneOf`).
/// Returns a tagged [InvalidInput] error message on the first violation.
pub fn validate_args(tool: &str, schema: &Value, args: &Value) -> Result<(), String> {
    if schema.get("type").and_then(|t| t.as_str()) != Some("object") {
        return Ok(());
    }
    if !args.is_object() {
        return Err(format!(
            "[InvalidInput] {tool} argument validation failed: arguments must be an object"
        ));
    }
    validate_object_fields(tool, schema, args)?;

    // oneOf is EXACTLY-one over `required`-based branches. Branches without a
    // `required` list are not candidates (they used to auto-satisfy every
    // object, making the whole oneOf a no-op); two matching branches violate
    // exclusivity.
    if let Some(alternatives) = schema.get("oneOf").and_then(|o| o.as_array()) {
        let satisfied = alternatives
            .iter()
            .filter(|alt| alt.get("required").and_then(|r| r.as_array()).is_some())
            .filter(|alt| {
                alt.get("required")
                    .and_then(|r| r.as_array())
                    .map(|required| {
                        required.iter().all(|n| {
                            n.as_str()
                                .map(|name| args.get(name).is_some())
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
            .count();
        match satisfied {
            0 => {
                return Err(format!(
                    "[InvalidInput] {tool} argument validation failed: oneOf condition not met \
                     (e.g. edit_file requires old_text or start_line)"
                ));
            }
            n if n > 1 => {
                return Err(format!(
                    "[InvalidInput] {tool} argument validation failed: oneOf branches are \
                     mutually exclusive but {n} were satisfied"
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Shared required/properties walk for top-level argument objects and nested
/// object fields.
fn validate_object_fields(tool: &str, schema: &Value, args: &Value) -> Result<(), String> {
    if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
        for name in required {
            let name = name.as_str().unwrap_or_default();
            if args.get(name).is_none() {
                return Err(format!(
                    "[InvalidInput] {tool} argument validation failed: missing required field '{name}'"
                ));
            }
        }
    }
    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        for (name, prop) in props {
            if let Some(value) = args.get(name) {
                validate_value(tool, name, prop, value)?;
            }
        }
    }
    Ok(())
}

fn validate_value(tool: &str, name: &str, prop: &Value, value: &Value) -> Result<(), String> {
    if let Some(expected) = prop.get("type").and_then(|t| t.as_str()) {
        let ok = match expected {
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "object" => value.is_object(),
            "array" => value.is_array(),
            _ => true,
        };
        if !ok {
            return Err(format!(
                "[InvalidInput] {tool} argument validation failed: field '{name}' should be {expected}"
            ));
        }
    }
    // Numeric bounds compare as f64 so fractional limits and fractional
    // values are actually checked (as_i64 silently skipped both).
    if let Some(min) = prop.get("minimum").and_then(|m| m.as_f64()) {
        let below = value.as_f64().map(|v| v < min).unwrap_or(false);
        if below {
            return Err(format!(
                "[InvalidInput] {tool} argument validation failed: field '{name}' cannot be less than {min}"
            ));
        }
    }
    if let Some(max) = prop.get("maximum").and_then(|m| m.as_f64()) {
        let above = value.as_f64().map(|v| v > max).unwrap_or(false);
        if above {
            return Err(format!(
                "[InvalidInput] {tool} argument validation failed: field '{name}' cannot be greater than {max}"
            ));
        }
    }
    if let Some(items_schema) = prop.get("items")
        && let Some(array) = value.as_array()
    {
        for (idx, item) in array.iter().enumerate() {
            validate_value(tool, &format!("{name}[{idx}]"), items_schema, item)?;
        }
    }
    if value.is_object() {
        validate_object_fields(tool, prop, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_missing_required() {
        let schema = json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"]
        });
        let err = validate_args("read_file", &schema, &json!({})).unwrap_err();
        assert!(err.contains("[InvalidInput]"));
        assert!(err.contains("path"));
    }

    #[test]
    fn rejects_wrong_type() {
        let schema = json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"]
        });
        let err = validate_args("read_file", &schema, &json!({"path": 42})).unwrap_err();
        assert!(err.contains("should be string"));
    }

    #[test]
    fn rejects_below_minimum() {
        let schema = json!({
            "type": "object",
            "properties": {"limit": {"type": "integer", "minimum": 1}},
            "required": []
        });
        let err = validate_args("read_file", &schema, &json!({"limit": 0})).unwrap_err();
        assert!(err.contains("cannot be less than 1"));
    }

    #[test]
    fn minimum_works_for_float_limits_and_values() {
        // Fractional limit + fractional value: both used to slip through
        // as_i64-based comparison.
        let schema = json!({
            "type": "object",
            "properties": {"gain": {"type": "number", "minimum": 0.5}}
        });
        assert!(validate_args("amp", &schema, &json!({"gain": 0.25})).is_err());
        assert!(validate_args("amp", &schema, &json!({"gain": 0.75})).is_ok());
    }

    #[test]
    fn maximum_is_enforced() {
        let schema = json!({
            "type": "object",
            "properties": {"limit": {"type": "integer", "maximum": 100}}
        });
        assert!(validate_args("read_file", &schema, &json!({"limit": 500})).is_err());
        assert!(validate_args("read_file", &schema, &json!({"limit": 50})).is_ok());
    }

    #[test]
    fn array_items_are_validated() {
        let schema = json!({
            "type": "object",
            "properties": {
                "pins": {"type": "array", "items": {"type": "integer"}}
            }
        });
        assert!(validate_args("board", &schema, &json!({"pins": [1, 2, 3]})).is_ok());
        let err = validate_args("board", &schema, &json!({"pins": [1, "x"]})).unwrap_err();
        assert!(err.contains("pins[1]"), "got: {err}");
    }

    #[test]
    fn oneof_requires_at_least_one_branch() {
        let schema = json!({
            "type": "object",
            "properties": {
                "old_text": {"type": "string"},
                "start_line": {"type": "integer"}
            },
            "oneOf": [
                {"required": ["old_text"]},
                {"required": ["start_line"]}
            ]
        });
        assert!(validate_args("edit_file", &schema, &json!({})).is_err());
        assert!(validate_args("edit_file", &schema, &json!({"old_text": "x"})).is_ok());
        assert!(validate_args("edit_file", &schema, &json!({"start_line": 3})).is_ok());
    }

    #[test]
    fn oneof_is_exclusive_and_branches_without_required_do_not_autopass() {
        let schema = json!({
            "type": "object",
            "properties": {
                "old_text": {"type": "string"},
                "start_line": {"type": "integer"},
                "note": {"type": "string"}
            },
            "oneOf": [
                {"required": ["old_text"]},
                {"required": ["start_line"]},
                // No `required`: previously auto-satisfied any arguments,
                // making the whole oneOf unenforceable.
                {"properties": {"note": {"type": "string"}}}
            ]
        });
        // Both exclusive branches at once -> error.
        assert!(
            validate_args(
                "edit_file",
                &schema,
                &json!({"old_text": "x", "start_line": 3})
            )
            .is_err()
        );
        // Exactly one -> fine.
        assert!(validate_args("edit_file", &schema, &json!({"old_text": "x"})).is_ok());
        // Properties-only branches are IGNORED (deliberate pragmatic subset:
        // treating them as always-satisfied candidates makes the branch list
        // unenforceable), so arguments matching none of the required-based
        // branches still fail.
        assert!(validate_args("edit_file", &schema, &json!({"note": "n"})).is_err());
    }

    #[test]
    fn accepts_valid_args() {
        let schema = json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"]
        });
        assert!(validate_args("read_file", &schema, &json!({"path": "a.txt"})).is_ok());
    }
}
