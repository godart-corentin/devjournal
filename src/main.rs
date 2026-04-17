mod config;
mod daemon;
mod db;
mod git_poller;
mod llm;
mod sem;
mod summary;
mod summary_pipeline;
mod update;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use serde::Serialize;
use spinners::{Spinner, Spinners, Stream};
use std::io::{self, IsTerminal};

#[derive(Clone, clap::ValueEnum, Default)]
enum Format {
    #[default]
    Markdown,
    Json,
}

#[derive(Parser)]
#[command(name = "devjournal", about = "Automatic intelligent work diary")]
struct Cli {
    #[arg(long, hide = true)]
    daemon_mode: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the optional background daemon for continuous polling
    Start,
    /// Stop the running daemon
    Stop,
    /// Sync today's commits, then generate today's summary
    Today {
        /// Bypass cache and regenerate even if events haven't changed
        #[arg(long)]
        force: bool,
        /// Print the internal summary pipeline report as JSON and skip LLM generation
        #[arg(long)]
        debug_pipeline: bool,
        /// Output format (markdown or json)
        #[arg(long, default_value = "markdown")]
        format: Format,
    },
    /// Sync a requested date or range, then generate that summary
    Summary {
        /// Date to summarise (YYYY-MM-DD). Omit to use today.
        date: Option<String>,
        /// Start of date range (YYYY-MM-DD), used with --to
        #[arg(long)]
        from: Option<String>,
        /// End of date range (YYYY-MM-DD), used with --from
        #[arg(long)]
        to: Option<String>,
        /// Bypass cache and regenerate even if events haven't changed
        #[arg(long)]
        force: bool,
        /// Print the internal summary pipeline report as JSON and skip LLM generation
        #[arg(long)]
        debug_pipeline: bool,
        /// Output format (markdown or json)
        #[arg(long, default_value = "markdown")]
        format: Format,
    },
    /// Sync and summarize the current rolling 7-day window (today minus 6 days through today)
    Week {
        /// Bypass cache and regenerate even if events haven't changed
        #[arg(long)]
        force: bool,
        /// Print the internal summary pipeline report as JSON and skip LLM generation
        #[arg(long)]
        debug_pipeline: bool,
        /// Output format (markdown or json)
        #[arg(long, default_value = "markdown")]
        format: Format,
    },
    /// Sync and summarize the current rolling 30-day window (today minus 29 days through today)
    Month {
        /// Bypass cache and regenerate even if events haven't changed
        #[arg(long)]
        force: bool,
        /// Print the internal summary pipeline report as JSON and skip LLM generation
        #[arg(long)]
        debug_pipeline: bool,
        /// Output format (markdown or json)
        #[arg(long, default_value = "markdown")]
        format: Format,
    },
    /// Add a git repository to watch (creates config if needed)
    Add {
        path: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Remove a git repository from the watch list by name or path
    Remove {
        #[arg(value_name = "REPO")]
        repo: String,
    },
    /// Show daemon status and watched repos
    Status,
    /// Show raw events for a date or range (for debugging)
    Log {
        /// Date to show (YYYY-MM-DD). Omit to use today.
        date: Option<String>,
        /// Start of date range (YYYY-MM-DD), used with --to
        #[arg(long)]
        from: Option<String>,
        /// End of date range (YYYY-MM-DD), used with --from
        #[arg(long)]
        to: Option<String>,
        /// Output format (markdown or json)
        #[arg(long, default_value = "markdown")]
        format: Format,
    },
    /// Print the path to the config file
    Config,
    /// Run the optional guided setup flow
    Init,
    /// List all watched repositories
    List,
    /// Search recorded events by keyword
    Search {
        /// Keyword to search for in commit messages
        keyword: String,
        /// Filter by repo name or path
        #[arg(long)]
        repo: Option<String>,
        /// Maximum number of results (default: 20)
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Output format (markdown or json)
        #[arg(long, default_value = "markdown")]
        format: Format,
    },
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for (bash, zsh, fish, elvish)
        shell: clap_complete::Shell,
    },
    /// Delete events older than a given number of days
    Prune {
        /// Number of days to keep (events older than this are deleted)
        days: u32,
    },
    /// Run advanced diagnostic checks on your devjournal setup
    Doctor,
    /// Backfill all git history for watched repos into the database
    Sync {
        /// Name or path of a specific repo to sync (syncs all if omitted)
        repo: Option<String>,
    },
    /// Update devjournal to the latest release
    Update,
}

