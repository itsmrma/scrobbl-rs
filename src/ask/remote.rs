//! Bring your own AI: interpret a question with an OpenAI-compatible server.
//!
//! The owner may point Ask at a server they already run (Ollama, LM Studio,
//! llama.cpp, vLLM, ...) or at a hosted API that speaks the same
//! `/chat/completions` protocol (OpenAI, OpenRouter, Groq, Together, ...).
//! Exactly what the local model gets is sent: the system prompt with the
//! vocabulary and the question text. The archive never leaves the instance and
//! the reply is only ever the same schema-checked JSON the local model emits,
//! which Rust turns into the DSL and executes itself.
use crate::{
    ask::{parser::Previous, prompt, schema},
    error::{AppError, Result},
};
use reqwest::{StatusCode, Url};
use serde_json::{Value, json};
use std::time::Duration;

/// A hosted model may take longer than a local one to answer.
const MIN_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_ERROR_CHARS: usize = 200;

#[derive(Clone, Debug)]
pub struct RemoteConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
}

/// How strictly the server is asked to shape its reply. Servers that do not
/// support a mode answer 400/422, and the next, looser mode is tried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    JsonSchema,
    JsonObject,
    Plain,
}

const MODES: [Mode; 3] = [Mode::JsonSchema, Mode::JsonObject, Mode::Plain];

/// Validate a base URL and return the chat-completions endpoint.
///
/// The URL is chosen by the signed-in owner and may be on the private network,
/// so only the scheme is restricted. Credentials in the URL are refused: the
/// API key has its own field and is never echoed back.
pub fn endpoint(base: &str) -> Result<Url> {
    let trimmed = base.trim().trim_end_matches('/');
    let mut url = Url::parse(trimmed).map_err(|_| {
        AppError::BadRequest(
            "Enter the AI server address as a full URL, for example http://192.168.1.20:11434/v1."
                .into(),
        )
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AppError::BadRequest(
            "The AI server address must start with http:// or https://.".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::BadRequest(
            "Do not put a password in the address. Use the API key field.".into(),
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    let path = url.path().trim_end_matches('/').to_owned();
    if !path.ends_with("/chat/completions") {
        url.set_path(&format!("{path}/chat/completions"));
    }
    Ok(url)
}

/// The user turn: the question, preceded by the previous question and its
/// interpretation (never its results) when this is a follow-up.
fn user_message(question: &str, previous: Option<&Previous>) -> String {
    let question = question.trim();
    let Some(previous) = previous.filter(|p| !p.interpretation.is_empty()) else {
        return question.to_owned();
    };
    let interpretation = previous
        .interpretation
        .iter()
        .map(|(label, value)| format!("{label}: {value}"))
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "Previous question: {}
Previous interpretation: {interpretation}
New question: {question}",
        previous.question.trim()
    )
}

fn body(config: &RemoteConfig, question: &str, previous: Option<&Previous>, mode: Mode) -> Value {
    let mut body = json!({
        "model": config.model,
        "temperature": 0,
        "max_tokens": 1200,
        "messages": [
            {"role": "system", "content": prompt::remote_system_prompt()},
            {"role": "user", "content": user_message(question, previous)},
        ],
    });
    match mode {
        Mode::JsonSchema => {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "scrobbl_ask",
                    "strict": true,
                    "schema": schema::advanced_schema(),
                },
            });
        }
        Mode::JsonObject => body["response_format"] = json!({"type": "json_object"}),
        Mode::Plain => {}
    }
    body
}

/// Pull the JSON object out of a reply that may carry reasoning blocks or a
/// Markdown fence around it.
pub fn extract_json(content: &str) -> Option<String> {
    // Reasoning blocks may contain braces: drop them before looking for JSON.
    let mut text = content.to_owned();
    while let Some(start) = text.find("<think>") {
        match text[start..].find("</think>") {
            Some(end) => text.replace_range(start..start + end + "</think>".len(), ""),
            None => text.truncate(start),
        }
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| text[start..=end].to_owned())
}

fn short(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= MAX_ERROR_CHARS {
        return text.to_owned();
    }
    let cut: String = text.chars().take(MAX_ERROR_CHARS).collect();
    format!("{cut}…")
}

