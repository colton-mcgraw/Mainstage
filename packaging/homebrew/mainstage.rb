# Homebrew formula for Mainstage.
#
# Intended for a tap (e.g. `colton-mcgraw/homebrew-tap`):
#   brew install colton-mcgraw/tap/mainstage
#
# The `version` and each `sha256` are filled in at release time from the
# checksums attached to the GitHub Release (see .github/workflows/release.yml).
class Mainstage < Formula
  desc "Declarative build and automation language"
  homepage "https://github.com/colton-mcgraw/mainstage"
  version "0.1.0"
  license "LicenseRef-Mainstage-Source-Available"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/colton-mcgraw/mainstage/releases/download/v#{version}/mainstage-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256_AARCH64_APPLE_DARWIN"
    else
      url "https://github.com/colton-mcgraw/mainstage/releases/download/v#{version}/mainstage-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256_X86_64_APPLE_DARWIN"
    end
  end

  on_linux do
    url "https://github.com/colton-mcgraw/mainstage/releases/download/v#{version}/mainstage-v#{version}-x86_64-unknown-linux-musl.tar.gz"
    sha256 "REPLACE_WITH_SHA256_X86_64_LINUX_MUSL"
  end

  def install
    bin.install "mainstage"
    bin.install "mainstage-lsp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/mainstage --version")
  end
end
