mod config;
mod daemon;
mod db;
mod git_poller;
mod llm;
mod summary;

use anyhow::Result;
use clap::{Parser, Subcommand};

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
    },
    /// Generate and display summary for a specific date (YYYY-MM-DD)
    Summary {
        date: Option<String>,
        /// Bypass cache and regenerate even if events haven't changed
        #[arg(long)]
        force: bool,
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
    /// Show raw events for today (for debugging)
    Log { date: Option<String> },
    /// Print the path to the config file
    Config,
    /// Initialize devjournal with guided setup
    Init,
    /// List all watched repositories
    List,
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

        Some(Commands::Today { force }) => {
            let date = summary::today();
            let config = config::load()?;
            let text = summary::generate(&date, &config.llm, force)?;
            println!("{}", text);
        }

        Some(Commands::Summary { date, force }) => {
            let date = date.unwrap_or_else(summary::today);
            let config = config::load()?;
            let text = summary::generate(&date, &config.llm, force)?;
            println!("{}", text);
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

        Some(Commands::Log { date }) => {
            let date = date.unwrap_or_else(summary::today);
            let conn = db::open()?;
            let events = db::get_events_for_date(&conn, &date)?;
            if events.is_empty() {
                println!("No events recorded for {}", date);
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
