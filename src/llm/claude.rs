use super::{build_prompt_with_custom, LlmBackend};
use crate::db::Event;
use anyhow::{Context, Result};
use serde_json::json;

pub struct ClaudeBackend {
    pub api_key: String,
    pub model: String,
}

impl LlmBackend for ClaudeBackend {
    fn summarize(
        &self,
        events: &[Event],
        date: &str,
        custom_prompt: Option<&str>,
    ) -> Result<String> {
        let prompt = build_prompt_with_custom(events, date, custom_prompt);
        let body = json!({
            "model": self.model,
            "max_tokens": 2048,
            "messages": [{"role": "user", "content": prompt}]
        });

        let response = ureq::post("https://api.anthropic.com/v1/messages")
            .set("x-api-key", &self.api_key)
            .set("anthropic-version", "2023-06-01")
            .set("content-type", "application/json")
            .send_json(body)
            .context("Failed to call Claude API")?;

        let json: serde_json::Value = response
            .into_json()
            .context("Failed to parse Claude API response")?;

        json["content"][0]["text"]
            .as_str()
            .map(|s| s.to_string())
            .context("Unexpected Claude API response shape")
    }
}
