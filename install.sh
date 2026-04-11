#!/bin/sh
set -eu

REPO="godart-corentin/devjournal"
INSTALL_DIR="${DEVJOURNAL_INSTALL_DIR:-$HOME/.local/bin}"
CHECKSUMS_ASSET="devjournal-checksums.txt"

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
    checksums_url="https://github.com/${REPO}/releases/download/${tag}/${CHECKSUMS_ASSET}"

    echo "Downloading ${asset}..."
    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT

    archive_path="${tmpdir}/${asset}"
    checksums_path="${tmpdir}/${CHECKSUMS_ASSET}"

    curl -fsSL -o "$archive_path" "$download_url"
    curl -fsSL -o "$checksums_path" "$checksums_url"

    expected_checksum=$(read_expected_checksum "$asset" "$checksums_path")
    verify_checksum "$archive_path" "$expected_checksum"

    tar xzf "$archive_path" -C "$tmpdir"

    # Install
    mkdir -p "$INSTALL_DIR"
    mv "$tmpdir/devjournal" "$INSTALL_DIR/devjournal"
    chmod +x "$INSTALL_DIR/devjournal"
    printf '%s\n' 'install.sh' >"$INSTALL_DIR/.devjournal-installed-by-install-sh"

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

read_expected_checksum() {
    asset="$1"
    checksums_path="$2"

    checksum=$(awk -v asset="$asset" '
        $2 == asset || $2 == "*" asset {
            print $1
            exit
        }
    ' "$checksums_path")

    if [ -z "$checksum" ]; then
        err "No checksum found for ${asset}"
    fi

    printf '%s\n' "$checksum"
}

verify_checksum() {
    file_path="$1"
    expected_checksum="$2"
    actual_checksum=$(sha256_file "$file_path")

    if [ "$actual_checksum" != "$expected_checksum" ]; then
        err "Checksum verification failed for ${file_path}"
    fi
}

sha256_file() {
    file_path="$1"

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

    err "No SHA256 tool found (expected one of: sha256sum, shasum, openssl)"
}

main
