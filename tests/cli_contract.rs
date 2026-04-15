use assert_cmd::Command;
use chrono::{Duration, Local};
use predicates::prelude::*;
use serde_json::Value;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

type TestResult<T = ()> = anyhow::Result<T>;

const AUTHOR: &str = "Fixture Tester";
const EMAIL: &str = "fixture@example.com";
const REPO_NAME: &str = "fixture-repo";

struct ContractFixture {
    _root: TempDir,
    repo_dir: PathBuf,
    home_dir: PathBuf,
    xdg_config_home: PathBuf,
    xdg_data_home: PathBuf,
    appdata_dir: PathBuf,
    localappdata_dir: PathBuf,
}

impl ContractFixture {
    fn new() -> TestResult<Self> {
        let root = TempDir::new()?;
        let repo_dir = root.path().join("repo");
        let home_dir = root.path().join("home");
        let xdg_config_home = root.path().join("xdg-config");
        let xdg_data_home = root.path().join("xdg-data");
        let appdata_dir = root.path().join("appdata");
        let localappdata_dir = root.path().join("localappdata");

        std::fs::create_dir_all(&repo_dir)?;
        std::fs::create_dir_all(&home_dir)?;
        std::fs::create_dir_all(&xdg_config_home)?;
        std::fs::create_dir_all(&xdg_data_home)?;
        std::fs::create_dir_all(&appdata_dir)?;
        std::fs::create_dir_all(&localappdata_dir)?;

        Ok(Self {
            _root: root,
            repo_dir,
            home_dir,
            xdg_config_home,
            xdg_data_home,
            appdata_dir,
            localappdata_dir,
        })
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_devjournal"));
        command
            .env("HOME", &self.home_dir)
            .env("USERPROFILE", &self.home_dir)
            .env("XDG_CONFIG_HOME", &self.xdg_config_home)
            .env("XDG_DATA_HOME", &self.xdg_data_home)
            .env("APPDATA", &self.appdata_dir)
            .env("LOCALAPPDATA", &self.localappdata_dir);
        command
    }

    fn git_repo_dir(&self) -> &Path {
        &self.repo_dir
    }

