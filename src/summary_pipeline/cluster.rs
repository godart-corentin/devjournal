use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::summary_pipeline::evidence::{CommitEvidence, MessageQuality};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkstreamCandidate {
    pub project_name: String,
    pub member_hashes: Vec<String>,
    pub member_messages: Vec<String>,
    pub ticket_ids: Vec<String>,
    pub file_areas: Vec<String>,
    pub semantic_entities: Vec<String>,
    pub signal_score: u32,
    pub confidence: u8,
    pub rationale: Vec<String>,
}

pub fn build_workstreams(evidence: &[CommitEvidence]) -> Vec<WorkstreamCandidate> {
    let mut clusters: Vec<WorkstreamCandidate> = Vec::new();

    let mut commits = evidence.iter().collect::<Vec<_>>();
    commits.sort_by(|left, right| canonical_sort_key(left).cmp(&canonical_sort_key(right)));

    for commit in commits {
        if let Some(existing) = clusters
            .iter_mut()
            .find(|cluster| should_merge(cluster, commit))
        {
            merge_commit(existing, commit);
        } else {
            clusters.push(seed_cluster(commit));
        }
    }

    clusters
}

fn canonical_sort_key(commit: &CommitEvidence) -> (String, String, String, String, String, String) {
    (
        commit.repo_name.clone(),
        commit.ticket_ids.join(","),
        commit.file_areas.join(","),
        commit.normalized_message.clone(),
        commit.timestamp.clone(),
        commit.hash.clone(),
    )
}

fn should_merge(cluster: &WorkstreamCandidate, commit: &CommitEvidence) -> bool {
    if cluster.project_name != commit.repo_name {
        return false;
    }

    if shares_ticket(cluster, commit) {
        return true;
    }

    overlaps_file_area(cluster, commit)
        && cluster.ticket_ids.is_empty()
        && commit.ticket_ids.is_empty()
}

fn shares_ticket(cluster: &WorkstreamCandidate, commit: &CommitEvidence) -> bool {
    cluster.ticket_ids.iter().any(|ticket| {
        commit
            .ticket_ids
            .iter()
            .any(|candidate| candidate == ticket)
    })
}

fn overlaps_file_area(cluster: &WorkstreamCandidate, commit: &CommitEvidence) -> bool {
    cluster
        .file_areas
        .iter()
        .any(|area| commit.file_areas.iter().any(|candidate| candidate == area))
}

fn seed_cluster(commit: &CommitEvidence) -> WorkstreamCandidate {
    WorkstreamCandidate {
        project_name: commit.repo_name.clone(),
        member_hashes: vec![commit.hash.clone()],
        member_messages: vec![commit.message.clone()],
        ticket_ids: commit.ticket_ids.clone(),
        file_areas: commit.file_areas.clone(),
        semantic_entities: commit.semantic_entities.clone(),
        signal_score: score_commit(commit),
        confidence: score_confidence(commit),
        rationale: vec![build_seed_rationale(commit)],
    }
}

fn merge_commit(cluster: &mut WorkstreamCandidate, commit: &CommitEvidence) {
    let merge_reason = build_merge_rationale(cluster, commit);

    cluster.member_hashes.push(commit.hash.clone());
    cluster.member_messages.push(commit.message.clone());
    cluster.ticket_ids = merge_unique(&cluster.ticket_ids, &commit.ticket_ids);
    cluster.file_areas = merge_unique(&cluster.file_areas, &commit.file_areas);
    cluster.semantic_entities = merge_unique(&cluster.semantic_entities, &commit.semantic_entities);
    cluster.signal_score += score_commit(commit);
    cluster.confidence = cluster.confidence.max(score_confidence(commit));
    cluster.rationale.push(merge_reason);
}

fn merge_unique(left: &[String], right: &[String]) -> Vec<String> {
    let mut merged = BTreeSet::new();
    for item in left.iter().chain(right.iter()) {
        merged.insert(item.clone());
    }
    merged.into_iter().collect()
}

fn score_commit(commit: &CommitEvidence) -> u32 {
    let mut score = 1;
    if !commit.ticket_ids.is_empty() {
        score += 3;
    }
    if commit.has_sem {
        score += 2;
    }
    if commit.message_quality != MessageQuality::LowSignal {
        score += 1;
    }
    if !commit.file_areas.is_empty() {
        score += 1;
    }
    score
}

