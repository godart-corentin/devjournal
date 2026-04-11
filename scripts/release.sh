#!/bin/sh
set -eu

REPO_SLUG="godart-corentin/devjournal"
FORMULA_PATH="Formula/devjournal.rb"
README_PATH="README.md"
RELEASING_PATH="RELEASING.md"
CARGO_PATH="Cargo.toml"

die() {
    echo "Error: $1" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Usage:
  scripts/release.sh prep <semver> [--repo <path>]
  scripts/release.sh finalize <semver> [--repo <path>]
  scripts/release.sh metadata-synced [--repo <path>]
  scripts/release.sh verify [--repo <path>]
EOF
}

require_file() {
    file_path=$1
    [ -f "$file_path" ] || die "Required file not found: $file_path"
}

normalize_repo() {
    repo_path=$1
    (CDPATH= cd -- "$repo_path" && pwd)
}

validate_semver() {
    version=$1
    printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' \
        || die "Version must be a semver triplet like 1.0.0"
}

cargo_version() {
    sed -n 's/^version = "\(.*\)"$/\1/p' "$1/$CARGO_PATH" | head -n 1
}

formula_version() {
    sed -n 's|^  url "https://github.com/'"$REPO_SLUG"'/archive/refs/tags/v\([0-9][0-9.]*\)\.tar\.gz"$|\1|p' \
        "$1/$FORMULA_PATH"
}

formula_sha() {
    sed -n 's/^  sha256 "\([0-9a-f][0-9a-f]*\)"$/\1/p' "$1/$FORMULA_PATH"
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

    die "No SHA256 tool found (expected one of: sha256sum, shasum, openssl)"
}

set_cargo_version() {
    repo_dir=$1
    version=$2
    require_file "$repo_dir/$CARGO_PATH"
    perl -0pi -e 's/^version = ".*"$/version = "'"$version"'"/m' "$repo_dir/$CARGO_PATH"
}

