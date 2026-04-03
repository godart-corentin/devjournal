use anyhow::{Context, Result};
use chrono::Local;
use git2::{Repository, Sort};
use rusqlite::Connection;
use serde_json::json;

use crate::config::RepoConfig;
use crate::db::{self, Event};
use crate::sem::{CliSemExtractor, SemExtractor, SemMetadata};

fn open_repo(path: &str) -> Result<Repository> {
    // Disable ownership check to support repos in directories owned by a different
    // Windows user account (e.g. AzureAD-joined machines where the profile path
    // contains a domain suffix that git treats as a different owner).
    #[cfg(target_os = "windows")]
    unsafe {
        git2::opts::set_verify_owner_validation(false)
            .context("Failed to disable git owner validation")?;
    }
    Repository::open(path).with_context(|| format!("Cannot open git repo at {}", path))
}

/// Sync all history for a repo into the DB, regardless of prior poll state.
/// Safe to run multiple times — duplicate commits are ignored via UNIQUE constraint.
pub fn sync_repo(
    repo_config: &RepoConfig,
    conn: &Connection,
    author_filter: Option<&str>,
) -> Result<usize> {
    let extractor = CliSemExtractor;
    sync_repo_with_extractor(repo_config, conn, author_filter, &extractor)
}

fn sync_repo_with_extractor<E: SemExtractor + ?Sized>(
    repo_config: &RepoConfig,
    conn: &Connection,
    author_filter: Option<&str>,
    extractor: &E,
) -> Result<usize> {
    let repo = open_repo(&repo_config.path)?;

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
        let event = build_commit_event(repo_config, &branch_name, &commit_info, extractor);
        db::insert_event(conn, &event)?;
    }

    db::update_poll_state(conn, &repo_config.path, &head_hash, &branch_name, &now)?;
    Ok(count)
}

pub fn poll_repo(
    repo_config: &RepoConfig,
    conn: &Connection,
    author_filter: Option<&str>,
) -> Result<usize> {
    let extractor = CliSemExtractor;
    poll_repo_with_extractor(repo_config, conn, author_filter, &extractor)
}

fn poll_repo_with_extractor<E: SemExtractor + ?Sized>(
    repo_config: &RepoConfig,
    conn: &Connection,
    author_filter: Option<&str>,
    extractor: &E,
) -> Result<usize> {
    let repo = open_repo(&repo_config.path)?;

    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return Ok(0), // empty repo, nothing to poll
    };

    let head_commit = head.peel_to_commit().context("Failed to get HEAD commit")?;
    let head_hash = head_commit.id().to_string();

    let branch_name = head.shorthand().unwrap_or("HEAD").to_string();

    let poll_state = db::get_poll_state(conn, &repo_config.path)?;

    let new_commits = if let Some(state) = &poll_state {
        if state.last_commit_hash.as_deref() == Some(&head_hash) {
            // nothing new — still update the poll timestamp so status stays accurate
            let now = Local::now().to_rfc3339();
            db::update_poll_state(conn, &repo_config.path, &head_hash, &branch_name, &now)?;
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
        let event = build_commit_event(repo_config, &branch_name, &commit_info, extractor);
        db::insert_event(conn, &event)?;
    }

    db::update_poll_state(conn, &repo_config.path, &head_hash, &branch_name, &now)?;
    Ok(count)
}

struct CommitInfo {
    hash: String,
    full_hash: String,
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
            full_hash: oid.to_string(),
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
            return (stats.files_changed(), stats.insertions(), stats.deletions());
        }
    }
    (0, 0, 0)
}

