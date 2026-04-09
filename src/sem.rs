use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemIntegrationStatus {
    Active,
    Unavailable,
    Degraded,
}

impl SemIntegrationStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Unavailable => "unavailable",
            Self::Degraded => "degraded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemProbe {
    pub status: SemIntegrationStatus,
    pub detail: String,
    pub install_hint: String,
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
        extract_with_command_runner(
            std::env::consts::OS,
            &sem_binary(),
            repo_path,
            commit_hash,
            |command, args, repo_path| {
                Command::new(command)
                    .current_dir(repo_path)
                    .args(args)
                    .output()
            },
        )
    }
}

pub fn parse_sem_output(stdout: &str) -> Result<Option<SemMetadata>> {
    let diff: SemCliDiff =
        serde_json::from_str(stdout).context("failed to parse sem diff JSON output")?;
    Ok(normalize_sem_diff(diff))
}

pub fn probe() -> SemProbe {
    probe_with_command_runner(std::env::consts::OS, &sem_binary(), |command, args| {
        Command::new(command).args(args).output()
    })
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

fn sem_binary() -> PathBuf {
    if let Ok(path) = std::env::var("DEVJOURNAL_SEM_BIN") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let bundled = dir.join(sem_executable_name());
            if bundled.exists() {
                return bundled;
            }
        }
    }

    PathBuf::from(sem_executable_name())
}

fn sem_executable_name() -> &'static str {
    #[cfg(windows)]
    {
        "sem.exe"
    }
    #[cfg(not(windows))]
    {
        "sem"
    }
}

fn command_exists(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn install_hint_for(os: &str, has_brew: bool, has_cargo: bool) -> String {
    if os == "windows" {
        return "semantic enrichment is currently unavailable on Windows".to_string();
    }

    if os == "macos" && has_brew {
        "brew install sem-cli".to_string()
    } else if has_cargo {
        "cargo install sem-cli".to_string()
    } else if has_brew {
        "brew install sem-cli".to_string()
    } else {
        "install sem-cli manually and re-run `devjournal sync`".to_string()
    }
}

fn sem_supported_on_os(os: &str) -> bool {
    os != "windows"
}

fn windows_unsupported_probe() -> SemProbe {
    SemProbe {
        status: SemIntegrationStatus::Unavailable,
        detail: "sem is not supported on Windows".to_string(),
        install_hint: install_hint_for("windows", false, false),
    }
}

fn probe_with_command_runner<F>(os: &str, sem_bin: &Path, run_command: F) -> SemProbe
where
    F: FnOnce(&Path, &[&str]) -> std::io::Result<std::process::Output>,
{
    if !sem_supported_on_os(os) {
        return windows_unsupported_probe();
    }

    let install_hint = install_hint_for(os, command_exists("brew"), command_exists("cargo"));

    match run_command(sem_bin, &["--version"]) {
        Ok(output) if output.status.success() => SemProbe {
            status: SemIntegrationStatus::Active,
            detail: format!("using {}", sem_bin.display()),
            install_hint,
        },
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let detail = if stderr.is_empty() {
                format!("{} exists but `sem --version` failed", sem_bin.display())
            } else {
                format!(
                    "{} exists but failed health check: {}",
                    sem_bin.display(),
                    stderr
                )
            };
            SemProbe {
                status: SemIntegrationStatus::Degraded,
                detail,
                install_hint,
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => SemProbe {
            status: SemIntegrationStatus::Unavailable,
            detail: "sem is not installed".to_string(),
            install_hint,
        },
        Err(err) => SemProbe {
            status: SemIntegrationStatus::Degraded,
            detail: format!("failed to execute sem: {}", err),
            install_hint,
        },
    }
}

fn extract_with_command_runner<F>(
    os: &str,
    sem_bin: &Path,
    repo_path: &str,
    commit_hash: &str,
    run_command: F,
) -> Result<Option<SemMetadata>>
where
    F: FnOnce(&Path, &[String], &str) -> std::io::Result<std::process::Output>,
{
    if !sem_supported_on_os(os) {
        return Ok(None);
    }

    let output = run_command(
        sem_bin,
        &CliSemExtractor::build_args(commit_hash),
        repo_path,
    )
    .with_context(|| {
        format!(
            "sem not found — install sem-cli and ensure the `sem` binary is available (try: {})",
            install_hint_for(os, command_exists("brew"), command_exists("cargo"))
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("sem diff failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8(output.stdout).context("sem diff output was not valid UTF-8")?;
    parse_sem_output(&stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_windows_sem_reports_unavailable() {
        let probe =
            probe_with_command_runner("windows", Path::new("sem.exe"), |_command, _args| {
                panic!("windows probe should not execute sem")
            });

        assert_eq!(probe.status, SemIntegrationStatus::Unavailable);
        assert!(probe.detail.contains("not supported on Windows"));
    }

    #[test]
    fn test_windows_sem_extractor_skips_process_execution() {
        let result = extract_with_command_runner(
            "windows",
            Path::new("sem.exe"),
            "/tmp/repo",
            "abc1234",
            |_command, _args, _repo_path| panic!("windows extraction should not execute sem"),
        )
        .unwrap();

        assert_eq!(result, None);
    }

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

    #[test]
    fn test_install_hint_prefers_brew_on_macos() {
        assert_eq!(
            install_hint_for("macos", true, true),
            "brew install sem-cli"
        );
    }

    #[test]
    fn test_install_hint_falls_back_to_cargo() {
        assert_eq!(
            install_hint_for("linux", false, true),
            "cargo install sem-cli"
        );
    }

    #[test]
    fn test_install_hint_without_supported_installer() {
        assert!(install_hint_for("linux", false, false).contains("install sem-cli"));
    }
}
