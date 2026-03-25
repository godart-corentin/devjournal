use anyhow::{Context, Result};
use chrono::Local;
use git2::{Repository, Sort};
use rusqlite::Connection;
use serde_json::json;

use crate::config::RepoConfig;
use crate::db::{self, Event};

/// Sync all history for a repo into the DB, regardless of prior poll state.
/// Safe to run multiple times — duplicate commits are ignored via UNIQUE constraint.
pub fn sync_repo(repo_config: &RepoConfig, conn: &Connection, author_filter: Option<&str>) -> Result<usize> {
    let repo = Repository::open(&repo_config.path)
        .with_context(|| format!("Cannot open git repo at {}", repo_config.path))?;

    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return Ok(0),
    };

    let head_commit = head.peel_to_commit().context("Failed to get HEAD commit")?;
    let head_hash = head_commit.id().to_string();
    let branch_name = head.shorthand().unwrap_or("HEAD").to_string();

    let all_commits = collect_new_commits(&repo, &head_hash, None)?;

    let now = Local::now().to_rfc3339();
    let mut count = 0;
    for commit_info in all_commits {
        if let Some(author) = author_filter {
            if commit_info.author != author {
                continue;
            }
        }
        count += 1;
        let event = Event {
            id: None,
            repo_path: repo_config.path.clone(),
            repo_name: repo_config.name.clone(),
            event_type: "commit".to_string(),
            timestamp: commit_info.timestamp.clone(),
            data: serde_json::json!({
                "hash": commit_info.hash,
                "author": commit_info.author,
                "message": commit_info.message,
                "branch": branch_name,
                "files_changed": commit_info.files_changed,
                "insertions": commit_info.insertions,
                "deletions": commit_info.deletions,
            }),
        };
        db::insert_event(conn, &event)?;
    }

    db::update_poll_state(conn, &repo_config.path, &head_hash, &branch_name, &now)?;
    Ok(count)
}

pub fn poll_repo(repo_config: &RepoConfig, conn: &Connection, author_filter: Option<&str>) -> Result<usize> {
    let repo = Repository::open(&repo_config.path)
        .with_context(|| format!("Cannot open git repo at {}", repo_config.path))?;

    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return Ok(0), // empty repo, nothing to poll
    };

    let head_commit = head.peel_to_commit()
        .context("Failed to get HEAD commit")?;
    let head_hash = head_commit.id().to_string();

    let branch_name = head
        .shorthand()
        .unwrap_or("HEAD")
        .to_string();

    let poll_state = db::get_poll_state(conn, &repo_config.path)?;

    let new_commits = if let Some(state) = &poll_state {
        if state.last_commit_hash.as_deref() == Some(&head_hash) {
            // nothing new
            return Ok(0);
        }
        collect_new_commits(&repo, &head_hash, state.last_commit_hash.as_deref())?
    } else {
        // First poll: only record the most recent commit, not the entire history
        collect_new_commits(&repo, &head_hash, None)?
            .into_iter()
            .take(1)
            .collect()
    };

    let now = Local::now().to_rfc3339();

    let mut count = 0;
    for commit_info in new_commits {
        if let Some(author) = author_filter {
            if commit_info.author != author {
                continue;
            }
        }
        count += 1;
        let event = Event {
            id: None,
            repo_path: repo_config.path.clone(),
            repo_name: repo_config.name.clone(),
            event_type: "commit".to_string(),
            timestamp: commit_info.timestamp.clone(),
            data: json!({
                "hash": commit_info.hash,
                "author": commit_info.author,
                "message": commit_info.message,
                "branch": branch_name,
                "files_changed": commit_info.files_changed,
                "insertions": commit_info.insertions,
                "deletions": commit_info.deletions,
            }),
        };
        db::insert_event(conn, &event)?;
    }

    db::update_poll_state(conn, &repo_config.path, &head_hash, &branch_name, &now)?;
    Ok(count)
}

struct CommitInfo {
    hash: String,
    author: String,
    message: String,
    timestamp: String,
    files_changed: usize,
    insertions: usize,
    deletions: usize,
}

