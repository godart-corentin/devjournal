mod config;
mod db;
mod git_poller;
mod daemon;
mod summary;
mod llm;

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
    Today,
    /// Generate and display summary for a specific date (YYYY-MM-DD)
    Summary {
        date: Option<String>,
    },
    /// Add a git repository to watch
    Add {
        path: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Remove a git repository from the watch list
    Remove {
        path: String,
    },
    /// Show daemon status and watched repos
    Status,
    /// Show raw events for today (for debugging)
    Log {
        date: Option<String>,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Start the daemon in the background
    Start,
    /// Stop the running daemon
    Stop,
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
        },

        Some(Commands::Today) => {
            let date = summary::today();
            let config = config::load()?;
            let text = summary::generate(&date, &config.llm)?;
            println!("{}", text);
        }

        Some(Commands::Summary { date }) => {
            let date = date.unwrap_or_else(summary::today);
            let config = config::load()?;
            let text = summary::generate(&date, &config.llm)?;
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

        Some(Commands::Log { date }) => {
            let date = date.unwrap_or_else(summary::today);
            let conn = db::open()?;
            let events = db::get_events_for_date(&conn, &date)?;
            if events.is_empty() {
                println!("No events recorded for {}", date);
            } else {
                for e in &events {
                    println!("[{}] {} — {}",
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
