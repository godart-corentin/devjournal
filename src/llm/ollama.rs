use super::{build_prompt_with_custom, LlmBackend};
use crate::summary_pipeline::outcome::OutcomeCandidate;
use anyhow::{Context, Result};
use serde_json::json;

pub struct OllamaBackend {
    pub base_url: String,
    pub model: String,
}

impl LlmBackend for OllamaBackend {
    fn summarize(
        &self,
        outcomes: &[OutcomeCandidate],
        date: &str,
        custom_prompt: Option<&str>,
    ) -> Result<String> {
        let prompt = build_prompt_with_custom(outcomes, date, custom_prompt);
        let body = json!({
            "model": self.model,
            "stream": false,
            "messages": [{"role": "user", "content": prompt}]
        });

        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));
        let response = ureq::post(&url)
            .set("content-type", "application/json")
            .send_json(body)
            .context("Failed to call Ollama API — is Ollama running? Try: ollama serve")?;

        let json: serde_json::Value = response
            .into_json()
            .context("Failed to parse Ollama API response")?;

        json["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .context("Unexpected Ollama API response shape")
    }
}
