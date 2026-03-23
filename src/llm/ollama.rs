use anyhow::{Context, Result};
use serde_json::json;
use crate::db::Event;
use super::{LlmBackend, build_prompt};

pub struct OllamaBackend {
    pub base_url: String,
    pub model: String,
}

impl LlmBackend for OllamaBackend {
    fn summarize(&self, events: &[Event], date: &str) -> Result<String> {
        let prompt = build_prompt(events, date);
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

        let json: serde_json::Value = response.into_json()
            .context("Failed to parse Ollama API response")?;

        json["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .context("Unexpected Ollama API response shape")
    }
}
