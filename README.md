# devjournal

A local CLI daemon that watches your git repositories, logs every commit to a local SQLite database, and generates action-oriented daily markdown summaries using an LLM (Claude or OpenAI).

## How it works

1. A background daemon polls your configured repositories on a fixed interval (default: 60 seconds)
2. New commits since the last poll are recorded as events in a local SQLite database (`~/.local/share/devjournal/events.db`)
3. When you run `devjournal today`, it reads today's events from the database and sends them to your configured LLM
4. The LLM returns a structured markdown summary grouped by project, focused on what was done — not git metadata
5. The summary is printed to stdout and saved to `~/.local/share/devjournal/summaries/YYYY-MM-DD.md`

The daemon and CLI share the same database directly — no IPC, no server process required. `devjournal today` works whether or not the daemon is running.

## File paths

| Purpose        | macOS                                                     | Linux                                      | Windows                                              |
| -------------- | --------------------------------------------------------- | ------------------------------------------ | ---------------------------------------------------- |
| Config         | `~/Library/Application Support/devjournal/config.toml`   | `~/.config/devjournal/config.toml`         | `%APPDATA%\devjournal\config.toml`                   |
| Database       | `~/Library/Application Support/devjournal/events.db`     | `~/.local/share/devjournal/events.db`      | `%LOCALAPPDATA%\devjournal\events.db`                |
| PID file       | `~/Library/Application Support/devjournal/devjournal.pid`| `~/.local/share/devjournal/devjournal.pid` | `%LOCALAPPDATA%\devjournal\devjournal.pid`           |
| Daemon log     | `~/Library/Application Support/devjournal/devjournal.log`| `~/.local/share/devjournal/devjournal.log` | `%LOCALAPPDATA%\devjournal\devjournal.log`           |
| Summaries      | `~/Library/Application Support/devjournal/summaries/`    | `~/.local/share/devjournal/summaries/`     | `%LOCALAPPDATA%\devjournal\summaries\`               |

## Install

Build from source (requires Rust):

```bash
git clone <repo-url> ~/dev-journal
cd ~/dev-journal
cargo build --release
```

Install the binary:

**macOS / Linux:**
```bash
cp target/release/devjournal ~/.local/bin/devjournal
```

**Windows:**
```powershell
cargo install --path .
```

## Setup

**1. Run the setup wizard:**

```bash
devjournal init
```

This walks you through author name, LLM provider, API key, and model selection, then optionally adds the current directory as a watched repo. The config file is written to the path shown at the end — you can edit it directly at any time.

Alternatively, add a repository manually (this also creates the config on first run):

```bash
devjournal add /path/to/your/repo --name my-project
```

**2. Set your author name:**

Add your git author name to the config so the daemon only records your own commits:

```toml
[general]
author = "Your Name"   # must match your git author name exactly
```

The daemon will refuse to start if this is not set.

**3. Set your LLM provider:**

**Ollama (free, local)** — no API key needed. [Install Ollama](https://ollama.com), pull a model, then configure devjournal to use it:

```bash
ollama pull llama3.2
```

```toml
# in your config.toml
[llm]
provider = "ollama"
model = "llama3.2"       # any model you have pulled
# base_url = "http://localhost:11434"  # default, change for remote instances
```

**Claude or OpenAI** — requires an API key. Export it in your shell:

```bash
export DEVJOURNAL_API_KEY=sk-ant-...
```

Or add it to the config file (see [Configuration](#configuration)).

**4. Start the daemon:**

```bash
devjournal daemon start
```

The daemon runs in the background and polls all configured repos on the interval set in your config. Its PID is written to the PID file so `stop` and `status` can find it.

**5. Generate today's summary:**

```bash
devjournal today
```

## Commands

| Command                          | Description                                              |
| -------------------------------- | -------------------------------------------------------- |
| `devjournal`                     | Show daemon state and watched repos (same as `status`)   |
| `devjournal init`                | Interactive setup wizard (first-time configuration)      |
| `devjournal add <path>`          | Add a git repository to the watch list                   |
| `devjournal remove <path>`       | Remove a repository from the watch list                  |
| `devjournal daemon start`        | Start the background polling daemon                      |
| `devjournal daemon stop`         | Stop the daemon                                          |
| `devjournal daemon logs`         | Print the path to the daemon log file                    |
| `devjournal sync [name]`         | Sync full git history into the DB (see below)            |
| `devjournal status`              | Show daemon state, watched repos, and today's event count |
| `devjournal today`               | Generate and print today's summary                       |
| `devjournal summary [YYYY-MM-DD]`| Generate and print the summary for a specific date       |
| `devjournal log [YYYY-MM-DD]`    | Show raw recorded events (useful for debugging)          |
| `devjournal list`                | List all watched repositories                            |
| `devjournal config`              | Print the path to the config file                        |

The `add` command uses the folder name as the display name by default. Use `--name` to override it:

```bash
devjournal add /path/to/my-api            # display name: "my-api"
devjournal add /path/to/my-api --name API # display name: "API"
```

## Configuration

The config file is TOML. It is created by `devjournal init` or automatically the first time you run `devjournal add`. You can edit it directly:

```toml
[general]
poll_interval_secs = 60   # How often the daemon polls each repo
author = "Your Name"      # Required — only commits by this author are recorded

