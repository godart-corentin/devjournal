use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};

const REPO: &str = "godart-corentin/dev-journal";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const CURRENT_TARGET: &str = env!("TARGET");
const CHECKSUMS_ASSET: &str = "devjournal-checksums.txt";
const RELEASE_URL_ENV: &str = "DEVJOURNAL_UPDATE_RELEASE_URL";
const ASSET_BASE_URL_ENV: &str = "DEVJOURNAL_UPDATE_ASSET_BASE_URL";

pub fn run_update() -> Result<()> {
    #[cfg(windows)]
    {
        println!("{}", manual_windows_update_message());
        return Ok(());
    }

    #[cfg(unix)]
    {
        run_update_unix()
    }
}

#[cfg(unix)]
fn run_update_unix() -> Result<()> {
    let sources = UpdateSources::from_env();

    println!("Current version: v{CURRENT_VERSION} ({CURRENT_TARGET})");
    println!("Checking for updates...");

    let release = fetch_latest_release(&sources.release_url)?;

    let tag = release["tag_name"]
        .as_str()
        .context("No tag_name in release")?;
    let latest = tag.strip_prefix('v').unwrap_or(tag);

    if latest == CURRENT_VERSION {
        println!("Already up to date.");
        return Ok(());
    }

    println!("New version available: v{latest}");

    let assets = release["assets"]
        .as_array()
        .context("No assets in release")?;
    let asset = find_asset_for_target(assets, CURRENT_TARGET)?;
    let asset_name = asset_name(asset)?;
    let download_url = sources.asset_download_url(asset)?;
    let checksums_asset = find_checksums_asset(assets)?;
    let checksums_url = sources.asset_download_url(checksums_asset)?;

    println!("Downloading...");

    let tmpdir = std::env::temp_dir().join("devjournal-update");
    let _ = std::fs::remove_dir_all(&tmpdir);
    std::fs::create_dir_all(&tmpdir)?;

    let archive_path = tmpdir.join(asset_name);
    download_to_file(&download_url, &archive_path).context("Failed to download update")?;
    let checksums =
        download_text(&checksums_url).context("Failed to download checksum manifest")?;
    let expected_checksum = expected_checksum_for_asset(asset_name, &checksums)?;
    verify_archive_checksum(&archive_path, &expected_checksum)?;

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

fn fetch_latest_release(url: &str) -> Result<serde_json::Value> {
    ureq::get(url)
        .set("User-Agent", "devjournal")
        .call()
        .context("Failed to check for updates")?
        .into_json()
        .context("Failed to parse release info")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdateSources {
    release_url: String,
    asset_base_url: Option<String>,
}

impl UpdateSources {
    fn from_env() -> Self {
        Self {
            release_url: env::var(RELEASE_URL_ENV).unwrap_or_else(|_| default_release_url()),
            asset_base_url: env::var(ASSET_BASE_URL_ENV).ok(),
        }
    }

    fn asset_download_url(&self, asset: &serde_json::Value) -> Result<String> {
        let name = asset_name(asset)?;

        if let Some(base_url) = &self.asset_base_url {
            return Ok(join_url(base_url, name));
        }

        asset["browser_download_url"]
            .as_str()
            .map(str::to_string)
            .context("No download URL for asset")
    }
}

fn default_release_url() -> String {
    format!("https://api.github.com/repos/{REPO}/releases/latest")
}

fn asset_name(asset: &serde_json::Value) -> Result<&str> {
    asset["name"]
        .as_str()
        .context("No asset name for release binary")
}

fn join_url(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), path)
}

fn find_asset_for_target<'a>(
    assets: &'a [serde_json::Value],
    target: &str,
) -> Result<&'a serde_json::Value> {
    let tarball_name = format!("devjournal-{target}.tar.gz");
    let zip_name = format!("devjournal-{target}.zip");

    assets
        .iter()
        .find(|a| {
            a["name"]
                .as_str()
                .is_some_and(|n| n == tarball_name || n == zip_name)
        })
        .context(format!("No pre-built binary for {target} in this release"))
}

fn find_checksums_asset(assets: &[serde_json::Value]) -> Result<&serde_json::Value> {
    assets
        .iter()
        .find(|a| a["name"].as_str() == Some(CHECKSUMS_ASSET))
        .context("No checksum manifest found in this release")
}

fn download_text(url: &str) -> Result<String> {
    let response = ureq::get(url).set("User-Agent", "devjournal").call()?;
    let mut text = String::new();
    response.into_reader().read_to_string(&mut text)?;
    Ok(text)
}

fn download_to_file(url: &str, path: &Path) -> Result<()> {
    let response = ureq::get(url).set("User-Agent", "devjournal").call()?;
    let mut file = std::fs::File::create(path)?;
    std::io::copy(&mut response.into_reader(), &mut file)?;
    Ok(())
}

fn parse_checksum_manifest(manifest: &str) -> Result<HashMap<String, String>> {
    let mut checksums = HashMap::new();

    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        let checksum = parts
            .next()
            .context("Malformed checksum manifest: missing digest")?;
        let file_name = parts
            .next()
            .context("Malformed checksum manifest: missing asset name")?
            .trim_start_matches('*');

        if parts.next().is_some() {
            bail!("Malformed checksum manifest: unexpected extra fields");
        }

        checksums.insert(file_name.to_string(), checksum.to_string());
    }

    if checksums.is_empty() {
        bail!("Checksum manifest is empty");
    }

    Ok(checksums)
}

fn expected_checksum_for_asset(asset_name: &str, manifest: &str) -> Result<String> {
    let checksums = parse_checksum_manifest(manifest)?;
    checksums
        .get(asset_name)
        .cloned()
        .context(format!("No checksum found for asset {asset_name}"))
}

