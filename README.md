# devjournal

A local CLI daemon that watches your git repositories, logs every commit to a local SQLite database, and generates action-oriented daily markdown summaries using an LLM (Claude or OpenAI).

## How it works

1. A background daemon polls your configured repositories on a fixed interval (default: 60 seconds)
2. New commits since the last poll are recorded as events in a local SQLite database (see [File paths](#file-paths))
3. When you run `devjournal today`, it reads today's events from the database and sends them to your configured LLM
4. The LLM returns a structured markdown summary grouped by project, focused on what was done — not git metadata
5. The summary is printed to stdout and cached to the summaries directory as `YYYY-MM-DD.md` (subsequent runs return the cached file unless events change or `--force` is used)

The daemon and CLI share the same database directly — no IPC, no server process required. `devjournal today` works whether or not the daemon is running.

## File paths

| Purpose    | macOS                                                     | Linux                                      | Windows                                    |
| ---------- | --------------------------------------------------------- | ------------------------------------------ | ------------------------------------------ |
| Config     | `~/Library/Application Support/devjournal/config.toml`    | `~/.config/devjournal/config.toml`         | `%APPDATA%\devjournal\config.toml`         |
| Database   | `~/Library/Application Support/devjournal/events.db`      | `~/.local/share/devjournal/events.db`      | `%LOCALAPPDATA%\devjournal\events.db`      |
| PID file   | `~/Library/Application Support/devjournal/devjournal.pid` | `~/.local/share/devjournal/devjournal.pid` | `%LOCALAPPDATA%\devjournal\devjournal.pid` |
| Daemon log | `~/Library/Application Support/devjournal/devjournal.log` | `~/.local/share/devjournal/devjournal.log` | `%LOCALAPPDATA%\devjournal\devjournal.log` |
| Summaries  | `~/Library/Application Support/devjournal/summaries/`     | `~/.local/share/devjournal/summaries/`     | `%LOCALAPPDATA%\devjournal\summaries\`     |

## Install

**macOS / Linux (recommended):**

```bash
curl -fsSL https://raw.githubusercontent.com/godart-corentin/dev-journal/main/install.sh | sh
```

This downloads the latest pre-built binary to `~/.local/bin`. Set `DEVJOURNAL_INSTALL_DIR` to install elsewhere.
The installer also tries to provision `sem` automatically when Homebrew or Cargo is available.

**Homebrew:**

You can install from the dedicated Homebrew tap:

```bash
brew tap godart-corentin/devjournal
brew install devjournal
brew install sem-cli
```

Homebrew installs `devjournal` itself. Install `sem` alongside it for the best summaries, then re-run `devjournal sync` to backfill semantic metadata.

The tap repository lives at [godart-corentin/homebrew-devjournal](https://github.com/godart-corentin/homebrew-devjournal).

Long term, plain `brew install devjournal` without adding the custom tap first will require a successful `homebrew-core` submission. For that, the project needs to keep tagged releases, GitHub source archives, and `Cargo.toml` versioning aligned.

**Update:**

```bash
devjournal update
```

**Build from source** (for contributors, requires Rust):

```bash
git clone git@github.com:godart-corentin/dev-journal.git
cd dev-journal
cargo build --release
cp target/release/devjournal ~/.local/bin/devjournal
```

## Homebrew roadmap

The repository includes [`Formula/devjournal.rb`](Formula/devjournal.rb), and the dedicated tap lives at [godart-corentin/homebrew-devjournal](https://github.com/godart-corentin/homebrew-devjournal). That tap is the recommended Homebrew installation path until a future `homebrew-core` submission.

Before submitting to `homebrew-core`, make sure:

- the git tag, GitHub release, and `Cargo.toml` version all match
- the source tarball URL and SHA256 are updated for the new release
- `brew audit --strict --new devjournal` passes cleanly
- `brew test devjournal` passes on a fresh install

## Setup

**1. Run the setup wizard:**

```bash
devjournal init
```

This walks you through author name, LLM provider, API key, and model selection, then optionally adds the current directory as a watched repo. It also reports whether semantic enrichment is active, unavailable, or degraded. You can edit the config file directly at any time (see [Configuration](#configuration)).

**2. Add repositories to watch:**

```bash
devjournal add /path/to/your/repo
devjournal add /path/to/another/repo --name my-project
```

**3. Start the daemon:**

```bash
devjournal start
```

The daemon runs in the background and polls all configured repos on the interval set in your config.

**4. Generate today's summary:**

```bash
devjournal today
```

## Commands

| Command                                                  | Description                                                                  |
| -------------------------------------------------------- | ---------------------------------------------------------------------------- |
| `devjournal`                                             | Show daemon state and watched repos (same as `status`)                       |
| `devjournal init`                                        | Interactive setup wizard (first-time configuration)                          |
| `devjournal add <path>`                                  | Add a git repository to the watch list                                       |
| `devjournal remove <path>`                               | Remove a repository from the watch list                                      |
| `devjournal start`                                       | Start the background polling daemon                                          |
| `devjournal stop`                                        | Stop the daemon                                                              |
| `devjournal sync [name]`                                 | Sync full git history into the DB (see below)                                |
| `devjournal status`                                      | Show daemon state, semantic enrichment health, watched repos, and today's event count |
| `devjournal today`                                       | Generate and print today's summary                                           |
| `devjournal summary [YYYY-MM-DD]`                        | Generate and print the summary for a specific date                           |
| `devjournal summary --from YYYY-MM-DD [--to YYYY-MM-DD]` | Generate a summary for a date range (defaults to today if `--to` is omitted) |
| `devjournal week`                                        | Generate a rolling 7-day summary (today minus 6 days)                        |
| `devjournal month`                                       | Generate a rolling 30-day summary                                            |
| `devjournal search <keyword>`                            | Search recorded events by keyword                                            |
| `devjournal log [YYYY-MM-DD]`                            | Show raw recorded events (useful for debugging, supports `--format json`)    |
| `devjournal log --from YYYY-MM-DD [--to YYYY-MM-DD]`     | Show raw events for a date range (supports `--format json`)                  |
| `devjournal list`                                        | List all watched repositories                                                |
| `devjournal doctor`                                      | Run diagnostic checks on your setup, including semantic enrichment health    |
| `devjournal prune <days>`                                | Delete events older than N days                                              |
| `devjournal completions <shell>`                         | Generate shell completions (bash, zsh, fish)                                 |
| `devjournal config`                                      | Print the path to the config file                                            |
| `devjournal update`                                      | Update devjournal to the latest release                                      |

The `add` command uses the folder name as the display name by default. Use `--name` to override it:

```bash
devjournal add /path/to/my-api            # display name: "my-api"
devjournal add /path/to/my-api --name API # display name: "API"
```

All summary commands (`today`, `summary`, `week`, `month`), `search`, and `log` accept `--format json` to output raw events as a JSON array instead of a markdown summary or line-based log view. For summary commands and `search`, this skips the LLM call entirely.

### Shell completions

Generate completions for your shell and source them:

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

## Configuration

The config file is TOML. It is created by `devjournal init` or automatically the first time you run `devjournal add`. You can edit it directly:

```toml
[general]
poll_interval_secs = 60   # How often the daemon polls each repo
author = "Your Name"      # Required — only commits by this author are recorded

[llm]
provider = "claude"       # "claude", "openai", "ollama", or "cursor"
api_key = "sk-ant-..."    # Optional — prefer DEVJOURNAL_API_KEY env var. Not needed for ollama.
model = "claude-sonnet-4-6"  # Optional — defaults per provider shown below
# base_url = "http://localhost:11434"  # Ollama only

[[repos]]
path = "/Users/tylia/workspace/perso/dev-journal"
name = "dev-journal"

[[repos]]
path = "/Users/tylia/workspace/work/my-api"
name = "my-api"
```

| Setting              | Default                  | Notes                                                                                                                                                                |
| -------------------- | ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `poll_interval_secs` | `60`                     | Minimum effective value is 1                                                                                                                                         |
| `author`             | —                        | **Required.** Must match your git author name exactly. Daemon refuses to start without it.                                                                           |
| `llm.provider`       | `"claude"`               | `"claude"`, `"openai"`, `"ollama"`, or `"cursor"`                                                                                                                    |
| `llm.model`          | `claude-sonnet-4-6`      | OpenAI: `gpt-4o`. Ollama: `llama3.2`. Cursor: `gpt-5.4-mini`.                                                                                                       |
| `llm.api_key`        | —                        | `DEVJOURNAL_API_KEY` env var takes precedence. Not required for Ollama or Cursor.                                                                                    |
| `llm.base_url`       | `http://localhost:11434` | Ollama only — change for remote instances                                                                                                                            |
| `retention_days`     | —                        | Optional. When set, the daemon automatically prunes events older than this many days each poll cycle. Can also be triggered manually with `devjournal prune <days>`. |
| `llm.system_prompt`  | —                        | Optional. Custom prompt that replaces the default summary generation rules. Use this to customize the summary style (e.g., changelog format, standup format, haiku). |
| `repos[].name`       | folder name              | Defaults to the repository folder name                                                                                                                               |

### First poll behaviour

On the very first poll of a repo, devjournal records only the most recent commit (HEAD), not the entire history. Subsequent polls record all new commits since the last seen hash.

### Syncing history manually

`devjournal sync` walks the full commit history of a repo and inserts any commits not already in the database. This is useful when you first set up the tool and want to backfill past work:

```bash
# Sync all watched repos
devjournal sync

# Sync a specific repo by name or path
devjournal sync my-project
devjournal sync /path/to/repo
```

Running `sync` multiple times is safe — duplicate commits are silently ignored. The daemon can continue running alongside it.

If `sem` is available, devjournal stores semantic diff metadata for commits and uses it to generate more concrete summaries. Every commit also stores compact structured diff metadata, and the summarizer only falls back to raw patch excerpts for small low-signal commits when semantic metadata is missing. Re-running `devjournal sync` can backfill semantic metadata for existing commits, and cached summaries are refreshed automatically when the underlying event payload changes.

## Summary format

Summaries follow these rules, enforced via the LLM prompt:

- Grouped by project with `##` section headers
- Action-oriented bullet points: what was done, fixed, tested, or shipped
- Ticket/issue references preserved (e.g. `TT-1234`, `PROJ-567`)
- When semantic metadata is available, summaries prefer concrete entities/files changed over commit-message-only guesses
- When semantic metadata is unavailable, summaries fall back to structured diff metadata before using a small patch excerpt
- No branch names, file counts, or other git metadata
- Saved to the summaries directory (`YYYY-MM-DD.md` for single days, `YYYY-MM-DD_to_YYYY-MM-DD.md` for ranges)

For date ranges, bullets are allowed to scale with the number of days (up to 5 per project) and are ordered chronologically within each section.

Example output:

```markdown
# Dev Journal — 2026-03-23

## dev-journal

- Scaffolded Rust project with cargo, wired up all module stubs
- Implemented SQLite database layer with WAL mode for concurrent daemon/CLI access
- Added libgit2-based git poller with incremental commit detection
- Wired up Claude and OpenAI LLM backends with a structured prompt builder
- Shipped full CLI with clap: add, remove, status, log, today, summary, start/stop
```

## Troubleshooting

**No events showing up in `devjournal log`?**
Check that the daemon is running (`devjournal status`). If it started but shows 0 events, wait one poll interval (default 60 seconds) and check again. To inspect daemon output, check the log file (see [File paths](#file-paths)). You can also backfill history immediately without the daemon using `devjournal sync`.

**`devjournal today` returns "No activity recorded"?**
The daemon must have polled at least once since you added the repo. Confirm with `devjournal log`. If you want to generate a summary for a past date, the events for that date must already be in the database.

**API key not found error?**
`DEVJOURNAL_API_KEY` in your environment takes precedence over `api_key` in the config file. Make sure it is exported (not just set) in the shell where you run `devjournal today`.

**`stop` times out on Windows?**
`devjournal stop` uses `TerminateProcess` on Windows, which requires the calling process to have sufficient privilege to open the daemon process. If the daemon was started in a different privilege context (e.g., an elevated terminal), the stop command may fail with "access denied". In that case, kill the process manually via Task Manager or `taskkill /PID <pid> /F`, then remove the stale PID file from `%LOCALAPPDATA%\devjournal\devjournal.pid`.

**Daemon already running after a crash?**
If the process died without cleaning up its PID file, `devjournal start` will detect the stale file and remove it automatically before starting a new process.

**Config file not found?**
Run `devjournal init` for guided setup, or `devjournal add <path>` to create the config with defaults.

**"no author configured" error on start?**
Add your git author name to `[general]` in the config file: `author = "Your Name"`. It must match your git author name exactly (check with `git log --format='%an' | head -1`).

**Ollama: "Failed to call Ollama API"?**
Ollama must be running before you generate a summary. Start it with `ollama serve`, then verify the model is pulled: `ollama list`. If you are running Ollama on a different machine, set `base_url` in your config to point at it.

**Cursor: "cursor agent not found"?**
Cursor must be installed and the `cursor` binary must be on your PATH. Install from [cursor.com](https://cursor.com) and ensure the CLI is available: `cursor --version`. No API key is required — Cursor uses your existing account auth.

**Warnings about `sem` extraction?**
If `sem` is not installed or temporarily fails for a commit, devjournal still records the commit and generates summaries using structured git diff metadata. Install or repair `sem`, then run `devjournal sync` to backfill semantic metadata for existing commits.
