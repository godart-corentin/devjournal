use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::db::Event;
use crate::sem;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageQuality {
    Clear,
    Mixed,
    LowSignal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeIntent {
    Feature,
    Bugfix,
    Refactor,
    Tests,
    Docs,
    Chore,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitEvidence {
    pub repo_name: String,
    pub timestamp: String,
    pub hash: String,
    pub message: String,
    pub normalized_message: String,
    pub ticket_ids: Vec<String>,
    pub file_paths: Vec<String>,
    pub file_areas: Vec<String>,
    pub semantic_entities: Vec<String>,
    pub semantic_change_types: Vec<String>,
    pub message_quality: MessageQuality,
    pub change_intent: ChangeIntent,
    pub has_sem: bool,
    pub has_patch_excerpt: bool,
}

pub fn normalize_events(events: &[Event]) -> Result<Vec<CommitEvidence>> {
    events.iter().map(normalize_event).collect()
}

pub fn normalize_event(event: &Event) -> Result<CommitEvidence> {
    let message = event.data["message"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let file_paths = event
        .data
        .get("diff")
        .and_then(|diff| diff.get("files"))
        .and_then(|files| files.as_array())
        .map(|files| {
            files
                .iter()
                .filter_map(|file| file.get("path").and_then(|value| value.as_str()))
                .map(|path| path.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let sem_metadata = event.data.get("sem").and_then(sem::from_value);
    let semantic_entities = sem_metadata
        .as_ref()
        .map(|sem| {
            sem.entities
                .iter()
                .map(|entity| format!("{}:{}", entity.kind, entity.name))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let semantic_change_types = sem_metadata
        .as_ref()
        .map(|sem| sem.change_types.clone())
        .unwrap_or_default();
    let has_patch_excerpt = event
        .data
        .get("diff")
        .and_then(|diff| diff.get("patch_excerpt"))
        .and_then(|value| value.as_str())
        .is_some();

    Ok(CommitEvidence {
        repo_name: event
            .repo_name
            .clone()
            .unwrap_or_else(|| event.repo_path.clone()),
        timestamp: event.timestamp.clone(),
        hash: event.data["hash"].as_str().unwrap_or("").to_string(),
        message: message.clone(),
        normalized_message: normalize_message(&message),
        ticket_ids: extract_ticket_ids(&message),
        file_paths: file_paths.clone(),
        file_areas: infer_file_areas(&file_paths),
        semantic_entities,
        semantic_change_types,
        message_quality: classify_message_quality(&message),
        change_intent: classify_change_intent(&message, &file_paths),
        has_sem: sem_metadata.is_some(),
        has_patch_excerpt,
    })
}

pub fn clean_message_for_display(message: &str, ticket_ids: &[String]) -> String {
    let without_ticket = strip_leading_ticket_token(message, ticket_ids);
    let without_prefix = strip_conventional_prefix(&without_ticket);
    let collapsed = without_prefix
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = collapsed
        .trim_matches(|ch: char| matches!(ch, ':' | ';' | ',' | '-' | '.'))
        .trim()
        .to_string();

    normalize_known_acronyms(&trimmed)
}

fn normalize_message(message: &str) -> String {
    message
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn extract_ticket_ids(message: &str) -> Vec<String> {
    let mut ticket_ids = Vec::new();
    let mut current = String::new();

    for ch in message.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            current.push(ch);
        } else if !current.is_empty() {
            push_ticket_id(&mut ticket_ids, &current);
            current.clear();
        }
    }

    if !current.is_empty() {
        push_ticket_id(&mut ticket_ids, &current);
    }

    ticket_ids
}

fn push_ticket_id(ticket_ids: &mut Vec<String>, candidate: &str) {
    let mut parts = candidate.split('-').collect::<Vec<_>>();
    if parts.len() < 2 {
        return;
    }

    let Some(last) = parts.pop() else {
        return;
    };
    if last.is_empty() || !last.chars().all(|ch| ch.is_ascii_digit()) {
        return;
    }
    if parts.is_empty() || !parts[0].chars().any(|ch| ch.is_ascii_alphabetic()) {
        return;
    }
    if !parts
        .iter()
        .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_alphanumeric()))
    {
        return;
    }

    let normalized = candidate.to_ascii_uppercase();
    if !ticket_ids.iter().any(|existing| existing == &normalized) {
        ticket_ids.push(normalized);
    }
}

fn infer_file_areas(file_paths: &[String]) -> Vec<String> {
    let mut file_areas = Vec::new();

    for path in file_paths {
        let area = infer_file_area(path);
        if !area.is_empty() && !file_areas.iter().any(|existing| existing == &area) {
            file_areas.push(area);
        }
    }

    file_areas
}

fn infer_file_area(path: &str) -> String {
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    match parts.as_slice() {
        [] => String::new(),
        [_single] => "root".to_string(),
        [first, second, ..] => format!("{}/{}", first, second),
    }
}

fn classify_message_quality(message: &str) -> MessageQuality {
    let normalized = normalize_message(message);
    if normalized.is_empty() {
        return MessageQuality::LowSignal;
    }

    if is_low_signal_message(&normalized) {
        return MessageQuality::LowSignal;
    }

    let word_count = normalized.split_whitespace().count();
    if word_count <= 2 {
        return MessageQuality::Mixed;
    }

    if contains_ticket_like(&normalized) || contains_any_word(&normalized, SPECIFIC_HINTS) {
        MessageQuality::Clear
    } else if contains_any_word(&normalized, MIXED_HINTS) {
        MessageQuality::Mixed
    } else {
        MessageQuality::Clear
    }
}

fn classify_change_intent(message: &str, file_paths: &[String]) -> ChangeIntent {
    let normalized = normalize_message(message);

    if is_tests_intent(&normalized, file_paths) {
        return ChangeIntent::Tests;
    }
    if is_docs_intent(&normalized, file_paths) {
        return ChangeIntent::Docs;
    }
    if contains_any_word(&normalized, REFACTOR_HINTS) {
        return ChangeIntent::Refactor;
    }
    if contains_any_word(&normalized, BUGFIX_HINTS) {
        return ChangeIntent::Bugfix;
    }
    if contains_any_word(&normalized, FEATURE_HINTS) {
        return ChangeIntent::Feature;
    }
    if contains_any_word(&normalized, CHORE_HINTS) {
        return ChangeIntent::Chore;
    }

    if let Some(intent) = classify_conventional_commit(message) {
        return intent;
    }

    ChangeIntent::Unknown
}

fn classify_conventional_commit(message: &str) -> Option<ChangeIntent> {
    let header = message.trim();
    let (head, _) = header.split_once(':')?;
    let head = head.strip_suffix('!').unwrap_or(head);
    let commit_type = head.split_once('(').map(|(kind, _)| kind).unwrap_or(head);

    match commit_type.trim().to_ascii_lowercase().as_str() {
        "feat" | "feature" => Some(ChangeIntent::Feature),
        "fix" | "bugfix" => Some(ChangeIntent::Bugfix),
        "refactor" => Some(ChangeIntent::Refactor),
        "test" | "tests" => Some(ChangeIntent::Tests),
        "doc" | "docs" => Some(ChangeIntent::Docs),
        "chore" => Some(ChangeIntent::Chore),
        _ => None,
    }
}

fn strip_leading_ticket_token(message: &str, ticket_ids: &[String]) -> String {
    let mut words = message.split_whitespace().collect::<Vec<_>>();
    if let Some(first) = words.first() {
        let first_token = first.trim_end_matches(':').trim_end_matches(',');
        if ticket_ids
            .iter()
            .any(|ticket| ticket.eq_ignore_ascii_case(first_token))
        {
            words.remove(0);
        }
    }

    words.join(" ").trim().to_string()
}

fn strip_conventional_prefix(message: &str) -> String {
    let trimmed = message.trim();
    let Some((head, tail)) = trimmed.split_once(':') else {
        return trimmed.to_string();
    };

    let head = head.strip_suffix('!').unwrap_or(head);
    let commit_type = head.split_once('(').map(|(kind, _)| kind).unwrap_or(head);
    let is_conventional = matches!(
        commit_type.trim().to_ascii_lowercase().as_str(),
        "feat"
            | "feature"
            | "fix"
            | "bugfix"
            | "refactor"
            | "test"
            | "tests"
            | "doc"
            | "docs"
            | "chore"
    );

    if is_conventional {
        tail.trim().to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_known_acronyms(message: &str) -> String {
    message
        .split_whitespace()
        .map(normalize_word_acronym)
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_word_acronym(word: &str) -> String {
    let start = word.find(|ch: char| ch.is_ascii_alphanumeric());
    let end = word.rfind(|ch: char| ch.is_ascii_alphanumeric());

    let (Some(start), Some(end)) = (start, end) else {
        return word.to_string();
    };

    let end = end + 1;
    let core = &word[start..end];
    let normalized = match core.to_ascii_lowercase().as_str() {
        "llm" => Some("LLM"),
        "cli" => Some("CLI"),
        "api" => Some("API"),
        "sql" => Some("SQL"),
        "ui" => Some("UI"),
        "ux" => Some("UX"),
        "id" => Some("ID"),
        "ids" => Some("IDs"),
        _ => None,
    };

    let Some(replacement) = normalized else {
        return word.to_string();
    };

    format!("{}{}{}", &word[..start], replacement, &word[end..])
}

fn is_low_signal_message(normalized: &str) -> bool {
    contains_any_word(normalized, LOW_SIGNAL_EXACT)
        || normalized.len() <= 3
        || normalized.split_whitespace().count() == 1
            && contains_any_word(normalized, LOW_SIGNAL_HINTS)
}

fn is_tests_intent(normalized: &str, file_paths: &[String]) -> bool {
    contains_any_word(normalized, TEST_HINTS)
        || file_paths.iter().any(|path| {
            let path = path.to_ascii_lowercase();
            path.contains("/tests/")
                || path.starts_with("tests/")
                || path.starts_with("test/")
                || path.ends_with("_test.rs")
                || path.ends_with(".test.rs")
        })
}

fn is_docs_intent(normalized: &str, file_paths: &[String]) -> bool {
    contains_any_word(normalized, DOC_HINTS)
        || file_paths.iter().any(|path| {
            let path = path.to_ascii_lowercase();
            path.starts_with("docs/")
                || path.contains("/docs/")
                || path.ends_with(".md")
                || path.ends_with(".rst")
        })
}

fn contains_ticket_like(normalized: &str) -> bool {
    normalized.split_whitespace().any(|token| {
        let mut parts = token.split('-');
        let Some(prefix) = parts.next() else {
            return false;
        };
        let Some(suffix) = parts.next() else {
            return false;
        };
        parts.next().is_none()
            && !prefix.is_empty()
            && prefix.chars().any(|ch| ch.is_ascii_alphabetic())
            && suffix.chars().all(|ch| ch.is_ascii_digit())
    })
}

fn contains_any_word(normalized: &str, patterns: &[&str]) -> bool {
    patterns
        .iter()
        .any(|pattern| normalized.split_whitespace().any(|word| word == *pattern))
}

const LOW_SIGNAL_EXACT: &[&str] = &["wip", "tbd", "todo", "temp", "misc", "stuff", "n/a", "na"];

const LOW_SIGNAL_HINTS: &[&str] = &[
    "wip", "tbd", "todo", "temp", "misc", "stuff", "fix", "update",
];

const MIXED_HINTS: &[&str] = &["refine", "adjust", "tweak", "cleanup", "small", "minor"];

const SPECIFIC_HINTS: &[&str] = &[
    "login", "signup", "auth", "invoice", "billing", "cache", "render", "summary", "pipeline",
    "export", "import", "api",
];

const TEST_HINTS: &[&str] = &["test", "tests", "spec", "specs", "coverage"];

const DOC_HINTS: &[&str] = &["doc", "docs", "readme", "changelog"];

const REFACTOR_HINTS: &[&str] = &["refactor", "restructure", "rename", "move", "cleanup"];

const BUGFIX_HINTS: &[&str] = &[
    "fix", "bug", "hotfix", "repair", "resolve", "crash", "error",
];

const FEATURE_HINTS: &[&str] = &[
    "add",
    "implement",
    "introduce",
    "support",
    "create",
    "enable",
];

const CHORE_HINTS: &[&str] = &[
    "chore",
    "bump",
    "deps",
    "dependency",
    "lint",
    "format",
    "release",
    "ci",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Event;

    fn make_event(message: &str, files: &[&str]) -> Event {
        Event {
            id: None,
            repo_path: "/tmp/proj".to_string(),
            repo_name: Some("proj".to_string()),
            event_type: "commit".to_string(),
            timestamp: "2026-04-15T09:00:00+02:00".to_string(),
            data: serde_json::json!({
                "hash": "abc12345",
                "message": message,
                "branch": "main",
                "diff": {
                    "stat_summary": "2 files changed, 6 insertions(+)",
                    "files": files.iter().map(|path| serde_json::json!({
                        "path": path,
                        "status": "modified",
                        "additions": 3,
                        "deletions": 0
                    })).collect::<Vec<_>>()
                },
                "sem": {
                    "summary": "2 semantic changes across 2 files",
                    "entities": [{
                        "name": "validate_login",
                        "kind": "function",
                        "change_type": "modified"
                    }],
                    "change_types": ["modified"],
                    "files": files
                }
            }),
        }
    }

    #[test]
    fn normalize_event_extracts_ticket_ids_and_file_areas() {
        let normalized = normalize_event(&make_event(
            "TT-42 refine login validation",
            &["src/auth/login.rs", "tests/auth/login_test.rs"],
        ))
        .unwrap();

        assert_eq!(normalized.ticket_ids, vec!["TT-42".to_string()]);
        assert!(normalized.file_areas.contains(&"src/auth".to_string()));
        assert!(normalized.file_areas.contains(&"tests/auth".to_string()));
    }

    #[test]
    fn normalize_event_marks_low_signal_messages() {
        let normalized = normalize_event(&make_event("wip", &["src/auth/login.rs"])).unwrap();

        assert_eq!(normalized.message_quality, MessageQuality::LowSignal);
    }

    #[test]
    fn normalize_event_classifies_conventional_bugfix_messages() {
        let normalized = normalize_event(&make_event(
            "fix(auth): handle nil session",
            &["src/auth/session.rs"],
        ))
        .unwrap();

        assert_eq!(normalized.change_intent, ChangeIntent::Bugfix);
    }

    #[test]
    fn normalize_event_classifies_conventional_feature_messages() {
        let normalized =
            normalize_event(&make_event("feat: add export", &["src/export.rs"])).unwrap();

        assert_eq!(normalized.change_intent, ChangeIntent::Feature);
    }

    #[test]
    fn normalize_event_infers_tests_intent_from_file_paths() {
        let normalized = normalize_event(&make_event(
            "chore: reorganize files",
            &["tests/auth/login_test.rs"],
        ))
        .unwrap();

        assert_eq!(normalized.change_intent, ChangeIntent::Tests);
    }

    #[test]
    fn normalize_event_infers_docs_intent_from_file_paths() {
        let normalized =
            normalize_event(&make_event("chore: reorganize files", &["docs/standup.md"])).unwrap();

        assert_eq!(normalized.change_intent, ChangeIntent::Docs);
    }

    #[test]
    fn normalize_event_extracts_semantic_entities_and_change_types() {
        let normalized =
            normalize_event(&make_event("feat: add export", &["src/export.rs"])).unwrap();

        assert_eq!(
            normalized.semantic_entities,
            vec!["function:validate_login".to_string()]
        );
        assert_eq!(
            normalized.semantic_change_types,
            vec!["modified".to_string()]
        );
        assert!(normalized.has_sem);
    }

    #[test]
    fn classify_conventional_commit_handles_scoped_feature_headers() {
        assert_eq!(
            classify_conventional_commit("feat(ui): export csv"),
            Some(ChangeIntent::Feature)
        );
    }

    #[test]
    fn classify_conventional_commit_handles_scoped_bugfix_headers() {
        assert_eq!(
            classify_conventional_commit("fix(auth): handle nil session"),
            Some(ChangeIntent::Bugfix)
        );
    }

    #[test]
    fn classify_conventional_commit_handles_breaking_feature_headers() {
        assert_eq!(
            classify_conventional_commit("feat!: export csv"),
            Some(ChangeIntent::Feature)
        );
    }

    #[test]
    fn classify_conventional_commit_handles_breaking_bugfix_headers() {
        assert_eq!(
            classify_conventional_commit("fix!: handle nil session"),
            Some(ChangeIntent::Bugfix)
        );
    }

    #[test]
    fn clean_message_for_display_strips_conventional_prefixes_and_normalizes_acronyms() {
        let cleaned = clean_message_for_display("feat(cli): inline llm setup for summaries", &[]);

        assert_eq!(cleaned, "inline LLM setup for summaries");
    }
}
