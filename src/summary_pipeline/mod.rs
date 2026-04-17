pub mod cluster;
pub mod evidence;
pub mod outcome;
pub mod render;

use anyhow::Result;
use std::collections::BTreeSet;

use crate::db::Event;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProjectReport {
    pub project_name: String,
    pub outcomes: Vec<outcome::OutcomeCandidate>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PipelineReport {
    pub date_label: String,
    pub projects: Vec<ProjectReport>,
}

pub fn build_report(events: &[Event], date_label: &str) -> Result<PipelineReport> {
    let evidence = evidence::normalize_events(events)?;
    let workstreams = cluster::build_workstreams(&evidence);
    let max_outcomes_per_project = max_outcomes_per_project_for_events(events);
    let projects = outcome::build_project_reports(workstreams, max_outcomes_per_project);

    Ok(PipelineReport {
        date_label: date_label.to_string(),
        projects,
    })
}

fn max_outcomes_per_project_for_events(events: &[Event]) -> usize {
    let mut dates = BTreeSet::new();
    for event in events {
        if let Some(date) = event.timestamp.split('T').next() {
            if !date.is_empty() {
                dates.insert(date.to_string());
            }
        }
    }

    if dates.len() > 1 {
        5
    } else {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Event;

    fn make_event(timestamp: &str, hash: &str, message: &str, path: &str) -> Event {
        Event {
            id: None,
            repo_path: "/tmp/proj".to_string(),
            repo_name: Some("proj".to_string()),
            event_type: "commit".to_string(),
            timestamp: timestamp.to_string(),
            data: serde_json::json!({
                "hash": hash,
                "message": message,
                "branch": "main",
                "diff": {
                    "stat_summary": "1 file changed, 3 insertions(+)",
                    "files": [{
                        "path": path,
                        "status": "modified",
                        "additions": 3,
                        "deletions": 0
                    }]
                }
            }),
        }
    }

    #[test]
    fn build_report_projects_from_events_end_to_end() {
        let report = build_report(
            &[make_event(
                "2026-04-15T09:00:00+02:00",
                "abc12345",
                "TT-42 improve login validation",
                "src/auth/login.rs",
            )],
            "2026-04-15",
        )
        .unwrap();

        assert_eq!(report.date_label, "2026-04-15");
        assert_eq!(report.projects.len(), 1);
        assert_eq!(report.projects[0].project_name, "proj");
        assert_eq!(report.projects[0].outcomes.len(), 1);
        assert!(report.projects[0].outcomes[0]
            .factual_headline
            .contains("TT-42"));
    }

    #[test]
    fn build_report_truncates_more_aggressively_for_single_day_than_range() {
        let single_day = build_report(
            &[
                make_event(
                    "2026-04-15T09:00:00+02:00",
                    "a1",
                    "TT-1 improve auth flow",
                    "src/auth/login.rs",
                ),
                make_event(
                    "2026-04-15T10:00:00+02:00",
                    "a2",
                    "TT-2 improve billing flow",
                    "src/billing/invoice.rs",
                ),
                make_event(
                    "2026-04-15T11:00:00+02:00",
                    "a3",
                    "TT-3 improve search flow",
                    "src/search/index.rs",
                ),
                make_event(
                    "2026-04-15T12:00:00+02:00",
                    "a4",
                    "TT-4 improve settings flow",
                    "src/settings/profile.rs",
                ),
            ],
            "2026-04-15",
        )
        .unwrap();

        let range = build_report(
            &[
                make_event(
                    "2026-04-14T09:00:00+02:00",
                    "b1",
                    "TT-1 improve auth flow",
                    "src/auth/login.rs",
                ),
                make_event(
                    "2026-04-14T10:00:00+02:00",
                    "b2",
                    "TT-2 improve billing flow",
                    "src/billing/invoice.rs",
                ),
                make_event(
                    "2026-04-15T11:00:00+02:00",
                    "b3",
                    "TT-3 improve search flow",
                    "src/search/index.rs",
                ),
                make_event(
                    "2026-04-15T12:00:00+02:00",
                    "b4",
                    "TT-4 improve settings flow",
                    "src/settings/profile.rs",
                ),
            ],
            "2026-04-14 to 2026-04-15",
        )
        .unwrap();

        assert_eq!(single_day.projects[0].outcomes.len(), 3);
        assert_eq!(range.projects[0].outcomes.len(), 4);
    }
}
