use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::summary_pipeline::cluster::WorkstreamCandidate;
use crate::summary_pipeline::evidence::{clean_message_for_display, extract_ticket_ids};
use crate::summary_pipeline::ProjectReport;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TonePolicy {
    PolishOk,
    StayLiteral,
    MentionUncertainty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeCandidate {
    pub project_name: String,
    pub factual_headline: String,
    pub probable_outcome: String,
    pub supporting_messages: Vec<String>,
    pub confidence: u8,
    pub importance: u32,
    pub tone_policy: TonePolicy,
}

pub fn derive_outcome(workstream: WorkstreamCandidate) -> OutcomeCandidate {
    let outcome_kind = classify_workstream_kind(&workstream);
    let factual_headline = build_factual_headline(&workstream);
    let probable_outcome = summarize_probable_outcome(&workstream, outcome_kind);
    let project_name = workstream.project_name;
    let supporting_messages = workstream
        .member_messages
        .iter()
        .map(|message| clean_message_for_display(message, &workstream.ticket_ids))
        .filter(|message| !message.is_empty())
        .collect();
    let confidence = workstream.confidence;
    let importance = workstream.signal_score;

    let tone_policy = if confidence >= 75 {
        TonePolicy::PolishOk
    } else if confidence >= 50 {
        TonePolicy::StayLiteral
    } else {
        TonePolicy::MentionUncertainty
    };

    OutcomeCandidate {
        project_name,
        factual_headline: factual_headline.trim().to_string(),
        probable_outcome,
        supporting_messages,
        confidence,
        importance,
        tone_policy,
    }
}

fn build_factual_headline(workstream: &WorkstreamCandidate) -> String {
    let cleaned_message = select_primary_message(workstream, classify_workstream_kind(workstream));
    let Some(ticket) = workstream.ticket_ids.first() else {
        return cleaned_message;
    };

    if cleaned_message.is_empty() {
        ticket.to_string()
    } else {
        format!("{ticket} {cleaned_message}")
    }
}

pub fn build_project_reports(
    workstreams: Vec<WorkstreamCandidate>,
    max_outcomes_per_project: usize,
) -> Vec<ProjectReport> {
    let mut grouped = BTreeMap::<String, Vec<(OutcomeKind, OutcomeCandidate)>>::new();
    for workstream in workstreams {
        let outcome_kind = classify_workstream_kind(&workstream);
        let outcome = derive_outcome(workstream);
        grouped
            .entry(outcome.project_name.clone())
            .or_default()
            .push((outcome_kind, outcome));
    }

    grouped
        .into_iter()
        .map(|(project_name, mut outcomes)| {
            let has_high_value = outcomes
                .iter()
                .any(|(kind, _)| matches!(kind, OutcomeKind::Feature | OutcomeKind::Bugfix));
            if has_high_value {
                outcomes.retain(|(kind, outcome)| {
                    !matches!(kind, OutcomeKind::Maintenance)
                        || outcome.tone_policy != TonePolicy::MentionUncertainty
                });
            }

            outcomes.sort_by(|(_, left), (_, right)| {
                right
                    .importance
                    .cmp(&left.importance)
                    .then_with(|| right.confidence.cmp(&left.confidence))
                    .then_with(|| left.factual_headline.cmp(&right.factual_headline))
            });
            let mut outcomes = outcomes
                .into_iter()
                .map(|(_, outcome)| outcome)
                .collect::<Vec<_>>();
            outcomes.truncate(max_outcomes_per_project);
            ProjectReport {
                project_name,
                outcomes,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutcomeKind {
    Feature,
    Bugfix,
    Maintenance,
    Unknown,
}

fn summarize_probable_outcome(
    workstream: &WorkstreamCandidate,
    outcome_kind: OutcomeKind,
) -> String {
    let message = select_primary_message(workstream, outcome_kind);
    if message.is_empty() {
        return fallback_outcome_from_context(workstream, outcome_kind);
    }

    match outcome_kind {
        OutcomeKind::Feature => {
            format_outcome_with_prefix(&message, "Added", FEATURE_LEADING_VERBS)
        }
        OutcomeKind::Bugfix => format_outcome_with_prefix(&message, "Fixed", BUGFIX_LEADING_VERBS),
        OutcomeKind::Maintenance => format_maintenance_outcome(&message),
        OutcomeKind::Unknown => capitalize_first(&message),
    }
}

fn select_primary_message(workstream: &WorkstreamCandidate, outcome_kind: OutcomeKind) -> String {
    workstream
        .member_messages
        .iter()
        .max_by_key(|message| score_message(message, &workstream.ticket_ids, outcome_kind))
        .map(|message| clean_message_for_display(message, &workstream.ticket_ids))
        .unwrap_or_default()
}

fn score_message(
    message: &str,
    ticket_ids: &[String],
    desired_kind: OutcomeKind,
) -> (u8, usize, usize) {
    let cleaned = clean_message_for_display(message, ticket_ids);
    let kind = classify_message_kind(message);
    let desired_bonus = if kind == desired_kind { 1 } else { 0 };
    (
        kind_rank(kind) + desired_bonus,
        cleaned.split_whitespace().count(),
        cleaned.len(),
    )
}

fn kind_rank(kind: OutcomeKind) -> u8 {
    match kind {
        OutcomeKind::Feature => 4,
        OutcomeKind::Bugfix => 3,
        OutcomeKind::Unknown => 2,
        OutcomeKind::Maintenance => 1,
    }
}

fn classify_workstream_kind(workstream: &WorkstreamCandidate) -> OutcomeKind {
    workstream
        .member_messages
        .iter()
        .map(|message| classify_message_kind(message))
        .max_by_key(|kind| kind_rank(*kind))
        .unwrap_or(OutcomeKind::Unknown)
}

fn classify_message_kind(message: &str) -> OutcomeKind {
    let normalized = message.to_ascii_lowercase();
    let trimmed = normalized.trim();
    let cleaned = clean_message_for_display(message, &extract_ticket_ids(message));

    let is_feature = trimmed.starts_with("feat")
        || trimmed.starts_with("feature")
        || starts_with_phrase(&cleaned, FEATURE_LEADING_VERBS);
    if is_feature {
        return OutcomeKind::Feature;
    }

    let is_bugfix = trimmed.starts_with("fix")
        || trimmed.starts_with("bugfix")
        || starts_with_phrase(
            &cleaned,
            &[
                "fix", "fixed", "resolve", "resolved", "repair", "repaired", "handle",
            ],
        );
    if is_bugfix {
        return OutcomeKind::Bugfix;
    }

    let is_maintenance = trimmed.starts_with("refactor")
        || trimmed.starts_with("chore")
        || trimmed.starts_with("docs")
        || trimmed.starts_with("test")
        || starts_with_phrase(
            &cleaned,
            &[
                "refactor",
                "refactored",
                "rename",
                "renamed",
                "cleanup",
                "clean up",
                "remove",
                "removed",
                "drop",
                "dropped",
            ],
        );
    if is_maintenance {
        return OutcomeKind::Maintenance;
    }

    OutcomeKind::Unknown
}

fn format_outcome_with_prefix(message: &str, prefix: &str, leading_verbs: &[&str]) -> String {
    let stripped = strip_leading_phrase(message, leading_verbs);
    let body = if stripped.is_empty() {
        message
    } else {
        &stripped
    };
    format!("{prefix} {body}")
}

fn format_maintenance_outcome(message: &str) -> String {
    if starts_with_phrase(
        message,
        &["remove", "removed", "drop", "dropped", "delete", "deleted"],
    ) {
        return format_outcome_with_prefix(
            message,
            "Removed",
            &["remove", "removed", "drop", "dropped", "delete", "deleted"],
        );
    }
    if starts_with_phrase(message, &["rename", "renamed"]) {
        return format_outcome_with_prefix(message, "Renamed", &["rename", "renamed"]);
    }
    if starts_with_phrase(
        message,
        &[
            "refactor",
            "refactored",
            "cleanup",
            "clean up",
            "simplify",
            "simplified",
        ],
    ) {
        return format_outcome_with_prefix(
            message,
            "Simplified",
            &[
                "refactor",
                "refactored",
                "cleanup",
                "clean up",
                "simplify",
                "simplified",
            ],
        );
    }

    capitalize_first(message)
}

fn strip_leading_phrase(message: &str, phrases: &[&str]) -> String {
    let trimmed = message.trim();
    for phrase in phrases {
        if trimmed.len() > phrase.len()
            && trimmed[..phrase.len()].eq_ignore_ascii_case(phrase)
            && trimmed[phrase.len()..].starts_with(' ')
        {
            return trimmed[phrase.len()..].trim().to_string();
        }
    }

    trimmed.to_string()
}

fn starts_with_phrase(message: &str, phrases: &[&str]) -> bool {
    let trimmed = message.trim();
    phrases.iter().any(|phrase| {
        trimmed.len() == phrase.len() && trimmed.eq_ignore_ascii_case(phrase)
            || trimmed.len() > phrase.len()
                && trimmed[..phrase.len()].eq_ignore_ascii_case(phrase)
                && trimmed[phrase.len()..].starts_with(' ')
    })
}

fn fallback_outcome_from_context(
    workstream: &WorkstreamCandidate,
    outcome_kind: OutcomeKind,
) -> String {
    let subject = workstream
        .semantic_entities
        .first()
        .cloned()
        .or_else(|| {
            workstream
                .file_areas
                .iter()
                .find(|area| area.as_str() != "root")
                .cloned()
        })
        .unwrap_or_else(|| "related work".to_string());

    match outcome_kind {
        OutcomeKind::Feature => format!("Added {subject}"),
        OutcomeKind::Bugfix => format!("Fixed {subject}"),
        OutcomeKind::Maintenance => format!("Simplified {subject}"),
        OutcomeKind::Unknown => format!("Worked on {subject}"),
    }
}

fn capitalize_first(message: &str) -> String {
    let mut chars = message.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

const FEATURE_LEADING_VERBS: &[&str] = &[
    "add",
    "added",
    "implement",
    "implemented",
    "introduce",
    "introduced",
    "enable",
    "enabled",
    "support",
    "supported",
    "create",
    "created",
];

const BUGFIX_LEADING_VERBS: &[&str] =
    &["fix", "fixed", "resolve", "resolved", "repair", "repaired"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary_pipeline::cluster::WorkstreamCandidate;

    #[test]
    fn outcome_uses_realistic_confidence_thresholds() {
        let polished = derive_outcome(WorkstreamCandidate {
            project_name: "proj".to_string(),
            member_hashes: vec!["a".to_string()],
            member_messages: vec!["TT-42 add login validation".to_string()],
            ticket_ids: vec!["TT-42".to_string()],
            file_areas: vec!["src/auth".to_string()],
            semantic_entities: vec!["function:validate_login".to_string()],
            signal_score: 8,
            confidence: 82,
            rationale: vec!["shared ticket".to_string()],
        });

        let literal = derive_outcome(WorkstreamCandidate {
            project_name: "proj".to_string(),
            member_hashes: vec!["b".to_string()],
            member_messages: vec!["Tightened login path".to_string()],
            ticket_ids: vec!["TT-42".to_string()],
            file_areas: vec!["src/auth".to_string()],
            semantic_entities: vec!["function:validate_login".to_string()],
            signal_score: 5,
            confidence: 60,
            rationale: vec!["shared ticket".to_string()],
        });

        let uncertain = derive_outcome(WorkstreamCandidate {
            project_name: "proj".to_string(),
            member_hashes: vec!["c".to_string()],
            member_messages: vec!["wip".to_string()],
            ticket_ids: vec!["TT-42".to_string()],
            file_areas: vec!["src/auth".to_string()],
            semantic_entities: vec!["function:validate_login".to_string()],
            signal_score: 2,
            confidence: 40,
            rationale: vec!["shared ticket".to_string()],
        });

        assert_eq!(polished.tone_policy, TonePolicy::PolishOk);
        assert_eq!(literal.tone_policy, TonePolicy::StayLiteral);
        assert_eq!(uncertain.tone_policy, TonePolicy::MentionUncertainty);
    }

    #[test]
    fn outcome_does_not_duplicate_leading_ticket_prefix() {
        let outcome = derive_outcome(WorkstreamCandidate {
            project_name: "proj".to_string(),
            member_hashes: vec!["a".to_string()],
            member_messages: vec!["TT-42 add login validation".to_string()],
            ticket_ids: vec!["TT-42".to_string()],
            file_areas: vec!["src/auth".to_string()],
            semantic_entities: vec!["function:validate_login".to_string()],
            signal_score: 8,
            confidence: 82,
            rationale: vec!["shared ticket".to_string()],
        });

        assert_eq!(outcome.factual_headline, "TT-42 add login validation");
    }

    #[test]
    fn outcome_strips_ticket_punctuation_from_probable_outcome() {
        let outcome = derive_outcome(WorkstreamCandidate {
            project_name: "proj".to_string(),
            member_hashes: vec!["a".to_string()],
            member_messages: vec!["TT-42, add login validation".to_string()],
            ticket_ids: vec!["TT-42".to_string()],
            file_areas: vec!["src/auth".to_string()],
            semantic_entities: vec!["function:validate_login".to_string()],
            signal_score: 8,
            confidence: 82,
            rationale: vec!["shared ticket".to_string()],
        });

        assert_eq!(outcome.probable_outcome, "Added login validation");
    }

    #[test]
    fn build_project_reports_orders_and_truncates_deterministically() {
        let reports = build_project_reports(
            vec![
                WorkstreamCandidate {
                    project_name: "proj".to_string(),
                    member_hashes: vec!["a".to_string()],
                    member_messages: vec!["TT-1 primary".to_string()],
                    ticket_ids: vec!["TT-1".to_string()],
                    file_areas: vec!["src/a".to_string()],
                    semantic_entities: vec![],
                    signal_score: 10,
                    confidence: 90,
                    rationale: vec!["a".to_string()],
                },
                WorkstreamCandidate {
                    project_name: "proj".to_string(),
                    member_hashes: vec!["b".to_string()],
                    member_messages: vec!["TT-2 tie high confidence".to_string()],
                    ticket_ids: vec!["TT-2".to_string()],
                    file_areas: vec!["src/b".to_string()],
                    semantic_entities: vec![],
                    signal_score: 8,
                    confidence: 80,
                    rationale: vec!["b".to_string()],
                },
                WorkstreamCandidate {
                    project_name: "proj".to_string(),
                    member_hashes: vec!["c".to_string()],
                    member_messages: vec!["TT-3 tie lower confidence".to_string()],
                    ticket_ids: vec!["TT-3".to_string()],
                    file_areas: vec!["src/c".to_string()],
                    semantic_entities: vec![],
                    signal_score: 8,
                    confidence: 70,
                    rationale: vec!["c".to_string()],
                },
                WorkstreamCandidate {
                    project_name: "proj".to_string(),
                    member_hashes: vec!["d".to_string()],
                    member_messages: vec!["TT-4 trimmed".to_string()],
                    ticket_ids: vec!["TT-4".to_string()],
                    file_areas: vec!["src/d".to_string()],
                    semantic_entities: vec![],
                    signal_score: 4,
                    confidence: 60,
                    rationale: vec!["d".to_string()],
                },
            ],
            3,
        );

        assert_eq!(reports.len(), 1);
        let outcomes = &reports[0].outcomes;
        assert_eq!(
            outcomes
                .iter()
                .map(|outcome| outcome.factual_headline.as_str())
                .collect::<Vec<_>>(),
            vec![
                "TT-1 primary",
                "TT-2 tie high confidence",
                "TT-3 tie lower confidence",
            ]
        );
    }

    #[test]
    fn multi_commit_feature_outcome_avoids_cluster_mechanics() {
        let outcome = derive_outcome(WorkstreamCandidate {
            project_name: "devjournal".to_string(),
            member_hashes: vec!["a".to_string(), "b".to_string()],
            member_messages: vec![
                "feat(cli): inline llm setup for summaries".to_string(),
                "chore: polish summary command".to_string(),
            ],
            ticket_ids: vec![],
            file_areas: vec!["root".to_string()],
            semantic_entities: vec![],
            signal_score: 7,
            confidence: 80,
            rationale: vec!["overlapping file area".to_string()],
        });

        assert_eq!(
            outcome.probable_outcome,
            "Added inline LLM setup for summaries"
        );
        assert!(outcome
            .supporting_messages
            .contains(&"inline LLM setup for summaries".to_string()));
        assert!(!outcome.probable_outcome.contains("across"));
        assert!(!outcome.probable_outcome.contains("root"));
    }

    #[test]
    fn build_project_reports_drops_low_confidence_maintenance_when_feature_exists() {
        let reports = build_project_reports(
            vec![
                WorkstreamCandidate {
                    project_name: "devjournal".to_string(),
                    member_hashes: vec!["a".to_string()],
                    member_messages: vec!["feat(cli): inline llm setup for summaries".to_string()],
                    ticket_ids: vec![],
                    file_areas: vec!["src/cli".to_string()],
                    semantic_entities: vec![],
                    signal_score: 8,
                    confidence: 82,
                    rationale: vec!["clear feature".to_string()],
                },
                WorkstreamCandidate {
                    project_name: "devjournal".to_string(),
                    member_hashes: vec!["b".to_string()],
                    member_messages: vec!["refactor: rename summary plumbing".to_string()],
                    ticket_ids: vec![],
                    file_areas: vec!["src/cli".to_string()],
                    semantic_entities: vec![],
                    signal_score: 3,
                    confidence: 40,
                    rationale: vec!["maintenance cleanup".to_string()],
                },
            ],
            3,
        );

        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].outcomes.len(), 1);
        assert_eq!(
            reports[0].outcomes[0].probable_outcome,
            "Added inline LLM setup for summaries"
        );
    }

    #[test]
    fn maintenance_outcome_with_support_noun_stays_maintenance() {
        let outcome = derive_outcome(WorkstreamCandidate {
            project_name: "devjournal".to_string(),
            member_hashes: vec!["a".to_string()],
            member_messages: vec!["refactor: remove cursor provider support".to_string()],
            ticket_ids: vec![],
            file_areas: vec!["src/llm".to_string()],
            semantic_entities: vec![],
            signal_score: 4,
            confidence: 60,
            rationale: vec!["maintenance cleanup".to_string()],
        });

        assert_eq!(outcome.probable_outcome, "Removed cursor provider support");
    }
}