    fn config_path(&self) -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            self.home_dir
                .join("Library")
                .join("Application Support")
                .join("devjournal/config.toml")
        }

        #[cfg(all(unix, not(target_os = "macos")))]
        {
            self.xdg_config_home.join("devjournal/config.toml")
        }

        #[cfg(windows)]
        {
            self.appdata_dir.join("devjournal/config.toml")
        }
    }

    fn data_path(&self) -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            self.home_dir
                .join("Library")
                .join("Application Support")
                .join("devjournal/events.db")
        }

        #[cfg(all(unix, not(target_os = "macos")))]
        {
            self.xdg_data_home.join("devjournal/events.db")
        }

        #[cfg(windows)]
        {
            self.localappdata_dir.join("devjournal/events.db")
        }
    }

    fn command_output(&self, args: &[&str]) -> TestResult<String> {
        let assert = self.command().args(args).assert().success();
        Ok(String::from_utf8(assert.get_output().stdout.clone())?)
    }

    fn init_git_repo(&self) -> TestResult<()> {
        self.run_git(self.git_repo_dir(), ["init", "-b", "main"])?;
        self.run_git(self.git_repo_dir(), ["config", "user.name", AUTHOR])?;
        self.run_git(self.git_repo_dir(), ["config", "user.email", EMAIL])?;
        self.run_git(self.git_repo_dir(), ["config", "commit.gpgsign", "false"])?;
        Ok(())
    }

    fn commit_file(
        &self,
        file_name: &str,
        contents: &str,
        message: &str,
        date: &str,
    ) -> TestResult {
        std::fs::write(self.git_repo_dir().join(file_name), contents)?;
        self.run_git(self.git_repo_dir(), ["add", file_name])?;
        self.run_git_with_dates(self.git_repo_dir(), ["commit", "-m", message], date)?;
        Ok(())
    }

    fn write_config(&self, config: &str) -> TestResult {
        let path = self.config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, config)?;
        Ok(())
    }

    fn today_date(&self) -> String {
        Local::now().format("%Y-%m-%d").to_string()
    }

    fn date_days_ago(&self, days: i64) -> String {
        (Local::now() - Duration::days(days))
            .format("%Y-%m-%d")
            .to_string()
    }

    fn config_toml(&self, repo_name: Option<&str>) -> String {
        let repo_name = repo_name.unwrap_or(REPO_NAME);
        format!(
            r#"[general]
poll_interval_secs = 1
author = "{author}"

[llm]
provider = "ollama"
model = "llama3.2"

[[repos]]
path = "{path}"
name = "{name}"
"#,
            author = AUTHOR,
            path = self.git_repo_dir().display(),
            name = repo_name
        )
    }

    fn config_toml_missing_llm_key(&self, repo_name: Option<&str>) -> String {
        let repo_name = repo_name.unwrap_or(REPO_NAME);
        format!(
            r#"[general]
poll_interval_secs = 1
author = "{author}"

[llm]
provider = "anthropic"

[[repos]]
path = "{path}"
name = "{name}"
"#,
            author = AUTHOR,
            path = self.git_repo_dir().display(),
            name = repo_name
        )
    }

    fn run_git<I, S>(&self, cwd: &Path, args: I) -> TestResult
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_git_with_env(cwd, args, None)
    }

    fn run_git_with_dates<I, S>(&self, cwd: &Path, args: I, date: &str) -> TestResult
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_git_with_env(cwd, args, Some(date))
    }

    fn run_git_with_env<I, S>(&self, cwd: &Path, args: I, date: Option<&str>) -> TestResult
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = std::process::Command::new("git");
        command.current_dir(cwd).args(args);
        command
            .env("GIT_AUTHOR_NAME", AUTHOR)
            .env("GIT_AUTHOR_EMAIL", EMAIL)
            .env("GIT_COMMITTER_NAME", AUTHOR)
            .env("GIT_COMMITTER_EMAIL", EMAIL);
        if let Some(date) = date {
            command
                .env("GIT_AUTHOR_DATE", format!("{date} 12:00:00 +0000"))
                .env("GIT_COMMITTER_DATE", format!("{date} 12:00:00 +0000"));
        }

        let output = command.output()?;
        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "git command failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
                output.status.code(),
                stdout,
                stderr
            );
        }

        Ok(())
    }

    fn assert_json_event_envelope(&self, output: &str, expected_message: &str) {
        let json: Value = serde_json::from_str(output).expect("valid JSON output");
        let events = json.as_array().expect("event output should be an array");
        assert_eq!(events.len(), 1, "expected one event in JSON output");

        let event = events[0].as_object().expect("event should be an object");
        let keys = event.keys().cloned().collect::<BTreeSet<_>>();
        assert_eq!(
            keys,
            BTreeSet::from([
                "event_type".to_string(),
                "payload".to_string(),
                "repo_name".to_string(),
                "repo_path".to_string(),
                "timestamp".to_string(),
            ])
        );
        assert!(event.get("id").is_none(), "stable envelope should omit id");
        assert!(
            event.get("data").is_none(),
            "stable envelope should omit data"
        );

        let payload = event
            .get("payload")
            .and_then(Value::as_object)
            .expect("payload should be an object");
        assert_eq!(
            payload
                .get("message")
                .and_then(Value::as_str)
                .expect("payload should include a message"),
            expected_message
        );
    }
}

