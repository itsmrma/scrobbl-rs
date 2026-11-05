//! Loads the fixed evaluation corpus.
//!
//! Cases are deliberately authored by hand: the spike measures the runtime
//! and the model on realistic EN/IT questions, not parser accuracy at scale
//! (that is the eval artifact of a later ticket).

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Case {
    pub id: String,
    pub lang: String,
    pub question: String,
    pub expected: Expected,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Expected {
    pub intent: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub period: Option<String>,
    #[serde(default)]
    pub weekday: Option<String>,
    #[serde(default)]
    pub limit: Option<String>,
}

impl Expected {
    /// Compares the model output with the expected interpretation.
    ///
    /// For `unsupported` only the intent matters: the remaining required
    /// fields are grammar padding, not part of the rejection.
    pub fn matches(&self, actual: &Value) -> bool {
        let Some(object) = actual.as_object() else {
            return false;
        };
        if object.get("intent").and_then(Value::as_str) != Some(self.intent.as_str()) {
            return false;
        }
        if self.intent == "unsupported" {
            return true;
        }

        let checks = [
            ("target", self.target.as_deref()),
            ("name", self.name.as_deref()),
            ("period", self.period.as_deref()),
            ("weekday", self.weekday.as_deref()),
            ("limit", self.limit.as_deref()),
        ];
        checks.iter().all(|(key, expected)| {
            let Some(expected) = expected else {
                return true;
            };
            object.get(*key).and_then(Value::as_str) == Some(*expected)
        })
    }
}

pub fn load(path: &Path) -> Result<Vec<Case>> {
    let text =
        fs::read_to_string(path).with_context(|| format!("reading corpus {}", path.display()))?;
    parse(&text)
}

pub fn parse(text: &str) -> Result<Vec<Case>> {
    let mut cases = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let case =
            serde_json::from_str(line).with_context(|| format!("corpus line {}", index + 1))?;
        cases.push(case);
    }
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::parse;

    const CORPUS: &str = include_str!("../corpus/questions.jsonl");

    #[test]
    fn corpus_has_balanced_languages_and_unique_ids() {
        let cases = parse(CORPUS).unwrap();
        assert_eq!(cases.len(), 30, "corpus size changed");
        assert_eq!(cases.iter().filter(|case| case.lang == "en").count(), 15);
        assert_eq!(cases.iter().filter(|case| case.lang == "it").count(), 15);

        let mut ids: Vec<&str> = cases.iter().map(|case| case.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), cases.len(), "duplicate corpus ids");
    }

    #[test]
    fn corpus_covers_unicode_and_special_characters() {
        let cases = parse(CORPUS).unwrap();
        let all: String = cases
            .iter()
            .map(|case| case.question.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for needle in [
            "Björk",
            "坂本龍一",
            "Blink-182",
            "Don't Stop Me Now",
            "Sigur Rós",
        ] {
            assert!(all.contains(needle), "corpus lost {needle:?}");
        }
    }

    #[test]
    fn unsupported_only_checks_intent() {
        let cases = parse(CORPUS).unwrap();
        let case = cases.iter().find(|case| case.id == "en-015").unwrap();
        assert!(case.expected.matches(&json!({"intent": "unsupported"})));
        assert!(!case.expected.matches(&json!({"intent": "count_plays"})));
    }

    #[test]
    fn compares_every_slot() {
        let cases = parse(CORPUS).unwrap();
        let case = cases.iter().find(|case| case.id == "en-001").unwrap();
        assert!(case.expected.matches(&json!({
            "intent": "count_plays",
            "target": "artist",
            "name": "Björk",
            "period": "all_time",
            "weekday": "any",
            "limit": "1"
        })));
        assert!(!case.expected.matches(&json!({
            "intent": "count_plays",
            "target": "artist",
            "name": "Bjork",
            "period": "all_time",
            "weekday": "any",
            "limit": "1"
        })));
    }
}
