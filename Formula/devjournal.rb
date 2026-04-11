class Devjournal < Formula
  desc "Automatic intelligent work diary for local git repositories"
  homepage "https://github.com/godart-corentin/devjournal"
  url "https://github.com/godart-corentin/devjournal/archive/refs/tags/v0.5.0.tar.gz"
  sha256 "1997d9d87861aacf23ec7f7ed071e1e2ce56a4c16097d9ac2cd606f515e16b82"
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
    ENV["HOME"] = testpath.to_s
    ENV.delete("XDG_CONFIG_HOME")
    config_path = shell_output("#{bin}/devjournal config").strip

    expected_path =
      if OS.mac?
        testpath/"Library/Application Support/devjournal/config.toml"
      else
        testpath/".config/devjournal/config.toml"
      end

    assert_equal expected_path.to_s, config_path
  end
end