#[test]
fn cli_help_surface_stays_stable() -> TestResult {
    let fixture = ContractFixture::new()?;

    fixture
        .command()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Add a git repository to watch (creates config if needed)",
        ))
        .stdout(predicate::str::contains("start"))
        .stdout(predicate::str::contains("today"))
        .stdout(predicate::str::contains("summary"))
        .stdout(predicate::str::contains("search"))
        .stdout(predicate::str::contains("update"));

    fixture
        .command()
        .args(["summary", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--from"))
        .stdout(predicate::str::contains("--to"))
        .stdout(predicate::str::contains("--format"));

    fixture
        .command()
        .args(["search", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--repo"))
        .stdout(predicate::str::contains("--limit"))
        .stdout(predicate::str::contains("--format"));

    fixture
        .command()
        .args(["today", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Sync today's commits"));

    Ok(())
}

#[test]
fn summary_and_log_reject_invalid_date_combinations() -> TestResult {
    let fixture = ContractFixture::new()?;

    fixture
        .command()
        .args(["summary", "2026-04-03", "--from", "2026-04-01"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Cannot combine a positional date with --from/--to",
        ));

    fixture
        .command()
        .args(["log", "--to", "2026-04-01"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--to requires --from"));

    Ok(())
}

#[test]
fn config_add_list_and_remove_round_trip_with_isolated_roots() -> TestResult {
    let fixture = ContractFixture::new()?;
    fixture.init_git_repo()?;

    let config_path = fixture.config_path();
    let printed_config = fixture.command_output(&["config"])?;
    assert_eq!(printed_config.trim(), config_path.to_string_lossy());

    fixture
        .command()
        .args([
            "add",
            fixture
                .git_repo_dir()
                .to_str()
                .expect("repo path is valid UTF-8"),
            "--name",
            REPO_NAME,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Now tracking"));

    fixture
        .command()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains(REPO_NAME))
        .stdout(predicate::str::contains(
            fixture.git_repo_dir().to_string_lossy(),
        ));

    fixture
        .command()
        .args(["remove", REPO_NAME])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed: fixture-repo"));

    fixture
        .command()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("No repos configured"));

    Ok(())
}

#[test]
fn add_bootstraps_config_without_inline_llm_setup() -> TestResult {
    let fixture = ContractFixture::new()?;
    fixture.init_git_repo()?;

    fixture
        .command()
        .args([
            "add",
            fixture
                .git_repo_dir()
                .to_str()
                .expect("repo path is valid UTF-8"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Now tracking"))
        .stdout(predicate::str::contains("No LLM configured.").not());

    let config_contents = std::fs::read_to_string(fixture.config_path())?;
    assert!(config_contents.contains("[[repos]]"));
    assert!(config_contents.contains(&fixture.git_repo_dir().to_string_lossy().to_string()));
    assert!(!config_contents.contains("api_key = "));

    Ok(())
}

#[test]
fn sync_today_summary_log_and_search_emit_stable_json_envelopes() -> TestResult {
    let fixture = ContractFixture::new()?;
    fixture.init_git_repo()?;

    let recent_date = fixture.today_date();
    let old_date = fixture.date_days_ago(10);

    fixture.commit_file(
        "README.md",
        "# fixture repo\n",
        "Old contract commit",
        &old_date,
    )?;
    fixture.commit_file(
        "notes.txt",
        "recent fixture notes\n",
        "Recent contract commit",
        &recent_date,
    )?;
    fixture.write_config(&fixture.config_toml(Some(REPO_NAME)))?;

    fixture
        .command()
        .arg("sync")
        .assert()
        .success()
        .stderr(predicate::str::contains("Syncing fixture-repo"))
        .stderr(predicate::str::contains("✓ Synced fixture-repo"))
        .stderr(predicate::str::contains("  added commits: 2"))
        .stderr(predicate::str::contains("  already there: 0"))
        .stderr(predicate::str::contains("  total processed: 2"));

    assert!(
        fixture.data_path().exists(),
        "sync should create the database at the isolated data path"
    );

    let today_json = fixture.command_output(&["today", "--format", "json"])?;
    fixture.assert_json_event_envelope(&today_json, "Recent contract commit");

    let summary_json = fixture.command_output(&["summary", &recent_date, "--format", "json"])?;
    fixture.assert_json_event_envelope(&summary_json, "Recent contract commit");

    let log_json = fixture.command_output(&["log", &recent_date, "--format", "json"])?;
    fixture.assert_json_event_envelope(&log_json, "Recent contract commit");

    let search_json =
        fixture.command_output(&["search", "Recent", "--repo", REPO_NAME, "--format", "json"])?;
    fixture.assert_json_event_envelope(&search_json, "Recent contract commit");

    Ok(())
}

#[test]
fn sync_reports_existing_commits_on_repeat_runs() -> TestResult {
    let fixture = ContractFixture::new()?;
    fixture.init_git_repo()?;

    let recent_date = fixture.today_date();

    fixture.commit_file(
        "README.md",
        "# fixture repo\n",
        "First contract commit",
        &recent_date,
    )?;
    fixture.commit_file(
        "notes.txt",
        "recent fixture notes\n",
        "Second contract commit",
        &recent_date,
    )?;
    fixture.write_config(&fixture.config_toml(Some(REPO_NAME)))?;

    fixture.command().arg("sync").assert().success();

    fixture
        .command()
        .arg("sync")
        .assert()
        .success()
        .stderr(predicate::str::contains("Syncing fixture-repo"))
        .stderr(predicate::str::contains("✓ Synced fixture-repo"))
        .stderr(predicate::str::contains("  added commits: 0"))
        .stderr(predicate::str::contains("  already there: 2"))
        .stderr(predicate::str::contains("  total processed: 2"));

    Ok(())
}

#[test]
fn summary_commands_scope_sync_before_json_output() -> TestResult {
    let fixture = ContractFixture::new()?;
    fixture.init_git_repo()?;

    let recent_date = fixture.today_date();
    let old_date = fixture.date_days_ago(10);

    fixture.commit_file("README.md", "# old\n", "Old contract commit", &old_date)?;
    fixture.commit_file(
        "notes.txt",
        "recent\n",
        "Recent contract commit",
        &recent_date,
    )?;
    fixture.write_config(&fixture.config_toml(Some(REPO_NAME)))?;

    let today_json = fixture.command_output(&["today", "--format", "json"])?;
    fixture.assert_json_event_envelope(&today_json, "Recent contract commit");

    let old_log_before_full_sync =
        fixture.command_output(&["log", &old_date, "--format", "json"])?;
    let old_events_before_full_sync: Value = serde_json::from_str(&old_log_before_full_sync)?;
    assert_eq!(
        old_events_before_full_sync
            .as_array()
            .map(std::vec::Vec::len),
        Some(0),
        "scoped summary sync should not backfill commits outside the requested window"
    );

    let old_summary_json = fixture.command_output(&["summary", &old_date, "--format", "json"])?;
    fixture.assert_json_event_envelope(&old_summary_json, "Old contract commit");

    fixture.command().arg("sync").assert().success();

    let old_log_after_full_sync =
        fixture.command_output(&["log", &old_date, "--format", "json"])?;
    fixture.assert_json_event_envelope(&old_log_after_full_sync, "Old contract commit");

    Ok(())
}

#[test]
fn today_prompts_for_inline_llm_setup_and_then_continues() -> TestResult {
    let fixture = ContractFixture::new()?;
    fixture.init_git_repo()?;
    fixture.write_config(&fixture.config_toml_missing_llm_key(Some(REPO_NAME)))?;
    let today = fixture.today_date();

    let assert = fixture
        .command()
        .arg("today")
        .write_stdin("2\nsk-test-inline\n\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "No activity recorded for this date.",
        ))
        .stderr(predicate::str::contains(format!(
            "Syncing activity for {today}"
        )))
        .stderr(predicate::str::contains(format!(
            "Syncing fixture-repo\n✓ fixture-repo\n\nNo LLM configured."
        )))
        .stderr(predicate::str::contains(format!(
            "Syncing activity for {today}\nSyncing fixture-repo"
        )))
        .stderr(predicate::str::contains("✓ fixture-repo"))
        .stderr(predicate::str::contains(
            "No LLM configured.\nChoose a provider:",
        ))
        .stderr(predicate::str::contains(
            "Choose a provider:\n1. Anthropic\n2. OpenAI\n3. Local model (Ollama)\n> ",
        ))
        .stderr(predicate::str::contains("Enter your API key:\n> "))
        .stderr(predicate::str::contains("Select model [gpt-4o-mini]:\n> "))
        .stderr(predicate::str::contains(format!(
            "Generating summary for {today}"
        )));

    let _ = assert.get_output();
    let config_contents = std::fs::read_to_string(fixture.config_path())?;
    assert!(config_contents.contains("provider = \"openai\""));
    assert!(config_contents.contains("api_key = \"sk-test-inline\""));
    assert!(config_contents.contains("model = \"gpt-4o-mini\""));

    Ok(())
}

#[test]
fn prune_removes_only_aged_events() -> TestResult {
    let fixture = ContractFixture::new()?;
    fixture.init_git_repo()?;

    let recent_date = fixture.today_date();
    let old_date = fixture.date_days_ago(10);

    fixture.commit_file(
        "README.md",
        "# fixture repo\n",
        "Old prune candidate",
        &old_date,
    )?;
    fixture.commit_file(
        "notes.txt",
        "recent prune notes\n",
        "Recent prune candidate",
        &recent_date,
    )?;
    fixture.write_config(&fixture.config_toml(Some(REPO_NAME)))?;

    fixture.command().arg("sync").assert().success();

    fixture
        .command()
        .args(["prune", "7"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Pruned 1 event(s) older than 7 days",
        ));

    let old_log = fixture.command_output(&["log", &old_date, "--format", "json"])?;
    let old_events: Value = serde_json::from_str(&old_log)?;
    assert_eq!(old_events.as_array().map(|events| events.len()), Some(0));

    let recent_log = fixture.command_output(&["log", &recent_date, "--format", "json"])?;
    fixture.assert_json_event_envelope(&recent_log, "Recent prune candidate");

    Ok(())
}

#[test]
fn status_reports_not_running_in_isolation() -> TestResult {
    let fixture = ContractFixture::new()?;
    fixture.write_config(
        r#"[general]
poll_interval_secs = 1
author = "Fixture Tester"

[llm]
provider = "ollama"
model = "llama3.2"
"#,
    )?;

    fixture
        .command()
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("devjournal daemon: not running"))
        .stdout(predicate::str::contains("No repos configured"));

    Ok(())
}