#[derive(Serialize)]
struct JsonEvent<'a> {
    timestamp: &'a str,
    repo_path: &'a str,
    repo_name: Option<&'a str>,
    event_type: &'a str,
    payload: &'a serde_json::Value,
}

fn serialize_events_json(events: &[db::Event]) -> Result<String> {
    let json: Vec<JsonEvent<'_>> = events
        .iter()
        .map(|event| JsonEvent {
            timestamp: &event.timestamp,
            repo_path: &event.repo_path,
            repo_name: event.repo_name.as_deref(),
            event_type: &event.event_type,
            payload: &event.data,
        })
        .collect();
    Ok(serde_json::to_string_pretty(&json)?)
}

fn print_events_json(events: &[db::Event]) -> Result<()> {
    println!("{}", serialize_events_json(events)?);
    Ok(())
}

fn run_with_spinner<T, F>(message: &str, success_message: &str, operation: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    if !io::stderr().is_terminal() {
        eprintln!("{message}");
        let result = operation();
        if !success_message.is_empty() {
            let status = if result.is_ok() { "✓" } else { "✗" };
            eprintln!("{status} {success_message}");
        }
        return result;
    }

    let mut spinner = Spinner::with_stream(Spinners::Dots, message.to_string(), Stream::Stderr);
    let result = operation();
    if success_message.is_empty() {
        spinner.stop();
    } else {
        let status = if result.is_ok() { "✓" } else { "✗" }.to_string();
        spinner.stop_and_persist(&status, success_message.to_string());
    }

    result
}

fn sync_summary_window(window: &summary::SummaryWindow, config: &config::Config) -> Result<()> {
    let conn = db::open()?;
    let author = config.general.author.as_deref();
    let label = window.display_label();
    eprintln!("Syncing activity for {label}");

    for repo_config in &config.repos {
        let repo_name = repo_config.display_name().to_string();
        let message = format!("Syncing {repo_name}");
        let success_message = repo_name.clone();
        run_with_spinner(&message, &success_message, || {
            git_poller::sync_repo_range(repo_config, &conn, author, window.from(), window.to())
                .map(|_| ())
        })?;
    }
    eprintln!();

    Ok(())
}

