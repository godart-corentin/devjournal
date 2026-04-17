pub mod anthropic;
pub mod ollama;
pub mod openai;

use crate::config::LlmProvider;
use crate::summary_pipeline::outcome::OutcomeCandidate;
use anyhow::Result;

pub trait LlmBackend {
    fn summarize(
        &self,
        outcomes: &[OutcomeCandidate],
        date: &str,
        custom_prompt: Option<&str>,
    ) -> Result<String>;
}

#[cfg(test)]
pub fn supported_providers() -> &'static [&'static str] {
    &["anthropic", "openai", "ollama"]
}

pub fn make_backend(
    provider: LlmProvider,
    api_key: &str,
    model: Option<&str>,
    base_url: Option<&str>,
) -> Box<dyn LlmBackend> {
    match provider {
        LlmProvider::OpenAi => Box::new(openai::OpenAiBackend {
            api_key: api_key.to_string(),
            model: model
                .unwrap_or(LlmProvider::OpenAi.default_model())
                .to_string(),
        }),
        LlmProvider::Ollama => Box::new(ollama::OllamaBackend {
            base_url: base_url.unwrap_or("http://localhost:11434").to_string(),
            model: model
                .unwrap_or(LlmProvider::Ollama.default_model())
                .to_string(),
        }),
        LlmProvider::Anthropic => Box::new(anthropic::AnthropicBackend {
            api_key: api_key.to_string(),
            model: model
                .unwrap_or(LlmProvider::Anthropic.default_model())
                .to_string(),
        }),
    }
}

#[cfg(test)]
fn build_prompt(outcomes: &[OutcomeCandidate], date: &str) -> String {
    build_prompt_with_custom(outcomes, date, None)
}

pub fn build_prompt_with_custom(
    outcomes: &[OutcomeCandidate],
    date: &str,
    custom_prompt: Option<&str>,
) -> String {
    let mut by_project = std::collections::BTreeMap::<String, Vec<&OutcomeCandidate>>::default();
    for outcome in outcomes {
        by_project
            .entry(outcome.project_name.clone())
            .or_default()
            .push(outcome);
    }

    let mut lines = vec![
        format!("Here are grouped standup outcome candidates for {}:", date),
        String::new(),
    ];

    for (project_name, project_outcomes) in &by_project {
        lines.push(format!("Project: {project_name}"));
        for outcome in project_outcomes {
            lines.push(format!("  - {}", writer_candidate_line(outcome)));
        }
        lines.push(String::new());
    }

    if let Some(custom) = custom_prompt {
        lines.push(custom.to_string());
    } else {
        lines.push("Write a Dev Journal summary for standup use.".to_string());
        lines.push("Rules:".to_string());
        lines.push(format!(
            "- Start the document with: # Dev Journal — {}",
            date
        ));
        lines.push("- Create exactly one ## section per project, using the exact project name above as the header. Do NOT invent additional sections or sub-sections.".to_string());
        lines.push("- Keep every bullet inside its own project. Never move, copy, or infer work across project sections.".to_string());
        lines.push("- Focus on OUTCOMES: what was shipped, fixed, or unblocked. Not the step-by-step process to get there.".to_string());
        lines.push(
            "- Treat the bullets above as the source material; prefer them over raw git metadata."
                .to_string(),
        );
        lines.push(
            "- Do NOT mention branch names, commit hashes, file counts, or other git metadata."
                .to_string(),
        );
        lines.push(
            "- Do NOT mention commit counts, clusters, workstreams, file areas, or internal pipeline mechanics."
                .to_string(),
        );
        lines.push(
            "- Preserve ticket/issue references if they appear in the candidates.".to_string(),
        );
        lines.push(
            "- Do NOT turn ticket/issue references into Markdown links unless a real URL is provided."
                .to_string(),
        );
        lines.push(
            "- Reuse the bullet wording above unless a small wording cleanup makes it clearer."
                .to_string(),
        );
        lines.push("- Write concise standup-ready bullets.".to_string());
    }

    lines.join("\n")
}

