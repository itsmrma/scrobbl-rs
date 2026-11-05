//! Validator for the same JSON-Schema subset used by the grammar generator.
//!
//! The grammar already makes invalid output *impossible* when it works, so
//! this validator is mostly a guard against a broken runtime or a wrong
//! grammar. It also gives the report a machine-readable pass/fail per case.

use anyhow::{Result, bail};
use serde_json::Value;

pub fn validate(value: &Value, schema: &Value) -> Result<()> {
    validate_at(value, schema, "$")
}

fn validate_at(value: &Value, schema: &Value, path: &str) -> Result<()> {
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        if !allowed.iter().any(|candidate| candidate == value) {
            bail!("{path}: {value} is not one of {allowed:?}");
        }
        return Ok(());
    }

    match schema.get("type").and_then(Value::as_str) {
        Some("object") => {
            let object = value
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("{path}: expected object"))?;

            if let Some(required) = schema.get("required").and_then(Value::as_array) {
                for key in required.iter().filter_map(Value::as_str) {
                    if !object.contains_key(key) {
                        bail!("{path}: missing required property {key:?}");
                    }
                }
            }

            let properties = schema.get("properties").and_then(Value::as_object);
            let additional = schema
                .get("additionalProperties")
                .and_then(Value::as_bool)
                .unwrap_or(true);

            for (key, child) in object {
                match properties.and_then(|properties| properties.get(key)) {
                    Some(child_schema) => {
                        validate_at(child, child_schema, &format!("{path}.{key}"))?;
                    }
                    None if !additional => bail!("{path}: unexpected property {key:?}"),
                    None => {}
                }
            }
        }
        Some("string") => {
            if !value.is_string() {
                bail!("{path}: expected string, got {value}");
            }
        }
        Some("integer") => {
            if value.as_i64().is_none() && value.as_u64().is_none() {
                bail!("{path}: expected integer, got {value}");
            }
        }
        Some("boolean") => {
            if !value.is_boolean() {
                bail!("{path}: expected boolean, got {value}");
            }
        }
        Some(other) => bail!("{path}: unsupported schema type {other:?}"),
        None => bail!("{path}: schema node has neither type nor enum"),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::validate;

    fn schema() -> serde_json::Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["intent", "name"],
            "properties": {
                "intent": {"enum": ["count_plays", "unsupported"]},
                "name": {"type": "string"}
            }
        })
    }

    #[test]
    fn accepts_unicode_strings() {
        let value = json!({"intent": "count_plays", "name": "坂本龍一"});
        validate(&value, &schema()).unwrap();
    }

    #[test]
    fn rejects_missing_required() {
        let value = json!({"intent": "count_plays"});
        let error = validate(&value, &schema()).unwrap_err().to_string();
        assert!(error.contains("missing required property"), "{error}");
    }

    #[test]
    fn rejects_unknown_property() {
        let value = json!({"intent": "count_plays", "name": "", "sql": "DROP TABLE"});
        let error = validate(&value, &schema()).unwrap_err().to_string();
        assert!(error.contains("unexpected property"), "{error}");
    }

    #[test]
    fn rejects_value_outside_enum() {
        let value = json!({"intent": "drop_table", "name": ""});
        let error = validate(&value, &schema()).unwrap_err().to_string();
        assert!(error.contains("not one of"), "{error}");
    }
}
