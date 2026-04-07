#!/bin/sh
set -eu

REPO="godart-corentin/dev-journal"
INSTALL_DIR="${DEVJOURNAL_INSTALL_DIR:-$HOME/.local/bin}"

main() {
    os=$(uname -s)
    arch=$(uname -m)

    case "$os" in
        Linux)  target_os="unknown-linux-gnu" ;;
        Darwin) target_os="apple-darwin" ;;
        *)      err "Unsupported OS: $os" ;;
    esac

    case "$arch" in
        x86_64)         target_arch="x86_64" ;;
        aarch64|arm64)  target_arch="aarch64" ;;
        *)              err "Unsupported architecture: $arch" ;;
    esac

    target="${target_arch}-${target_os}"
    echo "Detected platform: ${target}"

    # Fetch latest release tag
    echo "Fetching latest release..."
    release_url="https://api.github.com/repos/${REPO}/releases/latest"
    tag=$(curl -fsSL "$release_url" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')

    if [ -z "$tag" ]; then
        err "Could not determine latest release"
    fi

    echo "Latest version: ${tag}"

    # Download and extract
    asset="devjournal-${target}.tar.gz"
    download_url="https://github.com/${REPO}/releases/download/${tag}/${asset}"

    echo "Downloading ${asset}..."
    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT

    curl -fsSL "$download_url" | tar xz -C "$tmpdir"

    # Install
    mkdir -p "$INSTALL_DIR"
    mv "$tmpdir/devjournal" "$INSTALL_DIR/devjournal"
    chmod +x "$INSTALL_DIR/devjournal"

    echo "Installed devjournal ${tag} to ${INSTALL_DIR}/devjournal"
    install_sem "$tmpdir"

    # PATH hint
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) ;;
        *) echo "Note: Add ${INSTALL_DIR} to your PATH if not already done." ;;
    esac
}

install_sem() {
    tmpdir="$1"

    if [ -x "${INSTALL_DIR}/sem" ] || command -v sem >/dev/null 2>&1; then
        echo "sem already available."
        return 0
    fi

    echo "Installing semantic enrichment helper (sem) when possible..."

    if command -v brew >/dev/null 2>&1; then
        if brew install sem-cli; then
            echo "Installed sem via Homebrew."
            return 0
        fi
        echo "Warning: Homebrew sem-cli install failed." >&2
    fi

    if command -v cargo >/dev/null 2>&1; then
        sem_root="${tmpdir}/sem-root"
        if cargo install --root "$sem_root" sem-cli >/dev/null 2>&1; then
            if [ -f "${sem_root}/bin/sem" ]; then
                mv "${sem_root}/bin/sem" "${INSTALL_DIR}/sem"
                chmod +x "${INSTALL_DIR}/sem"
                echo "Installed sem to ${INSTALL_DIR}/sem via cargo."
                return 0
            fi
        fi
        echo "Warning: cargo install sem-cli failed." >&2
    fi

    echo "Warning: sem could not be installed automatically." >&2
    echo "You can continue with reduced-fidelity summaries, or install sem-cli manually and run \`devjournal sync\` later." >&2
}

err() {
    echo "Error: $1" >&2
    exit 1
}

main
