//! The optional model. FastMind works without one: Dream merges, links and archives with plain
//! text statistics, and Remember answers with passages from your notes. A model only writes
//! better topic titles, summaries and answers. Offline by default: Local talks to a server on
//! this machine (Ollama, LM Studio, llama.cpp); Claude is used only when chosen.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

mod claude_code;
pub use claude_code::find as find_claude_code;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase", tag = "provider")]
pub enum ModelSettings {
    /// No model: everything runs on text statistics alone.
    #[default]
    None,
    /// A server on this machine that speaks the OpenAI chat-completions protocol.
    Local {
        #[serde(default = "default_local_url")]
        base_url: String,
        #[serde(default = "default_local_model")]
        model: String,
    },
    /// Anthropic's Claude API. The key comes from `ANTHROPIC_API_KEY`.
    Claude {
        #[serde(default = "default_claude_model")]
        model: String,
    },
    /// The user's own Claude Code (`claude`), signed in to their plan: no API key needed. Run
    /// with no tools, no MCP servers and no settings, once per request.
    #[serde(rename = "claude-code")]
    ClaudeCode {
        #[serde(default = "default_claude_code_model")]
        model: String,
    },
}

fn default_claude_code_model() -> String {
    "sonnet".into()
}

/// An image sent with a request: a screenshot or photo the user captured.
#[derive(Clone, Debug)]
pub struct Image {
    pub media_type: String,
    pub data: Vec<u8>,
}

impl Image {
    fn base64(&self) -> String {
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &self.data)
    }
}

fn default_local_url() -> String {
    "http://localhost:11434/v1".into()
}

fn default_local_model() -> String {
    "llama3.2".into()
}

fn default_claude_model() -> String {
    "claude-opus-5-5".into()
}

impl ModelSettings {
    pub fn label(&self) -> String {
        match self {
            ModelSettings::None => "No model (text statistics only)".into(),
            ModelSettings::Local { model, base_url } => format!("Local: {model} at {base_url}"),
            ModelSettings::Claude { model } => format!("Claude: {model}"),
            ModelSettings::ClaudeCode { model } => format!("Claude Code: {model}"),
        }
    }

    /// Short, for a note's `written_by:`: `claude/claude-opus-5-5`, `local/llama3.2`.
    pub fn id(&self) -> String {
        match self {
            ModelSettings::None => "none".into(),
            ModelSettings::Local { model, .. } => format!("local/{model}"),
            ModelSettings::Claude { model } => format!("claude/{model}"),
            ModelSettings::ClaudeCode { model } => format!("claude-code/{model}"),
        }
    }

    pub fn is_on(&self) -> bool {
        !matches!(self, ModelSettings::None)
    }
}

pub struct Model {
    settings: ModelSettings,
    agent: ureq::Agent,
}

impl Model {
    /// The model `settings` describe, or `None` when no model is chosen.
    pub fn from_settings(settings: &ModelSettings) -> Option<Self> {
        if !settings.is_on() {
            return None;
        }
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout(Duration::from_secs(180))
            .build();
        Some(Self {
            settings: settings.clone(),
            agent,
        })
    }

    /// One request, one answer: `system` sets the job, `user` holds the material.
    pub fn complete(&self, system: &str, user: &str) -> Result<String> {
        match &self.settings {
            ModelSettings::None => Err(Error::msg("no model is chosen")),
            ModelSettings::Local { base_url, model } => self.local(base_url, model, system, user),
            ModelSettings::Claude { model } => self.claude(model, system, user),
            ModelSettings::ClaudeCode { model } => claude_code::run(model, system, user, &[], None),
        }
    }

