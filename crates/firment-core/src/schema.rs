use serde_json::Value;

/// Validate tool call arguments against the tool's JSON Schema
/// (`type: object`, `properties`, `required`, `minimum`, `oneOf`).
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
    if let Some(alternatives) = schema.get("oneOf").and_then(|o| o.as_array()) {
        let any_ok = alternatives.iter().any(|alt| {
            alt.get("required")
                .and_then(|r| r.as_array())
                .map(|required| {
                    required.iter().all(|n| {
                        n.as_str()
                            .map(|name| args.get(name).is_some())
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(true)
        });
        if !any_ok {
            return Err(format!(
                "[InvalidInput] {tool} argument validation failed: oneOf exclusive condition not \
                 met (e.g. edit_file requires old_text or start_line)"
            ));
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
    if let Some(min) = prop.get("minimum").and_then(|m| m.as_i64()) {
        let below = match value {
            Value::Number(n) => n.as_i64().map(|v| v < min).unwrap_or(false),
            _ => false,
        };
        if below {
            return Err(format!(
                "[InvalidInput] {tool} argument validation failed: field '{name}' cannot be less than {min}"
            ));
        }
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
    fn accepts_valid_args() {
        let schema = json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"]
        });
        assert!(validate_args("read_file", &schema, &json!({"path": "a.txt"})).is_ok());
    }
}
