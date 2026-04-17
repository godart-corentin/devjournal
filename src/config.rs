use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Config {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub repos: Vec<RepoConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeneralConfig {
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub retention_days: Option<u32>,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: default_poll_interval_secs(),
            author: None,
            retention_days: None,
        }
    }
}

const fn default_poll_interval_secs() -> u64 {
    60
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct LlmConfig {
    #[serde(default)]
    pub provider: LlmProvider,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    #[default]
    Anthropic,
    OpenAi,
    Ollama,
}

impl LlmProvider {
    pub const ALL: [Self; 3] = [Self::Anthropic, Self::OpenAi, Self::Ollama];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::Ollama => "ollama",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic",
            Self::OpenAi => "OpenAI",
            Self::Ollama => "Local model (Ollama)",
        }
    }

    pub const fn default_model(self) -> &'static str {
        match self {
            Self::Anthropic => "claude-sonnet-4-6",
            Self::OpenAi => "gpt-4o-mini",
            Self::Ollama => "llama3.2",
        }
    }

    pub const fn requires_api_key(self) -> bool {
        !matches!(self, Self::Ollama)
    }

    pub const fn suggested_models(self) -> &'static [&'static str] {
        match self {
            Self::Anthropic => &["claude-sonnet-4-6", "claude-opus-4-6", "claude-haiku-4-5"],
            Self::OpenAi => &["gpt-4o-mini"],
            Self::Ollama => &["llama3.2"],
        }
    }
}

impl std::fmt::Display for LlmProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoConfig {
    pub path: String,
    #[serde(default)]
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

fn pathbuf_to_string(path: &Path) -> Cow<'_, str> {
    let raw = path.to_string_lossy();

    #[cfg(windows)]
    {
        if let Some(stripped) = raw.strip_prefix(r"\\?\") {
            return Cow::Owned(stripped.to_string());
        }
    }

    raw
}

fn normalize_path(path: &Path) -> Result<String> {
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("Path does not exist: {}", path.display()))?;
    Ok(pathbuf_to_string(&canonical).into_owned())
}

fn repo_display_name_in_use(repos: &[RepoConfig], candidate: &str) -> bool {
    repos.iter().any(|repo| repo.display_name() == candidate)
}

fn unique_repo_display_name(repos: &[RepoConfig], base: &str) -> String {
    if !repo_display_name_in_use(repos, base) {
        return base.to_string();
    }

    for suffix in 2u32.. {
        let candidate = format!("{base}-{suffix}");
        if !repo_display_name_in_use(repos, &candidate) {
            return candidate;
        }
    }

    unreachable!("u32 suffix space exhausted")
}

pub fn resolve_repo<'a>(repos: &'a [RepoConfig], query: &str) -> Result<&'a RepoConfig> {
    if let Some(repo) = repos.iter().find(|repo| repo.path == query) {
        return Ok(repo);
    }

    let matches: Vec<_> = repos
        .iter()
        .filter(|repo| repo.display_name() == query)
        .collect();

    match matches.as_slice() {
        [repo] => Ok(*repo),
        [] => {
            if Path::new(query).exists() {
                let canonical = normalize_path(Path::new(query))
                    .with_context(|| format!("Failed to resolve repo path: {query}"))?;
                if let Some(repo) = repos.iter().find(|repo| repo.path == canonical) {
                    return Ok(repo);
                }
            }
            bail!("Repo '{query}' not found in config");
        }
        _ => bail!(
            "Repo name '{query}' is ambiguous. Use the exact path shown by `devjournal list`."
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
        bail!(
            "Config file not found at {}. Run `devjournal add <path>` to get started, or `devjournal init` for guided setup.",
            path.display()
        );
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config at {}", path.display()))?;

    toml::from_str(&content).with_context(|| format!("Failed to parse TOML in {}", path.display()))
}

pub fn load_or_default() -> Config {
    load().unwrap_or_default()
}

pub fn save(config: &Config) -> Result<()> {
    let path = config_path();
    let parent = path
        .parent()
        .context("Config path has no parent directory")?;

    std::fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create config directory {}", parent.display()))?;

    let content = toml::to_string_pretty(config).context("Failed to serialize config to TOML")?;

    std::fs::write(&path, content)
        .with_context(|| format!("Failed to write config to {}", path.display()))?;

    Ok(())
}

pub fn add_repo(path: &str, name: Option<String>) -> Result<()> {
    let mut config = load_or_default();
    let path_str = normalize_path(Path::new(path))?;

    if config.repos.iter().any(|r| r.path == path_str) {
        println!("Repo already tracked: {path_str}");
        return Ok(());
    }

    let default_name = Path::new(&path_str)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned());

    let name = name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or(default_name)
        .map(|base| unique_repo_display_name(&config.repos, &base));

    config.repos.push(RepoConfig {
        path: path_str.clone(),
        name,
    });

    save(&config)?;
    println!("Now tracking: {path_str}");
    Ok(())
}

