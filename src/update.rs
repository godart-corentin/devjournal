use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

const REPO: &str = "godart-corentin/dev-journal";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const CURRENT_TARGET: &str = env!("TARGET");

pub fn run_update() -> Result<()> {
    println!("Current version: v{CURRENT_VERSION} ({CURRENT_TARGET})");
    println!("Checking for updates...");

    let release: serde_json::Value = ureq::get(&format!(
        "https://api.github.com/repos/{REPO}/releases/latest"
    ))
    .set("User-Agent", "devjournal")
    .call()
    .context("Failed to check for updates")?
    .into_json()
    .context("Failed to parse release info")?;

    let tag = release["tag_name"]
        .as_str()
        .context("No tag_name in release")?;
    let latest = tag.strip_prefix('v').unwrap_or(tag);

    if latest == CURRENT_VERSION {
        println!("Already up to date.");
        return Ok(());
    }

    println!("New version available: v{latest}");

    // Find matching asset
    let assets = release["assets"]
        .as_array()
        .context("No assets in release")?;

    let asset = assets
        .iter()
        .find(|a| {
            a["name"]
                .as_str()
                .is_some_and(|n| n.contains(CURRENT_TARGET))
        })
        .context(format!(
            "No pre-built binary for {CURRENT_TARGET} in this release"
        ))?;

    let download_url = asset["browser_download_url"]
        .as_str()
        .context("No download URL for asset")?;

    println!("Downloading...");
    let response = ureq::get(download_url)
        .set("User-Agent", "devjournal")
        .call()
        .context("Failed to download update")?;

    let tmpdir = std::env::temp_dir().join("devjournal-update");
    let _ = std::fs::remove_dir_all(&tmpdir);
    std::fs::create_dir_all(&tmpdir)?;

    let archive_path = tmpdir.join("archive");
    let mut file = std::fs::File::create(&archive_path)?;
    std::io::copy(&mut response.into_reader(), &mut file)?;
    drop(file);

    // Extract
    let new_binary = extract(&archive_path, &tmpdir)?;

    // Replace current binary
    let current_exe = std::env::current_exe().context("Cannot determine current executable")?;
    let backup = current_exe.with_extension("old");

    std::fs::rename(&current_exe, &backup)
        .context("Failed to move current binary (try running with appropriate permissions)")?;

    if let Err(e) = std::fs::rename(&new_binary, &current_exe) {
        // Restore backup on failure
        let _ = std::fs::rename(&backup, &current_exe);
        return Err(e).context("Failed to install new binary");
    }

    let _ = std::fs::remove_file(&backup);
    let _ = std::fs::remove_dir_all(&tmpdir);

    println!("Updated devjournal from v{CURRENT_VERSION} to v{latest}");
    Ok(())
}

#[cfg(unix)]
fn extract(archive: &Path, dest: &Path) -> Result<PathBuf> {
    let status = std::process::Command::new("tar")
        .args([
            "xzf",
            &archive.to_string_lossy(),
            "-C",
            &dest.to_string_lossy(),
        ])
        .status()
        .context("Failed to run tar")?;

    if !status.success() {
        bail!("tar extraction failed");
    }

    let binary = dest.join("devjournal");
    if !binary.exists() {
        bail!("Extracted archive does not contain devjournal binary");
    }

    Ok(binary)
}

#[cfg(windows)]
fn extract(archive: &Path, dest: &Path) -> Result<PathBuf> {
    let status = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                archive.display(),
                dest.display()
            ),
        ])
        .status()
        .context("Failed to run PowerShell Expand-Archive")?;

    if !status.success() {
        bail!("zip extraction failed");
    }

    let binary = dest.join("devjournal.exe");
    if !binary.exists() {
        bail!("Extracted archive does not contain devjournal.exe");
    }

    Ok(binary)
}
