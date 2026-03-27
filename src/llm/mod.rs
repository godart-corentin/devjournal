pub mod claude;
pub mod cursor;
pub mod ollama;
pub mod openai;

use crate::db::Event;
use anyhow::Result;

pub trait LlmBackend {
    fn summarize(
        &self,
        events: &[Event],
        date: &str,
        custom_prompt: Option<&str>,
    ) -> Result<String>;
}

pub fn make_backend(
    provider: &str,
    api_key: &str,
    model: Option<&str>,
    base_url: Option<&str>,
) -> Box<dyn LlmBackend> {
    match provider {
        "openai" => Box::new(openai::OpenAiBackend {
            api_key: api_key.to_string(),
            model: model.unwrap_or("gpt-4o").to_string(),
        }),
        "ollama" => Box::new(ollama::OllamaBackend {
            base_url: base_url.unwrap_or("http://localhost:11434").to_string(),
            model: model.unwrap_or("llama3.2").to_string(),
        }),
        "cursor" => Box::new(cursor::CursorBackend {
            model: model.unwrap_or(cursor::DEFAULT_MODEL).to_string(),
        }),
        _ => Box::new(claude::ClaudeBackend {
            api_key: api_key.to_string(),
            model: model.unwrap_or("claude-sonnet-4-6").to_string(),
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

    // Group by repo
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

    fn make_event(repo_name: &str, message: &str, branch: &str) -> Event {
        Event {
            id: None,
            repo_path: "/tmp/repo".to_string(),
            repo_name: Some(repo_name.to_string()),
            event_type: "commit".to_string(),
            timestamp: "2026-03-23T10:00:00Z".to_string(),
            data: serde_json::json!({
                "hash": "abc123",
                "author": "Dev",
                "message": message,
                "branch": branch,
                "files_changed": 2,
                "insertions": 10,
                "deletions": 5
            }),
        }
    }

    #[test]
    fn test_prompt_groups_by_project() {
        let events = vec![
            make_event("project-alpha", "Fix TT-1234 bug", "main"),
            make_event("project-beta", "Add feature X", "feature/x"),
            make_event("project-alpha", "Refactor auth", "main"),
        ];
        let prompt = build_prompt(&events, "2026-03-23");
        assert!(prompt.contains("Project: project-alpha"));
        assert!(prompt.contains("Project: project-beta"));
        assert!(prompt.contains("Fix TT-1234 bug"));
        assert!(prompt.contains("# Dev Journal — 2026-03-23"));
    }

    #[test]
    fn test_prompt_no_metadata_instructions() {
        let events = vec![make_event("proj", "commit msg", "main")];
        let prompt = build_prompt(&events, "2026-03-23");
        assert!(prompt.contains("Do NOT mention branch names"));
        assert!(prompt.contains("OUTCOMES"));
        assert!(prompt.contains("standup"));
        assert!(prompt.contains("ticket ID"));
    }

    #[test]
    fn test_build_prompt_with_custom_system_prompt() {
        let events = vec![make_event("proj", "commit msg", "main")];
        let custom = "Write a haiku summarizing the work.";
        let prompt = build_prompt_with_custom(&events, "2026-03-23", Some(custom));
        assert!(prompt.contains("Project: proj"));
        assert!(prompt.contains("Write a haiku"));
        // Should NOT contain default rules
        assert!(!prompt.contains("OUTCOMES"));
        assert!(!prompt.contains("standup"));
    }
}