fn writer_candidate_line(outcome: &OutcomeCandidate) -> String {
    match outcome.tone_policy {
        crate::summary_pipeline::outcome::TonePolicy::PolishOk => outcome.probable_outcome.clone(),
        crate::summary_pipeline::outcome::TonePolicy::StayLiteral => {
            outcome.factual_headline.clone()
        }
        crate::summary_pipeline::outcome::TonePolicy::MentionUncertainty => {
            format!("Worked on {}", outcome.factual_headline)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary_pipeline::outcome::{OutcomeCandidate, TonePolicy};

    #[test]
    fn test_supported_providers_excludes_cursor() {
        assert_eq!(supported_providers(), &["anthropic", "openai", "ollama"]);
    }

    #[test]
    fn prompt_uses_outcome_candidates_and_concise_writer_rules() {
        let prompt = build_prompt(
            &[OutcomeCandidate {
                project_name: "proj-a".to_string(),
                factual_headline: "TT-42 improved login validation".to_string(),
                probable_outcome: "Improved login validation and fixed edge-case handling"
                    .to_string(),
                supporting_messages: vec![
                    "TT-42 add login validation".to_string(),
                    "TT-42 fix login edge case".to_string(),
                ],
                confidence: 4,
                importance: 9,
                tone_policy: TonePolicy::PolishOk,
            }],
            "2026-04-15",
        );

        let expected = [
            "Here are grouped standup outcome candidates for 2026-04-15:",
            "",
            "Project: proj-a",
            "  - Improved login validation and fixed edge-case handling",
            "",
            "Write a Dev Journal summary for standup use.",
            "Rules:",
            "- Start the document with: # Dev Journal — 2026-04-15",
            "- Create exactly one ## section per project, using the exact project name above as the header. Do NOT invent additional sections or sub-sections.",
            "- Keep every bullet inside its own project. Never move, copy, or infer work across project sections.",
            "- Focus on OUTCOMES: what was shipped, fixed, or unblocked. Not the step-by-step process to get there.",
            "- Treat the bullets above as the source material; prefer them over raw git metadata.",
            "- Do NOT mention branch names, commit hashes, file counts, or other git metadata.",
            "- Do NOT mention commit counts, clusters, workstreams, file areas, or internal pipeline mechanics.",
            "- Preserve ticket/issue references if they appear in the candidates.",
            "- Do NOT turn ticket/issue references into Markdown links unless a real URL is provided.",
            "- Reuse the bullet wording above unless a small wording cleanup makes it clearer.",
            "- Write concise standup-ready bullets.",
        ]
        .join("\n");

        assert_eq!(prompt, expected);
    }

    #[test]
    fn prompt_groups_multiple_projects_in_sorted_order() {
        let prompt = build_prompt(
            &[
                OutcomeCandidate {
                    project_name: "proj-b".to_string(),
                    factual_headline: "TT-7 tightened release checks".to_string(),
                    probable_outcome: "Tightened release checks".to_string(),
                    supporting_messages: vec!["TT-7 tighten checks".to_string()],
                    confidence: 80,
                    importance: 1,
                    tone_policy: TonePolicy::StayLiteral,
                },
                OutcomeCandidate {
                    project_name: "proj-a".to_string(),
                    factual_headline: "TT-42 improved login validation".to_string(),
                    probable_outcome: "Improved login validation".to_string(),
                    supporting_messages: vec!["TT-42 add login validation".to_string()],
                    confidence: 90,
                    importance: 2,
                    tone_policy: TonePolicy::PolishOk,
                },
            ],
            "2026-04-15 to 2026-04-16",
        );

        assert!(prompt.contains("Project: proj-a"));
        assert!(prompt.contains("Project: proj-b"));
        assert!(prompt.find("Project: proj-a").unwrap() < prompt.find("Project: proj-b").unwrap());
    }

    #[test]
    fn prompt_includes_custom_prompt_when_provided() {
        let prompt = build_prompt_with_custom(
            &[OutcomeCandidate {
                project_name: "proj-a".to_string(),
                factual_headline: "TT-1 add feature".to_string(),
                probable_outcome: "Added feature".to_string(),
                supporting_messages: vec!["TT-1 add feature".to_string()],
                confidence: 80,
                importance: 1,
                tone_policy: TonePolicy::PolishOk,
            }],
            "2026-03-23",
            Some("Custom instructions here."),
        );

        assert!(prompt.contains("Custom instructions here."));
        assert!(!prompt.contains("Rules:"));
    }

    #[test]
    fn prompt_explicitly_bans_pipeline_mechanics_and_fake_ticket_links() {
        let prompt = build_prompt(
            &[OutcomeCandidate {
                project_name: "proj-a".to_string(),
                factual_headline: "TT-42 inline LLM setup for summaries".to_string(),
                probable_outcome: "Added inline LLM setup for summaries".to_string(),
                supporting_messages: vec!["inline LLM setup for summaries".to_string()],
                confidence: 80,
                importance: 1,
                tone_policy: TonePolicy::PolishOk,
            }],
            "2026-04-15",
        );

        assert!(prompt.contains("Do NOT mention commit counts, clusters, workstreams, file areas, or internal pipeline mechanics."));
        assert!(prompt.contains("Do NOT turn ticket/issue references into Markdown links unless a real URL is provided."));
    }

    #[test]
    fn prompt_does_not_label_bullets_as_candidate_lines() {
        let prompt = build_prompt(
            &[OutcomeCandidate {
                project_name: "proj-a".to_string(),
                factual_headline: "TT-42 tightened login checks".to_string(),
                probable_outcome: "Tightened login checks".to_string(),
                supporting_messages: vec!["TT-42 tighten login checks".to_string()],
                confidence: 80,
                importance: 1,
                tone_policy: TonePolicy::PolishOk,
            }],
            "2026-04-15",
        );

        assert!(prompt.contains("  - Tightened login checks"));
        assert!(!prompt.contains("candidate line:"));
    }

    #[test]
    fn prompt_does_not_expose_control_metadata_fields() {
        let prompt = build_prompt(
            &[OutcomeCandidate {
                project_name: "proj-a".to_string(),
                factual_headline: "TT-42 tightened login checks".to_string(),
                probable_outcome: "Improved login validation".to_string(),
                supporting_messages: vec!["TT-42 improve login".to_string()],
                confidence: 60,
                importance: 8,
                tone_policy: TonePolicy::StayLiteral,
            }],
            "2026-04-15",
        );

        assert!(!prompt.contains("confidence:"));
        assert!(!prompt.contains("tone policy:"));
        assert!(!prompt.contains("PolishOk"));
        assert!(!prompt.contains("StayLiteral"));
        assert!(!prompt.contains("MentionUncertainty"));
    }

    #[test]
    fn prompt_does_not_include_supporting_evidence_lines() {
        let prompt = build_prompt(
            &[OutcomeCandidate {
                project_name: "proj-a".to_string(),
                factual_headline: "TT-42 tightened login checks".to_string(),
                probable_outcome: "Improved login validation".to_string(),
                supporting_messages: vec![
                    "TT-42 improve login".to_string(),
                    "TT-42 fix edge case".to_string(),
                ],
                confidence: 60,
                importance: 8,
                tone_policy: TonePolicy::StayLiteral,
            }],
            "2026-04-15",
        );

        assert!(!prompt.contains("supporting evidence:"));
        assert!(!prompt.contains("supported by"));
    }
}
