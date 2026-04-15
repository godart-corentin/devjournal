<h1 align="center">
  Devjournal
</h1>

<p align="center">
  <strong>Turn your git activity into standup-ready daily notes.</strong>
</p>

<p align="center">
  A local-first CLI that watches your repositories, stores commit events in SQLite,
  and generates clean work summaries in Markdown.
</p>

<p align="center">
  <a href="https://github.com/godart-corentin/devjournal/actions/workflows/ci.yml">
    <img alt="CI" src="https://img.shields.io/github/actions/workflow/status/godart-corentin/devjournal/ci.yml?branch=main&label=CI">
  </a>
  <a href="https://github.com/godart-corentin/devjournal/releases">
    <img alt="Latest release" src="https://img.shields.io/github/v/release/godart-corentin/devjournal">
  </a>
  <a href="https://github.com/godart-corentin/devjournal/blob/main/LICENSE">
    <img alt="License" src="https://img.shields.io/github/license/godart-corentin/devjournal">
  </a>
  <img alt="Rust" src="https://img.shields.io/badge/built%20with-Rust-orange">
  <img alt="Platform" src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-blue">
  <img alt="Privacy" src="https://img.shields.io/badge/privacy-local--first-lightgrey">
</p>

---

`Devjournal` is a local-first CLI that watches one or more git repositories, stores commit events in a local SQLite database, and generates action-oriented work summaries in Markdown using Anthropic, OpenAI, or Ollama.

It also works **without** the background daemon: summary commands can sync the exact time window they need on demand.

## Table of contents

