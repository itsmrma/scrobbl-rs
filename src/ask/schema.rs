//! Model-facing JSON output contract and JSON Schema for local SQL generation.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SqlAnswerKind {
    Value,
    Table,
    Series,
    Ranking,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ColumnFormat {
    Count,
    DurationMs,
    Date,
    Datetime,
    Percent,
    Text,
    Artist,
    Album,
    Track,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SqlColumnSpec {
    pub name: String,
    pub format: ColumnFormat,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SqlModelOutput {
    pub sql: String,
    pub title: String,
    pub computed: String,
    pub answer_kind: SqlAnswerKind,
    pub columns: Vec<SqlColumnSpec>,
}

/// JSON Schema converted to GBNF to constrain local model SQL generation.
pub fn sql_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["sql", "title", "computed", "answer_kind", "columns"],
        "properties": {
            "sql": { "type": "string" },
            "title": { "type": "string" },
            "computed": { "type": "string" },
            "answer_kind": {
                "type": "string",
                "enum": ["value", "table", "series", "ranking", "unsupported"]
            },
            "columns": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name", "format"],
                    "properties": {
                        "name": { "type": "string" },
                        "format": {
                            "type": "string",
                            "enum": ["count", "duration_ms", "date", "datetime", "percent", "text", "artist", "album", "track"]
                        }
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_schema_requires_every_contract_field() {
        let schema = sql_schema();
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(
            schema["required"],
            json!(["sql", "title", "computed", "answer_kind", "columns"])
        );
        let sample = json!({
            "sql": "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1",
            "title": "Total plays",
            "computed": "Counted plays across all time.",
            "answer_kind": "value",
            "columns": [{ "name": "plays", "format": "count" }]
        });
        let parsed: SqlModelOutput = serde_json::from_value(sample).unwrap();
        assert_eq!(parsed.answer_kind, SqlAnswerKind::Value);
        assert_eq!(parsed.columns.len(), 1);
    }
}
