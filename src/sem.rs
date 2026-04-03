use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemEntity {
    pub name: String,
    pub kind: String,
    pub change_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemMetadata {
    pub summary: String,
    pub entities: Vec<SemEntity>,
    pub change_types: Vec<String>,
    pub files: Vec<String>,
}

pub trait SemExtractor {
    fn extract(&self, repo_path: &str, commit_hash: &str) -> Result<Option<SemMetadata>>;
}

pub struct CliSemExtractor;

impl CliSemExtractor {
    fn build_args(commit_hash: &str) -> Vec<String> {
        vec![
            "diff".to_string(),
            "--commit".to_string(),
            commit_hash.to_string(),
            "--format".to_string(),
            "json".to_string(),
        ]
    }
}

impl SemExtractor for CliSemExtractor {
    fn extract(&self, repo_path: &str, commit_hash: &str) -> Result<Option<SemMetadata>> {
        let output = Command::new("sem")
            .current_dir(repo_path)
            .args(Self::build_args(commit_hash))
            .output()
            .context("sem not found — install sem and ensure it is on PATH")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("sem diff failed: {}", stderr.trim());
        }

        let stdout =
            String::from_utf8(output.stdout).context("sem diff output was not valid UTF-8")?;
        parse_sem_output(&stdout)
    }
}

pub fn parse_sem_output(stdout: &str) -> Result<Option<SemMetadata>> {
    let diff: SemCliDiff =
        serde_json::from_str(stdout).context("failed to parse sem diff JSON output")?;
    Ok(normalize_sem_diff(diff))
}

pub fn from_value(value: &serde_json::Value) -> Option<SemMetadata> {
    serde_json::from_value(value.clone()).ok()
}

#[derive(Debug, Deserialize)]
struct SemCliDiff {
    #[serde(default)]
    summary: Option<SemCliSummary>,
    #[serde(default)]
    changes: Vec<SemCliChange>,
}

#[derive(Debug, Deserialize)]
struct SemCliSummary {
    #[serde(rename = "fileCount")]
    file_count: usize,
    #[serde(default)]
    added: usize,
    #[serde(default)]
    modified: usize,
    #[serde(default)]
    deleted: usize,
    #[serde(default)]
    total: usize,
}

#[derive(Debug, Deserialize)]
struct SemCliChange {
    #[serde(rename = "changeType")]
    change_type: String,
    #[serde(rename = "entityType")]
    entity_type: String,
    #[serde(rename = "entityName")]
    entity_name: String,
    #[serde(rename = "filePath")]
    file_path: String,
}

fn normalize_sem_diff(diff: SemCliDiff) -> Option<SemMetadata> {
    if diff.changes.is_empty() {
        return None;
    }

    let mut entities = Vec::new();
    let mut change_types = Vec::new();
    let mut files = Vec::new();

    for change in diff.changes {
        push_unique(&mut change_types, change.change_type.clone());
        push_unique(&mut files, change.file_path.clone());

        if entities.len() < 8
            && !change.entity_name.is_empty()
            && !entities.iter().any(|entity: &SemEntity| {
                entity.name == change.entity_name
                    && entity.kind == change.entity_type
                    && entity.change_type == change.change_type
            })
        {
            entities.push(SemEntity {
                name: change.entity_name,
                kind: change.entity_type,
                change_type: change.change_type,
            });
        }
    }

    Some(SemMetadata {
        summary: format_summary(diff.summary.as_ref(), files.len(), entities.len()),
        entities,
        change_types,
        files,
    })
}

fn format_summary(
    summary: Option<&SemCliSummary>,
    file_count: usize,
    entity_count: usize,
) -> String {
    match summary {
        Some(summary) => {
            let mut parts = Vec::new();
            if summary.added > 0 {
                parts.push(format!("{} added", summary.added));
            }
            if summary.modified > 0 {
                parts.push(format!("{} modified", summary.modified));
            }
            if summary.deleted > 0 {
                parts.push(format!("{} deleted", summary.deleted));
            }

            let total = if summary.total > 0 {
                summary.total
            } else {
                entity_count
            };
            let file_count = if summary.file_count > 0 {
                summary.file_count
            } else {
                file_count
            };

            if parts.is_empty() {
                format!("{} semantic changes across {} files", total, file_count)
            } else {
                format!(
                    "{} semantic changes across {} files ({})",
                    total,
                    file_count,
                    parts.join(", ")
                )
            }
        }
        None => format!(
            "{} semantic changes across {} files",
            entity_count, file_count
        ),
    }
}

fn push_unique(items: &mut Vec<String>, value: String) {
    if !items.iter().any(|existing| existing == &value) {
        items.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sem_output_normalizes_changes() {
        let sem_json = r#"
        {
          "summary": {
            "fileCount": 2,
            "added": 1,
            "modified": 1,
            "deleted": 0,
            "total": 2
          },
          "changes": [
            {
              "entityId": "src/auth.ts::function::validateToken",
              "changeType": "added",
              "entityType": "function",
              "entityName": "validateToken",
              "filePath": "src/auth.ts"
            },
            {
              "entityId": "src/auth.ts::function::authenticateUser",
              "changeType": "modified",
              "entityType": "function",
              "entityName": "authenticateUser",
              "filePath": "src/auth.ts"
            }
          ]
        }
        "#;

        let metadata = parse_sem_output(sem_json).unwrap().unwrap();

        assert_eq!(
            metadata.summary,
            "2 semantic changes across 2 files (1 added, 1 modified)"
        );
        assert_eq!(metadata.change_types, vec!["added", "modified"]);
        assert_eq!(metadata.files, vec!["src/auth.ts"]);
        assert_eq!(
            metadata.entities,
            vec![
                SemEntity {
                    name: "validateToken".to_string(),
                    kind: "function".to_string(),
                    change_type: "added".to_string(),
                },
                SemEntity {
                    name: "authenticateUser".to_string(),
                    kind: "function".to_string(),
                    change_type: "modified".to_string(),
                },
            ]
        );
    }

    #[test]
    fn test_parse_sem_output_returns_none_for_empty_changes() {
        let sem_json = r#"{ "summary": { "fileCount": 0, "total": 0 }, "changes": [] }"#;
        assert_eq!(parse_sem_output(sem_json).unwrap(), None);
    }

    #[test]
    fn test_build_args_for_commit_json_diff() {
        assert_eq!(
            CliSemExtractor::build_args("abc1234"),
            vec!["diff", "--commit", "abc1234", "--format", "json"]
        );
    }
}
