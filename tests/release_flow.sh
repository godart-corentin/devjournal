#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
SCRIPT="$ROOT_DIR/scripts/release.sh"
. "$ROOT_DIR/scripts/release_text.sh"

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

assert_contains() {
    needle=$1
    file=$2
    if ! grep -F "$needle" "$file" >/dev/null 2>&1; then
        fail "expected '$needle' in $file"
    fi
}

assert_not_contains() {
    needle=$1
    file=$2
    if grep -F "$needle" "$file" >/dev/null 2>&1; then
        fail "did not expect '$needle' in $file"
    fi
}

assert_matches() {
    pattern=$1
    file=$2
    if ! grep -E "$pattern" "$file" >/dev/null 2>&1; then
        fail "expected pattern '$pattern' in $file"
    fi
}

cargo_version() {
    repo_dir=$1
    sed -n 's/^version = "\(.*\)"$/\1/p' "$repo_dir/Cargo.toml" | head -n 1
}

next_patch_version() {
    version=$1
    printf '%s\n' "$version" | awk -F. '{print $1 "." $2 "." ($3 + 1)}'
}

sync_fixture_formula_version() {
    repo_dir=$1
    version=$(cargo_version "$repo_dir")
    perl -0pi -e 's#/refs/tags/v[^"]+\.tar\.gz#/refs/tags/v'"$version"'.tar.gz#' \
        "$repo_dir/Formula/devjournal.rb"
}

make_fixture() {
    fixture_dir=$(mktemp -d)
    cp "$ROOT_DIR/Cargo.toml" "$fixture_dir/Cargo.toml"
    cp "$ROOT_DIR/README.md" "$fixture_dir/README.md"
    if [ -f "$ROOT_DIR/RELEASING.md" ]; then
        cp "$ROOT_DIR/RELEASING.md" "$fixture_dir/RELEASING.md"
    fi
    mkdir -p "$fixture_dir/Formula"
    cp "$ROOT_DIR/Formula/devjournal.rb" "$fixture_dir/Formula/devjournal.rb"
    sync_fixture_formula_version "$fixture_dir"
    printf '%s\n' "$fixture_dir"
}

sha256_file() {
    file_path=$1
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file_path" | awk '{print $1}'
        return 0
    fi

    shasum -a 256 "$file_path" | awk '{print $1}'
}

test_prep_updates_repo_metadata() {
    fixture_dir=$(make_fixture)
    target_version=$(next_patch_version "$(cargo_version "$fixture_dir")")
    "$SCRIPT" prep "$target_version" --repo "$fixture_dir"

    assert_contains "version = \"$target_version\"" "$fixture_dir/Cargo.toml"
    assert_contains "$README_RELEASING_GUIDE_SENTENCE" "$fixture_dir/README.md"
    assert_contains "$README_FORMULA_SOURCE_SENTENCE" "$fixture_dir/README.md"
    assert_not_contains "$README_OUTDATED_TAP_WORDING" "$fixture_dir/README.md"
}

test_prep_rejects_stale_release_docs() {
    fixture_dir=$(make_fixture)
    target_version=$(next_patch_version "$(cargo_version "$fixture_dir")")
    perl -0pi -e 's/\[RELEASING\.md\]\(RELEASING\.md\)/`RELEASING.md`/' "$fixture_dir/README.md"

    if "$SCRIPT" prep "$target_version" --repo "$fixture_dir" >/tmp/release-prep-docs.out 2>&1; then
        fail "prep should reject stale README release guidance"
    fi

    assert_contains 'README is missing the maintainers link to RELEASING.md' /tmp/release-prep-docs.out
}

test_verify_rejects_version_drift() {
    fixture_dir=$(make_fixture)
    perl -0pi -e 's#/refs/tags/v[^"]+\.tar\.gz#/refs/tags/v9.9.9.tar.gz#' \
        "$fixture_dir/Formula/devjournal.rb"

    if "$SCRIPT" verify --repo "$fixture_dir" >/tmp/release-verify.out 2>&1; then
        fail "verify should reject Cargo/formula drift"
    fi

    assert_contains 'Formula version does not match Cargo.toml version' /tmp/release-verify.out
}

