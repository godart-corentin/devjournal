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
    GeneralConfig {
        poll_interval_secs: 60,
        author: None,
    }
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
            "Config file not found at {}. Run `devjournal init` to get started.",
            path.display()
        );
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config at {}", path.display()))?;
    let config: Config = toml::from_str(&content).with_context(|| "Failed to parse config.toml")?;
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
    let canonical =
        std::fs::canonicalize(path).with_context(|| format!("Path does not exist: {}", path))?;
    let name = name.or_else(|| {
        canonical
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
    });
    let path_str = canonical.to_string_lossy().to_string();
    // Strip Windows extended-length path prefix (\\?\) which canonicalize adds on Windows
    #[cfg(windows)]
    let path_str = path_str
        .strip_prefix(r"\\?\")
        .unwrap_or(&path_str)
        .to_string();
    if config.repos.iter().any(|r| r.path == path_str) {
        println!("Repo already tracked: {}", path_str);
        return Ok(());
    }
    config.repos.push(RepoConfig {
        path: path_str.clone(),
        name,
    });
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

fn prompt_line(message: &str) -> String {
    use std::io::Write;
    print!("{}", message);
    std::io::stdout().flush().ok();
    let mut buf = String::new();
    std::io::stdin().read_line(&mut buf).ok();
    buf.trim().to_string()
}

pub fn build_config(
    author: Option<String>,
    provider: &str,
    api_key: Option<String>,
    model: &str,
    repo_path: Option<String>,
) -> Config {
    let repos = match repo_path {
        Some(path) => {
            let name = std::path::Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned());
            vec![RepoConfig { path, name }]
        }
        None => vec![],
    };
    Config {
        general: GeneralConfig {
            poll_interval_secs: 60,
            author,
        },
        llm: LlmConfig {
            provider: provider.to_string(),
            api_key,
            model: Some(model.to_string()),
            base_url: None,
        },
        repos,
    }
}

pub fn init() -> Result<()> {
    let path = config_path();
    if path.exists() {
        println!("Config already exists at {}", path.display());
        println!("Edit that file to update your settings.");
        return Ok(());
    }

    println!("Welcome to devjournal! Let's set up your configuration.\n");

    // Author
    let git_name = git2::Repository::open(".")
        .ok()
        .and_then(|r| r.config().ok())
        .and_then(|c| c.get_string("user.name").ok());
    let author_prompt = match &git_name {
        Some(name) => format!("Author [{}]: ", name),
        None => "Author (leave blank to skip): ".to_string(),
    };
    let author_input = prompt_line(&author_prompt);
    let author = if author_input.is_empty() {
        git_name
    } else {
        Some(author_input)
    };

    // LLM provider
    println!("\nLLM provider:");
    println!("  1) claude (default)");
    println!("  2) openai");
    println!("  3) ollama");
    let provider_input = prompt_line("Choose [1]: ");
    let provider = match provider_input.as_str() {
        "2" => "openai",
        "3" => "ollama",
        _ => "claude",
    };

    // API key (not for ollama)
    let api_key = if provider != "ollama" {
        loop {
            let key = prompt_line("API key: ");
            if !key.is_empty() {
                break Some(key);
            }
            println!("API key is required for {}.", provider);
        }
    } else {
        None
    };

    // Model
    let model = match provider {
        "claude" => {
            println!("\nModel:");
            println!("  1) claude-sonnet-4-6 (default)");
            println!("  2) claude-opus-4-6");
            println!("  3) claude-haiku-4-5");
            let m = prompt_line("Choose [1]: ");
            match m.as_str() {
                "2" => "claude-opus-4-6",
                "3" => "claude-haiku-4-5",
                _ => "claude-sonnet-4-6",
            }
        }
        "openai" => {
            println!("\nModel:");
            println!("  1) gpt-4o (default)");
            println!("  2) gpt-5.4");
            let m = prompt_line("Choose [1]: ");
            match m.as_str() {
                "2" => "gpt-5.4",
                _ => "gpt-4o",
            }
        }
        _ => {
            // ollama
            println!("\nModel set to llama3.2 (default for ollama).");
            "llama3.2"
        }
    };

    // Current dir as repo?
    let repo_path = if git2::Repository::open(".").is_ok() {
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let answer = prompt_line(&format!("\nAdd {} to watched repos? [Y/n]: ", cwd));
        if answer.is_empty() || answer.eq_ignore_ascii_case("y") {
            Some(cwd)
        } else {
            None
        }
    } else {
        None
    };

    let config = build_config(author, provider, api_key, model, repo_path);
    save(&config)?;

    println!("\nConfig written to {}", path.display());
    println!("You can edit that file directly to update your settings.");
    println!("\nRun `devjournal daemon start` to begin tracking.");

    Ok(())
}

pub fn api_key(config: &LlmConfig) -> Option<String> {
    std::env::var("DEVJOURNAL_API_KEY")
        .ok()
        .or_else(|| config.api_key.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_build_config_claude() {
        let config = build_config(
            Some("Alice".to_string()),
            "claude",
            Some("sk-ant-test".to_string()),
            "claude-sonnet-4-6",
            None,
        );
        assert_eq!(config.general.author, Some("Alice".to_string()));
        assert_eq!(config.llm.provider, "claude");
        assert_eq!(config.llm.api_key, Some("sk-ant-test".to_string()));
        assert_eq!(config.llm.model, Some("claude-sonnet-4-6".to_string()));
        assert!(config.repos.is_empty());
    }

    #[test]
    fn test_build_config_ollama_with_repo() {
        let config = build_config(
            None,
            "ollama",
            None,
            "llama3.2",
            Some("/tmp/repo".to_string()),
        );
        assert_eq!(config.llm.provider, "ollama");
        assert_eq!(config.llm.api_key, None);
        assert_eq!(config.llm.model, Some("llama3.2".to_string()));
        assert_eq!(config.repos.len(), 1);
        assert_eq!(config.repos[0].path, "/tmp/repo");
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