fn collect_new_commits(
    repo: &Repository,
    head_hash: &str,
    stop_at_hash: Option<&str>,
) -> Result<Vec<CommitInfo>> {
    let mut revwalk = repo.revwalk()?;
    revwalk.set_sorting(Sort::TIME)?;
    revwalk.push(repo.revparse_single(head_hash)?.id())?;

    let mut commits = Vec::new();

    for oid in revwalk {
        let oid = oid?;
        if let Some(stop) = stop_at_hash {
            if oid.to_string() == stop {
                break;
            }
        }
        let commit = repo.find_commit(oid)?;

        // Skip merge commits (more than one parent)
        if commit.parent_count() > 1 {
            continue;
        }

        let author = commit.author().name().unwrap_or("Unknown").to_string();
        let message = commit.summary().unwrap_or("").to_string();
        let timestamp = chrono::DateTime::from_timestamp(commit.time().seconds(), 0)
            .map(|dt| dt.with_timezone(&Local))
            .unwrap_or_else(Local::now)
            .to_rfc3339();

        let (files_changed, insertions, deletions) = diff_stats(repo, &commit);

        commits.push(CommitInfo {
            hash: oid.to_string()[..8].to_string(),
            author,
            message,
            timestamp,
            files_changed,
            insertions,
            deletions,
        });
    }

    Ok(commits)
}

fn diff_stats(repo: &Repository, commit: &git2::Commit) -> (usize, usize, usize) {
    let tree = commit.tree().ok();
    let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());

    let diff = match (tree, parent_tree) {
        (Some(t), Some(pt)) => repo.diff_tree_to_tree(Some(&pt), Some(&t), None).ok(),
        (Some(t), None) => repo.diff_tree_to_tree(None, Some(&t), None).ok(),
        _ => None,
    };

    if let Some(diff) = diff {
        if let Ok(stats) = diff.stats() {
            return (
                stats.files_changed(),
                stats.insertions(),
                stats.deletions(),
            );
        }
    }
    (0, 0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Repository, Signature};
    use tempfile::TempDir;

    fn make_test_repo_with_commit(dir: &TempDir, message: &str) -> Repository {
        let repo = Repository::init(dir.path()).unwrap();
        let sig = Signature::now("Test User", "test@test.com").unwrap();
        {
            let mut index = repo.index().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[]).unwrap();
        }
        repo
    }

    fn make_test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_path TEXT NOT NULL, repo_name TEXT,
                event_type TEXT NOT NULL, timestamp TEXT NOT NULL,
                commit_hash TEXT, data TEXT NOT NULL,
                UNIQUE(repo_path, commit_hash)
            );
            CREATE TABLE IF NOT EXISTS poll_state (
                repo_path TEXT PRIMARY KEY,
                last_commit_hash TEXT, last_branch TEXT, last_polled_at TEXT
            );
        ").unwrap();
        conn
    }

    #[test]
    fn test_poll_empty_repo_returns_zero() {
        let dir = TempDir::new().unwrap();
        let conn = make_test_conn();

        // init repo but don't commit anything
        Repository::init(dir.path()).unwrap();

        let repo_config = crate::config::RepoConfig {
            path: dir.path().to_string_lossy().to_string(),
            name: None,
        };
        let result = poll_repo(&repo_config, &conn, None).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_poll_new_repo_records_one_commit() {
        let dir = TempDir::new().unwrap();
        let conn = make_test_conn();

        make_test_repo_with_commit(&dir, "Initial commit");

        let repo_config = crate::config::RepoConfig {
            path: dir.path().to_string_lossy().to_string(),
            name: Some("test-repo".to_string()),
        };
        let count = poll_repo(&repo_config, &conn, None).unwrap();
        assert_eq!(count, 1);

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let events = db::get_events_for_date(&conn, &today).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data["message"], "Initial commit");
    }

    #[test]
    fn test_poll_same_state_returns_zero() {
        let dir = TempDir::new().unwrap();
        let conn = make_test_conn();

        make_test_repo_with_commit(&dir, "First commit");

        let repo_config = crate::config::RepoConfig {
            path: dir.path().to_string_lossy().to_string(),
            name: None,
        };
        poll_repo(&repo_config, &conn, None).unwrap(); // first poll
        let count = poll_repo(&repo_config, &conn, None).unwrap(); // second poll - no new commits
        assert_eq!(count, 0);
    }
}