- [Why Devjournal](#why-devjournal)
- [Quick start](#quick-start)
- [Example output](#example-output)
- [What makes it different](#what-makes-it-different)
- [What leaves your machine](#what-leaves-your-machine)
- [Install](#install)
- [First-time setup](#first-time-setup)
- [Command overview](#command-overview)
- [Common workflows](#common-workflows)
- [How it works](#how-it-works)
- [Providers](#providers)
- [Platform support](#platform-support)
- [Configuration](#configuration)
- [No-LLM and local-only usage](#no-llm-and-local-only-usage)
- [Behavior and limitations](#behavior-and-limitations)
- [Troubleshooting](#troubleshooting)
- [File locations](#file-locations)
- [Shell completions](#shell-completions)
- [Contributing](#contributing)
- [Code of Conduct](#code-of-conduct)
- [Security](#security)
- [License](#license)
- [Maintainers](#maintainers)

## Why Devjournal

Use `devjournal` if you want to:

- turn commits into daily updates without writing notes by hand
- keep a searchable local history of what you worked on
- generate summaries only when you need them
- stay local-first, with optional fully local generation through Ollama
- export raw structured data as JSON for scripts and integrations

`devjournal` focuses on what changed in your projects rather than dumping raw git metadata, and it can enrich summaries further with optional semantic data from `sem` when available.

## Quick start

Install it, add a repo, and generate today’s summary:

```bash
brew tap godart-corentin/devjournal
brew install devjournal
brew install sem-cli

devjournal add /path/to/your/repo
devjournal today
```

What happens here:

- `devjournal add` creates the config file automatically if it does not exist yet
- `devjournal today` syncs today’s commits before generating output
- if no LLM is configured yet, `devjournal` starts inline setup and then continues automatically

## Example output

```markdown
# Dev Journal — 2026-03-23

## devjournal

- Implemented SQLite database layer with WAL mode for concurrent daemon/CLI access
- Added libgit2-based git poller with incremental commit detection
- Wired up Anthropic and OpenAI backends with a structured prompt builder
- Shipped CLI commands for add, remove, status, log, today, summary, start, and stop
- Improved Windows daemon shutdown handling and PID cleanup
```

The generated output is grouped by project and written for humans, while JSON mode returns raw recorded events for automation.

## What makes it different

- **Local-first by default** — commit events are stored in a local SQLite database on your machine
- **Daemon optional** — background polling is useful, but summary commands still work without it
- **Built for real updates** — output is grouped by project and focused on what changed
- **Human and machine friendly** — Markdown for people, JSON for scripts
- **Provider choice** — works with Anthropic, OpenAI, or Ollama
- **Optional semantic enrichment** — uses `sem` when available for more concrete summaries

## What leaves your machine

- recorded commit events stay in your local SQLite database
- Markdown summary commands send event data to your configured provider to generate summary text
- `--format json` skips the LLM call entirely and returns recorded event objects directly
- if you use Ollama with a local `base_url`, summary generation stays on your machine

## Install

### Homebrew

```bash
brew tap godart-corentin/devjournal
brew install devjournal
brew install sem-cli
```

Install `sem` alongside `devjournal` for the best summaries, then run `devjournal sync` to backfill semantic metadata.

For Homebrew installs, upgrade with:

```bash
brew upgrade devjournal
```

### macOS / Linux install script

```bash
curl -fsSL https://raw.githubusercontent.com/godart-corentin/devjournal/main/install.sh | sh
```

The install script:

- downloads the latest matching release archive
- verifies it using the published SHA256 checksum manifest
- installs the binary to `~/.local/bin` by default
- writes an install marker for future `devjournal update` support
- tries to install `sem` automatically when Homebrew or Cargo is available

Set `DEVJOURNAL_INSTALL_DIR` to install somewhere else.

### Windows

Install from the latest GitHub release.

The shell installer is macOS/Linux only, and `devjournal update` is currently disabled on Windows while the replacement flow is being hardened.

### Build from source

Requires Rust:

```bash
git clone git@github.com:godart-corentin/devjournal.git
cd devjournal
cargo build --release
cp target/release/devjournal ~/.local/bin/devjournal
```

## First-time setup

### 1) Add one or more repositories

```bash
devjournal add /path/to/your/repo
devjournal add /path/to/another/repo --name my-project
```

If the config file does not exist yet, `devjournal add` creates it automatically.

LLM configuration is not required up front.

If another tracked repo already uses the same display name, `devjournal` auto-suffixes the new one, such as `my-project-2`.

### 2) Generate your first summary

```bash
devjournal today
```

This syncs only today’s commits before generating output.

If you have not configured an LLM yet, `devjournal` launches inline setup and then continues directly to today’s summary.

### 3) Optional guided setup

```bash
devjournal init
```

Use `init` if you want a guided flow for author and LLM settings rather than relying on automatic setup during your first summary command.

### 4) Optional background polling

```bash
devjournal start
```

Use the daemon if you want your tracked repositories polled continuously between summary runs.

It is optional.

## Command overview

| Command                  | What it does                                    |
| ------------------------ | ----------------------------------------------- |
| `devjournal add`         | Track a repository                              |
| `devjournal remove`      | Stop tracking a repository                      |
| `devjournal today`       | Generate today’s summary                        |
| `devjournal week`        | Generate a summary for the current week         |
| `devjournal month`       | Generate a summary for the current month        |
| `devjournal summary`     | Generate a summary for a specific date or range |
| `devjournal log`         | Inspect recorded events                         |
| `devjournal sync`        | Backfill history into the local database        |
| `devjournal start`       | Start the optional background daemon            |
| `devjournal stop`        | Stop the daemon                                 |
| `devjournal status`      | Check daemon state                              |
| `devjournal doctor`      | Validate your setup                             |
| `devjournal update`      | Self-update script-installed binaries           |
| `devjournal config`      | Print the active config path                    |
| `devjournal list`        | List tracked repositories                       |
| `devjournal search`      | Search recorded commit events by keyword        |
| `devjournal prune`       | Delete events older than a retention window     |
| `devjournal completions` | Generate shell completions                      |

## Common workflows

### Daily use

```bash
devjournal today
```

### Backfill older history

```bash
devjournal sync
devjournal sync my-project
devjournal sync /path/to/repo
```

### Summarize a specific date or date range

```bash
devjournal summary 2026-03-23
devjournal summary --from 2026-03-01 --to 2026-03-07
devjournal week
devjournal month
```

### Search your recorded history

```bash
devjournal search "sqlite"
devjournal search "auth" --repo my-project
```

### Return raw JSON instead of Markdown

```bash
devjournal today --format json
devjournal summary --from 2026-03-01 --to 2026-03-07 --format json
devjournal log --from 2026-03-01 --to 2026-03-07 --format json
```

For summary-style commands, JSON mode skips the LLM call entirely and returns recorded event objects directly.

Summary commands still sync the requested time window before returning JSON output.

### Check setup and update

```bash
devjournal doctor
devjournal update
```

For the full CLI surface:

```bash
devjournal --help
```

## How it works

1. The optional background daemon polls your configured repositories on a fixed interval
2. New commits since the last poll are recorded as events in the local SQLite database
3. When you run a summary command like `devjournal today`, the CLI first syncs just the requested time window into the database
4. The CLI reads the relevant events for that same window
5. For Markdown summaries, `devjournal` sends those events to the configured provider and asks for a structured summary grouped by project
6. When activity exists, the generated Markdown is printed to stdout and cached in the summaries directory

The daemon and CLI share the same database directly.

There is no separate API server or IPC layer, and summary commands work whether or not the daemon is running.

## Providers

| Provider    | Default model       | API key required | Local-only possible |
| ----------- | ------------------- | ---------------- | ------------------- |
| `anthropic` | `claude-sonnet-4-6` | Yes              | No                  |
| `openai`    | `gpt-4o-mini`       | Yes              | No                  |
| `ollama`    | `llama3.2`          | No               | Yes                 |

If you point Ollama at a local `base_url`, summary generation can stay entirely on your machine.

## Platform support

| Platform | Notes                           |
| -------- | ------------------------------- |
| macOS    | Supported by the install script |
| Linux    | Supported by the install script |
| Windows  | Install from GitHub releases    |

`devjournal update` self-updates only for binaries installed through `install.sh`.

Homebrew installs should use `brew upgrade devjournal`, and self-update is currently disabled on Windows.

## Configuration

The config file is TOML.

It is created automatically the first time you run `devjournal add`, or by `devjournal init` if you prefer guided setup.

Example:

```toml
[general]
poll_interval_secs = 60
author = "Your Name"

[llm]
provider = "anthropic"
api_key = "sk-ant-..."
model = "claude-sonnet-4-6"
# base_url = "http://localhost:11434"

[[repos]]
path = "/Users/yourname/workspace/devjournal"
name = "devjournal"
```

### Important settings

| Setting                      | Default                  | Notes                                                                     |
| ---------------------------- | ------------------------ | ------------------------------------------------------------------------- |
| `general.poll_interval_secs` | `60`                     | Poll interval in seconds                                                  |
| `general.author`             | none                     | Required; only commits by this exact author name are recorded             |
| `general.retention_days`     | none                     | Optional automatic retention window for old events                        |
| `llm.provider`               | `"anthropic"`            | One of `"anthropic"`, `"openai"`, or `"ollama"`                           |
| `llm.model`                  | provider-specific        | Anthropic: `claude-sonnet-4-6`, OpenAI: `gpt-4o-mini`, Ollama: `llama3.2` |
| `llm.api_key`                | none                     | `DEVJOURNAL_API_KEY` takes precedence; not required for Ollama            |
| `llm.base_url`               | `http://localhost:11434` | Ollama only                                                               |
| `llm.system_prompt`          | none                     | Optional custom prompt that replaces the default summary instructions     |
| `repos[].name`               | folder name              | Defaults to the repository folder name when omitted                       |

To print the active config path:

```bash
devjournal config
```

## No-LLM and local-only usage

You do not need an LLM to collect and store activity.

- `devjournal add`, `sync`, `log`, and database-backed workflows still work without summary generation
- `--format json` on summary-style commands skips the LLM call entirely
- Ollama with a local `base_url` keeps summary generation on your machine

## Behavior and limitations

- on the very first poll of a repository, `devjournal` records only the current `HEAD`, not the full history
- use `devjournal sync` to backfill older commits
- `today`, `week`, `month`, and `summary` sync only the time window they need before reading events
- `sem` is optional but recommended
- without `sem`, `devjournal` still works using structured git diff metadata and selective patch fallbacks
- the configured author must match your git author name exactly or commits will not be recorded
- Non-empty Markdown summaries are cached in the summaries directory and reused unless events change or you pass `--force`

## Troubleshooting

### No events showing up in `devjournal log`

Check whether the daemon is running:

```bash
devjournal status
```

If it started but still shows 0 events, wait one poll interval and check again, or backfill immediately with:

```bash
devjournal sync
```

### `devjournal today` returns `No activity recorded`

`devjournal today` syncs today’s window before reading events, so this usually means there are no matching commits for today.

For past dates, use:

```bash
devjournal summary 2026-03-23
devjournal summary --from 2026-03-01 --to 2026-03-07
```

Use `devjournal sync` for full backfill.

### API key not found

`DEVJOURNAL_API_KEY` takes precedence over `api_key` in the config file.

Make sure it is exported in the shell where you run summary commands.

### `stop` times out on Windows

`devjournal stop` uses `TerminateProcess` on Windows and can fail if the daemon was started from a different privilege context.

Stop it manually with Task Manager or:

```bash
taskkill /PID <pid> /F
```

Then remove the stale PID file from:

```text
%LOCALAPPDATA%\devjournal\devjournal.pid
```

### Config file not found

Run:

```bash
devjournal add /path/to/repo
```

to create the config with defaults, or:

```bash
devjournal init
```

for guided setup.

### Database schema error on startup

`devjournal` applies lightweight automatic database migrations when opening SQLite.

If you see an unsupported newer schema version error, upgrade `devjournal` or restore a compatible database backup.

### `no author configured` on start

Add your git author name under `[general]`:

```toml
author = "Your Name"
```

It must match your git author exactly.

### Ollama: `Failed to call Ollama API`

Start Ollama and confirm the model exists:

```bash
ollama serve
ollama list
```

If Ollama runs on another machine, set `base_url` accordingly.

### Warnings about `sem` extraction

`devjournal` still records commits and generates summaries without `sem`.

Install or repair it, then run `devjournal sync` again to backfill richer semantic metadata.

## File locations

| Purpose    | macOS                                                     | Linux                                      | Windows                                    |
| ---------- | --------------------------------------------------------- | ------------------------------------------ | ------------------------------------------ |
| Config     | `~/Library/Application Support/devjournal/config.toml`    | `~/.config/devjournal/config.toml`         | `%APPDATA%\devjournal\config.toml`         |
| Database   | `~/Library/Application Support/devjournal/events.db`      | `~/.local/share/devjournal/events.db`      | `%LOCALAPPDATA%\devjournal\events.db`      |
| PID file   | `~/Library/Application Support/devjournal/devjournal.pid` | `~/.local/share/devjournal/devjournal.pid` | `%LOCALAPPDATA%\devjournal\devjournal.pid` |
| Daemon log | `~/Library/Application Support/devjournal/devjournal.log` | `~/.local/share/devjournal/devjournal.log` | `%LOCALAPPDATA%\devjournal\devjournal.log` |
| Summaries  | `~/Library/Application Support/devjournal/summaries/`     | `~/.local/share/devjournal/summaries/`     | `%LOCALAPPDATA%\devjournal\summaries\`     |

## Shell completions

### Bash

```bash
devjournal completions bash > ~/.local/share/bash-completion/completions/devjournal
```

### Zsh

```zsh
devjournal completions zsh > ~/.zfunc/_devjournal
# Add to .zshrc:
fpath+=~/.zfunc
autoload -Uz compinit
compinit
```

### Fish

```fish
devjournal completions fish > ~/.config/fish/completions/devjournal.fish
```

## Contributing

Contributions are welcome.

See `CONTRIBUTING.md` for bug reports, feature requests, local setup, and focused pull request guidance.

## Code of Conduct

This project follows `CODE_OF_CONDUCT.md` to help keep the community welcoming and respectful.

## Security

If you believe you found a security issue, use the private reporting guidance in `SECURITY.md` rather than opening a public issue.

## License

`devjournal` is licensed under the Apache-2.0 License.

## Maintainers

Release flow, packaging details, and versioning checks live in [RELEASING.md](RELEASING.md).

The in-repo [Formula/devjournal.rb](Formula/devjournal.rb) remains the canonical formula source for releases, while maintainer-specific workflow details stay out of the user-facing README.
