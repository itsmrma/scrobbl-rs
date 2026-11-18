//! JSON-Schema-subset to GBNF conversion for the Ask local SQL grammar.
use serde_json::Value;
use std::fmt::Write as _;

/// Rules shared by every generated grammar.
///
/// `string` mirrors llama.cpp's built-in JSON grammar: it accepts any Unicode
/// bytes and the standard JSON escape sequences but nothing that could escape
/// the string slot.
const COMMON_RULES: &str = r#"ws ::= " "?
string ::= "\"" ( [^"\\\x7F\x00-\x1F] | "\\" (["\\/bfnrt] | "u" [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F]) )* "\""
integer ::= "-"? ([0-9] | [1-9] [0-9]*)
"#;

pub fn from_sql_schema() -> Result<String, String> {
    to_gbnf(&crate::ask::schema::sql_schema())
}

pub fn to_gbnf(schema: &Value) -> Result<String, String> {
    let mut generator = Generator::default();
    generator.add_rule("root", schema)?;

    let mut out = String::new();
    for (name, body) in &generator.rules {
        writeln!(out, "{name} ::= {body}").map_err(|error| error.to_string())?;
    }
    out.push_str(COMMON_RULES);
    Ok(out)
}

#[derive(Default)]
struct Generator {
    rules: Vec<(String, String)>,
}

impl Generator {
    fn add_rule(&mut self, name: &str, schema: &Value) -> Result<String, String> {
        let name = sanitize(name);
        if self.rules.iter().any(|(existing, _)| existing == &name) {
            return Ok(name);
        }

        // Reserve the name before recursing so a self-referential schema
        // cannot loop forever.
        self.rules.push((name.clone(), String::new()));
        let body = self.body(schema, &name)?;
        let slot = self
            .rules
            .iter_mut()
            .find(|(existing, _)| existing == &name)
            .ok_or_else(|| format!("missing reserved rule {name}"))?;
        slot.1 = body;
        Ok(name)
    }

    fn expr(&mut self, schema: &Value, name: &str) -> Result<String, String> {
        if schema.get("type").and_then(Value::as_str) == Some("object") {
            self.add_rule(name, schema)
        } else {
            self.body(schema, name)
        }
    }

    fn body(&mut self, schema: &Value, name: &str) -> Result<String, String> {
        if let Some(values) = schema.get("enum") {
            let values = values
                .as_array()
                .ok_or_else(|| format!("enum in {name} must be an array"))?;
            if values.is_empty() {
                return Err(format!("empty enum in {name}"));
            }
            let mut parts = Vec::with_capacity(values.len());
            for value in values {
                let Value::String(value) = value else {
                    return Err(format!(
                        "only string enums are supported, got {value} in {name}"
                    ));
                };
                parts.push(json_literal(value));
            }
            return Ok(format!("({})", parts.join(" | ")));
        }

        match schema.get("type").and_then(Value::as_str) {
            Some("string") => Ok("string".to_string()),
            Some("integer") => Ok("integer".to_string()),
            Some("boolean") => Ok("(\"true\" | \"false\")".to_string()),
            Some("array") => self.array_body(schema, name),
            Some("object") => self.object_body(schema, name),
            Some(other) => Err(format!("unsupported schema type {other:?} in {name}")),
            None => Err(format!("schema node {name} has neither type nor enum")),
        }
    }

    fn array_body(&mut self, schema: &Value, name: &str) -> Result<String, String> {
        let items = schema
            .get("items")
            .ok_or_else(|| format!("array {name} has no items"))?;
        let item_name = format!("{name}-item");
        let item = self.expr(items, &item_name)?;
        Ok(format!("\"[\" ws ({item} (ws \",\" ws {item})*)? ws \"]\""))
    }

    fn object_body(&mut self, schema: &Value, name: &str) -> Result<String, String> {
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("object {name} has no properties"))?;

        if schema
            .get("additionalProperties")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            return Err(format!(
                "object {name} must set additionalProperties to false"
            ));
        }

        let required: Vec<&str> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();

        for key in properties.keys() {
            if !required.contains(&key.as_str()) {
                return Err(format!(
                    "optional property {key:?} is not supported; use an explicit enum value instead"
                ));
            }
        }

        let mut parts = Vec::with_capacity(required.len());
        for &key in &required {
            let child = properties.get(key).ok_or_else(|| {
                format!("required property {key:?} is missing from properties in {name}")
            })?;
            let child_name = format!("{name}-{key}");
            let value = self.expr(child, &child_name)?;
            parts.push(format!("{} ws \":\" ws {value}", json_literal(key)));
        }

        let sequence = parts.join(" ws \",\" ws ");
        Ok(format!("\"{{\" ws {sequence} ws \"}}\""))
    }
}

