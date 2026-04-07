class Devjournal < Formula
  desc "Automatic intelligent work diary for local git repositories"
  homepage "https://github.com/godart-corentin/dev-journal"
  url "https://github.com/godart-corentin/dev-journal/archive/refs/tags/v0.4.0.tar.gz"
  sha256 "8edeba50ce211c593901a9447db339710dfa528b28a6b9828e396bf69f60ffb5"
  license "Apache-2.0"
  head "https://github.com/godart-corentin/dev-journal.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: ".")

    generate_completions_from_executable(bin/"devjournal", "completions", shells: [:bash, :zsh, :fish])
  end

  def caveats
    <<~EOS
      For semantic enrichment, install `sem` as well:
        brew install sem-cli

      If `sem` is unavailable, devjournal still works and falls back to regular git metadata.
      Re-run `devjournal sync` after installing `sem` to backfill richer summaries.
    EOS
  end

  test do
    config_path = shell_output("#{bin}/devjournal config").strip
    assert_match "devjournal", config_path
  end
end
