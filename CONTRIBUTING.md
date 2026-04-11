# Contributing

Thanks for your interest in improving `devjournal`.

## Reporting bugs and requesting features

- Search existing issues before opening a new one.
- Open a GitHub issue for bug reports and feature requests.
- For bug reports, include your OS, `devjournal` version, how you installed it, and clear reproduction steps when possible.

## Local setup

1. Clone the repository:

   ```bash
   git clone git@github.com:godart-corentin/devjournal.git
   cd devjournal
   ```

2. Install a current Rust toolchain.
3. Build the project:

   ```bash
   cargo build --release
   ```

4. Run the CLI locally:

   ```bash
   cargo run -- --help
   ```

## Running tests

Before opening a pull request, run the checks that match the changes you made:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --verbose
sh tests/release_flow.sh
```

## Pull request expectations

- Keep pull requests focused on a single change or tightly related set of changes.
- Update documentation and tests when your change affects behavior, workflows, or user-facing output.
- Include enough context in the PR description for reviewers to understand the goal and the validation you ran.