fn print_summary_window(
    window: &summary::SummaryWindow,
    force: bool,
    debug_pipeline: bool,
    format: Format,
) -> Result<()> {
    let config = config::load()?;
    sync_summary_window(window, &config)?;
    let conn = db::open()?;
    let events = window.load_events(&conn)?;

    if debug_pipeline {
        let report = summary_pipeline::build_report(&events, &window.display_label())?;
        println!(
            "{}",
            summary_pipeline::render::render_pipeline_debug_json(&report)?
        );
        return Ok(());
    }

    match format {
        Format::Json => {
            print_events_json(&events)?;
        }
        Format::Markdown => {
            let label = window.display_label();
            let message = format!("Generating summary for {label}");
            let success_message = format!("Generated summary for {label}.");
            let text = run_with_spinner(&message, &success_message, || {
                window.generate_markdown(&config.llm, force)
            })?;
            println!("{}", text);
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Hidden daemon mode — runs the polling loop
    if cli.daemon_mode {
        return daemon::run_daemon_loop();
    }

    match cli.command {
        None => {
            // Default: show status
            daemon::status()?;
        }

        Some(Commands::Start) => daemon::start()?,
        Some(Commands::Stop) => daemon::stop()?,

        Some(Commands::Today {
            force,
            debug_pipeline,
            format,
        }) => {
            let window = summary::SummaryWindow::for_date(summary::today());
            print_summary_window(&window, force, debug_pipeline, format)?;
        }

        Some(Commands::Summary {
            date,
            from,
            to,
            force,
            debug_pipeline,
            format,
        }) => {
            let window = summary::SummaryWindow::from_summary_args(date, from, to)?;
            print_summary_window(&window, force, debug_pipeline, format)?;
        }

        Some(Commands::Week {
            force,
            debug_pipeline,
            format,
        }) => {
            let window = summary::SummaryWindow::rolling_days(7);
            print_summary_window(&window, force, debug_pipeline, format)?;
        }

        Some(Commands::Month {
            force,
            debug_pipeline,
            format,
        }) => {
            let window = summary::SummaryWindow::rolling_days(30);
            print_summary_window(&window, force, debug_pipeline, format)?;
        }

        Some(Commands::Add { path, name }) => {
            config::add_repo(&path, name)?;
        }

        Some(Commands::Remove { repo }) => {
            config::remove_repo(&repo)?;
        }

        Some(Commands::Status) => {
            daemon::status()?;
        }

        Some(Commands::Config) => {
            println!("{}", config::config_path().display());
        }

        Some(Commands::Sync { repo }) => {
            let config = config::load()?;
            let author = config.general.author.as_deref();
            let conn = db::open()?;

            let repos: Vec<_> = match &repo {
                None => config.repos.iter().collect(),
                Some(name) => {
                    let found = config::resolve_repo(&config.repos, name)?;
                    vec![found]
                }
            };

            for repo_config in repos {
                let repo_name = repo_config.display_name().to_string();
                let message = format!("Syncing {repo_name}");
                let success_message = format!("Synced {repo_name}");
                let stats = run_with_spinner(&message, &success_message, || {
                    git_poller::sync_repo(repo_config, &conn, author)
                })?;
                eprintln!("  added commits: {}", stats.added);
                eprintln!("  already there: {}", stats.already_there);
                eprintln!("  total processed: {}", stats.total_processed);
            }
        }

        Some(Commands::Completions { shell }) => {
            let mut cmd = Cli::command();
            clap_complete::generate(shell, &mut cmd, "devjournal", &mut std::io::stdout());
        }

        Some(Commands::Prune { days }) => {
            let cutoff = (chrono::Local::now() - chrono::Duration::days(days as i64))
                .format("%Y-%m-%d")
                .to_string();
            let conn = db::open()?;
            let deleted = db::prune_events_before(&conn, &cutoff)?;
            println!(
                "Pruned {} event(s) older than {} days (before {})",
                deleted, days, cutoff
            );
        }

        Some(Commands::Doctor) => {
            let mut issues = 0u32;

            // 1. Config
            print!("Config file... ");
            match config::load() {
                Ok(cfg) => {
                    println!("OK ({})", config::config_path().display());

                    // 2. Author
                    print!("Author... ");
                    match &cfg.general.author {
                        Some(author) => println!("OK (\"{}\")", author),
                        None => {
                            println!("MISSING — set [general] author in config");
                            issues += 1;
                        }
                    }

                    // 3. LLM API key
                    print!("LLM API key... ");
                    if cfg.llm.provider == config::LlmProvider::Ollama {
                        println!("SKIPPED ({} does not need a key)", cfg.llm.provider);
                    } else if config::api_key(&cfg.llm).is_some() {
                        println!("OK");
                    } else {
                        println!(
                            "MISSING — set DEVJOURNAL_API_KEY or api_key in config for {}",
                            cfg.llm.provider
                        );
                        issues += 1;
                    }

                    // 3b. sem CLI
                    print!("sem CLI... ");
                    let sem_probe = sem::probe();
                    match sem_probe.status {
                        sem::SemIntegrationStatus::Active => {
                            println!("ACTIVE — {}", sem_probe.detail)
                        }
                        sem::SemIntegrationStatus::Unavailable => {
                            println!(
                                "UNAVAILABLE — {} (try: {})",
                                sem_probe.detail, sem_probe.install_hint
                            );
                        }
                        sem::SemIntegrationStatus::Degraded => {
                            println!(
                                "DEGRADED — {} (try: {})",
                                sem_probe.detail, sem_probe.install_hint
                            );
                        }
                    }

                    // 4. Repos
                    if cfg.repos.is_empty() {
                        println!("Repos... NONE configured — use `devjournal add <path>`");
                        issues += 1;
                    } else {
                        for repo in &cfg.repos {
                            print!("Repo {}... ", repo.display_name());
                            let p = std::path::Path::new(&repo.path);
                            if !p.exists() {
                                println!("MISSING — path does not exist");
                                issues += 1;
                            } else if git2::Repository::open(&repo.path).is_err() {
                                println!("NOT A GIT REPO");
                                issues += 1;
                            } else {
                                println!("OK");
                            }
                        }
                    }
                }
                Err(_) => {
                    println!("MISSING — run `devjournal add <path>` or `devjournal init`");
                    issues += 1;
                }
            }

            // 5. Database
            print!("Database... ");
            match db::open() {
                Ok(_) => println!("OK ({})", db::db_path().display()),
                Err(e) => {
                    println!("ERROR — {}", e);
                    issues += 1;
                }
            }

            // 6. Daemon
            print!("Daemon... ");
            match daemon::read_pid_public() {
                Ok(Some(pid)) => println!("RUNNING (PID: {})", pid),
                Ok(None) => println!("NOT RUNNING"),
                Err(_) => {
                    println!("UNKNOWN (could not read PID file)");
                    issues += 1;
                }
            }

            if issues == 0 {
                println!("\nAll checks passed.");
            } else {
                println!("\n{} issue(s) found.", issues);
            }
        }

        Some(Commands::Init) => {
            config::init()?;
        }

        Some(Commands::List) => {
            let config = config::load_or_default();
            if config.repos.is_empty() {
                println!("No repos configured. Use `devjournal add <path>` to add one.");
            } else {
                for repo in &config.repos {
                    println!("{} ({})", repo.display_name(), repo.path);
                }
            }
        }

        Some(Commands::Search {
            keyword,
            repo,
            limit,
            format,
        }) => {
            let conn = db::open()?;
            let events = db::search_events(&conn, &keyword, repo.as_deref(), limit)?;
            match format {
                Format::Json => {
                    print_events_json(&events)?;
                }
                Format::Markdown => {
                    if events.is_empty() {
                        println!("No events matching \"{}\"", keyword);
                    } else {
                        for e in &events {
                            println!(
                                "[{}] {} — {}",
                                &e.timestamp[..10],
                                e.repo_name.as_deref().unwrap_or(&e.repo_path),
                                e.data["message"].as_str().unwrap_or("?")
                            );
                        }
                        println!("\n{} result(s)", events.len());
                    }
                }
            }
        }

        Some(Commands::Update) => update::run_update()?,

        Some(Commands::Log {
            date,
            from,
            to,
            format,
        }) => {
            let conn = db::open()?;
            let (events, label) = match (date, from, to) {
                (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                    anyhow::bail!("Cannot combine a positional date with --from/--to");
                }
                (_, None, Some(_)) => {
                    anyhow::bail!("--to requires --from");
                }
                (_, Some(from), to) => {
                    let to = to.unwrap_or_else(summary::today);
                    let label = format!("{} to {}", from, to);
                    let events = db::get_events_for_range(&conn, &from, &to)?;
                    (events, label)
                }
                (date, None, None) => {
                    let date = date.unwrap_or_else(summary::today);
                    let events = db::get_events_for_date(&conn, &date)?;
                    (events, date)
                }
            };
            match format {
                Format::Json => {
                    print_events_json(&events)?;
                }
                Format::Markdown => {
                    if events.is_empty() {
                        println!("No events recorded for {}", label);
                    } else {
                        for e in &events {
                            println!(
                                "[{}] {} — {}",
                                e.timestamp,
                                e.repo_name.as_deref().unwrap_or(&e.repo_path),
                                e.data["message"].as_str().unwrap_or("?")
                            );
                        }
                        println!("\n{} event(s) total", events.len());
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_log_command_accepts_json_format() {
        let cli =
            Cli::try_parse_from(["devjournal", "log", "2026-04-03", "--format", "json"]).unwrap();

        match cli.command {
            Some(Commands::Log {
                date,
                from,
                to,
                format,
            }) => {
                assert_eq!(date.as_deref(), Some("2026-04-03"));
                assert!(from.is_none());
                assert!(to.is_none());
                assert!(matches!(format, Format::Json));
            }
            _ => panic!("expected log command"),
        }
    }

    #[test]
    fn test_serialize_events_json_includes_stable_event_envelope() {
        let payload = json!({
            "hash": "abc123",
            "message": "Fix stable JSON contract",
            "branch": "main"
        });
        let events = vec![db::Event {
            id: Some(7),
            repo_path: "/tmp/dev-journal".to_string(),
            repo_name: Some("dev-journal".to_string()),
            event_type: "commit".to_string(),
            timestamp: "2026-04-03T10:00:00Z".to_string(),
            data: payload.clone(),
        }];

        let rendered = serialize_events_json(&events).unwrap();
        let json: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        let event = &json.as_array().unwrap()[0];

        assert_eq!(event["timestamp"], json!("2026-04-03T10:00:00Z"));
        assert_eq!(event["repo_path"], json!("/tmp/dev-journal"));
        assert_eq!(event["repo_name"], json!("dev-journal"));
        assert_eq!(event["event_type"], json!("commit"));
        assert_eq!(event["payload"], payload);
        assert!(event.get("id").is_none());
        assert!(event.get("data").is_none());
    }

    #[test]
    fn test_serialize_events_json_preserves_null_repo_name() {
        let events = vec![db::Event {
            id: None,
            repo_path: "/tmp/no-name".to_string(),
            repo_name: None,
            event_type: "commit".to_string(),
            timestamp: "2026-04-04T09:30:00Z".to_string(),
            data: json!({ "hash": "def456" }),
        }];

        let rendered = serialize_events_json(&events).unwrap();
        let json: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        let event = &json.as_array().unwrap()[0];

        assert_eq!(event["repo_name"], serde_json::Value::Null);
    }
}