fn sanitize(name: &str) -> String {
    // llama.cpp GBNF rule names only allow ASCII letters, digits, and `-`.
    let mut out: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    if out.is_empty() || out.starts_with(|ch: char| !ch.is_ascii_alphabetic()) {
        out.insert(0, 'r');
    }
    out
}

fn escape_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

/// A GBNF literal that matches a **JSON string**, quotes included.
fn json_literal(value: &str) -> String {
    format!("\"{}\"", escape_literal(&format!("\"{value}\"")))
}

#[cfg(test)]
mod tests {
    use super::{from_sql_schema, to_gbnf};
    use serde_json::json;

    fn trial_schema() -> serde_json::Value {
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

    fn gbnf_json_string(value: &str) -> String {
        format!("\"\\\"{value}\\\"\"")
    }

    #[test]
    fn generates_a_root_rule_and_auxiliary_rules() {
        let grammar = to_gbnf(&trial_schema()).unwrap();
        assert!(grammar.starts_with("root ::= "), "{grammar}");
        assert!(grammar.contains(&gbnf_json_string("intent")), "{grammar}");
        assert!(
            grammar.contains(&gbnf_json_string("count_plays")),
            "{grammar}"
        );
        assert!(
            grammar.contains(&gbnf_json_string("unsupported")),
            "{grammar}"
        );
        assert!(grammar.contains("string ::= "), "{grammar}");
        assert!(grammar.contains("ws ::= "), "{grammar}");
    }

    #[test]
    fn the_string_rule_accepts_any_unicode_byte() {
        let grammar = to_gbnf(&trial_schema()).unwrap();
        assert!(
            grammar.contains("[^\"\\\\\\x7F\\x00-\\x1F]"),
            "the string rule must disallow raw control chars without restricting names to ASCII: {grammar}"
        );
        assert!(!grammar.contains("[a-zA-Z]"), "{grammar}");
    }

    #[test]
    fn rejects_optional_properties() {
        let mut schema = trial_schema();
        schema["required"] = json!(["intent"]);
        let error = to_gbnf(&schema).unwrap_err();
        assert!(error.contains("optional property"), "{error}");
    }

    #[test]
    fn rejects_open_objects() {
        let mut schema = trial_schema();
        schema["additionalProperties"] = json!(true);
        let error = to_gbnf(&schema).unwrap_err();
        assert!(error.contains("additionalProperties"), "{error}");
    }

    #[test]
    fn sql_schema_generates_gbnf_with_free_sql_string_and_columns_array() {
        let grammar = from_sql_schema().unwrap();
        assert!(grammar.starts_with("root ::= "), "{grammar}");
        assert!(
            grammar.contains("\"\\\"sql\\\"\" ws \":\" ws string"),
            "sql field must be a free JSON string: {grammar}"
        );
        assert!(
            grammar.contains("\"\\\"columns\\\"\" ws \":\" ws \"[\""),
            "{grammar}"
        );
        assert!(grammar.contains("root-columns-item ::= "), "{grammar}");
        assert!(
            grammar.contains(&gbnf_json_string("unsupported")),
            "{grammar}"
        );
        assert!(
            grammar.contains(&gbnf_json_string("duration_ms")),
            "{grammar}"
        );
    }

    #[test]
    fn sql_schema_gbnf_preserves_required_field_order() {
        let grammar = from_sql_schema().unwrap();
        let root_line = grammar
            .lines()
            .find(|line| line.starts_with("root ::= "))
            .expect("missing root rule");
        let title_pos = root_line.find(&gbnf_json_string("title")).unwrap();
        let computed_pos = root_line.find(&gbnf_json_string("computed")).unwrap();
        let kind_pos = root_line.find(&gbnf_json_string("answer_kind")).unwrap();
        let sql_pos = root_line.find(&gbnf_json_string("sql")).unwrap();
        let cols_pos = root_line.find(&gbnf_json_string("columns")).unwrap();
        assert!(
            title_pos < computed_pos
                && computed_pos < kind_pos
                && kind_pos < sql_pos
                && sql_pos < cols_pos,
            "unexpected field order in root rule: {root_line}"
        );

        let item_line = grammar
            .lines()
            .find(|line| line.starts_with("root-columns-item ::= "))
            .expect("missing root-columns-item rule");
        let name_pos = item_line.find(&gbnf_json_string("name")).unwrap();
        let format_pos = item_line.find(&gbnf_json_string("format")).unwrap();
        assert!(
            name_pos < format_pos,
            "unexpected field order in column item rule: {item_line}"
        );
    }
}
