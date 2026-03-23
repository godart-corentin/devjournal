use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
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

pub fn open_at(path: &PathBuf) -> Result<Connection> {
    let conn = Connection::open(path)?;
    init(&conn)?;
    Ok(conn)
}

fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            repo_path TEXT NOT NULL,
            repo_name TEXT,
            event_type TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            data TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_events_repo ON events(repo_path);

        CREATE TABLE IF NOT EXISTS poll_state (
            repo_path TEXT PRIMARY KEY,
            last_commit_hash TEXT,
            last_branch TEXT,
            last_polled_at TEXT
        );
    ")?;
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
    conn.execute(
        "INSERT INTO events (repo_path, repo_name, event_type, timestamp, data)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            event.repo_path,
            event.repo_name,
            event.event_type,
            event.timestamp,
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
         ORDER BY timestamp ASC"
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
pub struct PollState {
    pub last_commit_hash: Option<String>,
    pub last_branch: Option<String>,
    pub last_polled_at: Option<String>,
}

pub fn get_poll_state(conn: &Connection, repo_path: &str) -> Result<Option<PollState>> {
    let mut stmt = conn.prepare(
        "SELECT last_commit_hash, last_branch, last_polled_at FROM poll_state WHERE repo_path = ?1"
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

pub fn event_count_for_date(conn: &Connection, date: &str) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE timestamp LIKE ?1",
        params![format!("{}%", date)],
        |row| row.get(0),
    )?;
    Ok(count)
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
        update_poll_state(&conn, "/tmp/repo", "hash2", "feature/x", "2026-03-23T10:00:00Z").unwrap();
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
}