    /// A reply that is one JSON object matching `schema`, with `images` sent before the text.
    /// The JSON comes back as text; a model that can't be held to the schema is asked for it
    /// in the prompt, so the caller still checks what it got.
    pub fn complete_json(
        &self,
        system: &str,
        user: &str,
        images: &[Image],
        schema: &Value,
    ) -> Result<String> {
        match &self.settings {
            ModelSettings::None => Err(Error::msg("no model is chosen")),
            ModelSettings::Local { base_url, model } => {
                let mut content = vec![json!({"type": "text", "text": user})];
                for image in images {
                    content.push(json!({
                        "type": "image_url",
                        "image_url": {"url": format!("data:{};base64,{}", image.media_type, image.base64())}
                    }));
                }
                let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
                let body = json!({
                    "model": model,
                    "messages": [
                        {"role": "system", "content": system},
                        {"role": "user", "content": if images.is_empty() { json!(user) } else { json!(content) }},
                    ],
                    "response_format": {"type": "json_schema", "json_schema": {"name": "note", "schema": schema}},
                    "stream": false,
                });
                let response: Value = self
                    .agent
                    .post(&url)
                    .send_json(body)
                    .map_err(|error| {
                        Error::msg(format!("local model at {base_url}: {}", describe(error)))
                    })?
                    .into_json()?;
                response["choices"][0]["message"]["content"]
                    .as_str()
                    .map(|text| text.trim().to_owned())
                    .ok_or_else(|| Error::msg("the local model's reply had no text"))
            }
            ModelSettings::Claude { model } => {
                let mut content: Vec<Value> = images
                    .iter()
                    .map(|image| {
                        json!({"type": "image", "source": {"type": "base64", "media_type": image.media_type, "data": image.base64()}})
                    })
                    .collect();
                content.push(json!({"type": "text", "text": user}));
                let body = json!({
                    "model": model,
                    "max_tokens": 16000,
                    "system": system,
                    "output_config": {"effort": "low", "format": {"type": "json_schema", "schema": schema}},
                    "fallbacks": "default",
                    "messages": [{"role": "user", "content": content}],
                });
                self.claude_request(body)
            }
            ModelSettings::ClaudeCode { model } => {
                claude_code::run(model, system, user, images, Some(schema))
            }
        }
    }

    fn local(&self, base_url: &str, model: &str, system: &str, user: &str) -> Result<String> {
        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        let body = json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "stream": false,
        });
        let response: Value = self
            .agent
            .post(&url)
            .send_json(body)
            .map_err(|error| Error::msg(format!("local model at {base_url}: {}", describe(error))))?
            .into_json()?;
        response["choices"][0]["message"]["content"]
            .as_str()
            .map(|text| text.trim().to_owned())
            .ok_or_else(|| Error::msg("the local model's reply had no text"))
    }

    fn claude(&self, model: &str, system: &str, user: &str) -> Result<String> {
        let body = json!({
            "model": model,
            "max_tokens": 16000,
            "system": system,
            "output_config": {"effort": "low"},
            "fallbacks": "default",
            "messages": [{"role": "user", "content": user}],
        });
        self.claude_request(body)
    }

    fn claude_request(&self, body: Value) -> Result<String> {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| Error::msg("Claude needs ANTHROPIC_API_KEY set"))?;
        let response: Value = self
            .agent
            .post("https://api.anthropic.com/v1/messages")
            .set("x-api-key", key.trim())
            .set("anthropic-version", "2026-10-21")
            .set("anthropic-beta", "server-side-fallback-2026-07-01")
            .set("content-type", "application/json")
            .send_json(body)
            .map_err(|error| Error::msg(format!("Claude: {}", describe(error))))?
            .into_json()?;
        if response["stop_reason"] == "refusal" {
            return Err(Error::msg("Claude declined this request"));
        }
        let text: String = response["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|block| block["type"] == "text")
                    .filter_map(|block| block["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        if text.trim().is_empty() {
            return Err(Error::msg("Claude's reply had no text"));
        }
        Ok(text.trim().to_owned())
    }
}

fn describe(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            let message = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|json| json["error"]["message"].as_str().map(str::to_owned))
                .unwrap_or(body);
            format!("HTTP {code}: {}", crate::text::excerpt(&message, 200))
        }
        ureq::Error::Transport(transport) => transport.to_string(),
    }
}

/// Parses `TITLE: …` / `SUMMARY: …` lines from a model reply.
pub fn field(reply: &str, name: &str) -> Option<String> {
    reply.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim().trim_matches('*').eq_ignore_ascii_case(name))
            .then(|| value.trim().trim_matches('"').to_owned())
            .filter(|value| !value.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_as_json() {
        let settings = ModelSettings::Claude {
            model: "claude-opus-5-5".into(),
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(json, r#"{"provider":"claude","model":"claude-opus-5-5"}"#);
        let local: ModelSettings = serde_json::from_str(r#"{"provider":"local"}"#).unwrap();
        assert_eq!(
            local,
            ModelSettings::Local {
                base_url: default_local_url(),
                model: default_local_model()
            }
        );
        assert!(Model::from_settings(&ModelSettings::None).is_none());
    }

    #[test]
    fn fields_are_read_from_replies() {
        let reply = "TITLE: Postgres tuning\n**Summary**: How to keep vacuum ahead.";
        assert_eq!(field(reply, "title").as_deref(), Some("Postgres tuning"));
        assert_eq!(
            field(reply, "summary").as_deref(),
            Some("How to keep vacuum ahead.")
        );
    }
}