fn build_commit_event<E: SemExtractor + ?Sized>(
    repo_config: &RepoConfig,
    branch_name: &str,
    commit_info: &CommitInfo,
    extractor: &E,
) -> Event {
    let mut data = json!({
        "hash": commit_info.hash,
        "author": commit_info.author,
        "message": commit_info.message,
        "branch": branch_name,
        "files_changed": commit_info.files_changed,
        "insertions": commit_info.insertions,
        "deletions": commit_info.deletions,
    });

    if let Some(sem) = extract_sem_metadata(extractor, &repo_config.path, &commit_info.full_hash) {
        data["sem"] = serde_json::to_value(sem).expect("sem metadata should serialize");
    }

    Event {
        id: None,
        repo_path: repo_config.path.clone(),
        repo_name: repo_config.name.clone(),
        event_type: "commit".to_string(),
        timestamp: commit_info.timestamp.clone(),
        data,
    }
}

fn extract_sem_metadata<E: SemExtractor + ?Sized>(
    extractor: &E,
    repo_path: &str,
    commit_hash: &str,
) -> Option<SemMetadata> {
    match extractor.extract(repo_path, commit_hash) {
        Ok(sem) => sem,
        Err(err) => {
            eprintln!(
                "Warning: failed to extract sem data for commit {}: {:#}",
                commit_hash, err
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sem::{SemEntity, SemExtractor, SemMetadata};
    use anyhow::anyhow;
    use git2::{Repository, Signature};
    use std::cell::RefCell;
    use tempfile::TempDir;

    fn make_test_repo_with_commit(dir: &TempDir, message: &str) -> Repository {
        let repo = Repository::init(dir.path()).unwrap();
        commit_in_repo(&repo, message);
        repo
    }

    fn commit_in_repo(repo: &Repository, message: &str) {
        let sig = Signature::now("Test User", "test@test.com").unwrap();
        let mut index = repo.index().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();

        let parent_commit = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .and_then(|oid| repo.find_commit(oid).ok());
        let parents: Vec<&git2::Commit<'_>> = parent_commit.iter().collect();

        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .unwrap();
    }

    fn make_test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
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
        ",
        )
        .unwrap();
        conn
    }

    struct StubSemExtractor {
        responses: RefCell<Vec<Result<Option<SemMetadata>>>>,
    }

    impl StubSemExtractor {
        fn new(responses: Vec<Result<Option<SemMetadata>>>) -> Self {
            Self {
                responses: RefCell::new(responses.into_iter().rev().collect()),
            }
        }
    }

    impl SemExtractor for StubSemExtractor {
        fn extract(&self, _repo_path: &str, _commit_hash: &str) -> Result<Option<SemMetadata>> {
            self.responses
                .borrow_mut()
                .pop()
                .unwrap_or_else(|| Ok(None))
        }
    }

    fn sample_sem_metadata() -> SemMetadata {
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
    fn test_poll_new_repo_records_sem_data_when_available() {
        let dir = TempDir::new().unwrap();
        let conn = make_test_conn();
        make_test_repo_with_commit(&dir, "Initial commit");

        let repo_config = crate::config::RepoConfig {
            path: dir.path().to_string_lossy().to_string(),
            name: Some("test-repo".to_string()),
        };
        let extractor = StubSemExtractor::new(vec![Ok(Some(sample_sem_metadata()))]);

        let count = poll_repo_with_extractor(&repo_config, &conn, None, &extractor).unwrap();
        assert_eq!(count, 1);

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let events = db::get_events_for_date(&conn, &today).unwrap();
        assert_eq!(
            events[0].data["sem"]["summary"],
            sample_sem_metadata().summary
        );
        assert_eq!(
            events[0].data["sem"]["change_types"],
            json!(["added", "modified"])
        );
    }

    #[test]
    fn test_sync_repo_backfills_sem_data_for_existing_commit() {
        let dir = TempDir::new().unwrap();
        let conn = make_test_conn();
        make_test_repo_with_commit(&dir, "Initial commit");

        let repo_config = crate::config::RepoConfig {
            path: dir.path().to_string_lossy().to_string(),
            name: Some("test-repo".to_string()),
        };

        let initial_extractor = StubSemExtractor::new(vec![Ok(None)]);
        poll_repo_with_extractor(&repo_config, &conn, None, &initial_extractor).unwrap();

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let events = db::get_events_for_date(&conn, &today).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].data.get("sem").is_none());

        let enriching_extractor = StubSemExtractor::new(vec![Ok(Some(sample_sem_metadata()))]);
        let count =
            sync_repo_with_extractor(&repo_config, &conn, None, &enriching_extractor).unwrap();
        assert_eq!(count, 1);

        let events = db::get_events_for_date(&conn, &today).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].data["sem"]["summary"],
            sample_sem_metadata().summary
        );
    }

    #[test]
    fn test_poll_new_repo_omits_sem_data_when_unavailable() {
        let dir = TempDir::new().unwrap();
        let conn = make_test_conn();
        make_test_repo_with_commit(&dir, "Initial commit");

        let repo_config = crate::config::RepoConfig {
            path: dir.path().to_string_lossy().to_string(),
            name: Some("test-repo".to_string()),
        };
        let extractor = StubSemExtractor::new(vec![Ok(None)]);

        let count = poll_repo_with_extractor(&repo_config, &conn, None, &extractor).unwrap();
        assert_eq!(count, 1);

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let events = db::get_events_for_date(&conn, &today).unwrap();
        assert!(events[0].data.get("sem").is_none());
    }

    #[test]
    fn test_poll_repo_succeeds_when_sem_extraction_errors() {
        let dir = TempDir::new().unwrap();
        let conn = make_test_conn();
        make_test_repo_with_commit(&dir, "Initial commit");

        let repo_config = crate::config::RepoConfig {
            path: dir.path().to_string_lossy().to_string(),
            name: Some("test-repo".to_string()),
        };
        let extractor = StubSemExtractor::new(vec![Err(anyhow!("sem failed"))]);

        let count = poll_repo_with_extractor(&repo_config, &conn, None, &extractor).unwrap();
        assert_eq!(count, 1);

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let events = db::get_events_for_date(&conn, &today).unwrap();
        assert_eq!(events[0].data["message"], "Initial commit");
        assert!(events[0].data.get("sem").is_none());
    }

    #[test]
    fn test_sync_repo_succeeds_when_sem_extraction_errors() {
        let dir = TempDir::new().unwrap();
        let conn = make_test_conn();
        let repo = make_test_repo_with_commit(&dir, "First commit");
        commit_in_repo(&repo, "Second commit");

        let repo_config = crate::config::RepoConfig {
            path: dir.path().to_string_lossy().to_string(),
            name: Some("test-repo".to_string()),
        };
        let extractor = StubSemExtractor::new(vec![Err(anyhow!("sem failed")), Ok(None)]);

        let count = sync_repo_with_extractor(&repo_config, &conn, None, &extractor).unwrap();
        assert_eq!(count, 2);

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let events = db::get_events_for_date(&conn, &today).unwrap();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| event.data.get("sem").is_none()));
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

    #[test]
    fn test_poll_updates_last_polled_at_even_when_no_new_commits() {
        let dir = TempDir::new().unwrap();
        let conn = make_test_conn();

        make_test_repo_with_commit(&dir, "First commit");

        let repo_config = crate::config::RepoConfig {
            path: dir.path().to_string_lossy().to_string(),
            name: None,
        };

        poll_repo(&repo_config, &conn, None).unwrap(); // first poll - sets last_polled_at
        let state_after_first = db::get_poll_state(&conn, &repo_config.path)
            .unwrap()
            .unwrap();
        let first_polled_at = state_after_first.last_polled_at.unwrap();

        std::thread::sleep(std::time::Duration::from_secs(1));

        poll_repo(&repo_config, &conn, None).unwrap(); // second poll - no new commits
        let state_after_second = db::get_poll_state(&conn, &repo_config.path)
            .unwrap()
            .unwrap();
        let second_polled_at = state_after_second.last_polled_at.unwrap();

        assert!(
            second_polled_at > first_polled_at,
            "last_polled_at should be updated even when no new commits"
        );
    }
}
