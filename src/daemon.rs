use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::config;
use crate::db;
use crate::git_poller;

pub fn pid_path() -> PathBuf {
    db::data_dir().join("devjournal.pid")
}

pub fn start() -> Result<()> {
    // Check if already running
    if let Some(pid) = read_pid()? {
        if is_process_alive(pid) {
            println!("devjournal daemon is already running (PID: {})", pid);
            return Ok(());
        }
        // Stale PID file — remove it
        let _ = std::fs::remove_file(pid_path());
    }

    // Spawn self in daemon mode
    let exe = std::env::current_exe().context("Cannot find current executable")?;
    let child = std::process::Command::new(exe)
        .arg("--daemon-mode")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn daemon process")?;

    println!("devjournal daemon started (PID: {})", child.id());
    Ok(())
}

pub fn stop() -> Result<()> {
    let pid = read_pid()?
        .context("No PID file found. Is the daemon running? Try `devjournal status`.")?;

    if !is_process_alive(pid) {
        println!("Daemon is not running (stale PID file). Cleaning up.");
        let _ = std::fs::remove_file(pid_path());
        return Ok(());
    }

    // Send SIGTERM
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }

    // Wait up to 5 seconds for it to exit
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(100));
        if !is_process_alive(pid) {
            let _ = std::fs::remove_file(pid_path());
            println!("devjournal daemon stopped.");
            return Ok(());
        }
    }

    anyhow::bail!("Daemon did not stop after 5 seconds. PID: {}", pid);
}

pub fn status() -> Result<()> {
    match read_pid()? {
        Some(pid) if is_process_alive(pid) => {
            println!("devjournal daemon: running (PID: {})", pid);
        }
        Some(_) => {
            println!("devjournal daemon: not running (stale PID file)");
        }
        None => {
            println!("devjournal daemon: not running");
        }
    }

    // Show watched repos and event counts
    let config = config::load_or_default();
    if config.repos.is_empty() {
        println!("No repos configured. Use `devjournal add <path>` to add one.");
    } else {
        println!("\nWatched repos:");
        if let Ok(conn) = db::open() {
            let today = crate::summary::today();
            for repo in &config.repos {
                let count = db::event_count_for_date(&conn, &today).unwrap_or(0);
                println!("  {} ({} events today)", repo.display_name(), count);
            }
        }
    }
    Ok(())
}

pub fn run_daemon_loop() -> Result<()> {
    // Write PID file
    let pid = std::process::id();
    std::fs::create_dir_all(pid_path().parent().unwrap())?;
    std::fs::write(pid_path(), pid.to_string())?;

    // Set up signal handler
    SHOULD_STOP.store(false, Ordering::SeqCst);
    unsafe {
        libc::signal(libc::SIGTERM, handle_sigterm as *const () as libc::sighandler_t);
    }

    eprintln!("[devjournal daemon] started with PID {}", pid);

    let config = config::load_or_default();
    let poll_interval = Duration::from_secs(config.general.poll_interval_secs);

    while !SHOULD_STOP.load(Ordering::SeqCst) {
        match db::open() {
            Ok(conn) => {
                let cfg = config::load_or_default();
                for repo in &cfg.repos {
                    match git_poller::poll_repo(repo, &conn) {
                        Ok(0) => {}
                        Ok(n) => eprintln!("[devjournal daemon] {} new commit(s) from {}", n, repo.display_name()),
                        Err(e) => eprintln!("[devjournal daemon] error polling {}: {}", repo.display_name(), e),
                    }
                }
            }
            Err(e) => eprintln!("[devjournal daemon] DB error: {}", e),
        }

        // Sleep in small increments so we can respond to SIGTERM
        let steps = poll_interval.as_secs().max(1);
        for _ in 0..steps {
            if SHOULD_STOP.load(Ordering::SeqCst) { break; }
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    eprintln!("[devjournal daemon] shutting down");
    let _ = std::fs::remove_file(pid_path());
    Ok(())
}

static SHOULD_STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigterm(_: libc::c_int) {
    SHOULD_STOP.store(true, Ordering::SeqCst);
}

fn read_pid() -> Result<Option<u32>> {
    let path = pid_path();
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    let pid: u32 = content.trim().parse()
        .with_context(|| format!("Invalid PID in {}", path.display()))?;
    Ok(Some(pid))
}

fn is_process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}