pub fn remove_repo(path: &str) -> Result<()> {
    let mut config = load()?;
    let repo = resolve_repo(&config.repos, path)?;
    let removed_path = repo.path.clone();
    let removed_name = repo.display_name().to_string();

    config.repos.retain(|r| r.path != removed_path);
    save(&config)?;

    println!("Removed: {removed_name}");
    Ok(())
}

fn prompt_line(message: &str) -> Result<String> {
    eprint!("{message}");
    io::stderr()
        .flush()
        .context("Failed to flush prompt to stderr")?;

    let mut buf = String::new();
    let read = io::stdin()
        .read_line(&mut buf)
        .context("Failed to read input from stdin")?;

    if read == 0 {
        bail!("Interactive setup requires a terminal with readable stdin.");
    }

    Ok(buf.trim().to_string())
}

fn prompt_for_provider() -> Result<LlmProvider> {
    eprintln!("Choose a provider:");
    for (i, provider) in LlmProvider::ALL.iter().enumerate() {
        eprintln!("{}. {}", i + 1, provider.label());
    }

    loop {
        let input = prompt_line("> ")?;

        if input.is_empty() {
            return Ok(LlmProvider::default());
        }

        if let Ok(index) = input.parse::<usize>() {
            if let Some(provider) = LlmProvider::ALL.get(index.saturating_sub(1)) {
                return Ok(*provider);
            }
        }

        eprintln!("Please choose a valid number.");
    }
}

fn prompt_for_api_key(provider: LlmProvider) -> Result<Option<String>> {
    if !provider.requires_api_key() {
        return Ok(None);
    }

    eprintln!();
    loop {
        let key = prompt_line("Enter your API key:\n> ")?;
        if !key.is_empty() {
            return Ok(Some(key));
        }
        eprintln!("API key is required for this provider.");
    }
}

fn prompt_for_model(provider: LlmProvider) -> Result<String> {
    match provider {
        LlmProvider::Anthropic => {
            let models = provider.suggested_models();

            eprintln!();
            eprintln!("Select model:");
            for (i, model) in models.iter().enumerate() {
                if i == 0 {
                    eprintln!("{}. {} (default)", i + 1, model);
                } else {
                    eprintln!("{}. {}", i + 1, model);
                }
            }

            loop {
                let input = prompt_line("> ")?;

                if input.is_empty() {
                    return Ok(models[0].to_string());
                }

                if let Ok(index) = input.parse::<usize>() {
                    if let Some(model) = models.get(index.saturating_sub(1)) {
                        return Ok((*model).to_string());
                    }
                }

                eprintln!("Please choose a valid number.");
            }
        }
        LlmProvider::OpenAi | LlmProvider::Ollama => {
            let default_model = provider.default_model();
            eprintln!();
            let entered = prompt_line(&format!("Select model [{default_model}]:\n> "))?;
            if entered.is_empty() {
                Ok(default_model.to_string())
            } else {
                Ok(entered)
            }
        }
    }
}

fn prompt_for_llm_config(current: &LlmConfig) -> Result<LlmConfig> {
    let provider = prompt_for_provider()?;
    let api_key = prompt_for_api_key(provider)?;
    let model = prompt_for_model(provider)?;

    Ok(LlmConfig {
        provider,
        api_key,
        model: Some(model),
        base_url: current.base_url.clone(),
        system_prompt: current.system_prompt.clone(),
    })
}

