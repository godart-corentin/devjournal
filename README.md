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

| Purpose        | Path                                                      |
| -------------- | --------------------------------------------------------- |
| Config         | `~/Library/Application Support/devjournal/config.toml` (macOS) / `~/.config/devjournal/config.toml` (Linux) |
| Database       | `~/.local/share/devjournal/events.db`                     |
| PID file       | `~/.local/share/devjournal/devjournal.pid`                |
| Summaries      | `~/.local/share/devjournal/summaries/YYYY-MM-DD.md`       |

## Install

Build from source (requires Rust):

```bash
git clone <repo-url> ~/dev-journal
cd ~/dev-journal
cargo build --release
```

Copy the binary somewhere on your `PATH`:

```bash
cp target/release/devjournal ~/.local/bin/devjournal
```

## Setup

**1. Add a repository to watch:**

```bash
devjournal add /path/to/your/repo --name my-project
```

This creates the config file on first run. You can add as many repos as you like.

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
| `devjournal add <path>`          | Add a git repository to the watch list                   |
| `devjournal remove <path>`       | Remove a repository from the watch list                  |
| `devjournal daemon start`        | Start the background polling daemon                      |
| `devjournal daemon stop`         | Stop the daemon                                          |
| `devjournal status`              | Show daemon state, watched repos, and today's event count |
| `devjournal today`               | Generate and print today's summary                       |
| `devjournal summary [YYYY-MM-DD]`| Generate and print the summary for a specific date       |
| `devjournal log [YYYY-MM-DD]`    | Show raw recorded events (useful for debugging)          |
| `devjournal list`                | List all watched repositories                            |
| `devjournal config`              | Print the path to the config file                        |

The `add` command accepts an optional `--name` flag to give the repo a display name used in summaries:

```bash
devjournal add /Users/tylia/workspace/perso/dev-journal --name dev-journal
```

## Configuration

The config file is TOML. It is created automatically the first time you run `devjournal add`. You can edit it directly:

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
| `repos[].name`        | —                    | Falls back to the full path if not set            |

### First poll behaviour

On the very first poll of a repo, devjournal records only the most recent commit (HEAD), not the entire history. Subsequent polls record all new commits since the last seen hash.

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
Check that the daemon is running (`devjournal status`). If it started but shows 0 events, wait one poll interval (default 60 seconds) and check again. The daemon logs to stderr — redirect it to a file if you need to inspect it:

```bash
devjournal daemon stop
devjournal --daemon-mode 2>/tmp/devjournal.log &
```

**`devjournal today` returns "No activity recorded"?**
The daemon must have polled at least once since you added the repo. Confirm with `devjournal log`. If you want to generate a summary for a past date, the events for that date must already be in the database.

**API key not found error?**
`DEVJOURNAL_API_KEY` in your environment takes precedence over `api_key` in the config file. Make sure it is exported (not just set) in the shell where you run `devjournal today`.

**Daemon already running after a crash?**
If the process died without cleaning up its PID file, `daemon start` will detect the stale file and remove it automatically before starting a new process.

**Config file not found?**
Run `devjournal add <path>` — this creates the config file with defaults if it does not exist yet.

**"no author configured" error on daemon start?**
Add your git author name to `[general]` in the config file: `author = "Your Name"`. It must match your git author name exactly (check with `git log --format='%an' | head -1`).

**Ollama: "Failed to call Ollama API"?**
Ollama must be running before you generate a summary. Start it with `ollama serve`, then verify the model is pulled: `ollama list`. If you are running Ollama on a different machine, set `base_url` in your config to point at it.
