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
  version "1.0.0"
  license "LicenseRef-Mainstage-Source-Available"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/colton-mcgraw/mainstage/releases/download/v#{version}/mainstage-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "c97609daaf130793ad59b4a256d0608b0cef1858ebea906e78c5424322c9441b"
    else
      url "https://github.com/colton-mcgraw/mainstage/releases/download/v#{version}/mainstage-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "db866d995f9c699af2e137aa90b4580f5b641ca4e3903be285bba04cea7af6b7"
    end
  end

  on_linux do
    url "https://github.com/colton-mcgraw/mainstage/releases/download/v#{version}/mainstage-v#{version}-x86_64-unknown-linux-musl.tar.gz"
    sha256 "4898a91f208a89ba2ff01b27088b184875ae5a7e80c2c962acb3f27bda68b241"
  end

  def install
    bin.install "mainstage"
    bin.install "mainstage-lsp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/mainstage --version")
  end
end
