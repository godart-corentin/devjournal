use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::config;
use crate::db;
use crate::git_poller;

pub fn pid_path() -> PathBuf {
    db::data_dir().join("devjournal.pid")
}

pub fn log_path() -> PathBuf {
    db::data_dir().join("devjournal.log")
}

pub fn start() -> Result<()> {
    if let Some(pid) = read_pid()? {
        if is_process_alive(pid) {
            println!("devjournal daemon is already running (PID: {})", pid);
            return Ok(());
        }
        let _ = std::fs::remove_file(pid_path());
    }

    std::fs::create_dir_all(db::data_dir())?;

    #[cfg(unix)]
    {
        return start_unix();
    }

    #[cfg(windows)]
    {
        return start_windows();
    }

    #[allow(unreachable_code)]
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

    terminate_process(pid)?;

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

fn format_elapsed(secs: i64) -> String {
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

pub fn status() -> Result<()> {
    status_with_db_opener(db::open)
}

fn status_with_db_opener<F>(mut open_db: F) -> Result<()>
where
    F: FnMut() -> Result<Connection>,
{
    let pid_state = read_pid()?;
    let is_running = matches!(&pid_state, Some(pid) if is_process_alive(*pid));

    match &pid_state {
        Some(pid) if is_running => {
            println!("devjournal daemon: running (PID: {})", pid);
        }
        Some(_) => {
            println!("devjournal daemon: not running (stale PID file)");
        }
        None => {
            println!("devjournal daemon: not running");
        }
    }

    let sem_probe = crate::sem::probe();
    println!(
        "semantic enrichment: {} ({})",
        sem_probe.status.label(),
        sem_probe.detail
    );

    let conn = open_db()?;
    let config = config::load_or_default();
    if let Some(last_polled) = db::get_latest_poll_time(&conn)? {
        if let Ok(polled_at) = last_polled.parse::<DateTime<Utc>>() {
            let elapsed_secs = (Utc::now() - polled_at).num_seconds().max(0);
            if is_running {
                println!("  Last polled: {}", format_elapsed(elapsed_secs));
                let interval = config.general.poll_interval_secs as i64;
                let next_in = interval - elapsed_secs;
                if next_in <= 0 {
                    println!("  Next poll in: imminent");
                } else {
                    println!("  Next poll in: ~{}s", next_in);
                }
            } else {
                println!(
                    "  Last polled: {} (daemon stopped)",
                    format_elapsed(elapsed_secs)
                );
            }
        }
    }

    if config.repos.is_empty() {
        println!("\nNo repos configured. Use `devjournal add <path>` to add one.");
    } else {
        let today = crate::summary::today();
        let mut total_events: i64 = 0;
        let mut repo_counts: Vec<(&crate::config::RepoConfig, i64)> = Vec::new();
        for repo in &config.repos {
            let count = db::event_count_for_date_by_repo(&conn, &repo.path, &today).unwrap_or(0);
            total_events += count;
            repo_counts.push((repo, count));
        }
        println!(
            "\nWatched repos ({} repos, {} events today):",
            config.repos.len(),
            total_events
        );
        for (repo, count) in repo_counts {
            println!("  {} ({} events today)", repo.display_name(), count);
        }
    }

    Ok(())
}

pub fn run_daemon_loop() -> Result<()> {
    run_daemon_loop_with_startup_hook(|_| Ok(()))
}

fn run_daemon_loop_with_startup_hook<F>(mut startup_hook: F) -> Result<()>
where
    F: FnMut(StartupEvent<'_>) -> Result<()>,
{
    let config = config::load_or_default();
    let author = resolve_author(&config, &mut startup_hook)?;

    SHOULD_STOP.store(false, Ordering::SeqCst);
    install_shutdown_handler();

    let pid = std::process::id();
    std::fs::create_dir_all(pid_path().parent().unwrap())?;
    std::fs::write(pid_path(), pid.to_string())?;

    eprintln!("[devjournal daemon] started with PID {}", pid);
    startup_hook(StartupEvent::Ready(pid))?;

    let result = run_poll_loop(
        Duration::from_secs(config.general.poll_interval_secs),
        &author,
    );

    eprintln!("[devjournal daemon] shutting down");
    let _ = std::fs::remove_file(pid_path());
    result
}

fn resolve_author<F>(config: &config::Config, startup_hook: &mut F) -> Result<String>
where
    F: FnMut(StartupEvent<'_>) -> Result<()>,
{
    let message = "no author configured. Set `author` in [general] config.";
    match config.general.author.clone() {
        Some(author) => Ok(author),
        None => {
            startup_hook(StartupEvent::Error(message))?;
            anyhow::bail!(message);
        }
    }
}

fn run_poll_loop(poll_interval: Duration, author: &str) -> Result<()> {
    run_poll_loop_with_db_opener(poll_interval, author, db::open)
}

fn run_poll_loop_with_db_opener<F>(
    poll_interval: Duration,
    author: &str,
    mut open_db: F,
) -> Result<()>
where
    F: FnMut() -> Result<Connection>,
{
    while !SHOULD_STOP.load(Ordering::SeqCst) {
        match open_db() {
            Ok(conn) => {
                let cfg = config::load_or_default();
                let poll_author = cfg.general.author.as_deref().unwrap_or(author);
                for repo in &cfg.repos {
                    match git_poller::poll_repo(repo, &conn, Some(poll_author)) {
                        Ok(0) => {}
                        Ok(n) => eprintln!(
                            "[devjournal daemon] {} new commit(s) from {}",
                            n,
                            repo.display_name()
                        ),
                        Err(e) => eprintln!(
                            "[devjournal daemon] error polling {}: {}",
                            repo.display_name(),
                            e
                        ),
                    }
                }

                if let Some(retention_days) = cfg.general.retention_days {
                    let cutoff = (chrono::Local::now()
                        - chrono::Duration::days(retention_days as i64))
                    .format("%Y-%m-%d")
                    .to_string();
                    match db::prune_events_before(&conn, &cutoff) {
                        Ok(0) => {}
                        Ok(n) => eprintln!(
                            "[devjournal daemon] pruned {} old event(s) (before {})",
                            n, cutoff
                        ),
                        Err(e) => eprintln!("[devjournal daemon] prune error: {}", e),
                    }
                }
            }
            Err(e) => return Err(e.context("DB error")),
        }

        let steps = poll_interval.as_secs().max(1);
        for _ in 0..steps {
            if SHOULD_STOP.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    Ok(())
}

enum StartupEvent<'a> {
    Ready(u32),
    Error(&'a str),
}

static SHOULD_STOP: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
fn start_unix() -> Result<()> {
    let log_file = open_log_file()?;
    let (read_fd, write_fd) = create_startup_pipe()?;

    let first_child = unsafe { libc::fork() };
    if first_child < 0 {
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
        anyhow::bail!("Failed to fork daemon process");
    }

    if first_child == 0 {
        unsafe {
            libc::close(read_fd);
        }
        let notifier = unsafe { StartupNotifier::from_raw_fd(write_fd) };
        daemonize_and_run(log_file, notifier);
    }

    unsafe {
        libc::close(write_fd);
    }

    let startup = read_startup_message(unsafe { std::os::fd::FromRawFd::from_raw_fd(read_fd) })?;
    unsafe {
        libc::waitpid(first_child, std::ptr::null_mut(), 0);
    }

    match startup {
        StartupMessage::Ready(pid) => {
            println!("devjournal daemon started (PID: {})", pid);
            Ok(())
        }
        StartupMessage::Error(message) => anyhow::bail!(message),
    }
}

#[cfg(windows)]
fn start_windows() -> Result<()> {
    let exe = std::env::current_exe().context("Cannot find current executable")?;
    let log_file = open_log_file()?;

    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--daemon-mode")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(log_file);

    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, DETACHED_PROCESS};
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);

    let child = cmd.spawn().context("Failed to spawn daemon process")?;
    println!("devjournal daemon started (PID: {})", child.id());
    Ok(())
}

fn open_log_file() -> Result<File> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .context("Failed to open daemon log file")
}

#[cfg(unix)]
fn daemonize_and_run(log_file: File, mut notifier: StartupNotifier) -> ! {
    if unsafe { libc::setsid() } < 0 {
        let _ = notifier.error("Failed to detach daemon session");
        unsafe { libc::_exit(1) };
    }

    if let Err(err) = ignore_signal(libc::SIGHUP) {
        let _ = notifier.error(&format!("Failed to ignore SIGHUP: {}", err));
        unsafe { libc::_exit(1) };
    }

    let second_child = unsafe { libc::fork() };
    if second_child < 0 {
        let _ = notifier.error("Failed to perform second daemon fork");
        unsafe { libc::_exit(1) };
    }

    if second_child > 0 {
        unsafe { libc::_exit(0) };
    }

    if let Err(err) = redirect_standard_streams(&log_file) {
        let _ = notifier.error(&format!("Failed to redirect daemon stdio: {}", err));
        unsafe { libc::_exit(1) };
    }

    let result = run_daemon_loop_with_startup_hook(|event| match event {
        StartupEvent::Ready(pid) => notifier.ready(pid),
        StartupEvent::Error(message) => notifier.error(message),
    });

    if let Err(err) = result {
        let _ = notifier.error(&err.to_string());
        eprintln!("[devjournal daemon] fatal error: {}", err);
        unsafe { libc::_exit(1) };
    }

    unsafe { libc::_exit(0) };
}

#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(unix)]
fn terminate_process(pid: u32) -> Result<()> {
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    Ok(())
}

#[cfg(unix)]
fn install_shutdown_handler() {
    if let Err(err) = install_signal_handler(
        libc::SIGTERM,
        handle_sigterm as *const () as libc::sighandler_t,
    ) {
        eprintln!(
            "[devjournal daemon] warning: failed to install SIGTERM handler: {}",
            err
        );
    }

    if let Err(err) = ignore_signal(libc::SIGHUP) {
        eprintln!(
            "[devjournal daemon] warning: failed to ignore SIGHUP for daemon lifecycle: {}",
            err
        );
    }
}

#[cfg(unix)]
extern "C" fn handle_sigterm(_: libc::c_int) {
    SHOULD_STOP.store(true, Ordering::SeqCst);
}

#[cfg(unix)]
fn install_signal_handler(signal: libc::c_int, handler: libc::sighandler_t) -> Result<()> {
    let previous = unsafe { libc::signal(signal, handler) };
    anyhow::ensure!(
        previous != libc::SIG_ERR,
        "signal() returned SIG_ERR for signal {}",
        signal
    );
    Ok(())
}

#[cfg(unix)]
fn ignore_signal(signal: libc::c_int) -> Result<()> {
    install_signal_handler(signal, libc::SIG_IGN)
}

#[cfg(unix)]
fn create_startup_pipe() -> Result<(libc::c_int, libc::c_int)> {
    let mut fds = [0; 2];
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    anyhow::ensure!(rc == 0, "Failed to create startup pipe");
    Ok((fds[0], fds[1]))
}

#[cfg(unix)]
fn redirect_standard_streams(log_file: &File) -> Result<()> {
    use std::os::fd::AsRawFd;

    let null = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")
        .context("Failed to open /dev/null for daemon stdio")?;

    let null_fd = null.as_raw_fd();
    let log_fd = log_file.as_raw_fd();

    anyhow::ensure!(
        unsafe { libc::dup2(null_fd, libc::STDIN_FILENO) } >= 0,
        "Failed to redirect daemon stdin"
    );
    anyhow::ensure!(
        unsafe { libc::dup2(null_fd, libc::STDOUT_FILENO) } >= 0,
        "Failed to redirect daemon stdout"
    );
    anyhow::ensure!(
        unsafe { libc::dup2(log_fd, libc::STDERR_FILENO) } >= 0,
        "Failed to redirect daemon stderr"
    );

    Ok(())
}

#[cfg(unix)]
struct StartupNotifier(Option<File>);

#[cfg(unix)]
impl StartupNotifier {
    unsafe fn from_raw_fd(fd: libc::c_int) -> Self {
        use std::os::fd::FromRawFd;
        Self(Some(File::from_raw_fd(fd)))
    }

    fn ready(&mut self, pid: u32) -> Result<()> {
        self.send(StartupMessage::Ready(pid))
    }

    fn error(&mut self, message: &str) -> Result<()> {
        self.send(StartupMessage::Error(message.to_string()))
    }

    fn send(&mut self, message: StartupMessage) -> Result<()> {
        let payload = message.encode();
        if let Some(mut file) = self.0.take() {
            file.write_all(payload.as_bytes())
                .context("Failed to write daemon startup status")?;
            let _ = file.flush();
        }
        Ok(())
    }
}

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum StartupMessage {
    Ready(u32),
    Error(String),
}

#[cfg(unix)]
impl StartupMessage {
    fn encode(&self) -> String {
        match self {
            Self::Ready(pid) => format!("READY {pid}\n"),
            Self::Error(message) => format!("ERROR {message}\n"),
        }
    }
}

#[cfg(unix)]
fn read_startup_message(mut reader: File) -> Result<StartupMessage> {
    let mut payload = String::new();
    reader
        .read_to_string(&mut payload)
        .context("Failed to read daemon startup status")?;
    parse_startup_message(&payload)
}

#[cfg(unix)]
fn parse_startup_message(payload: &str) -> Result<StartupMessage> {
    let payload = payload.trim_end();
    if let Some(pid) = payload.strip_prefix("READY ") {
        return Ok(StartupMessage::Ready(pid.parse().with_context(|| {
            format!("Invalid daemon PID in startup payload: {pid}")
        })?));
    }

    if let Some(message) = payload.strip_prefix("ERROR ") {
        return Ok(StartupMessage::Error(message.to_string()));
    }

    anyhow::bail!("Unexpected daemon startup payload: {payload:?}");
}

#[cfg(windows)]
fn is_process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut exit_code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        ok != 0 && exit_code == STILL_ACTIVE as u32
    }
}

#[cfg(windows)]
fn terminate_process(pid: u32) -> Result<()> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        anyhow::ensure!(
            !handle.is_null(),
            "Failed to open process {}: access denied or not found",
            pid
        );
        let ok = TerminateProcess(handle, 1);
        CloseHandle(handle);
        anyhow::ensure!(ok != 0, "TerminateProcess failed for PID {}", pid);
    }
    Ok(())
}

