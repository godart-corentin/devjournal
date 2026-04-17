use super::{build_prompt_with_custom, LlmBackend};
use crate::summary_pipeline::outcome::OutcomeCandidate;
use anyhow::{Context, Result};
use serde_json::json;

pub struct OpenAiBackend {
    pub api_key: String,
    pub model: String,
}

impl LlmBackend for OpenAiBackend {
    fn summarize(
        &self,
        outcomes: &[OutcomeCandidate],
        date: &str,
        custom_prompt: Option<&str>,
    ) -> Result<String> {
        let prompt = build_prompt_with_custom(outcomes, date, custom_prompt);
        let body = json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}]
        });

        let response = ureq::post("https://api.openai.com/v1/chat/completions")
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("content-type", "application/json")
            .send_json(body)
            .context("Failed to call OpenAI API")?;

        let json: serde_json::Value = response
            .into_json()
            .context("Failed to parse OpenAI API response")?;

        json["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .context("Unexpected OpenAI API response shape")
    }
}
