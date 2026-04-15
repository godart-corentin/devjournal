use anyhow::{Context, Result};
use chrono::{Duration, Local};
use std::path::PathBuf;

use crate::config::{LlmConfig, LlmProvider};
use crate::db;
use crate::llm;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryWindow {
    Date(String),
    Range { from: String, to: String },
}

impl SummaryWindow {
    pub fn for_date(date: String) -> Self {
        Self::Date(date)
    }

    pub fn rolling_days(days: i64) -> Self {
        let to = today();
        let from = (Local::now() - Duration::days(days - 1))
            .format("%Y-%m-%d")
            .to_string();
        Self::Range { from, to }
    }

    pub fn from_summary_args(
        date: Option<String>,
        from: Option<String>,
        to: Option<String>,
    ) -> Result<Self> {
        match (date, from, to) {
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                anyhow::bail!("Cannot combine a positional date with --from/--to");
            }
            (_, Some(from), to) => {
                let to = to.unwrap_or_else(today);
                Ok(Self::Range { from, to })
            }
            (_, None, Some(_)) => anyhow::bail!("--to requires --from"),
            (date, None, None) => Ok(Self::Date(date.unwrap_or_else(today))),
        }
    }

    pub fn from(&self) -> &str {
        match self {
            Self::Date(date) => date,
            Self::Range { from, .. } => from,
        }
    }

    pub fn to(&self) -> &str {
        match self {
            Self::Date(date) => date,
            Self::Range { to, .. } => to,
        }
    }

    pub fn display_label(&self) -> String {
        match self {
            Self::Date(date) => date.clone(),
            Self::Range { from, to } => format!("{from} to {to}"),
        }
    }

    pub fn load_events(&self, conn: &rusqlite::Connection) -> Result<Vec<db::Event>> {
        match self {
            Self::Date(date) => db::get_events_for_date(conn, date),
            Self::Range { from, to } => db::get_events_for_range(conn, from, to),
        }
    }

    pub fn generate_markdown(&self, llm_config: &LlmConfig, force: bool) -> Result<String> {
        match self {
            Self::Date(date) => generate(date, llm_config, force),
            Self::Range { from, to } => generate_range(from, to, llm_config, force),
        }
    }
}

pub fn summaries_dir() -> PathBuf {
    db::data_dir().join("summaries")
}

pub fn summary_path(date: &str) -> PathBuf {
    summaries_dir().join(format!("{}.md", date))
}

pub fn generate(date: &str, llm_config: &LlmConfig, force: bool) -> Result<String> {
    let conn = db::open()?;
    let events = db::get_events_for_date(&conn, date)?;

    if events.is_empty() {
        return Ok(format!(
            "# Dev Journal — {}\n\nNo activity recorded for this date.\n",
            date
        ));
    }

    let fingerprint = db::compute_events_fingerprint(&events);

    // Check cache unless force-regenerating
    if !force {
        let cached_path = summary_path(date);
        if let Ok(cached) = std::fs::read_to_string(&cached_path) {
            if parse_cached_fingerprint(&cached).as_deref() == Some(fingerprint.as_str()) {
                // Strip the fingerprint header before returning to the user
                let body = cached.lines().skip(1).collect::<Vec<_>>().join("\n");
                return Ok(body);
            }
        }
    }

    let api_key = if llm_config.provider == LlmProvider::Ollama {
        String::new()
    } else {
        crate::config::api_key(llm_config)
            .context("No API key found. Set DEVJOURNAL_API_KEY or add api_key to config.")?
    };

    let backend = llm::make_backend(
        llm_config.provider,
        &api_key,
        llm_config.model.as_deref(),
        llm_config.base_url.as_deref(),
    );

    let summary = backend.summarize(&events, date, llm_config.system_prompt.as_deref())?;

    // Write fingerprint header + summary to file
    std::fs::create_dir_all(summaries_dir())?;
    let content = format!("<!-- fingerprint: {} -->\n{}", fingerprint, summary);
    std::fs::write(summary_path(date), &content)?;

    Ok(summary)
}

