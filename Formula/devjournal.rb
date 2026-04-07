class Devjournal < Formula
  desc "Automatic intelligent work diary for local git repositories"
  homepage "https://github.com/godart-corentin/dev-journal"
  url "https://github.com/godart-corentin/dev-journal/archive/refs/tags/v0.3.0.tar.gz"
  sha256 "2b507a764bf3399480c71c4a09b0c8b85086f2a4e3c490d2b7a52993fdbbeeb4"
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
