class Devjournal < Formula
  desc "Automatic intelligent work diary for local git repositories"
  homepage "https://github.com/godart-corentin/devjournal"
  url "https://github.com/godart-corentin/devjournal/archive/refs/tags/v1.1.0.tar.gz"
  sha256 "a982339fa3887bfd1db141d500635a30b404ba766a485adad3184b6bd1679684"
  license "Apache-2.0"
  head "https://github.com/godart-corentin/devjournal.git", branch: "main"

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
