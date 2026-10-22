class Q < Formula
  desc "CLI tool for quick, one-shot Gemini queries with no API key required"
  homepage "https://github.com/jarpex/q"
  version "0.4.0"
  license "MIT"
  head "https://github.com/jarpex/q.git", branch: "main"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/jarpex/q/releases/download/v0.4.0/q-macos-apple-silicon.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    else
      url "https://github.com/jarpex/q/releases/download/v0.4.0/q-macos-intel.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/jarpex/q/releases/download/v0.4.0/q-linux-arm64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    else
      url "https://github.com/jarpex/q/releases/download/v0.4.0/q-linux-amd64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  def install
    if build.head?
      system "cargo", "install", "--path", ".", "--locked", "--root", prefix
    else
      bin.install "q"
    end
    # The man page came with the release after 0.4.0.
    man1.install "man/q.1" if File.exist?("man/q.1")
  end

  test do
    assert_match "q", shell_output("#{bin}/q --version")
  end
end
