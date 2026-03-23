pub mod claude;
pub mod openai;

use anyhow::Result;
use crate::db::Event;

pub trait LlmBackend {
    fn summarize(&self, events: &[Event], date: &str) -> Result<String>;
}

pub fn make_backend(provider: &str, api_key: &str, model: Option<&str>) -> Box<dyn LlmBackend> {
    match provider {
        "openai" => Box::new(openai::OpenAiBackend {
            api_key: api_key.to_string(),
            model: model.unwrap_or("gpt-4o").to_string(),
        }),
        _ => Box::new(claude::ClaudeBackend {
            api_key: api_key.to_string(),
            model: model.unwrap_or("claude-sonnet-4-6").to_string(),
        }),
    }
}

pub fn build_prompt(events: &[Event], date: &str) -> String {
    let mut lines = vec![
        format!("Here are all git commits recorded on {}:", date),
        String::new(),
    ];

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

    lines.push("Please write a daily work summary from the perspective of the developer who made these commits.".to_string());
    lines.push("Rules:".to_string());
    lines.push("- Group entries by project (use the project names above as section headers with ## markdown)".to_string());
    lines.push("- Write concise action-oriented bullet points describing what was done, fixed, tested, or shipped".to_string());
    lines.push("- Preserve ticket/issue references (e.g. TT-1234, PROJ-567) if present in commit messages".to_string());
    lines.push("- Do NOT mention branch names, file counts, or other git metadata".to_string());
    lines.push("- Do NOT add a reflections section or subjective commentary".to_string());
    lines.push(format!("- Start the document with: # Dev Journal — {}", date));

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
        assert!(prompt.contains("action-oriented"));
    }
}
