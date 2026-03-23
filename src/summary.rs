use anyhow::{Context, Result};
use chrono::Local;
use std::path::PathBuf;

use crate::config::LlmConfig;
use crate::db;
use crate::llm;

pub fn summaries_dir() -> PathBuf {
    db::data_dir().join("summaries")
}

pub fn summary_path(date: &str) -> PathBuf {
    summaries_dir().join(format!("{}.md", date))
}

pub fn generate(date: &str, llm_config: &LlmConfig) -> Result<String> {
    let conn = db::open()?;
    let events = db::get_events_for_date(&conn, date)?;

    if events.is_empty() {
        return Ok(format!(
            "# Dev Journal — {}\n\nNo activity recorded for this date.\n",
            date
        ));
    }

    let api_key = crate::config::api_key(llm_config)
        .context("No API key found. Set DEVJOURNAL_API_KEY or add api_key to config.")?;

    let backend = llm::make_backend(
        &llm_config.provider,
        &api_key,
        llm_config.model.as_deref(),
    );

    let summary = backend.summarize(&events, date)?;

    // Save to file
    std::fs::create_dir_all(summaries_dir())?;
    std::fs::write(summary_path(date), &summary)?;

    Ok(summary)
}

pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}
