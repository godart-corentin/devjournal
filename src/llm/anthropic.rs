use super::{build_prompt_with_custom, LlmBackend};
use crate::summary_pipeline::outcome::OutcomeCandidate;
use anyhow::{Context, Result};
use serde_json::json;

pub struct AnthropicBackend {
    pub api_key: String,
    pub model: String,
}

impl LlmBackend for AnthropicBackend {
    fn summarize(
        &self,
        outcomes: &[OutcomeCandidate],
        date: &str,
        custom_prompt: Option<&str>,
    ) -> Result<String> {
        let prompt = build_prompt_with_custom(outcomes, date, custom_prompt);
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
            .context("Failed to call Anthropic API")?;

        let json: serde_json::Value = response
            .into_json()
            .context("Failed to parse Anthropic API response")?;

        json["content"][0]["text"]
            .as_str()
            .map(|s| s.to_string())
            .context("Unexpected Anthropic API response shape")
    }
}
