mod config;
mod daemon;
mod db;
mod git_poller;
mod llm;
mod summary;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};

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
    /// Manage the background daemon
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Generate and display today's summary
    Today {
        /// Bypass cache and regenerate even if events haven't changed
        #[arg(long)]
        force: bool,
        /// Output format (markdown or json)
        #[arg(long, default_value = "markdown")]
        format: Format,
    },
    /// Generate and display summary for a specific date or range
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
        /// Output format (markdown or json)
        #[arg(long, default_value = "markdown")]
        format: Format,
    },
    /// Generate a rolling 7-day summary (today minus 6 days through today)
    Week {
        /// Bypass cache and regenerate even if events haven't changed
        #[arg(long)]
        force: bool,
        /// Output format (markdown or json)
        #[arg(long, default_value = "markdown")]
        format: Format,
    },
    /// Generate a rolling 30-day summary (today minus 29 days through today)
    Month {
        /// Bypass cache and regenerate even if events haven't changed
        #[arg(long)]
        force: bool,
        /// Output format (markdown or json)
        #[arg(long, default_value = "markdown")]
        format: Format,
    },
    /// Add a git repository to watch
    Add {
        path: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Remove a git repository from the watch list
    Remove { path: String },
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
    },
    /// Print the path to the config file
    Config,
    /// Initialize devjournal with guided setup
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
    /// Run diagnostic checks on your devjournal setup
    Doctor,
    /// Sync all git history for watched repos into the database
    Sync {
        /// Name or path of a specific repo to sync (syncs all if omitted)
        repo: Option<String>,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Start the daemon in the background
    Start,
    /// Stop the running daemon
    Stop,
    /// Print the path to the daemon log file
    Logs,
}

fn print_events_json(events: &[db::Event]) -> Result<()> {
    let json: Vec<&serde_json::Value> = events.iter().map(|e| &e.data).collect();
    println!("{}", serde_json::to_string_pretty(&json)?);
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

        Some(Commands::Daemon { action }) => match action {
            DaemonAction::Start => daemon::start()?,
            DaemonAction::Stop => daemon::stop()?,
            DaemonAction::Logs => println!("{}", daemon::log_path().display()),
        },

        Some(Commands::Today { force, format }) => {
            let date = summary::today();
            match format {
                Format::Json => {
                    let conn = db::open()?;
                    let events = db::get_events_for_date(&conn, &date)?;
                    print_events_json(&events)?;
                }
                Format::Markdown => {
                    let config = config::load()?;
                    let text = summary::generate(&date, &config.llm, force)?;
                    println!("{}", text);
                }
            }
        }

        Some(Commands::Summary {
            date,
            from,
            to,
            force,
            format,
        }) => match (date, from, to) {
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                anyhow::bail!("Cannot combine a positional date with --from/--to");
            }
            (_, Some(from), to) => {
                let to = to.unwrap_or_else(summary::today);
                match format {
                    Format::Json => {
                        let conn = db::open()?;
                        let events = db::get_events_for_range(&conn, &from, &to)?;
                        print_events_json(&events)?;
                    }
                    Format::Markdown => {
                        let config = config::load()?;
                        let text = summary::generate_range(&from, &to, &config.llm, force)?;
                        println!("{}", text);
                    }
                }
            }
            (_, None, Some(_)) => {
                anyhow::bail!("--to requires --from");
            }
            (date, None, None) => {
                let date = date.unwrap_or_else(summary::today);
                match format {
                    Format::Json => {
                        let conn = db::open()?;
                        let events = db::get_events_for_date(&conn, &date)?;
                        print_events_json(&events)?;
                    }
                    Format::Markdown => {
                        let config = config::load()?;
                        let text = summary::generate(&date, &config.llm, force)?;
                        println!("{}", text);
                    }
                }
            }
        },

        Some(Commands::Week { force, format }) => {
            use chrono::Duration;
            let to = summary::today();
            let from = (chrono::Local::now() - Duration::days(6))
                .format("%Y-%m-%d")
                .to_string();
            match format {
                Format::Json => {
                    let conn = db::open()?;
                    let events = db::get_events_for_range(&conn, &from, &to)?;
                    print_events_json(&events)?;
                }
                Format::Markdown => {
                    let config = config::load()?;
                    let text = summary::generate_range(&from, &to, &config.llm, force)?;
                    println!("{}", text);
                }
            }
        }

        Some(Commands::Month { force, format }) => {
            use chrono::Duration;
            let to = summary::today();
            let from = (chrono::Local::now() - Duration::days(29))
                .format("%Y-%m-%d")
                .to_string();
            match format {
                Format::Json => {
                    let conn = db::open()?;
                    let events = db::get_events_for_range(&conn, &from, &to)?;
                    print_events_json(&events)?;
                }
                Format::Markdown => {
                    let config = config::load()?;
                    let text = summary::generate_range(&from, &to, &config.llm, force)?;
                    println!("{}", text);
                }
            }
        }

        Some(Commands::Add { path, name }) => {
            config::add_repo(&path, name)?;
        }

        Some(Commands::Remove { path }) => {
            config::remove_repo(&path)?;
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
                    let found = config
                        .repos
                        .iter()
                        .find(|r| r.display_name() == name || r.path == *name);
                    match found {
                        Some(r) => vec![r],
                        None => anyhow::bail!(
                            "Repo '{}' not found. Use `devjournal list` to see tracked repos.",
                            name
                        ),
                    }
                }
            };

            for repo_config in repos {
                print!("Syncing {}... ", repo_config.display_name());
                let count = git_poller::sync_repo(repo_config, &conn, author)?;
                println!("{} commit(s) added", count);
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
                    if cfg.llm.provider == "ollama" {
                        println!("SKIPPED (ollama does not need a key)");
                    } else if config::api_key(&cfg.llm).is_some() {
                        println!("OK");
                    } else {
                        println!(
                            "MISSING — set DEVJOURNAL_API_KEY or api_key in config for {}",
                            cfg.llm.provider
                        );
                        issues += 1;
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
                    println!("MISSING — run `devjournal init`");
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

        Some(Commands::Log { date, from, to }) => {
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

    Ok(())
}