pub fn build_config(
    author: Option<String>,
    provider: LlmProvider,
    api_key: Option<String>,
    model: &str,
    repo_path: Option<String>,
) -> Config {
    let repos = repo_path
        .map(|path| {
            let name = Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned());

            vec![RepoConfig { path, name }]
        })
        .unwrap_or_default();

    Config {
        general: GeneralConfig {
            poll_interval_secs: default_poll_interval_secs(),
            author,
            retention_days: None,
        },
        llm: LlmConfig {
            provider,
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

    let git_name = git2::Config::open_default()
        .ok()
        .and_then(|c| c.get_string("user.name").ok());

    let author_prompt = match &git_name {
        Some(name) => format!("Author [{name}]: "),
        None => "Author (leave blank to skip): ".to_string(),
    };

    let author_input = prompt_line(&author_prompt)?;
    let author = if author_input.is_empty() {
        git_name
    } else {
        Some(author_input)
    };

    let llm = prompt_for_llm_config(&LlmConfig::default())?;

    let repo_path = if git2::Repository::open(".").is_ok() {
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        let answer = prompt_line(&format!("\nAdd {cwd} to watched repos? [Y/n]: "))?;
        if answer.is_empty() || answer.eq_ignore_ascii_case("y") {
            Some(cwd)
        } else {
            None
        }
    } else {
        None
    };

    let config = build_config(
        author,
        llm.provider,
        llm.api_key.clone(),
        llm.model.as_deref().unwrap_or(llm.provider.default_model()),
        repo_path,
    );
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

    println!("\nDefault flow: `devjournal add <path>` then `devjournal today`.");
    println!("Optional: run `devjournal start` for background polling.");
    Ok(())
}

pub fn api_key(config: &LlmConfig) -> Option<String> {
    std::env::var("DEVJOURNAL_API_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| config.api_key.clone().filter(|s| !s.is_empty()))
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
provider = "anthropic"
api_key = "sk-test"
model = "claude-sonnet-4-6"

[[repos]]
path = "/tmp/repo1"
name = "my-repo"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.llm.provider, LlmProvider::Anthropic);
        assert_eq!(config.repos.len(), 1);
        assert_eq!(config.repos[0].display_name(), "my-repo");
        assert_eq!(config.general.poll_interval_secs, 60);
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
            LlmProvider::Anthropic,
            Some("sk-ant-test".to_string()),
            "claude-sonnet-4-6",
            None,
        );
        assert_eq!(config.general.author, Some("Alice".to_string()));
        assert_eq!(config.llm.provider, LlmProvider::Anthropic);
        assert_eq!(config.llm.api_key, Some("sk-ant-test".to_string()));
        assert_eq!(config.llm.model, Some("claude-sonnet-4-6".to_string()));
        assert!(config.repos.is_empty());
    }

    #[test]
    fn test_build_config_ollama_with_repo() {
        let config = build_config(
            None,
            LlmProvider::Ollama,
            None,
            "llama3.2",
            Some("/tmp/repo".to_string()),
        );
        assert_eq!(config.llm.provider, LlmProvider::Ollama);
        assert_eq!(config.llm.api_key, None);
        assert_eq!(config.llm.model, Some("llama3.2".to_string()));
        assert_eq!(config.repos.len(), 1);
        assert_eq!(config.repos[0].path, "/tmp/repo");
    }

    #[test]
    fn test_api_key_env_override() {
        let _guard = env_var_test_mutex().lock().unwrap();
        let previous = std::env::var("DEVJOURNAL_API_KEY").ok();

        std::env::set_var("DEVJOURNAL_API_KEY", "env-key");
        let llm = LlmConfig {
            provider: LlmProvider::Anthropic,
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
    fn test_inline_llm_model_default_for_openai() {
        assert_eq!(LlmProvider::OpenAi.default_model(), "gpt-4o-mini");
    }

    #[test]
    fn test_llm_provider_reports_config_string_and_defaults() {
        assert_eq!(LlmProvider::Anthropic.as_str(), "anthropic");
        assert_eq!(LlmProvider::OpenAi.as_str(), "openai");
        assert_eq!(LlmProvider::Ollama.as_str(), "ollama");
        assert_eq!(LlmProvider::Anthropic.default_model(), "claude-sonnet-4-6");
        assert_eq!(LlmProvider::OpenAi.default_model(), "gpt-4o-mini");
        assert_eq!(LlmProvider::Ollama.default_model(), "llama3.2");
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