fn verify_archive_checksum(path: &Path, expected: &str) -> Result<()> {
    let actual = sha256_file(path)?;

    if actual != expected.to_ascii_lowercase() {
        bail!(
            "Checksum verification failed for {}: expected {}, got {}",
            path.display(),
            expected,
            actual
        );
    }

    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg_attr(not(any(test, windows)), allow(dead_code))]
fn manual_windows_update_message() -> &'static str {
    "Manual update required on Windows: please reinstall devjournal from the latest release. Self-update is temporarily disabled until the Windows replacement flow is hardened."
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

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    fn non_current_target() -> &'static str {
        match CURRENT_TARGET {
            "x86_64-unknown-linux-gnu" => "aarch64-apple-darwin",
            _ => "x86_64-unknown-linux-gnu",
        }
    }

    #[test]
    fn finds_asset_for_current_target() {
        let other_target = non_current_target();
        let release = serde_json::json!({
            "assets": [
                {
                    "name": format!("devjournal-{other_target}.tar.gz"),
                    "browser_download_url": "https://example.invalid/linux"
                },
                {
                    "name": format!("devjournal-{CURRENT_TARGET}.tar.gz"),
                    "browser_download_url": "https://example.invalid/current"
                }
            ]
        });

        let asset =
            find_asset_for_target(release["assets"].as_array().unwrap(), CURRENT_TARGET).unwrap();

        assert_eq!(
            asset["browser_download_url"].as_str(),
            Some("https://example.invalid/current")
        );
    }

    #[test]
    fn finds_asset_by_exact_target_name_not_substring() {
        let release = serde_json::json!({
            "assets": [
                {
                    "name": "devjournal-x86_64-unknown-linux-gnu.tar.gz",
                    "browser_download_url": "https://example.invalid/linux"
                },
                {
                    "name": "devjournal-unknown-linux-gnu.tar.gz",
                    "browser_download_url": "https://example.invalid/current"
                }
            ]
        });

        let asset =
            find_asset_for_target(release["assets"].as_array().unwrap(), "unknown-linux-gnu")
                .unwrap();

        assert_eq!(
            asset["browser_download_url"].as_str(),
            Some("https://example.invalid/current")
        );
    }

    #[test]
    fn update_sources_default_to_github_release_endpoint() {
        let sources = UpdateSources {
            release_url: default_release_url(),
            asset_base_url: None,
        };

        assert_eq!(
            sources.release_url,
            "https://api.github.com/repos/godart-corentin/dev-journal/releases/latest"
        );
    }

    #[test]
    fn asset_download_url_uses_override_base_when_present() {
        let sources = UpdateSources {
            release_url: default_release_url(),
            asset_base_url: Some("http://127.0.0.1:8765/releases".to_string()),
        };
        let asset = serde_json::json!({
            "name": "devjournal-x86_64-unknown-linux-gnu.tar.gz",
            "browser_download_url": "https://example.invalid/download"
        });

        let download_url = sources.asset_download_url(&asset).unwrap();

        assert_eq!(
            download_url,
            "http://127.0.0.1:8765/releases/devjournal-x86_64-unknown-linux-gnu.tar.gz"
        );
    }

    #[test]
    fn asset_download_url_falls_back_to_asset_metadata() {
        let sources = UpdateSources {
            release_url: default_release_url(),
            asset_base_url: None,
        };
        let asset = serde_json::json!({
            "name": "devjournal-x86_64-unknown-linux-gnu.tar.gz",
            "browser_download_url": "https://example.invalid/download"
        });

        let download_url = sources.asset_download_url(&asset).unwrap();

        assert_eq!(download_url, "https://example.invalid/download");
    }

    #[test]
    fn parses_checksum_manifest_line() {
        let checksums = parse_checksum_manifest(
            "abc123  devjournal-x86_64-unknown-linux-gnu.tar.gz\n\
             def456 *devjournal-aarch64-apple-darwin.tar.gz\n",
        )
        .unwrap();

        assert_eq!(
            checksums.get("devjournal-x86_64-unknown-linux-gnu.tar.gz"),
            Some(&"abc123".to_string())
        );
        assert_eq!(
            checksums.get("devjournal-aarch64-apple-darwin.tar.gz"),
            Some(&"def456".to_string())
        );
    }

    #[test]
    fn verifies_matching_digest() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("archive.tar.gz");
        std::fs::write(&archive, b"verified-by-sha").unwrap();

        let expected = format!("{:x}", Sha256::digest(b"verified-by-sha"));

        verify_archive_checksum(&archive, &expected).unwrap();
    }

    #[test]
    fn fails_when_checksum_entry_missing() {
        let err = expected_checksum_for_asset(
            "devjournal-x86_64-unknown-linux-gnu.tar.gz",
            "abc123  devjournal-aarch64-apple-darwin.tar.gz\n",
        )
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("No checksum found for asset devjournal-x86_64-unknown-linux-gnu.tar.gz"));
    }

    #[test]
    fn fails_when_digest_mismatches() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("archive.tar.gz");
        std::fs::write(&archive, b"tampered").unwrap();

        let err = verify_archive_checksum(&archive, "deadbeef").unwrap_err();

        assert!(err.to_string().contains("Checksum verification failed"));
    }

    #[test]
    fn windows_update_requires_manual_reinstall_message() {
        assert!(manual_windows_update_message().contains("Manual update required"));
    }

    #[test]
    fn reads_expected_checksum_for_asset() {
        let checksum = expected_checksum_for_asset(
            "devjournal-x86_64-unknown-linux-gnu.tar.gz",
            "abc123  devjournal-x86_64-unknown-linux-gnu.tar.gz\n",
        )
        .unwrap();

        assert_eq!(checksum, "abc123");
    }
}
