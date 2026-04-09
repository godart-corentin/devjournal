# devjournal

`devjournal` is a local-first CLI and background daemon that watches your git repositories, stores commit events in a local SQLite database, and turns your recent work into action-oriented markdown summaries using Claude, OpenAI, Ollama, or Cursor.

- Keeps your raw activity history on disk in a local SQLite database
- Watches multiple repositories in the background, but summary commands still work without the daemon running
- Groups output by project and focuses on what you actually changed, not git metadata
- Supports markdown output for humans and JSON output for scripts and integrations
- Uses optional semantic enrichment via `sem` for more concrete summaries when available

## Example output

```markdown
# Dev Journal - 2026-03-23

## devjournal

- Scaffolded Rust project with cargo, wired up all module stubs
- Implemented SQLite database layer with WAL mode for concurrent daemon/CLI access
- Added libgit2-based git poller with incremental commit detection
- Wired up Claude and OpenAI LLM backends with a structured prompt builder
- Shipped full CLI with clap: add, remove, status, log, today, summary, start/stop
```

## Quickstart

### 1. Install

**macOS / Linux (install script):**

```bash
curl -fsSL https://raw.githubusercontent.com/godart-corentin/dev-journal/main/install.sh | sh
```

This downloads the latest pre-built binary to `~/.local/bin` by default. Set `DEVJOURNAL_INSTALL_DIR` to install elsewhere.

The installer:

- Downloads the matching release archive
- Downloads the published SHA256 checksum manifest
- Verifies the archive before extracting it
- Tries to install `sem` automatically when Homebrew or Cargo is available

**Homebrew:**

```bash
brew tap godart-corentin/devjournal
brew install devjournal
brew install sem-cli
```

Homebrew installs `devjournal` itself. Install `sem` alongside it for the best summaries, then re-run `devjournal sync` to backfill semantic metadata.

**Windows:**

