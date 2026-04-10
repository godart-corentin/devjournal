# Releasing devjournal

`Cargo.toml` is the canonical release version source. The in-repo [`Formula/devjournal.rb`](Formula/devjournal.rb) is the canonical Homebrew formula that gets copied to the external tap after `finalize`. The user-facing README intentionally links here for maintainer-only release flow details instead of embedding them inline.

## Two-phase release flow

1. Run `scripts/release.sh prep <semver>`.
2. Review the diff, commit the prep changes, and create tag `v<semver>`.
3. Push the branch and the tag.
4. Wait for GitHub Actions to publish the GitHub release archives and `devjournal-checksums.txt`.
5. Run `scripts/release.sh finalize <semver>`.
6. Review and commit the formula refresh.
7. Sync `Formula/devjournal.rb` to [godart-corentin/homebrew-devjournal](https://github.com/godart-corentin/homebrew-devjournal).
8. Run `scripts/release.sh verify` before or after the tap sync to confirm the repo metadata still agrees.

GitHub Actions release workflow sits between `prep` and `finalize`.

## Command reference

- `scripts/release.sh prep <semver>` updates `Cargo.toml` for the next version.
- `scripts/release.sh finalize <semver>` requires the matching remote tag, downloads the published tag archive, computes its SHA256, and rewrites `Formula/devjournal.rb`.
- `scripts/release.sh verify` checks that `Cargo.toml`, `Formula/devjournal.rb`, the README maintainer handoff, and this guide still agree.

## Validation

- Run `cargo fmt --all -- --check`
- Run `cargo clippy --all-targets -- -D warnings`
- Run `cargo test --verbose`
- Run `sh tests/release_flow.sh`
- Run `sh scripts/release.sh metadata-synced && sh scripts/release.sh verify` once `Formula/devjournal.rb` has been refreshed by `finalize`
- Run `brew audit --strict Formula/devjournal.rb` and `brew test devjournal` when Homebrew is available