test_verify_rejects_roadmap_language() {
    fixture_dir=$(make_fixture)
    perl -0pi -e 's/## Homebrew release flow/## Homebrew roadmap/' "$fixture_dir/README.md"
    printf '\nfuture work\n' >>"$fixture_dir/README.md"

    if "$SCRIPT" verify --repo "$fixture_dir" >/tmp/release-roadmap.out 2>&1; then
        fail "verify should reject stale roadmap wording"
    fi

    assert_contains 'README still contains outdated release-roadmap wording' /tmp/release-roadmap.out
}

test_verify_accepts_user_facing_readme() {
    fixture_dir=$(make_fixture)

    "$SCRIPT" verify --repo "$fixture_dir" >/tmp/release-verify-readme.out 2>&1

    assert_contains 'Release metadata is synchronized for version' /tmp/release-verify-readme.out
}

test_metadata_synced_reports_release_state() {
    fixture_dir=$(make_fixture)

    "$SCRIPT" metadata-synced --repo "$fixture_dir"

    target_version=$(next_patch_version "$(cargo_version "$fixture_dir")")
    "$SCRIPT" prep "$target_version" --repo "$fixture_dir"

    if "$SCRIPT" metadata-synced --repo "$fixture_dir" >/tmp/release-metadata-synced.out 2>&1; then
        fail "metadata-synced should fail after prep until finalize updates the formula"
    fi

    assert_contains 'Formula version does not match Cargo.toml version' /tmp/release-metadata-synced.out
}

test_finalize_rejects_missing_remote_tag() {
    fixture_dir=$(make_fixture)
    target_version=$(next_patch_version "$(cargo_version "$fixture_dir")")
    "$SCRIPT" prep "$target_version" --repo "$fixture_dir"

    if "$SCRIPT" finalize "$target_version" --repo "$fixture_dir" >/tmp/release-finalize.out 2>&1; then
        fail "finalize should reject missing remote tag"
    fi

    assert_contains "Remote tag v$target_version is not available yet" /tmp/release-finalize.out
}

test_finalize_writes_formula_from_published_archive() {
    fixture_dir=$(make_fixture)
    target_version=$(next_patch_version "$(cargo_version "$fixture_dir")")
    "$SCRIPT" prep "$target_version" --repo "$fixture_dir"
    archive_dir=$(mktemp -d)
    archive_path="$archive_dir/v$target_version.tar.gz"
    printf 'release-archive\n' >"$archive_dir/archive.txt"
    tar -czf "$archive_path" -C "$archive_dir" archive.txt
    checksum=$(sha256_file "$archive_path")

    DEVJOURNAL_RELEASE_TEST_ARCHIVE="$archive_path" \
    DEVJOURNAL_RELEASE_TEST_TAG_EXISTS=1 \
        "$SCRIPT" finalize "$target_version" --repo "$fixture_dir"

    assert_contains "version = \"$target_version\"" "$fixture_dir/Cargo.toml"
    assert_contains "v$target_version.tar.gz" "$fixture_dir/Formula/devjournal.rb"
    assert_contains "$checksum" "$fixture_dir/Formula/devjournal.rb"
    assert_matches 'sha256 "[0-9a-f]{64}"' "$fixture_dir/Formula/devjournal.rb"
    assert_contains 'For semantic enrichment, install `sem` as well:' "$fixture_dir/Formula/devjournal.rb"
    assert_contains 'If `sem` is unavailable, devjournal still works and falls back to regular git metadata.' "$fixture_dir/Formula/devjournal.rb"
    assert_contains 'Re-run `devjournal sync` after installing `sem` to backfill richer summaries.' "$fixture_dir/Formula/devjournal.rb"
}

main() {
    test_prep_updates_repo_metadata
    test_prep_rejects_stale_release_docs
    test_verify_rejects_version_drift
    test_verify_rejects_roadmap_language
    test_verify_accepts_user_facing_readme
    test_metadata_synced_reports_release_state
    test_finalize_rejects_missing_remote_tag
    test_finalize_writes_formula_from_published_archive
}

main "$@"