fn score_confidence(commit: &CommitEvidence) -> u8 {
    let mut confidence: u8 = 30;
    if !commit.ticket_ids.is_empty() {
        confidence += 40;
    }
    if !commit.file_areas.is_empty() {
        confidence += 15;
    }
    if commit.has_sem {
        confidence += 10;
    }
    match commit.message_quality {
        MessageQuality::Clear => confidence += 5,
        MessageQuality::Mixed => confidence += 0,
        MessageQuality::LowSignal => confidence = confidence.saturating_sub(10),
    }
    confidence.min(100)
}

fn build_seed_rationale(commit: &CommitEvidence) -> String {
    let mut reasons = Vec::new();
    reasons.push(format!("seeded from {}", commit.hash));
    if !commit.ticket_ids.is_empty() {
        reasons.push(format!("ticket ids: {}", commit.ticket_ids.join(", ")));
    }
    if !commit.file_areas.is_empty() {
        reasons.push(format!("file areas: {}", commit.file_areas.join(", ")));
    }
    if commit.has_sem {
        reasons.push("semantic metadata available".to_string());
    }
    reasons.join("; ")
}

fn build_merge_rationale(cluster: &WorkstreamCandidate, commit: &CommitEvidence) -> String {
    let reason = if shares_ticket(cluster, commit) {
        "shared ticket id"
    } else if overlaps_file_area(cluster, commit) {
        "overlapping file area"
    } else {
        "same project"
    };

    format!(
        "merged {} into {} via {}",
        commit.hash, cluster.project_name, reason
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary_pipeline::evidence::{ChangeIntent, CommitEvidence, MessageQuality};

    fn evidence(message: &str, ticket_ids: &[&str], file_areas: &[&str]) -> CommitEvidence {
        CommitEvidence {
            repo_name: "proj".to_string(),
            timestamp: "2026-04-15T09:00:00+02:00".to_string(),
            hash: message.to_string(),
            message: message.to_string(),
            normalized_message: message.to_ascii_lowercase(),
            ticket_ids: ticket_ids.iter().map(|s| s.to_string()).collect(),
            file_paths: vec!["src/auth/login.rs".to_string()],
            file_areas: file_areas.iter().map(|s| s.to_string()).collect(),
            semantic_entities: vec!["function:validate_login".to_string()],
            semantic_change_types: vec!["modified".to_string()],
            message_quality: MessageQuality::Clear,
            change_intent: ChangeIntent::Feature,
            has_sem: true,
            has_patch_excerpt: false,
        }
    }

    fn evidence_in_repo(
        repo_name: &str,
        hash: &str,
        message: &str,
        ticket_ids: &[&str],
        file_areas: &[&str],
        message_quality: MessageQuality,
        has_sem: bool,
    ) -> CommitEvidence {
        CommitEvidence {
            repo_name: repo_name.to_string(),
            timestamp: format!("2026-04-15T09:00:00+02:00-{hash}"),
            hash: hash.to_string(),
            message: message.to_string(),
            normalized_message: message.to_ascii_lowercase(),
            ticket_ids: ticket_ids.iter().map(|s| s.to_string()).collect(),
            file_paths: vec!["src/auth/login.rs".to_string()],
            file_areas: file_areas.iter().map(|s| s.to_string()).collect(),
            semantic_entities: vec!["function:validate_login".to_string()],
            semantic_change_types: vec!["modified".to_string()],
            message_quality,
            change_intent: ChangeIntent::Feature,
            has_sem,
            has_patch_excerpt: false,
        }
    }

    #[test]
    fn clusters_commits_with_same_ticket_id() {
        let workstreams = build_workstreams(&vec![
            evidence("TT-42 add login validation", &["TT-42"], &["src/auth"]),
            evidence("TT-42 fix login edge case", &["TT-42"], &["src/auth"]),
        ]);

        assert_eq!(workstreams.len(), 1);
        assert_eq!(workstreams[0].member_hashes.len(), 2);
    }

    #[test]
    fn does_not_merge_unrelated_workstreams() {
        let workstreams = build_workstreams(&vec![
            evidence("TT-42 add login validation", &["TT-42"], &["src/auth"]),
            evidence("TT-90 tweak invoice export", &["TT-90"], &["src/billing"]),
        ]);

        assert_eq!(workstreams.len(), 2);
    }

    #[test]
    fn respects_same_project_guard() {
        let workstreams = build_workstreams(&vec![
            CommitEvidence {
                repo_name: "proj-a".to_string(),
                timestamp: "2026-04-15T09:00:00+02:00".to_string(),
                hash: "a".to_string(),
                message: "TT-42 add login validation".to_string(),
                normalized_message: "tt-42 add login validation".to_string(),
                ticket_ids: vec!["TT-42".to_string()],
                file_paths: vec!["src/auth/login.rs".to_string()],
                file_areas: vec!["src/auth".to_string()],
                semantic_entities: vec!["function:validate_login".to_string()],
                semantic_change_types: vec!["modified".to_string()],
                message_quality: MessageQuality::Clear,
                change_intent: ChangeIntent::Feature,
                has_sem: true,
                has_patch_excerpt: false,
            },
            CommitEvidence {
                repo_name: "proj-b".to_string(),
                timestamp: "2026-04-15T09:00:00+02:00".to_string(),
                hash: "b".to_string(),
                message: "TT-42 add login validation".to_string(),
                normalized_message: "tt-42 add login validation".to_string(),
                ticket_ids: vec!["TT-42".to_string()],
                file_paths: vec!["src/auth/login.rs".to_string()],
                file_areas: vec!["src/auth".to_string()],
                semantic_entities: vec!["function:validate_login".to_string()],
                semantic_change_types: vec!["modified".to_string()],
                message_quality: MessageQuality::Clear,
                change_intent: ChangeIntent::Feature,
                has_sem: true,
                has_patch_excerpt: false,
            },
        ]);

        assert_eq!(workstreams.len(), 2);
        assert!(workstreams
            .iter()
            .all(|workstream| workstream.member_hashes.len() == 1));
    }

    #[test]
    fn merges_file_area_only_commits_when_no_tickets_are_present() {
        let workstreams = build_workstreams(&vec![
            evidence("refine login validation", &[], &["src/auth"]),
            evidence("tighten login guards", &[], &["src/auth"]),
        ]);

        assert_eq!(workstreams.len(), 1);
        assert_eq!(workstreams[0].member_hashes.len(), 2);
        assert!(workstreams[0].confidence >= 35);
        assert!(workstreams[0].rationale[0].contains("file areas: src/auth"));
    }

    #[test]
    fn does_not_bridge_unrelated_ticketed_workstreams_through_file_area_overlap() {
        let workstreams = build_workstreams(&vec![
            evidence("TT-42 add login validation", &["TT-42"], &["src/auth"]),
            evidence("cleanup login validation", &[], &["src/auth"]),
            evidence("TT-99 fix login edge case", &["TT-99"], &["src/auth"]),
        ]);

        assert_eq!(workstreams.len(), 3);

        let ticketed_clusters: Vec<_> = workstreams
            .iter()
            .filter(|workstream| !workstream.ticket_ids.is_empty())
            .collect();
        assert_eq!(ticketed_clusters.len(), 2);
        assert!(ticketed_clusters
            .iter()
            .all(|workstream| workstream.member_hashes.len() == 1));
    }

    #[test]
    fn equivalent_evidence_sets_cluster_the_same_way_regardless_of_input_order() {
        let evidence_a = vec![
            evidence_in_repo(
                "proj-b",
                "b1",
                "TT-90 tweak invoice export",
                &["TT-90"],
                &["src/billing"],
                MessageQuality::Clear,
                true,
            ),
            evidence_in_repo(
                "proj-a",
                "a2",
                "refine login validation",
                &[],
                &["src/auth"],
                MessageQuality::Clear,
                true,
            ),
            evidence_in_repo(
                "proj-a",
                "a1",
                "TT-42 add login validation",
                &["TT-42"],
                &["src/auth"],
                MessageQuality::Clear,
                true,
            ),
        ];

        let evidence_b = vec![
            evidence_a[2].clone(),
            evidence_a[0].clone(),
            evidence_a[1].clone(),
        ];

        let workstreams_a = build_workstreams(&evidence_a);
        let workstreams_b = build_workstreams(&evidence_b);

        assert_eq!(workstreams_a, workstreams_b);
        assert_eq!(
            workstreams_a
                .iter()
                .map(|workstream| (
                    workstream.project_name.as_str(),
                    workstream.member_hashes.len()
                ))
                .collect::<Vec<_>>(),
            vec![("proj-a", 1), ("proj-a", 1), ("proj-b", 1)]
        );
    }

    #[test]
    fn scores_signal_and_confidence_exactly_for_ticketed_clear_semantic_commit() {
        let workstreams = build_workstreams(&[evidence_in_repo(
            "proj",
            "exact-1",
            "TT-42 add login validation",
            &["TT-42"],
            &["src/auth"],
            MessageQuality::Clear,
            true,
        )]);

        assert_eq!(workstreams.len(), 1);
        assert_eq!(workstreams[0].signal_score, 8);
        assert_eq!(workstreams[0].confidence, 100);
        assert_eq!(
            workstreams[0].rationale[0],
            "seeded from exact-1; ticket ids: TT-42; file areas: src/auth; semantic metadata available"
        );
    }
}
