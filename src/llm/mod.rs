pub mod anthropic;
pub mod ollama;
pub mod openai;

use crate::config::LlmProvider;
use crate::db::Event;
use crate::sem::from_value as sem_from_value;
use anyhow::Result;

pub trait LlmBackend {
    fn summarize(
        &self,
        events: &[Event],
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
fn build_prompt(events: &[Event], date: &str) -> String {
    build_prompt_with_custom(events, date, None)
}

pub fn build_prompt_with_custom(
    events: &[Event],
    date: &str,
    custom_prompt: Option<&str>,
) -> String {
    let is_range = date.contains(" to ");

    let mut lines = if is_range {
        vec![
            format!("Here are all git commits recorded from {}:", date),
            String::new(),
        ]
    } else {
        vec![
            format!("Here are all git commits recorded on {}:", date),
            String::new(),
        ]
    };

    let mut repos: std::collections::BTreeMap<String, Vec<&Event>> = Default::default();
    for e in events {
        let name = e.repo_name.clone().unwrap_or_else(|| e.repo_path.clone());
        repos.entry(name).or_default().push(e);
    }

    for (repo_name, events) in &repos {
        lines.push(format!("Project: {}", repo_name));
        for e in events {
            let branch = e.data["branch"].as_str().unwrap_or("unknown");
            let message = e.data["message"].as_str().unwrap_or("no message");
            let hash = e.data["hash"].as_str().unwrap_or("?");
            lines.push(format!("  - [{}] ({}) {}", hash, branch, message));
            let sem = sem_from_value(&e.data["sem"]);
            if let Some(sem) = sem {
                lines.push(format!("    semantic summary: {}", sem.summary));

                if !sem.entities.is_empty() {
                    let entities = sem
                        .entities
                        .iter()
                        .map(|entity| {
                            format!("{} {} [{}]", entity.kind, entity.name, entity.change_type)
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    lines.push(format!("    entities: {}", entities));
                }

                if !sem.change_types.is_empty() {
                    lines.push(format!("    change types: {}", sem.change_types.join(", ")));
                }

                if !sem.files.is_empty() {
                    lines.push(format!("    files: {}", sem.files.join(", ")));
                }
            } else if let Some(diff) = e.data.get("diff") {
                if let Some(stat_summary) =
                    diff.get("stat_summary").and_then(|value| value.as_str())
                {
                    lines.push(format!("    diff summary: {}", stat_summary));
                }

                if let Some(files) = diff.get("files").and_then(|value| value.as_array()) {
                    let files = files
                        .iter()
                        .filter_map(|file| {
                            let path = file.get("path")?.as_str()?;
                            let status = file
                                .get("status")
                                .and_then(|value| value.as_str())
                                .unwrap_or("modified");
                            let additions = file
                                .get("additions")
                                .and_then(|value| value.as_u64())
                                .unwrap_or(0);
                            let deletions = file
                                .get("deletions")
                                .and_then(|value| value.as_u64())
                                .unwrap_or(0);
                            Some(format!(
                                "{} {} (+{}/-{})",
                                status, path, additions, deletions
                            ))
                        })
                        .collect::<Vec<_>>();
                    if !files.is_empty() {
                        lines.push(format!("    diff files: {}", files.join(", ")));
                    }
                }

                if let Some(patch_excerpt) =
                    diff.get("patch_excerpt").and_then(|value| value.as_str())
                {
                    lines.push("    patch excerpt:".to_string());
                    lines.push(format!("```diff\n{}\n```", patch_excerpt));
                }
            }
        }
        lines.push(String::new());
    }

    if let Some(custom) = custom_prompt {
        lines.push(custom.to_string());
    } else {
        if is_range {
            lines.push("Please write a multi-day summary from the perspective of the developer who made these commits.".to_string());
            lines.push(
                "This covers multiple days — highlight key outcomes and progress across the period."
                    .to_string(),
            );
        } else {
            lines.push("Please write a daily standup summary from the perspective of the developer who made these commits.".to_string());
            lines.push("This will be read aloud in a standup meeting — it must take no more than 1-3 minutes to read.".to_string());
        }
        lines.push("Rules:".to_string());
        lines.push(format!(
            "- Start the document with: # Dev Journal — {}",
            date
        ));
        lines.push("- Create exactly one ## section per project, using the exact project name listed above as the header. Do NOT invent additional sections or sub-sections.".to_string());
        lines.push("- STRICT ATTRIBUTION: each bullet must only describe commits listed under that specific project. Never move, copy, or infer work across project sections. A ticket number appearing in multiple projects must be described independently in each.".to_string());
        if is_range {
            lines.push("- Each project section should have 1-5 bullets (scale with the number of days). Merge closely related commits.".to_string());
            lines.push(
                "- Within each project section, organize bullets chronologically.".to_string(),
            );
        } else {
            lines.push(
                "- Each project section should have 1-3 bullets max. Merge related commits aggressively."
                    .to_string(),
            );
        }
        lines.push("- Focus on OUTCOMES: what was shipped, fixed, or unblocked. Not the step-by-step process to get there.".to_string());
        lines.push("- When semantic summary, entities, or files are present for a commit, prefer those concrete signals to infer the real outcome instead of relying only on the commit message.".to_string());
        lines.push("- When semantic metadata is missing, use structured diff summaries and diff file lists before falling back to any raw patch excerpt.".to_string());
        lines.push("- Treat any patch excerpt as supporting evidence only. Do not narrate the patch line-by-line in the final summary.".to_string());
        lines.push("- Collapse all iterative commits toward the same goal (lint fixes, import moves, minor fixes, test adjustments) into the final outcome bullet. Do not list them separately.".to_string());
        lines.push("- Group all commits sharing the same ticket ID (e.g. TT-1234) into a single bullet describing the net result.".to_string());
        lines.push(
            "- Preserve ticket/issue references (e.g. TT-1234, PROJ-567) if present in commit messages"
                .to_string(),
        );
        lines.push(
            "- Do NOT mention branch names, file counts, commit hashes, or other git metadata"
                .to_string(),
        );
        lines.push("- Do NOT add a reflections section or subjective commentary".to_string());
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Event;
    use crate::sem::{SemEntity, SemMetadata};

    #[test]
    fn test_supported_providers_excludes_cursor() {
        assert_eq!(supported_providers(), &["anthropic", "openai", "ollama"]);
    }

    fn make_event(
        repo_name: &str,
        message: &str,
        branch: &str,
        sem: Option<SemMetadata>,
        diff: Option<serde_json::Value>,
    ) -> Event {
        let mut data = serde_json::json!({
            "hash": "abc123",
            "author": "Dev",
            "message": message,
            "branch": branch,
            "files_changed": 2,
            "insertions": 10,
            "deletions": 5
        });
        if let Some(sem) = sem {
            data["sem"] = serde_json::to_value(sem).unwrap();
        }
        if let Some(diff) = diff {
            data["diff"] = diff;
        }

        Event {
            id: None,
            repo_path: "/tmp/repo".to_string(),
            repo_name: Some(repo_name.to_string()),
            event_type: "commit".to_string(),
            timestamp: "2026-03-23T10:00:00Z".to_string(),
            data,
        }
    }

    fn sample_sem() -> SemMetadata {
        SemMetadata {
            summary: "2 semantic changes across 1 files (1 added, 1 modified)".to_string(),
            entities: vec![
                SemEntity {
                    name: "validate_token".to_string(),
                    kind: "function".to_string(),
                    change_type: "added".to_string(),
                },
                SemEntity {
                    name: "authenticate_user".to_string(),
                    kind: "function".to_string(),
                    change_type: "modified".to_string(),
                },
            ],
            change_types: vec!["added".to_string(), "modified".to_string()],
            files: vec!["src/auth.rs".to_string()],
        }
    }

    #[test]
    fn prompt_uses_project_sections_and_commit_metadata() {
        let prompt = build_prompt(
            &[
                make_event(
                    "proj-a",
                    "Add auth token validation",
                    "main",
                    Some(sample_sem()),
                    None,
                ),
                make_event(
                    "proj-b",
                    "Fix UI spacing",
                    "feature/ui",
                    None,
                    Some(serde_json::json!({
                        "stat_summary": "1 file changed, 4 insertions(+), 1 deletion(-)",
                        "files": [{
                            "path": "src/ui.rs",
                            "status": "modified",
                            "additions": 4,
                            "deletions": 1
                        }]
                    })),
                ),
            ],
            "2026-03-23",
        );

        assert!(prompt.contains("Project: proj-a"));
        assert!(prompt.contains("Project: proj-b"));
        assert!(prompt.contains("[abc123] (main) Add auth token validation"));
        assert!(prompt.contains("semantic summary: 2 semantic changes across 1 files"));
        assert!(prompt.contains("diff summary: 1 file changed, 4 insertions(+), 1 deletion(-)"));
    }

    #[test]
    fn prompt_uses_multi_day_instructions_for_ranges() {
        let prompt = build_prompt(
            &[make_event("proj-a", "Add feature", "main", None, None)],
            "2026-03-01 to 2026-03-07",
        );

        assert!(prompt.contains("Here are all git commits recorded from 2026-03-01 to 2026-03-07:"));
        assert!(prompt.contains("Please write a multi-day summary"));
    }

    #[test]
    fn prompt_includes_custom_prompt_when_provided() {
        let prompt = build_prompt_with_custom(
            &[make_event("proj-a", "Add feature", "main", None, None)],
            "2026-03-23",
            Some("Custom instructions here."),
        );

        assert!(prompt.contains("Custom instructions here."));
        assert!(!prompt.contains("Rules:"));
    }
}
