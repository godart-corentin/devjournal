use anyhow::Result;
use chrono::{Duration, Local};
use serde::Serialize;
use std::path::PathBuf;

use crate::config::{LlmConfig, LlmProvider};
use crate::db;
use crate::llm::{self, LlmBackend};
use crate::summary_pipeline;
use crate::summary_pipeline::outcome::{OutcomeCandidate, TonePolicy};

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

pub fn summary_diagnostics_path() -> PathBuf {
    db::data_dir().join("summary-debug.jsonl")
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

    let summary = generate_summary_body(&events, date, llm_config)?;

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

    let summary = generate_summary_body(&events, &date_label, llm_config)?;

    std::fs::create_dir_all(summaries_dir())?;
    let content = format!("<!-- fingerprint: {} -->\n{}", fingerprint, summary);
    std::fs::write(&cached_path, &content)?;

    Ok(summary)
}

fn generate_summary_body(
    events: &[db::Event],
    date_label: &str,
    llm_config: &LlmConfig,
) -> Result<String> {
    let report = summary_pipeline::build_report(events, date_label)?;
    generate_summary_from_report(&report, llm_config)
}

fn generate_summary_from_report(
    report: &summary_pipeline::PipelineReport,
    llm_config: &LlmConfig,
) -> Result<String> {
    let fallback_summary = summary_pipeline::render::render_project_markdown(report);

    let api_key = match llm_config.provider {
        LlmProvider::Ollama => Some(String::new()),
        _ => crate::config::api_key(llm_config),
    };
    let Some(api_key) = api_key else {
        let _ = append_summary_diagnostics(report, "fallback", &fallback_summary);
        return Ok(fallback_summary);
    };

    let backend = llm::make_backend(
        llm_config.provider,
        &api_key,
        llm_config.model.as_deref(),
        llm_config.base_url.as_deref(),
    );

    summarize_report_with_backend(report, llm_config, &*backend, fallback_summary)
}

fn summarize_report_with_backend(
    report: &summary_pipeline::PipelineReport,
    llm_config: &LlmConfig,
    backend: &dyn LlmBackend,
    fallback_summary: String,
) -> Result<String> {
    let outcomes = flatten_outcomes(report);
    match backend.summarize(
        &outcomes,
        &report.date_label,
        llm_config.system_prompt.as_deref(),
    ) {
        Ok(summary) => {
            if llm_summary_covers_all_candidates(report, &summary) {
                let _ = append_summary_diagnostics(report, "llm", &summary);
                Ok(summary)
            } else {
                let _ =
                    append_summary_diagnostics(report, "fallback_validation", &fallback_summary);
                Ok(fallback_summary)
            }
        }
        Err(_) => {
            let _ = append_summary_diagnostics(report, "fallback", &fallback_summary);
            Ok(fallback_summary)
        }
    }
}

fn flatten_outcomes(report: &summary_pipeline::PipelineReport) -> Vec<OutcomeCandidate> {
    report
        .projects
        .iter()
        .flat_map(|project| project.outcomes.clone())
        .collect()
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

fn llm_summary_covers_all_candidates(
    report: &summary_pipeline::PipelineReport,
    summary: &str,
) -> bool {
    if summary_exposes_prompt_scaffolding(summary) {
        return false;
    }

    let bullet_counts = parse_project_bullet_counts(summary);

    report.projects.iter().all(|project| {
        let expected = project.outcomes.len();
        if expected == 0 {
            return true;
        }

        bullet_counts
            .get(project.project_name.as_str())
            .is_some_and(|actual| *actual >= expected)
    })
}

fn summary_exposes_prompt_scaffolding(summary: &str) -> bool {
    summary.lines().any(|line| {
        let trimmed = line.trim_start_matches(['-', '*', '•', '+', ' ']).trim();
        let normalized = trimmed.to_ascii_lowercase();
        normalized.contains("candidate line:")
            || normalized.contains("supporting evidence:")
            || normalized.contains("tone policy:")
            || normalized.contains("confidence:")
    })
}

fn parse_project_bullet_counts(summary: &str) -> std::collections::BTreeMap<&str, usize> {
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    let mut current_project: Option<&str> = None;

    for line in summary.lines() {
        if let Some(project_name) = line.strip_prefix("## ").map(str::trim) {
            current_project = Some(project_name);
            counts.entry(project_name).or_insert(0);
            continue;
        }

        let is_bullet = line.starts_with("- ")
            || line.starts_with("* ")
            || line.starts_with("• ")
            || line.starts_with("+ ");
        if is_bullet {
            if let Some(project_name) = current_project {
                *counts.entry(project_name).or_insert(0) += 1;
            }
        }
    }

    counts
}

#[derive(Serialize)]
struct SummaryDiagnosticsEntry<'a> {
    generated_at: String,
    date_label: &'a str,
    source: &'a str,
    final_summary: &'a str,
    projects: Vec<SummaryDiagnosticsProject<'a>>,
}