fn server_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("error"))
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .map(short)
        })
        .unwrap_or_else(|| short(body))
}

/// Ask the server to interpret `question`. Returns the JSON text of the reply.
pub async fn infer(
    config: &RemoteConfig,
    question: &str,
    previous: Option<&Previous>,
    timeout: Duration,
) -> Result<String> {
    let url = endpoint(&config.base_url)?;
    let host = url.host_str().unwrap_or("the AI server").to_owned();
    let client = reqwest::Client::builder()
        .timeout(timeout.max(MIN_TIMEOUT))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|_| AppError::Internal)?;

    let mut last_problem = String::new();
    for (index, mode) in MODES.iter().enumerate() {
        let mut request = client
            .post(url.clone())
            .json(&body(config, question, previous, *mode));
        if let Some(key) = config.api_key.as_deref().filter(|key| !key.is_empty()) {
            request = request.bearer_auth(key);
        }
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                AppError::BadRequest(format!(
                    "The AI server at {host} did not answer in time. Try again."
                ))
            } else {
                AppError::BadRequest(format!(
                    "Could not reach the AI server at {host}. Check the address and that the server is running."
                ))
            }
        })?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if status.is_success() {
            let value: Value = serde_json::from_str(&text).map_err(|_| {
                AppError::BadRequest(
                    "The AI server did not return an OpenAI-compatible reply. Check the address ends in /v1."
                        .into(),
                )
            })?;
            let content = value
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return extract_json(content).ok_or_else(|| {
                AppError::BadRequest(
                    "The AI model did not return an interpretation. Try rephrasing the question or choose a more capable model."
                        .into(),
                )
            });
        }
        let message = server_message(&text);
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::BadRequest(
                    "The AI server refused the API key. Check the key in Settings.".into(),
                ));
            }
            StatusCode::NOT_FOUND => {
                return Err(AppError::BadRequest(format!(
                    "The AI server at {host} has no such endpoint or model ({message}). Check the address and model name."
                )));
            }
            StatusCode::TOO_MANY_REQUESTS => {
                return Err(AppError::BadRequest(
                    "The AI server is rate limiting requests. Try again in a moment.".into(),
                ));
            }
            // An unsupported response_format: retry with a looser one.
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY
                if index + 1 < MODES.len() =>
            {
                last_problem = message;
            }
            _ => {
                let detail = if message.is_empty() {
                    status.to_string()
                } else {
                    message
                };
                return Err(AppError::BadRequest(format!(
                    "The AI server could not answer: {detail}."
                )));
            }
        }
    }
    Err(AppError::BadRequest(format!(
        "The AI server rejected the request: {last_problem}. Check the model name."
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_appends_chat_completions_once() {
        for base in [
            "http://192.168.1.20:11434/v1",
            "http://192.168.1.20:11434/v1/",
            "http://192.168.1.20:11434/v1/chat/completions",
        ] {
            assert_eq!(
                endpoint(base).unwrap().as_str(),
                "http://192.168.1.20:11434/v1/chat/completions"
            );
        }
        assert_eq!(
            endpoint("https://api.example.com").unwrap().as_str(),
            "https://api.example.com/chat/completions"
        );
    }

    #[test]
    fn endpoint_refuses_other_schemes_and_embedded_credentials() {
        assert!(endpoint("ftp://host/v1").is_err());
        assert!(endpoint("file:///etc/passwd").is_err());
        assert!(endpoint("not a url").is_err());
        assert!(endpoint("http://user:secret@host/v1").is_err());
    }

    #[test]
    fn extract_json_handles_fences_and_reasoning() {
        assert_eq!(extract_json("{\"a\":1}").as_deref(), Some("{\"a\":1}"));
        assert_eq!(
            extract_json("```json\n{\"a\":1}\n```").as_deref(),
            Some("{\"a\":1}")
        );
        assert_eq!(
            extract_json("<think>maybe {\"x\":0}</think>\n{\"a\":1}").as_deref(),
            Some("{\"a\":1}")
        );
        assert_eq!(extract_json("no json here"), None);
    }
}