Install from the [latest GitHub release](https://github.com/godart-corentin/dev-journal/releases/latest). The shell installer is macOS/Linux-only, and `devjournal update` is currently disabled on Windows while the replacement flow is being hardened.

**Build from source** (requires Rust):

```bash
git clone git@github.com:godart-corentin/dev-journal.git
cd dev-journal
cargo build --release
cp target/release/devjournal ~/.local/bin/devjournal
```

### 2. Run the setup wizard

```bash
devjournal init
```

This walks you through author name, LLM provider, API key when needed, and model selection. It can also add the current directory as a watched repo and reports whether semantic enrichment is active, unavailable, or degraded.

### 3. Add a repository

```bash
devjournal add /path/to/your/repo
devjournal add /path/to/another/repo --name my-project
```

### 4. Start the watcher

```bash
devjournal start
```

The daemon polls all configured repos on the interval set in your config. On Unix, `devjournal start` detaches from the invoking terminal so it keeps running after the shell closes.

### 5. Generate a summary

```bash
devjournal today
```

## What leaves your machine

- Commit events are stored locally in SQLite on your machine
- Markdown summary commands send event data to your configured provider to generate the summary text
- `--format json` on summary-style commands returns recorded events directly and skips the LLM call
- If you use Ollama with a local `base_url`, summary generation stays on your machine

## How it works

1. A background daemon polls your configured repositories on a fixed interval. The default is 60 seconds.
2. New commits since the last poll are recorded as events in a local SQLite database.
3. When you run a summary command such as `devjournal today`, the CLI reads the relevant events from the database.
4. For markdown summaries, `devjournal` sends those events to the configured provider and asks for a structured summary grouped by project.
5. The generated markdown is printed to stdout and cached in the summaries directory. Subsequent runs reuse the cache unless events changed or you pass `--force`.

The daemon and CLI share the same database directly. There is no separate API server or IPC layer, and summary commands work whether or not the daemon is currently running.

## Providers and platform support

### LLM providers

`devjournal` supports these providers:

- `claude`
- `openai`
- `ollama`
- `cursor`

Provider defaults from the current CLI:

- Claude default model: `claude-sonnet-4-6`
- OpenAI default model: `gpt-4o`
- Ollama default model: `llama3.2`
- Cursor default model: `gpt-5.4-mini`

### Platform notes

- The install script supports macOS and Linux
- Windows users should install from the GitHub releases page
- `devjournal update` works on macOS and Linux and verifies the downloaded archive before replacing the binary
- Self-update is currently disabled on Windows

## Configuration essentials

The config file is TOML. It is created by `devjournal init`, or automatically the first time you run `devjournal add`.

```toml
[general]
poll_interval_secs = 60
author = "Your Name"

[llm]
provider = "claude"
api_key = "sk-ant-..."
model = "claude-sonnet-4-6"
# base_url = "http://localhost:11434"

[[repos]]
path = "/Users/tylia/workspace/perso/devjournal"
name = "devjournal"
```

Important settings:

| Setting | Default | Notes |
| ------- | ------- | ----- |
| `general.poll_interval_secs` | `60` | Poll interval in seconds. Minimum effective value is 1. |
| `general.author` | — | Required. Only commits by this author are recorded, and the match must be exact. |
| `general.retention_days` | — | Optional automatic retention window for old events. |
| `llm.provider` | `"claude"` | One of `"claude"`, `"openai"`, `"ollama"`, or `"cursor"`. |
| `llm.model` | provider-specific | Claude: `claude-sonnet-4-6`. OpenAI: `gpt-4o`. Ollama: `llama3.2`. Cursor: `gpt-5.4-mini`. |
| `llm.api_key` | — | `DEVJOURNAL_API_KEY` takes precedence. Not required for Ollama or Cursor. |
| `llm.base_url` | `http://localhost:11434` | Ollama only. Change this for remote Ollama instances. |
| `llm.system_prompt` | — | Optional custom prompt that replaces the default summary instructions. |
| `repos[].name` | folder name | Defaults to the repository folder name when omitted. |

To print the active config path:

```bash
devjournal config
```

## Common workflows

### First-time setup

```bash
devjournal init
devjournal add /path/to/repo
devjournal start
devjournal today
```

### Backfill history after setup

```bash
devjournal sync
devjournal sync my-project
devjournal sync /path/to/repo
```

### Generate summaries for other periods

```bash
devjournal summary 2026-03-23
devjournal summary --from 2026-03-01 --to 2026-03-07
devjournal week
devjournal month
```

### Return raw JSON instead of markdown

```bash
devjournal today --format json
devjournal summary --from 2026-03-01 --to 2026-03-07 --format json
devjournal log --from 2026-03-01 --to 2026-03-07 --format json
```

For summary commands, JSON output skips the LLM call entirely and returns recorded event objects instead.

### Check your setup and update the binary

```bash
devjournal doctor
devjournal update
```

For the complete CLI surface, run `devjournal --help` or `devjournal <command> --help`.

## Behavior and limitations

- On the very first poll of a repo, `devjournal` records only the current `HEAD`, not the full history
- Use `devjournal sync` to backfill older commits into the database
- `sem` is optional but recommended; if it is unavailable, `devjournal` still works using structured git diff metadata and selective patch fallbacks
- The configured author must match your git author name exactly or commits will not be recorded
- Markdown summaries are cached in the summaries directory and reused unless events change or you pass `--force`

## Troubleshooting

**No events showing up in `devjournal log`?**  
Check that the daemon is running with `devjournal status`. If it started but shows 0 events, wait one poll interval and check again. You can also backfill history immediately with `devjournal sync`.

**`devjournal today` returns "No activity recorded"?**  
The daemon must have polled at least once since you added the repo. Confirm with `devjournal log`. For past dates, the events for that date must already be in the database.

**API key not found?**  
`DEVJOURNAL_API_KEY` takes precedence over `api_key` in the config file. Make sure it is exported in the shell where you run summary commands.

**`stop` times out on Windows?**  
`devjournal stop` uses `TerminateProcess` on Windows and can fail if the daemon was started from a different privilege context. In that case, stop it manually with Task Manager or `taskkill /PID <pid> /F`, then remove the stale PID file from `%LOCALAPPDATA%\devjournal\devjournal.pid`.

**Config file not found?**  
Run `devjournal init` for guided setup, or `devjournal add <path>` to create the config with defaults.

**Database schema error on startup?**  
`devjournal` applies lightweight automatic database migrations when opening SQLite. If you see an unsupported newer schema version error, upgrade `devjournal` or restore a compatible database backup.

**"no author configured" on start?**  
Add your git author name under `[general]` as `author = "Your Name"`. It must match your git author exactly.

**Ollama: "Failed to call Ollama API"?**  
Start Ollama with `ollama serve`, then confirm the model is available with `ollama list`. If Ollama runs on another machine, set `base_url` accordingly.

**Cursor: "cursor agent not found"?**  
Install Cursor from [cursor.com](https://cursor.com) and make sure the `cursor` binary is on your `PATH`. Verify with `cursor --version`.

**Warnings about `sem` extraction?**  
`devjournal` still records commits and generates summaries without `sem`. Install or repair `sem`, then re-run `devjournal sync` to backfill richer semantic metadata.

## Reference

### File paths

| Purpose | macOS | Linux | Windows |
| ------- | ----- | ----- | ------- |
| Config | `~/Library/Application Support/devjournal/config.toml` | `~/.config/devjournal/config.toml` | `%APPDATA%\devjournal\config.toml` |
| Database | `~/Library/Application Support/devjournal/events.db` | `~/.local/share/devjournal/events.db` | `%LOCALAPPDATA%\devjournal\events.db` |
| PID file | `~/Library/Application Support/devjournal/devjournal.pid` | `~/.local/share/devjournal/devjournal.pid` | `%LOCALAPPDATA%\devjournal\devjournal.pid` |
| Daemon log | `~/Library/Application Support/devjournal/devjournal.log` | `~/.local/share/devjournal/devjournal.log` | `%LOCALAPPDATA%\devjournal\devjournal.log` |
| Summaries | `~/Library/Application Support/devjournal/summaries/` | `~/.local/share/devjournal/summaries/` | `%LOCALAPPDATA%\devjournal\summaries\` |

### Shell completions

**Bash:**

```bash
devjournal completions bash > ~/.local/share/bash-completion/completions/devjournal
```

**Zsh:**

```zsh
devjournal completions zsh > ~/.zfunc/_devjournal
# Add to .zshrc: fpath+=~/.zfunc; autoload -Uz compinit; compinit
```

**Fish:**

```fish
devjournal completions fish > ~/.config/fish/completions/devjournal.fish
```

## Maintainers

Release flow, packaging details, and versioning checks live in [RELEASING.md](RELEASING.md). The in-repo [Homebrew formula](Formula/devjournal.rb) remains the canonical formula source for releases, but the maintainer workflow is intentionally kept out of the user-facing README.