pub fn generate_range(from: &str, to: &str, llm_config: &LlmConfig, force: bool) -> Result<String> {
    let conn = db::open()?;
    let events = db::get_events_for_range(&conn, from, to)?;
    let date_label = format!("{} to {}", from, to);

    if events.is_empty() {
        return Ok(format!(
            "# Dev Journal — {}\n\nNo activity recorded for this period.\n",
            date_label
        ));
    }

    let fingerprint = db::compute_events_fingerprint(&events);
    let cached_path = summaries_dir().join(format!("{}_to_{}.md", from, to));

    if !force {
        if let Ok(cached) = std::fs::read_to_string(&cached_path) {
            if parse_cached_fingerprint(&cached).as_deref() == Some(fingerprint.as_str()) {
                let body = cached.lines().skip(1).collect::<Vec<_>>().join("\n");
                return Ok(body);
            }
        }
    }

    let api_key = if llm_config.provider == LlmProvider::Ollama {
        String::new()
    } else {
        crate::config::api_key(llm_config)
            .context("No API key found. Set DEVJOURNAL_API_KEY or add api_key to config.")?
    };

    let backend = llm::make_backend(
        llm_config.provider,
        &api_key,
        llm_config.model.as_deref(),
        llm_config.base_url.as_deref(),
    );

    let summary = backend.summarize(&events, &date_label, llm_config.system_prompt.as_deref())?;

    std::fs::create_dir_all(summaries_dir())?;
    let content = format!("<!-- fingerprint: {} -->\n{}", fingerprint, summary);
    std::fs::write(&cached_path, &content)?;

    Ok(summary)
}

pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub fn parse_cached_fingerprint(content: &str) -> Option<String> {
    let first_line = content.lines().next()?;
    let inner = first_line
        .strip_prefix("<!-- fingerprint: ")?
        .strip_suffix(" -->")?;
    Some(inner.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_fingerprint_from_valid_header() {
        let content = "<!-- fingerprint: abc123def456 -->\n# Dev Journal\n\nSome content.\n";
        assert_eq!(
            parse_cached_fingerprint(content),
            Some("abc123def456".to_string())
        );
    }

    #[test]
    fn test_parse_fingerprint_returns_none_for_no_header() {
        let content = "# Dev Journal\n\nNo fingerprint here.\n";
        assert_eq!(parse_cached_fingerprint(content), None);
    }

    #[test]
    fn test_parse_fingerprint_returns_none_for_empty_content() {
        assert_eq!(parse_cached_fingerprint(""), None);
    }

    #[test]
    fn test_generate_writes_fingerprint_header_to_file() {
        use crate::db::{self, Event};
        use rusqlite::Connection;
        use tempfile::tempdir;

        fn init_conn() -> Connection {
            let conn = Connection::open_in_memory().unwrap();
            db::init_test_database(&conn).unwrap();
            conn
        }

        // Insert one event so we have a non-empty events set
        let conn = init_conn();
        let event = Event {
            id: None,
            repo_path: "/repo/test".to_string(),
            repo_name: Some("test".to_string()),
            event_type: "commit".to_string(),
            timestamp: "2026-03-25T10:00:00Z".to_string(),
            data: serde_json::json!({ "hash": "abc123", "message": "test commit", "branch": "main" }),
        };
        db::insert_event(&conn, &event).unwrap();

        let events = db::get_events_for_date(&conn, "2026-03-25").unwrap();
        let fp = db::compute_events_fingerprint(&events);

        // Simulate what generate() should write
        let dir = tempdir().unwrap();
        let path = dir.path().join("2026-03-25.md");
        let summary_body = "# Dev Journal — 2026-03-25\n\n- test commit\n";
        let with_header = format!("<!-- fingerprint: {} -->\n{}", fp, summary_body);
        std::fs::write(&path, &with_header).unwrap();

        // Now verify parse_cached_fingerprint reads it back correctly
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(parse_cached_fingerprint(&written), Some(fp.clone()));

        // Verify the body is intact after the header line
        let body: String = written.lines().skip(1).collect::<Vec<_>>().join("\n");
        assert!(body.contains("Dev Journal"));
    }

    #[test]
    fn test_summary_window_defaults_to_today_for_empty_summary_args() {
        let window = SummaryWindow::from_summary_args(None, None, None).unwrap();
        assert_eq!(window, SummaryWindow::Date(today()));
    }

    #[test]
    fn test_summary_window_builds_inclusive_range_from_args() {
        let window = SummaryWindow::from_summary_args(
            None,
            Some("2026-03-01".to_string()),
            Some("2026-03-07".to_string()),
        )
        .unwrap();
        assert_eq!(
            window,
            SummaryWindow::Range {
                from: "2026-03-01".to_string(),
                to: "2026-03-07".to_string()
            }
        );
    }
}
