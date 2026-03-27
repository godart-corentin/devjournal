use super::{build_prompt_with_custom, LlmBackend};
use crate::db::Event;
use anyhow::{Context, Result};

pub const DEFAULT_MODEL: &str = "gpt-5.4-mini";

pub struct CursorBackend {
    pub model: String,
}

impl CursorBackend {
    fn build_args(&self, prompt: &str) -> Vec<String> {
        let mut args = vec![
            "agent".to_string(),
            "--trust".to_string(),
            "-p".to_string(),
            prompt.to_string(),
            "--output-format".to_string(),
            "text".to_string(),
        ];
        if self.model != DEFAULT_MODEL {
            args.push("--model".to_string());
            args.push(self.model.clone());
        }
        args
    }
}

impl LlmBackend for CursorBackend {
    fn summarize(
        &self,
        events: &[Event],
        date: &str,
        custom_prompt: Option<&str>,
    ) -> Result<String> {
        let prompt = build_prompt_with_custom(events, date, custom_prompt);
        let args = self.build_args(&prompt);
        let output = std::process::Command::new("cursor")
            .args(&args)
            .output()
            .context("cursor agent not found — is Cursor installed and on PATH?")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("cursor agent failed: {}", stderr.trim());
        }

        let text = String::from_utf8(output.stdout)
            .context("cursor agent output was not valid UTF-8")?
            .trim()
            .to_string();
        if text.is_empty() {
            anyhow::bail!("cursor agent returned empty output");
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_args_default_model() {
        let backend = CursorBackend {
            model: DEFAULT_MODEL.to_string(),
        };
        let args = backend.build_args("hello");
        assert_eq!(
            args,
            vec!["agent", "--trust", "-p", "hello", "--output-format", "text"]
        );
    }

    #[test]
    fn test_build_args_custom_model() {
        let backend = CursorBackend {
            model: "gpt-4o".to_string(),
        };
        let args = backend.build_args("hello");
        assert_eq!(
            args,
            vec![
                "agent",
                "--trust",
                "-p",
                "hello",
                "--output-format",
                "text",
                "--model",
                "gpt-4o"
            ]
        );
    }
}
