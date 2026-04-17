use anyhow::Result;

use crate::summary_pipeline::outcome::{OutcomeCandidate, TonePolicy};
use crate::summary_pipeline::{PipelineReport, ProjectReport};

pub fn render_markdown(date_label: &str, projects: &[(String, Vec<OutcomeCandidate>)]) -> String {
    let mut lines = vec![format!("# Dev Journal — {date_label}"), String::new()];

    for (project_name, outcomes) in projects {
        lines.push(format!("## {project_name}"));
        for outcome in outcomes {
            let line = match outcome.tone_policy {
                TonePolicy::PolishOk => outcome.probable_outcome.clone(),
                TonePolicy::StayLiteral => outcome.factual_headline.clone(),
                TonePolicy::MentionUncertainty => format!("Worked on {}", outcome.factual_headline),
            };
            lines.push(format!("- {line}"));
        }
        lines.push(String::new());
    }

    lines.join("\n")
}

pub fn render_pipeline_debug_json(report: &PipelineReport) -> Result<String> {
    Ok(serde_json::to_string_pretty(report)?)
}

pub fn render_project_markdown(report: &PipelineReport) -> String {
    let projects = report
        .projects
        .iter()
        .map(|project: &ProjectReport| (project.project_name.clone(), project.outcomes.clone()))
        .collect::<Vec<_>>();
    render_markdown(&report.date_label, &projects)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary_pipeline::outcome::{OutcomeCandidate, TonePolicy};

    #[test]
    fn fallback_renderer_outputs_project_sections() {
        let markdown = render_markdown(
            "2026-04-15",
            &[(
                "proj".to_string(),
                vec![OutcomeCandidate {
                    project_name: "proj".to_string(),
                    factual_headline: "TT-42 improved login validation".to_string(),
                    probable_outcome: "Improved login validation and tightened edge-case handling"
                        .to_string(),
                    supporting_messages: vec!["TT-42 add login validation".to_string()],
                    confidence: 4,
                    importance: 8,
                    tone_policy: TonePolicy::PolishOk,
                }],
            )],
        );

        assert!(markdown.contains("# Dev Journal — 2026-04-15"));
        assert!(markdown.contains("## proj"));
        assert!(markdown.contains("- Improved login validation and tightened edge-case handling"));
    }

    #[test]
    fn fallback_renderer_uses_literal_and_uncertain_branches() {
        let markdown = render_markdown(
            "2026-04-15",
            &[(
                "proj".to_string(),
                vec![
                    OutcomeCandidate {
                        project_name: "proj".to_string(),
                        factual_headline: "TT-42 tightened login checks".to_string(),
                        probable_outcome: "Improved login validation".to_string(),
                        supporting_messages: vec!["TT-42 improve login".to_string()],
                        confidence: 60,
                        importance: 8,
                        tone_policy: TonePolicy::StayLiteral,
                    },
                    OutcomeCandidate {
                        project_name: "proj".to_string(),
                        factual_headline: "TT-43 investigate flaky login test".to_string(),
                        probable_outcome: "Investigated flaky login test".to_string(),
                        supporting_messages: vec!["TT-43 investigate flaky login test".to_string()],
                        confidence: 40,
                        importance: 5,
                        tone_policy: TonePolicy::MentionUncertainty,
                    },
                ],
            )],
        );

        assert!(markdown.contains("- TT-42 tightened login checks"));
        assert!(markdown.contains("- Worked on TT-43 investigate flaky login test"));
    }
}