write_formula() {
    repo_dir=$1
    version=$2
    checksum=$3
    cat >"$repo_dir/$FORMULA_PATH" <<EOF
class Devjournal < Formula
  desc "Automatic intelligent work diary for local git repositories"
  homepage "https://github.com/$REPO_SLUG"
  url "https://github.com/$REPO_SLUG/archive/refs/tags/v$version.tar.gz"
  sha256 "$checksum"
  license "Apache-2.0"
  head "https://github.com/$REPO_SLUG.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: ".")

    generate_completions_from_executable(bin/"devjournal", "completions", shells: [:bash, :zsh, :fish])
  end

  def caveats
    <<~EOS
      For semantic enrichment, install \`sem\` as well:
        brew install sem-cli

      If \`sem\` is unavailable, devjournal still works and falls back to regular git metadata.
      Re-run \`devjournal sync\` after installing \`sem\` to backfill richer summaries.
    EOS
  end

  test do
    config_path = shell_output("#{bin}/devjournal config").strip
    assert_match "devjournal", config_path
  end
end
EOF
}

archive_url() {
    version=$1
    printf 'https://github.com/%s/archive/refs/tags/v%s.tar.gz\n' "$REPO_SLUG" "$version"
}

remote_tag_exists() {
    version=$1
    if [ "${DEVJOURNAL_RELEASE_TEST_TAG_EXISTS:-}" = "1" ]; then
        return 0
    fi

    git ls-remote --exit-code --tags "https://github.com/$REPO_SLUG" "refs/tags/v$version" >/dev/null 2>&1
}

download_archive() {
    version=$1
    output_path=$2

    if [ -n "${DEVJOURNAL_RELEASE_TEST_ARCHIVE:-}" ]; then
        cp "$DEVJOURNAL_RELEASE_TEST_ARCHIVE" "$output_path"
        return 0
    fi

    curl -fsSL -o "$output_path" "$(archive_url "$version")"
}

verify_readme_text() {
    repo_dir=$1
    readme="$repo_dir/$README_PATH"
    require_file "$readme"

    grep -F '## Maintainers' "$readme" >/dev/null 2>&1 \
        || die "README is missing the maintainers handoff section"
    grep -F '[RELEASING.md](RELEASING.md)' "$readme" >/dev/null 2>&1 \
        || die "README is missing the maintainers link to RELEASING.md"
    grep -F 'canonical formula source for releases' "$readme" >/dev/null 2>&1 \
        || die "README is missing the canonical formula-source wording"
    if grep -F 'future `homebrew-core` submission' "$readme" >/dev/null 2>&1 ||
        grep -F '## Homebrew release flow' "$readme" >/dev/null 2>&1 ||
        grep -F '## Homebrew roadmap' "$readme" >/dev/null 2>&1 ||
        grep -F 'future work' "$readme" >/dev/null 2>&1; then
        die "README still contains outdated release-roadmap wording"
    fi
}

verify_releasing_guide() {
    repo_dir=$1
    guide="$repo_dir/$RELEASING_PATH"
    require_file "$guide"

    grep -F 'scripts/release.sh prep <semver>' "$guide" >/dev/null 2>&1 \
        || die "RELEASING.md is missing the prep command"
    grep -F 'scripts/release.sh finalize <semver>' "$guide" >/dev/null 2>&1 \
        || die "RELEASING.md is missing the finalize command"
    grep -F 'scripts/release.sh verify' "$guide" >/dev/null 2>&1 \
        || die "RELEASING.md is missing the verify command"
    grep -F 'GitHub Actions release workflow sits between `prep` and `finalize`.' "$guide" >/dev/null 2>&1 \
        || die "RELEASING.md is missing the release workflow boundary"
}

verify_formula_state() {
    repo_dir=$1
    cargo_ver=$(cargo_version "$repo_dir")
    formula_ver=$(formula_version "$repo_dir")
    checksum=$(formula_sha "$repo_dir")

    [ -n "$cargo_ver" ] || die "Unable to read Cargo.toml version"
    [ -n "$formula_ver" ] || die "Formula version is missing or malformed"
    [ -n "$checksum" ] || die "Formula checksum is missing or malformed"

    [ "$cargo_ver" = "$formula_ver" ] || die "Formula version does not match Cargo.toml version"
    printf '%s\n' "$checksum" | grep -Eq '^[0-9a-f]{64}$' \
        || die "Formula checksum is missing or malformed"

    expected_url=$(archive_url "$formula_ver")
    grep -F "  url \"$expected_url\"" "$repo_dir/$FORMULA_PATH" >/dev/null 2>&1 \
        || die "Formula URL does not match the expected tag archive"
}

run_verify() {
    repo_dir=$1
    require_file "$repo_dir/$CARGO_PATH"
    require_file "$repo_dir/$FORMULA_PATH"
    verify_formula_state "$repo_dir"
    verify_readme_text "$repo_dir"
    verify_releasing_guide "$repo_dir"
    echo "Release metadata is synchronized for version $(cargo_version "$repo_dir")."
}

run_metadata_synced() {
    repo_dir=$1
    require_file "$repo_dir/$CARGO_PATH"
    require_file "$repo_dir/$FORMULA_PATH"
    verify_formula_state "$repo_dir"
}

run_prep() {
    repo_dir=$1
    version=$2

    require_file "$repo_dir/$CARGO_PATH"
    require_file "$repo_dir/$README_PATH"

    current_version=$(cargo_version "$repo_dir")
    [ -n "$current_version" ] || die "Unable to read Cargo.toml version"

    set_cargo_version "$repo_dir" "$version"

    cat <<EOF
Prepared release metadata for v$version in $repo_dir

Next steps:
  1. Review the diff.
  2. Commit the prep changes.
  3. Create tag v$version.
  4. Push the branch and the tag to trigger GitHub Releases.
  5. After the release archives are published, run:
     scripts/release.sh finalize $version --repo $repo_dir
EOF
}

run_finalize() {
    repo_dir=$1
    version=$2

    require_file "$repo_dir/$CARGO_PATH"
    require_file "$repo_dir/$FORMULA_PATH"

    cargo_ver=$(cargo_version "$repo_dir")
    [ "$cargo_ver" = "$version" ] || die "Cargo.toml version must already be $version before finalize"

    remote_tag_exists "$version" || die "Remote tag v$version is not available yet"

    archive_file=$(mktemp)
    trap 'rm -f "$archive_file"' EXIT INT TERM
    download_archive "$version" "$archive_file"
    checksum=$(sha256_file "$archive_file")
    write_formula "$repo_dir" "$version" "$checksum"
    verify_formula_state "$repo_dir"

    cat <<EOF
Finalized release metadata for v$version in $repo_dir

Next steps:
  1. Review and commit the updated Formula/devjournal.rb.
  2. Sync Formula/devjournal.rb to godart-corentin/homebrew-devjournal.
  3. Run scripts/release.sh verify --repo $repo_dir.
EOF

    trap - EXIT INT TERM
    rm -f "$archive_file"
}

main() {
    [ $# -ge 1 ] || {
        usage
        exit 1
    }

    command_name=$1
    shift
    repo_dir='.'
    version=''

    case "$command_name" in
        prep|finalize)
            [ $# -ge 1 ] || die "$command_name requires a version"
            version=$1
            shift
            validate_semver "$version"
            ;;
        metadata-synced|verify)
            ;;
        *)
            usage
            exit 1
            ;;
    esac

    while [ $# -gt 0 ]; do
        case "$1" in
            --repo)
                [ $# -ge 2 ] || die "--repo requires a path"
                repo_dir=$2
                shift 2
                ;;
            --help|-h)
                usage
                exit 0
                ;;
            *)
                die "Unknown argument: $1"
                ;;
        esac
    done

    repo_dir=$(normalize_repo "$repo_dir")

    case "$command_name" in
        prep)
            run_prep "$repo_dir" "$version"
            ;;
        finalize)
            run_finalize "$repo_dir" "$version"
            ;;
        metadata-synced)
            run_metadata_synced "$repo_dir"
            ;;
        verify)
            run_verify "$repo_dir"
            ;;
    esac
}

main "$@"
