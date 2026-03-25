use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("devjournal")
}

pub fn db_path() -> PathBuf {
    data_dir().join("events.db")
}

pub fn open() -> Result<Connection> {
    let path = db_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    let conn = Connection::open(&path)
        .with_context(|| format!("Failed to open database at {}", path.display()))?;
    init(&conn)?;
    Ok(conn)
}

fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            repo_path TEXT NOT NULL,
            repo_name TEXT,
            event_type TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            commit_hash TEXT,
            data TEXT NOT NULL,
            UNIQUE(repo_path, commit_hash)
        );
        CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_events_repo ON events(repo_path);

        CREATE TABLE IF NOT EXISTS poll_state (
            repo_path TEXT PRIMARY KEY,
            last_commit_hash TEXT,
            last_branch TEXT,
            last_polled_at TEXT
        );
    ",
    )?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Event {
    pub id: Option<i64>,
    pub repo_path: String,
    pub repo_name: Option<String>,
    pub event_type: String,
    pub timestamp: String,
    pub data: serde_json::Value,
}

pub fn insert_event(conn: &Connection, event: &Event) -> Result<()> {
    let commit_hash = event.data["hash"].as_str().map(|s| s.to_string());
    conn.execute(
        "INSERT OR IGNORE INTO events (repo_path, repo_name, event_type, timestamp, commit_hash, data)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            event.repo_path,
            event.repo_name,
            event.event_type,
            event.timestamp,
            commit_hash,
            serde_json::to_string(&event.data)?
        ],
    )?;
    Ok(())
}