[llm]
provider = "claude"       # "claude", "openai", or "ollama"
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

| Setting               | Default              | Notes                                             |
| --------------------- | -------------------- | ------------------------------------------------- |
| `poll_interval_secs`  | `60`                 | Minimum effective value is 1                      |
| `author`              | —                    | **Required.** Must match your git author name exactly. Daemon refuses to start without it. |
| `llm.provider`        | `"claude"`           | `"claude"` or `"openai"`                         |
| `llm.model`           | `claude-sonnet-4-6`  | For OpenAI: defaults to `gpt-4o`                 |
| `llm.api_key`         | —                    | `DEVJOURNAL_API_KEY` env var takes precedence. Not required for Ollama. |
| `llm.base_url`        | `http://localhost:11434` | Ollama only — change for remote instances     |
| `repos[].name`        | folder name          | Defaults to the repository folder name            |

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

## Summary format

Summaries follow these rules, enforced via the LLM prompt:

- Grouped by project with `##` section headers
- Action-oriented bullet points: what was done, fixed, tested, or shipped
- Ticket/issue references preserved (e.g. `TT-1234`, `PROJ-567`)
- No branch names, file counts, or other git metadata
- Saved as `YYYY-MM-DD.md` in the summaries directory

Example output:

```markdown
# Dev Journal — 2026-03-23

## dev-journal

- Scaffolded Rust project with cargo, wired up all module stubs
- Implemented SQLite database layer with WAL mode for concurrent daemon/CLI access
- Added libgit2-based git poller with incremental commit detection
- Wired up Claude and OpenAI LLM backends with a structured prompt builder
- Shipped full CLI with clap: add, remove, status, log, today, summary, daemon start/stop
```

## Troubleshooting

**No events showing up in `devjournal log`?**
Check that the daemon is running (`devjournal status`). If it started but shows 0 events, wait one poll interval (default 60 seconds) and check again. To inspect daemon output, check the log file:

```bash
# macOS / Linux
cat "$(devjournal daemon logs)"
```

```powershell
# Windows (PowerShell)
Get-Content "$(devjournal daemon logs)"
# or open in an editor:
cursor "$(devjournal daemon logs)"
```

You can also backfill history immediately without the daemon using `devjournal sync`.

**`devjournal today` returns "No activity recorded"?**
The daemon must have polled at least once since you added the repo. Confirm with `devjournal log`. If you want to generate a summary for a past date, the events for that date must already be in the database.

**API key not found error?**
`DEVJOURNAL_API_KEY` in your environment takes precedence over `api_key` in the config file. Make sure it is exported (not just set) in the shell where you run `devjournal today`.

**`daemon stop` times out on Windows?**
`devjournal daemon stop` uses `TerminateProcess` on Windows, which requires the calling process to have sufficient privilege to open the daemon process. If the daemon was started in a different privilege context (e.g., an elevated terminal), the stop command may fail with "access denied". In that case, kill the process manually via Task Manager or `taskkill /PID <pid> /F`, then remove the stale PID file from `%LOCALAPPDATA%\devjournal\devjournal.pid`.

**Daemon already running after a crash?**
If the process died without cleaning up its PID file, `daemon start` will detect the stale file and remove it automatically before starting a new process.

**Config file not found?**
Run `devjournal init` for guided setup, or `devjournal add <path>` to create the config with defaults.

**"no author configured" error on daemon start?**
Add your git author name to `[general]` in the config file: `author = "Your Name"`. It must match your git author name exactly (check with `git log --format='%an' | head -1`).

**Ollama: "Failed to call Ollama API"?**
Ollama must be running before you generate a summary. Start it with `ollama serve`, then verify the model is pulled: `ollama list`. If you are running Ollama on a different machine, set `base_url` in your config to point at it.
