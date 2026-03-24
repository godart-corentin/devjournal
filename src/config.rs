use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(default = "default_general")]
    pub general: GeneralConfig,
    pub llm: LlmConfig,
    #[serde(default)]
    pub repos: Vec<RepoConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeneralConfig {
    pub poll_interval_secs: u64,
    pub author: Option<String>,
}

fn default_general() -> GeneralConfig {
    GeneralConfig { poll_interval_secs: 60, author: None }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LlmConfig {
    pub provider: String,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoConfig {
    pub path: String,
    pub name: Option<String>,
}

impl RepoConfig {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.path)
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("devjournal")
        .join("config.toml")
}

pub fn load() -> Result<Config> {
    let path = config_path();
    if !path.exists() {
        anyhow::bail!(
            "Config file not found at {}. Run `devjournal add <repo-path>` to create it.",
            path.display()
        );
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config at {}", path.display()))?;
    let config: Config = toml::from_str(&content)
        .with_context(|| "Failed to parse config.toml")?;
    Ok(config)
}

pub fn load_or_default() -> Config {
    load().unwrap_or_else(|_| Config {
        general: default_general(),
        llm: LlmConfig {
            provider: "claude".to_string(),
            api_key: None,
            model: None,
            base_url: None,
        },
        repos: vec![],
    })
}

pub fn save(config: &Config) -> Result<()> {
    let path = config_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    let content = toml::to_string_pretty(config)?;
    std::fs::write(&path, content)?;
    Ok(())
}

pub fn add_repo(path: &str, name: Option<String>) -> Result<()> {
    let mut config = load_or_default();
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("Path does not exist: {}", path))?;
    let path_str = canonical.to_string_lossy().to_string();
    if config.repos.iter().any(|r| r.path == path_str) {
        println!("Repo already tracked: {}", path_str);
        return Ok(());
    }
    config.repos.push(RepoConfig { path: path_str.clone(), name });
    save(&config)?;
    println!("Now tracking: {}", path_str);
    Ok(())
}

pub fn remove_repo(path: &str) -> Result<()> {
    let mut config = load()?;
    let before = config.repos.len();
    config.repos.retain(|r| r.path != path);
    if config.repos.len() == before {
        anyhow::bail!("Repo not found in config: {}", path);
    }
    save(&config)?;
    println!("Removed: {}", path);
    Ok(())
}

pub fn api_key(config: &LlmConfig) -> Option<String> {
    std::env::var("DEVJOURNAL_API_KEY").ok()
        .or_else(|| config.api_key.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn with_config_dir<F: FnOnce(PathBuf)>(f: F) {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("devjournal");
        fs::create_dir_all(&config_dir).unwrap();
        f(config_dir);
    }

    #[test]
    fn test_parse_valid_config() {
        let toml = r#"
[llm]
provider = "claude"
api_key = "sk-test"
model = "claude-sonnet-4-6"

[[repos]]
path = "/tmp/repo1"
name = "my-repo"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.llm.provider, "claude");
        assert_eq!(config.repos.len(), 1);
        assert_eq!(config.repos[0].display_name(), "my-repo");
        assert_eq!(config.general.poll_interval_secs, 60); // default
    }

    #[test]
    fn test_repo_display_name_fallback() {
        let repo = RepoConfig {
            path: "/tmp/my-project".to_string(),
            name: None,
        };
        assert_eq!(repo.display_name(), "/tmp/my-project");
    }

    #[test]
    fn test_api_key_env_override() {
        std::env::set_var("DEVJOURNAL_API_KEY", "env-key");
        let llm = LlmConfig {
            provider: "claude".to_string(),
            api_key: Some("config-key".to_string()),
            model: None,
            base_url: None,
        };
        assert_eq!(api_key(&llm).as_deref(), Some("env-key"));
        std::env::remove_var("DEVJOURNAL_API_KEY");
    }
}