pub fn get_events_for_date(conn: &Connection, date: &str) -> Result<Vec<Event>> {
    // date is YYYY-MM-DD; match timestamps starting with that prefix
    let mut stmt = conn.prepare(
        "SELECT id, repo_path, repo_name, event_type, timestamp, data
         FROM events
         WHERE timestamp LIKE ?1
         ORDER BY timestamp ASC",
    )?;
    let rows = stmt.query_map(params![format!("{}%", date)], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    let mut events = Vec::new();
    for row in rows {
        let (id, repo_path, repo_name, event_type, timestamp, data_str) = row?;
        events.push(Event {
            id: Some(id),
            repo_path,
            repo_name,
            event_type,
            timestamp,
            data: serde_json::from_str(&data_str)?,
        });
    }
    Ok(events)
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct PollState {
    pub last_commit_hash: Option<String>,
    pub last_branch: Option<String>,
    pub last_polled_at: Option<String>,
}

pub fn get_poll_state(conn: &Connection, repo_path: &str) -> Result<Option<PollState>> {
    let mut stmt = conn.prepare(
        "SELECT last_commit_hash, last_branch, last_polled_at FROM poll_state WHERE repo_path = ?1",
    )?;
    let mut rows = stmt.query(params![repo_path])?;
    if let Some(row) = rows.next()? {
        Ok(Some(PollState {
            last_commit_hash: row.get(0)?,
            last_branch: row.get(1)?,
            last_polled_at: row.get(2)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn update_poll_state(
    conn: &Connection,
    repo_path: &str,
    commit_hash: &str,
    branch: &str,
    polled_at: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO poll_state (repo_path, last_commit_hash, last_branch, last_polled_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(repo_path) DO UPDATE SET
             last_commit_hash = excluded.last_commit_hash,
             last_branch = excluded.last_branch,
             last_polled_at = excluded.last_polled_at",
        params![repo_path, commit_hash, branch, polled_at],
    )?;
    Ok(())
}

/// Compute a stable SHA-256 fingerprint of a set of events.
/// Sorts by (repo_path, commit_hash) before hashing so order doesn't matter.
pub fn compute_events_fingerprint(events: &[Event]) -> String {
    let mut keys: Vec<String> = events
        .iter()
        .map(|e| {
            let hash = e.data["hash"].as_str().unwrap_or("");
            format!("{}:{}", e.repo_path, hash)
        })
        .collect();
    keys.sort();

    let mut hasher = Sha256::new();
    for key in &keys {
        hasher.update(key.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

pub fn get_events_for_range(conn: &Connection, from: &str, to: &str) -> Result<Vec<Event>> {
    let mut stmt = conn.prepare(
        "SELECT id, repo_path, repo_name, event_type, timestamp, data
         FROM events
         WHERE date(timestamp) >= ?1 AND date(timestamp) <= ?2
         ORDER BY timestamp ASC",
    )?;
    let rows = stmt.query_map(params![from, to], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    let mut events = Vec::new();
    for row in rows {
        let (id, repo_path, repo_name, event_type, timestamp, data_str) = row?;
        events.push(Event {
            id: Some(id),
            repo_path,
            repo_name,
            event_type,
            timestamp,
            data: serde_json::from_str(&data_str)?,
        });
    }
    Ok(events)
}

pub fn event_count_for_date_by_repo(conn: &Connection, repo_path: &str, date: &str) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE repo_path = ?1 AND timestamp LIKE ?2",
        params![repo_path, format!("{}%", date)],
        |row| row.get(0),
    )?;
    Ok(count)
}

pub fn get_latest_poll_time(conn: &Connection) -> Result<Option<String>> {
    let mut stmt = conn
        .prepare("SELECT MAX(last_polled_at) FROM poll_state WHERE last_polled_at IS NOT NULL")?;
    let mut rows = stmt.query([])?;
    if let Some(row) = rows.next()? {
        Ok(row.get(0)?)
    } else {
        Ok(None)
    }
}

pub fn search_events(
    conn: &Connection,
    keyword: &str,
    repo_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<Event>> {
    let pattern = format!("%{}%", keyword);
    let mut events = Vec::new();

    match repo_filter {
        Some(repo) => {
            let mut stmt = conn.prepare(
                "SELECT id, repo_path, repo_name, event_type, timestamp, data
                 FROM events
                 WHERE data LIKE ?1 AND (repo_name = ?2 OR repo_path = ?2)
                 ORDER BY timestamp DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![pattern, repo, limit as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })?;
            for row in rows {
                let (id, repo_path, repo_name, event_type, timestamp, data_str) = row?;
                events.push(Event {
                    id: Some(id),
                    repo_path,
                    repo_name,
                    event_type,
                    timestamp,
                    data: serde_json::from_str(&data_str)?,
                });
            }
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT id, repo_path, repo_name, event_type, timestamp, data
                 FROM events
                 WHERE data LIKE ?1
                 ORDER BY timestamp DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![pattern, limit as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })?;
            for row in rows {
                let (id, repo_path, repo_name, event_type, timestamp, data_str) = row?;
                events.push(Event {
                    id: Some(id),
                    repo_path,
                    repo_name,
                    event_type,
                    timestamp,
                    data: serde_json::from_str(&data_str)?,
                });
            }
        }
    }

    Ok(events)
}

pub fn prune_events_before(conn: &Connection, before_date: &str) -> Result<usize> {
    let deleted = conn.execute(
        "DELETE FROM events WHERE date(timestamp) < ?1",
        params![before_date],
    )?;
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        conn
    }

    #[test]
    fn test_insert_and_retrieve_event() {
        let conn = test_conn();
        let event = Event {
            id: None,
            repo_path: "/tmp/repo".to_string(),
            repo_name: Some("my-repo".to_string()),
            event_type: "commit".to_string(),
            timestamp: "2026-03-23T10:00:00Z".to_string(),
            data: serde_json::json!({
                "hash": "abc123",
                "message": "Fix bug TT-1234",
                "author": "Tylia",
                "branch": "main"
            }),
        };
        insert_event(&conn, &event).unwrap();

        let events = get_events_for_date(&conn, "2026-03-23").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].repo_path, "/tmp/repo");
        assert_eq!(events[0].data["message"], "Fix bug TT-1234");
    }

    #[test]
    fn test_get_events_wrong_date_returns_empty() {
        let conn = test_conn();
        let event = Event {
            id: None,
            repo_path: "/tmp/repo".to_string(),
            repo_name: None,
            event_type: "commit".to_string(),
            timestamp: "2026-03-23T10:00:00Z".to_string(),
            data: serde_json::json!({"hash": "abc"}),
        };
        insert_event(&conn, &event).unwrap();
        let events = get_events_for_date(&conn, "2026-03-24").unwrap();
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn test_poll_state_upsert() {
        let conn = test_conn();
        update_poll_state(&conn, "/tmp/repo", "hash1", "main", "2026-03-23T09:00:00Z").unwrap();
        let state = get_poll_state(&conn, "/tmp/repo").unwrap().unwrap();
        assert_eq!(state.last_commit_hash.as_deref(), Some("hash1"));

        // upsert
        update_poll_state(
            &conn,
            "/tmp/repo",
            "hash2",
            "feature/x",
            "2026-03-23T10:00:00Z",
        )
        .unwrap();
        let state = get_poll_state(&conn, "/tmp/repo").unwrap().unwrap();
        assert_eq!(state.last_commit_hash.as_deref(), Some("hash2"));
        assert_eq!(state.last_branch.as_deref(), Some("feature/x"));
    }

    #[test]
    fn test_poll_state_missing_returns_none() {
        let conn = test_conn();
        let state = get_poll_state(&conn, "/nonexistent").unwrap();
        assert!(state.is_none());
    }

    fn make_event(repo_path: &str, hash: &str) -> Event {
        Event {
            id: None,
            repo_path: repo_path.to_string(),
            repo_name: None,
            event_type: "commit".to_string(),
            timestamp: "2026-03-25T10:00:00Z".to_string(),
            data: serde_json::json!({ "hash": hash }),
        }
    }

    #[test]
    fn test_fingerprint_is_stable_regardless_of_order() {
        let events_a = vec![make_event("/repo/a", "aaa"), make_event("/repo/b", "bbb")];
        let events_b = vec![make_event("/repo/b", "bbb"), make_event("/repo/a", "aaa")];
        assert_eq!(
            compute_events_fingerprint(&events_a),
            compute_events_fingerprint(&events_b)
        );
    }

    #[test]
    fn test_fingerprint_changes_when_events_change() {
        let events_a = vec![make_event("/repo/a", "aaa")];
        let events_b = vec![make_event("/repo/a", "bbb")];
        assert_ne!(
            compute_events_fingerprint(&events_a),
            compute_events_fingerprint(&events_b)
        );
    }

    #[test]
    fn test_fingerprint_empty_events_is_deterministic() {
        let fp1 = compute_events_fingerprint(&[]);
        let fp2 = compute_events_fingerprint(&[]);
        assert_eq!(fp1, fp2);
        assert!(!fp1.is_empty());
    }

    #[test]
    fn test_get_latest_poll_time_returns_none_when_empty() {
        let conn = test_conn();
        let result = get_latest_poll_time(&conn).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_latest_poll_time_returns_max_across_repos() {
        let conn = test_conn();
        update_poll_state(&conn, "/repo/a", "h1", "main", "2026-03-25T09:00:00Z").unwrap();
        update_poll_state(&conn, "/repo/b", "h2", "main", "2026-03-25T10:30:00Z").unwrap();
        update_poll_state(&conn, "/repo/c", "h3", "main", "2026-03-25T08:00:00Z").unwrap();
        let result = get_latest_poll_time(&conn).unwrap();
        assert_eq!(result.as_deref(), Some("2026-03-25T10:30:00Z"));
    }

    #[test]
    fn test_search_events_by_keyword() {
        let conn = test_conn();
        let e1 = Event {
            id: None,
            repo_path: "/repo/a".to_string(),
            repo_name: Some("alpha".to_string()),
            event_type: "commit".to_string(),
            timestamp: "2026-03-20T10:00:00Z".to_string(),
            data: serde_json::json!({"hash": "a1", "message": "Fix auth bug TT-1234"}),
        };
        let e2 = Event {
            id: None,
            repo_path: "/repo/a".to_string(),
            repo_name: Some("alpha".to_string()),
            event_type: "commit".to_string(),
            timestamp: "2026-03-21T10:00:00Z".to_string(),
            data: serde_json::json!({"hash": "a2", "message": "Add logging to API"}),
        };
        insert_event(&conn, &e1).unwrap();
        insert_event(&conn, &e2).unwrap();

        // Search for "auth" — should match e1 only
        let results = search_events(&conn, "auth", None, 50).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].data["hash"], "a1");

        // Search for "TT-1234" — should match e1
        let results = search_events(&conn, "TT-1234", None, 50).unwrap();
        assert_eq!(results.len(), 1);

        // Search with repo filter
        let results = search_events(&conn, "auth", Some("alpha"), 50).unwrap();
        assert_eq!(results.len(), 1);

        // Search with wrong repo filter — no match
        let results = search_events(&conn, "auth", Some("beta"), 50).unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_prune_events_before_date() {
        let conn = test_conn();
        let old = Event {
            id: None,
            repo_path: "/repo/a".to_string(),
            repo_name: None,
            event_type: "commit".to_string(),
            timestamp: "2025-01-01T10:00:00Z".to_string(),
            data: serde_json::json!({"hash": "old1", "message": "old commit"}),
        };
        let recent = Event {
            id: None,
            repo_path: "/repo/a".to_string(),
            repo_name: None,
            event_type: "commit".to_string(),
            timestamp: "2026-03-25T10:00:00Z".to_string(),
            data: serde_json::json!({"hash": "new1", "message": "recent commit"}),
        };
        insert_event(&conn, &old).unwrap();
        insert_event(&conn, &recent).unwrap();

        let deleted = prune_events_before(&conn, "2026-01-01").unwrap();
        assert_eq!(deleted, 1);

        let remaining = get_events_for_date(&conn, "2026-03-25").unwrap();
        assert_eq!(remaining.len(), 1);
        let gone = get_events_for_date(&conn, "2025-01-01").unwrap();
        assert_eq!(gone.len(), 0);
    }

    #[test]
    fn test_event_count_for_date_by_repo_only_counts_that_repo() {
        let conn = test_conn();
        let event_a = Event {
            id: None,
            repo_path: "/repo/a".to_string(),
            repo_name: None,
            event_type: "commit".to_string(),
            timestamp: "2026-03-25T10:00:00Z".to_string(),
            data: serde_json::json!({"hash": "a1"}),
        };
        let event_b = Event {
            id: None,
            repo_path: "/repo/b".to_string(),
            repo_name: None,
            event_type: "commit".to_string(),
            timestamp: "2026-03-25T11:00:00Z".to_string(),
            data: serde_json::json!({"hash": "b1"}),
        };
        insert_event(&conn, &event_a).unwrap();
        insert_event(&conn, &event_b).unwrap();

        let count_a = event_count_for_date_by_repo(&conn, "/repo/a", "2026-03-25").unwrap();
        let count_b = event_count_for_date_by_repo(&conn, "/repo/b", "2026-03-25").unwrap();
        let count_c = event_count_for_date_by_repo(&conn, "/repo/c", "2026-03-25").unwrap();
        assert_eq!(count_a, 1);
        assert_eq!(count_b, 1);
        assert_eq!(count_c, 0);
    }
}