#[derive(Serialize)]
struct SummaryDiagnosticsProject<'a> {
    project_name: &'a str,
    outcomes: Vec<SummaryDiagnosticsOutcome<'a>>,
}

#[derive(Serialize)]
struct SummaryDiagnosticsOutcome<'a> {
    factual_headline: &'a str,
    probable_outcome: &'a str,
    confidence: u8,
    tone_policy: &'a TonePolicy,
}

fn append_summary_diagnostics(
    report: &summary_pipeline::PipelineReport,
    source: &str,
    final_summary: &str,
) -> Result<()> {
    append_summary_diagnostics_to_path(&summary_diagnostics_path(), report, source, final_summary)
}

fn append_summary_diagnostics_to_path(
    path: &std::path::Path,
    report: &summary_pipeline::PipelineReport,
    source: &str,
    final_summary: &str,
) -> Result<()> {
    let entry = SummaryDiagnosticsEntry {
        generated_at: Local::now().to_rfc3339(),
        date_label: &report.date_label,
        source,
        final_summary,
        projects: report
            .projects
            .iter()
            .map(|project| SummaryDiagnosticsProject {
                project_name: &project.project_name,
                outcomes: project
                    .outcomes
                    .iter()
                    .map(|outcome| SummaryDiagnosticsOutcome {
                        factual_headline: &outcome.factual_headline,
                        probable_outcome: &outcome.probable_outcome,
                        confidence: outcome.confidence,
                        tone_policy: &outcome.tone_policy,
                    })
                    .collect(),
            })
            .collect(),
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(&entry)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    use std::io::Write;
    writeln!(file, "{line}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use tempfile::tempdir;

    use crate::llm::LlmBackend;
    use crate::summary_pipeline::outcome::{OutcomeCandidate, TonePolicy};
    use crate::summary_pipeline::{PipelineReport, ProjectReport};

    struct RecordingBackend {
        seen_outcomes: Rc<RefCell<Vec<OutcomeCandidate>>>,
        response: String,
    }

    impl LlmBackend for RecordingBackend {
        fn summarize(
            &self,
            outcomes: &[OutcomeCandidate],
            _date: &str,
            _custom_prompt: Option<&str>,
        ) -> Result<String> {
            self.seen_outcomes.borrow_mut().extend_from_slice(outcomes);
            Ok(self.response.clone())
        }
    }

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
    fn summarize_report_forwards_flattened_outcomes_to_backend() {
        let project_a_outcomes = vec![
            OutcomeCandidate {
                project_name: "proj-a".to_string(),
                factual_headline: "TT-42 improved login validation".to_string(),
                probable_outcome: "Improved login validation".to_string(),
                supporting_messages: vec!["TT-42 add login validation".to_string()],
                confidence: 90,
                importance: 2,
                tone_policy: TonePolicy::PolishOk,
            },
            OutcomeCandidate {
                project_name: "proj-a".to_string(),
                factual_headline: "TT-43 fixed logout flow".to_string(),
                probable_outcome: "Fixed logout flow".to_string(),
                supporting_messages: vec!["TT-43 fix logout".to_string()],
                confidence: 70,
                importance: 1,
                tone_policy: TonePolicy::StayLiteral,
            },
        ];
        let project_b_outcomes = vec![OutcomeCandidate {
            project_name: "proj-b".to_string(),
            factual_headline: "TT-7 tightened release checks".to_string(),
            probable_outcome: "Tightened release checks".to_string(),
            supporting_messages: vec!["TT-7 tighten checks".to_string()],
            confidence: 80,
            importance: 1,
            tone_policy: TonePolicy::StayLiteral,
        }];
        let report = PipelineReport {
            date_label: "2026-04-15".to_string(),
            projects: vec![
                ProjectReport {
                    project_name: "proj-a".to_string(),
                    outcomes: project_a_outcomes.clone(),
                },
                ProjectReport {
                    project_name: "proj-b".to_string(),
                    outcomes: project_b_outcomes.clone(),
                },
            ],
        };
        let seen_outcomes = Rc::new(RefCell::new(Vec::new()));
        let backend = RecordingBackend {
            seen_outcomes: seen_outcomes.clone(),
            response: "# Dev Journal — 2026-04-15\n\n## proj-a\n- Improved login validation\n- TT-43 fixed logout flow\n\n## proj-b\n- TT-7 tightened release checks\n".to_string(),
        };
        let llm_config = LlmConfig::default();

        let summary = summarize_report_with_backend(
            &report,
            &llm_config,
            &backend,
            "fallback markdown".to_string(),
        )
        .unwrap();

        assert!(summary.contains("## proj-a"));
        assert!(summary.contains("## proj-b"));
        assert_eq!(
            seen_outcomes.borrow().clone(),
            vec![
                project_a_outcomes[0].clone(),
                project_a_outcomes[1].clone(),
                project_b_outcomes[0].clone(),
            ]
        );
    }

    #[test]
    fn summarize_report_falls_back_when_llm_omits_a_candidate() {
        let report = PipelineReport {
            date_label: "2026-04-08".to_string(),
            projects: vec![ProjectReport {
                project_name: "core-service".to_string(),
                outcomes: vec![
                    OutcomeCandidate {
                        project_name: "core-service".to_string(),
                        factual_headline: "TT-5359 upload identity documents for signees"
                            .to_string(),
                        probable_outcome: "Upload identity documents for signees".to_string(),
                        supporting_messages: vec![],
                        confidence: 100,
                        importance: 10,
                        tone_policy: TonePolicy::PolishOk,
                    },
                    OutcomeCandidate {
                        project_name: "core-service".to_string(),
                        factual_headline:
                            "TT-5368 remove unsupported investment and verification transitions"
                                .to_string(),
                        probable_outcome:
                            "Remove unsupported investment and verification transitions".to_string(),
                        supporting_messages: vec![],
                        confidence: 100,
                        importance: 10,
                        tone_policy: TonePolicy::PolishOk,
                    },
                ],
            }],
        };
        let backend = RecordingBackend {
            seen_outcomes: Rc::new(RefCell::new(Vec::new())),
            response:
                "# Dev Journal — 2026-04-08\n\n## core-service\n- Upload identity documents for signees\n"
                    .to_string(),
        };
        let llm_config = LlmConfig::default();
        let fallback = summary_pipeline::render::render_project_markdown(&report);

        let summary =
            summarize_report_with_backend(&report, &llm_config, &backend, fallback.clone())
                .unwrap();

        assert_eq!(summary, fallback);
        assert!(summary.contains("Upload identity documents for signees"));
        assert!(summary.contains("Remove unsupported investment and verification transitions"));
    }

    #[test]
    fn summarize_report_falls_back_when_llm_leaks_prompt_scaffolding() {
        let report = PipelineReport {
            date_label: "2026-04-07".to_string(),
            projects: vec![ProjectReport {
                project_name: "devjournal".to_string(),
                outcomes: vec![OutcomeCandidate {
                    project_name: "devjournal".to_string(),
                    factual_headline: "TT-42 enrich summaries with structured diff fallbacks"
                        .to_string(),
                    probable_outcome: "Enrich summaries with structured diff fallbacks".to_string(),
                    supporting_messages: vec![],
                    confidence: 100,
                    importance: 10,
                    tone_policy: TonePolicy::PolishOk,
                }],
            }],
        };
        let backend = RecordingBackend {
            seen_outcomes: Rc::new(RefCell::new(Vec::new())),
            response: "# Dev Journal — 2026-04-07\n\n## devjournal\n- [candidate line: enrich summaries with structured diff fallbacks]\n".to_string(),
        };
        let llm_config = LlmConfig::default();
        let fallback = summary_pipeline::render::render_project_markdown(&report);

        let summary =
            summarize_report_with_backend(&report, &llm_config, &backend, fallback.clone())
                .unwrap();

        assert_eq!(summary, fallback);
        assert!(!summary.contains("candidate line:"));
        assert!(summary.contains("Enrich summaries with structured diff fallbacks"));
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

    #[test]
    fn summary_diagnostics_log_entry_includes_source_and_outcome_metadata() {
        let report = PipelineReport {
            date_label: "2026-04-14".to_string(),
            projects: vec![ProjectReport {
                project_name: "devjournal".to_string(),
                outcomes: vec![OutcomeCandidate {
                    project_name: "devjournal".to_string(),
                    factual_headline: "TT-42 inline LLM setup for summaries".to_string(),
                    probable_outcome: "Added inline LLM setup for summaries".to_string(),
                    supporting_messages: vec!["inline LLM setup for summaries".to_string()],
                    confidence: 60,
                    importance: 8,
                    tone_policy: TonePolicy::StayLiteral,
                }],
            }],
        };
        let dir = tempdir().unwrap();
        let path = dir.path().join("summary-debug.jsonl");

        append_summary_diagnostics_to_path(&path, &report, "llm", "# Dev Journal — 2026-04-14")
            .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let line = content.lines().next().unwrap();
        let value: serde_json::Value = serde_json::from_str(line).unwrap();

        assert_eq!(value["date_label"], "2026-04-14");
        assert_eq!(value["source"], "llm");
        assert_eq!(value["final_summary"], "# Dev Journal — 2026-04-14");
        assert_eq!(value["projects"][0]["project_name"], "devjournal");
        assert_eq!(value["projects"][0]["outcomes"][0]["confidence"], 60);
        assert_eq!(
            value["projects"][0]["outcomes"][0]["tone_policy"],
            "StayLiteral"
        );
    }
}
