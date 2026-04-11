use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    pub retention_days: Option<u32>,
}

fn default_general() -> GeneralConfig {
    GeneralConfig {
        poll_interval_secs: 60,
        author: None,
        retention_days: None,
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LlmConfig {
    pub provider: String,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub system_prompt: Option<String>,
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

#[cfg(test)]
fn test_config_path_override() -> &'static std::sync::Mutex<Option<PathBuf>> {
    use std::sync::{Mutex, OnceLock};

    static CONFIG_PATH_OVERRIDE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    CONFIG_PATH_OVERRIDE.get_or_init(|| Mutex::new(None))
}

fn repo_display_name_in_use(repos: &[RepoConfig], candidate: &str) -> bool {
    repos.iter().any(|repo| repo.display_name() == candidate)
}

fn unique_repo_display_name(repos: &[RepoConfig], base: &str) -> String {
    let mut suffix = 1u32;
    let mut candidate = base.to_string();

    while repo_display_name_in_use(repos, &candidate) {
        suffix += 1;
        candidate = format!("{}-{}", base, suffix);
    }

    candidate
}

pub fn resolve_repo<'a>(repos: &'a [RepoConfig], query: &str) -> Result<&'a RepoConfig> {
    if let Some(repo) = repos.iter().find(|repo| repo.path == query) {
        return Ok(repo);
    }

    let matches: Vec<&RepoConfig> = repos
        .iter()
        .filter(|repo| repo.display_name() == query)
        .collect();

    match matches.len() {
        1 => Ok(matches[0]),
        0 => {
            if Path::new(query).exists() {
                let canonical = std::fs::canonicalize(query)
                    .with_context(|| format!("Failed to resolve repo path: {}", query))?;
                let canonical = canonical.to_string_lossy().to_string();
                let canonical = canonical.strip_prefix(r"\\?\").unwrap_or(&canonical);
                if let Some(repo) = repos.iter().find(|repo| repo.path == canonical) {
                    return Ok(repo);
                }
            }
            anyhow::bail!("Repo '{}' not found in config", query)
        }
        _ => anyhow::bail!(
            "Repo name '{}' is ambiguous. Use the exact path shown by `devjournal list`.",
            query
        ),
    }
}

pub fn config_path() -> PathBuf {
    #[cfg(test)]
    {
        if let Some(path) = test_config_path_override().lock().unwrap().clone() {
            return path;
        }
    }
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
            system_prompt: None,
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

    let name = name
        .and_then(|name| {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .or_else(|| {
            canonical
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .map(|base| unique_repo_display_name(&config.repos, &base));

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
    let repo = resolve_repo(&config.repos, path)?;
    let removed_path = repo.path.clone();
    let removed_name = repo.display_name().to_string();
    config.repos.retain(|r| r.path != removed_path);
    save(&config)?;
    println!("Removed: {}", removed_name);
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
            retention_days: None,
        },
        llm: LlmConfig {
            provider: provider.to_string(),
            api_key,
            model: Some(model.to_string()),
            base_url: None,
            system_prompt: None,
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
    let git_name = git2::Config::open_default()
        .ok()
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
    println!("  4) cursor");
    let provider_input = prompt_line("Choose [1]: ");
    let provider = match provider_input.as_str() {
        "2" => "openai",
        "3" => "ollama",
        "4" => "cursor",
        _ => "claude",
    };

    // API key (not for ollama or cursor)
    let api_key = if provider != "ollama" && provider != "cursor" {
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
        "cursor" => {
            println!("\nModel set to gpt-5.4-mini (default for cursor).");
            "gpt-5.4-mini"
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
    let sem_probe = crate::sem::probe();
    println!(
        "Semantic enrichment: {} ({})",
        sem_probe.status.label(),
        sem_probe.detail
    );
    if sem_probe.status != crate::sem::SemIntegrationStatus::Active {
        println!("Install hint: {}", sem_probe.install_hint);
    }
    println!("\nRun `devjournal start` to begin tracking.");

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
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;

    fn env_var_test_mutex() -> &'static Mutex<()> {
        static ENV_VAR_TEST_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_VAR_TEST_MUTEX.get_or_init(|| Mutex::new(()))
    }

    fn config_home_test_mutex() -> &'static Mutex<()> {
        static CONFIG_HOME_TEST_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
        CONFIG_HOME_TEST_MUTEX.get_or_init(|| Mutex::new(()))
    }

    fn cwd_test_mutex() -> &'static Mutex<()> {
        static CWD_TEST_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
        CWD_TEST_MUTEX.get_or_init(|| Mutex::new(()))
    }

    struct ConfigPathGuard {
        previous: Option<PathBuf>,
    }

    impl Drop for ConfigPathGuard {
        fn drop(&mut self) {
            *test_config_path_override().lock().unwrap() = self.previous.take();
        }
    }

    struct CwdGuard {
        previous: PathBuf,
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.previous).unwrap();
        }
    }

    fn set_temp_config_path() -> (tempfile::TempDir, ConfigPathGuard) {
        let dir = tempdir().unwrap();
        let override_path = dir.path().join("devjournal").join("config.toml");
        let previous = test_config_path_override()
            .lock()
            .unwrap()
            .replace(override_path);
        (dir, ConfigPathGuard { previous })
    }

    fn set_temp_current_dir(path: &Path) -> CwdGuard {
        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(path).unwrap();
        CwdGuard { previous }
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
    fn test_build_config_cursor_no_api_key() {
        let config = build_config(
            Some("Dev".to_string()),
            "cursor",
            None,
            "gpt-5.4-mini",
            None,
        );
        assert_eq!(config.llm.provider, "cursor");
        assert_eq!(config.llm.api_key, None);
        assert_eq!(config.llm.model, Some("gpt-5.4-mini".to_string()));
    }

    #[test]
    fn test_api_key_env_override() {
        let _guard = env_var_test_mutex().lock().unwrap();
        let previous = std::env::var("DEVJOURNAL_API_KEY").ok();

        std::env::set_var("DEVJOURNAL_API_KEY", "env-key");
        let llm = LlmConfig {
            provider: "claude".to_string(),
            api_key: Some("config-key".to_string()),
            model: None,
            base_url: None,
            system_prompt: None,
        };
        assert_eq!(api_key(&llm).as_deref(), Some("env-key"));

        match previous {
            Some(value) => std::env::set_var("DEVJOURNAL_API_KEY", value),
            None => std::env::remove_var("DEVJOURNAL_API_KEY"),
        }
    }

    #[test]
    fn test_add_repo_suffixes_duplicate_display_names() {
        let _guard = config_home_test_mutex().lock().unwrap();
        let (_dir, _config_path_guard) = set_temp_config_path();

        let root = tempdir().unwrap();
        let repo_a = root.path().join("one/project");
        let repo_b = root.path().join("two/project");
        let repo_c = root.path().join("three/project");
        std::fs::create_dir_all(&repo_a).unwrap();
        std::fs::create_dir_all(&repo_b).unwrap();
        std::fs::create_dir_all(&repo_c).unwrap();

        add_repo(repo_a.to_str().unwrap(), None).unwrap();
        add_repo(repo_b.to_str().unwrap(), None).unwrap();
        add_repo(repo_c.to_str().unwrap(), None).unwrap();

        let config = load().unwrap();
        let names: Vec<_> = config
            .repos
            .iter()
            .map(|repo| repo.display_name().to_string())
            .collect();

        assert_eq!(names, vec!["project", "project-2", "project-3"]);
    }

    #[test]
    fn test_resolve_repo_by_path_name_and_missing() {
        let repos = vec![
            RepoConfig {
                path: "/repo/a".to_string(),
                name: Some("alpha".to_string()),
            },
            RepoConfig {
                path: "/repo/b".to_string(),
                name: Some("beta".to_string()),
            },
        ];

        assert_eq!(resolve_repo(&repos, "/repo/a").unwrap().path, "/repo/a");
        assert_eq!(resolve_repo(&repos, "beta").unwrap().path, "/repo/b");

        let err = resolve_repo(&repos, "missing").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_resolve_repo_rejects_ambiguous_display_name() {
        let repos = vec![
            RepoConfig {
                path: "/repo/a".to_string(),
                name: Some("dup".to_string()),
            },
            RepoConfig {
                path: "/repo/b".to_string(),
                name: Some("dup".to_string()),
            },
        ];

        let err = resolve_repo(&repos, "dup").unwrap_err();
        assert!(err.to_string().contains("ambiguous"));
    }

    #[test]
    fn test_remove_repo_by_display_name() {
        let _guard = config_home_test_mutex().lock().unwrap();
        let (_dir, _config_path_guard) = set_temp_config_path();

        let root = tempdir().unwrap();
        let repo_a = root.path().join("one/project");
        let repo_b = root.path().join("two/project");
        std::fs::create_dir_all(&repo_a).unwrap();
        std::fs::create_dir_all(&repo_b).unwrap();

        add_repo(repo_a.to_str().unwrap(), None).unwrap();
        add_repo(repo_b.to_str().unwrap(), None).unwrap();

        remove_repo("project-2").unwrap();

        let config = load().unwrap();
        assert_eq!(config.repos.len(), 1);
        assert_eq!(
            config.repos[0].path,
            std::fs::canonicalize(&repo_a)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        );
    }

    #[test]
    fn test_remove_repo_from_current_directory() {
        let _config_guard = config_home_test_mutex().lock().unwrap();
        let _cwd_guard = cwd_test_mutex().lock().unwrap();
        let (_dir, _config_path_guard) = set_temp_config_path();

        let repo = tempdir().unwrap();
        let _cwd = set_temp_current_dir(repo.path());

        add_repo(".", None).unwrap();
        remove_repo(".").unwrap();

        let config = load().unwrap();
        assert!(config.repos.is_empty());
    }
}