#[cfg(windows)]
fn install_shutdown_handler() {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    unsafe {
        let ok = SetConsoleCtrlHandler(Some(ctrl_handler), 1);
        if ok == 0 {
            eprintln!("[devjournal daemon] warning: failed to install console ctrl handler");
        }
    }
}

#[cfg(windows)]
unsafe extern "system" fn ctrl_handler(ctrl_type: u32) -> i32 {
    use windows_sys::Win32::System::Console::{
        CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
    };
    SHOULD_STOP.store(true, Ordering::SeqCst);
    if ctrl_type == CTRL_CLOSE_EVENT
        || ctrl_type == CTRL_LOGOFF_EVENT
        || ctrl_type == CTRL_SHUTDOWN_EVENT
    {
        let _ = std::fs::remove_file(pid_path());
    }
    1
}

pub fn read_pid_public() -> Result<Option<u32>> {
    match read_pid()? {
        Some(pid) if is_process_alive(pid) => Ok(Some(pid)),
        _ => Ok(None),
    }
}

fn read_pid() -> Result<Option<u32>> {
    let path = pid_path();
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    let pid: u32 = content
        .trim()
        .parse()
        .with_context(|| format!("Invalid PID in {}", path.display()))?;
    Ok(Some(pid))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn signal_test_mutex() -> &'static Mutex<()> {
        static SIGNAL_TEST_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
        SIGNAL_TEST_MUTEX.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn install_shutdown_handler_ignores_sighup() {
        let _guard = signal_test_mutex().lock().unwrap();

        unsafe {
            let previous_hup = libc::signal(libc::SIGHUP, libc::SIG_DFL);
            install_shutdown_handler();
            let installed_hup = libc::signal(libc::SIGHUP, libc::SIG_DFL);
            libc::signal(libc::SIGHUP, previous_hup);

            assert_eq!(
                installed_hup,
                libc::SIG_IGN,
                "SIGHUP should be ignored so the daemon survives terminal/session closure"
            );
        }
    }

    #[test]
    fn handle_sigterm_sets_shutdown_flag() {
        SHOULD_STOP.store(false, Ordering::SeqCst);
        handle_sigterm(libc::SIGTERM);
        assert!(SHOULD_STOP.load(Ordering::SeqCst));
    }

    #[test]
    fn parse_startup_message_supports_ready_payloads() {
        assert_eq!(
            parse_startup_message("READY 4242\n").unwrap(),
            StartupMessage::Ready(4242)
        );
    }

    #[test]
    fn parse_startup_message_supports_error_payloads() {
        assert_eq!(
            parse_startup_message("ERROR failed to initialize\n").unwrap(),
            StartupMessage::Error("failed to initialize".to_string())
        );
    }

    #[test]
    fn resolve_author_reports_startup_errors() {
        let config = config::Config {
            general: config::GeneralConfig {
                poll_interval_secs: 60,
                author: None,
                retention_days: None,
            },
            llm: config::LlmConfig {
                provider: config::LlmProvider::Anthropic,
                api_key: None,
                model: None,
                base_url: None,
                system_prompt: None,
            },
            repos: vec![],
        };
        let mut reported = None;

        let err = resolve_author(&config, &mut |event| {
            if let StartupEvent::Error(message) = event {
                reported = Some(message.to_string());
            }
            Ok(())
        })
        .unwrap_err();

        assert_eq!(
            reported.as_deref(),
            Some("no author configured. Set `author` in [general] config.")
        );
        assert_eq!(
            err.to_string(),
            "no author configured. Set `author` in [general] config."
        );
    }

    #[test]
    fn status_returns_db_errors_instead_of_masking_them() {
        let err = status_with_db_opener(|| anyhow::bail!("migration failed")).unwrap_err();

        assert_eq!(err.to_string(), "migration failed");
    }

    #[test]
    fn run_poll_loop_returns_db_errors_instead_of_retrying() {
        SHOULD_STOP.store(false, Ordering::SeqCst);

        let err = run_poll_loop_with_db_opener(Duration::from_secs(1), "author", || {
            anyhow::bail!("migration failed")
        })
        .unwrap_err();

        assert!(err.to_string().contains("DB error"));
        assert!(format!("{err:#}").contains("migration failed"));
    }
}
