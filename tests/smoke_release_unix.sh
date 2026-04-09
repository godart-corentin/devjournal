#!/bin/sh
set -eu

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

assert_contains() {
    haystack=$1
    needle=$2
    label=$3

    if ! printf '%s' "$haystack" | grep -F "$needle" >/dev/null 2>&1; then
        echo "$haystack" >&2
        fail "$label"
    fi
}

sha256_file() {
    file_path=$1

    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file_path" | awk '{print $1}'
        return 0
    fi

    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$file_path" | awk '{print $1}'
        return 0
    fi

    if command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 "$file_path" | awk '{print $NF}'
        return 0
    fi

    fail "No SHA256 tool found"
}

if [ "$#" -ne 1 ]; then
    fail "Usage: tests/smoke_release_unix.sh <release-archive>"
fi

ARCHIVE_PATH=$1
[ -f "$ARCHIVE_PATH" ] || fail "Archive not found: $ARCHIVE_PATH"

command -v git >/dev/null 2>&1 || fail "git is required"
command -v tar >/dev/null 2>&1 || fail "tar is required"
command -v python3 >/dev/null 2>&1 || fail "python3 is required"

WORK_DIR=$(mktemp -d)
INSTALL_DIR="$WORK_DIR/bin"
FIXTURE_DIR="$WORK_DIR/release-fixture"
PORT_FILE="$WORK_DIR/http-port"
SERVER_PID=""
BIN=""

cleanup() {
    if [ -n "$BIN" ] && [ -x "$BIN" ]; then
        "$BIN" stop >/dev/null 2>&1 || true
    fi
    if [ -n "$SERVER_PID" ]; then
        kill "$SERVER_PID" >/dev/null 2>&1 || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    rm -rf "$WORK_DIR"
}

trap cleanup EXIT INT TERM

mkdir -p "$INSTALL_DIR" "$FIXTURE_DIR"

export HOME="$WORK_DIR/home"
export XDG_CONFIG_HOME="$WORK_DIR/xdg-config"
export XDG_DATA_HOME="$WORK_DIR/xdg-data"

mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME"

tar -xzf "$ARCHIVE_PATH" -C "$INSTALL_DIR"
BIN="$INSTALL_DIR/devjournal"
[ -x "$BIN" ] || fail "Extracted archive does not contain an executable devjournal binary"

REPO_DIR="$WORK_DIR/repo"
mkdir -p "$REPO_DIR"

(
    cd "$REPO_DIR"
    if ! git init -b main >/dev/null 2>&1; then
        git init >/dev/null 2>&1
        git checkout -b main >/dev/null 2>&1
    fi
    git config user.name "Smoke Tester"
    git config user.email "smoke@example.com"
    printf '%s\n' '# smoke repo' >README.md
    git add README.md
    git commit -m "smoke commit" >/dev/null 2>&1
)

CONFIG_PATH=$("$BIN" config)
mkdir -p "$(dirname "$CONFIG_PATH")"

cat >"$CONFIG_PATH" <<EOF
[general]
poll_interval_secs = 1
author = "Smoke Tester"

[llm]
provider = "cursor"
model = "gpt-5.4-mini"

[[repos]]
path = "$REPO_DIR"
name = "smoke-repo"
EOF

"$BIN" start >/dev/null
sleep 2
STATUS_RUNNING=$("$BIN" status)
assert_contains "$STATUS_RUNNING" "devjournal daemon: running" "daemon did not report running after start"

"$BIN" stop >/dev/null
STATUS_STOPPED=$("$BIN" status)
assert_contains "$STATUS_STOPPED" "devjournal daemon: not running" "daemon did not report stopped after stop"

SYNC_OUTPUT=$("$BIN" sync 2>&1)
assert_contains "$SYNC_OUTPUT" "Syncing smoke-repo..." "sync command did not run for the fixture repo"

SUMMARY_JSON=$("$BIN" summary --format json)
assert_contains "$SUMMARY_JSON" '"event_type": "commit"' "summary JSON did not contain a commit event"
assert_contains "$SUMMARY_JSON" '"message": "smoke commit"' "summary JSON did not include the fixture commit"

ASSET_NAME=$(basename "$ARCHIVE_PATH")
cp "$ARCHIVE_PATH" "$FIXTURE_DIR/$ASSET_NAME"
CHECKSUM=$(sha256_file "$FIXTURE_DIR/$ASSET_NAME")
printf '%s  %s\n' "$CHECKSUM" "$ASSET_NAME" >"$FIXTURE_DIR/devjournal-checksums.txt"

cat >"$FIXTURE_DIR/release.json" <<EOF
{
  "tag_name": "v9.9.9",
  "assets": [
    {
      "name": "$ASSET_NAME",
      "browser_download_url": "https://example.invalid/$ASSET_NAME"
    },
    {
      "name": "devjournal-checksums.txt",
      "browser_download_url": "https://example.invalid/devjournal-checksums.txt"
    }
  ]
}
EOF

python3 -u - "$FIXTURE_DIR" "$PORT_FILE" <<'PY' &
import functools
import http.server
import pathlib
import socketserver
import sys

root = sys.argv[1]
port_file = pathlib.Path(sys.argv[2])
handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=root)

with socketserver.TCPServer(("127.0.0.1", 0), handler) as httpd:
    port_file.write_text(str(httpd.server_address[1]), encoding="utf-8")
    httpd.serve_forever()
PY
SERVER_PID=$!

for _ in 1 2 3 4 5 6 7 8 9 10; do
    if [ -s "$PORT_FILE" ]; then
        break
    fi
    sleep 1
done

[ -s "$PORT_FILE" ] || fail "HTTP fixture server did not start"
PORT=$(cat "$PORT_FILE")

UPDATE_OUTPUT=$(
    DEVJOURNAL_UPDATE_RELEASE_URL="http://127.0.0.1:$PORT/release.json" \
    DEVJOURNAL_UPDATE_ASSET_BASE_URL="http://127.0.0.1:$PORT" \
        "$BIN" update 2>&1
)
assert_contains "$UPDATE_OUTPUT" "Updated devjournal from" "update command did not report a successful Unix update"

POST_UPDATE_STATUS=$("$BIN" status)
assert_contains "$POST_UPDATE_STATUS" "devjournal daemon: not running" "updated binary was not runnable after self-update"

echo "Unix release smoke passed for $ASSET_NAME"
