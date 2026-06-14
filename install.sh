#!/bin/sh
# Mainstage installer.
#
#   curl -fsSL https://raw.githubusercontent.com/colton-mcgraw/mainstage/main/install.sh | sh
#
# Downloads the release binary matching the host platform, verifies its SHA-256
# checksum, and installs `mainstage` and `mainstage-lsp` into a bin directory.
#
# Environment overrides:
#   MAINSTAGE_VERSION   release tag to install (default: latest, e.g. v0.1.0)
#   MAINSTAGE_BIN_DIR   install directory     (default: $HOME/.local/bin)
#   MAINSTAGE_REPO      owner/repo to fetch from (default: colton-mcgraw/mainstage)

set -eu

REPO="${MAINSTAGE_REPO:-colton-mcgraw/mainstage}"
BIN_DIR="${MAINSTAGE_BIN_DIR:-$HOME/.local/bin}"

err() {
	printf 'error: %s\n' "$1" >&2
	exit 1
}

need() {
	command -v "$1" >/dev/null 2>&1 || err "required command not found: $1"
}

# Pick a downloader once, up front.
if command -v curl >/dev/null 2>&1; then
	dl() { curl -fsSL "$1" -o "$2"; }
	fetch() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
	dl() { wget -qO "$2" "$1"; }
	fetch() { wget -qO- "$1"; }
else
	err "need curl or wget to download"
fi

need tar

# Map the host OS/arch to a release target triple.
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
	Linux)
		case "$arch" in
			x86_64 | amd64) target="x86_64-unknown-linux-musl" ;;
			*) err "unsupported Linux architecture: $arch (no prebuilt binary; build from source)" ;;
		esac
		;;
	Darwin)
		case "$arch" in
			x86_64) target="x86_64-apple-darwin" ;;
			arm64 | aarch64) target="aarch64-apple-darwin" ;;
			*) err "unsupported macOS architecture: $arch" ;;
		esac
		;;
	*)
		err "unsupported OS: $os (on Windows use Scoop or winget — see the README)" ;;
esac

# Resolve the version (default: latest release tag).
version="${MAINSTAGE_VERSION:-}"
if [ -z "$version" ]; then
	version="$(fetch "https://api.github.com/repos/$REPO/releases/latest" \
		| grep '"tag_name"' | head -n1 | cut -d'"' -f4)"
	[ -n "$version" ] || err "could not determine the latest release tag; set MAINSTAGE_VERSION"
fi

archive="mainstage-$version-$target.tar.gz"
base="https://github.com/$REPO/releases/download/$version"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

printf 'Downloading %s (%s)...\n' "mainstage $version" "$target"
dl "$base/$archive" "$tmp/$archive" || err "download failed: $base/$archive"
dl "$base/$archive.sha256" "$tmp/$archive.sha256" || err "checksum download failed"

# Verify the SHA-256 checksum.
printf 'Verifying checksum...\n'
expected="$(cut -d' ' -f1 <"$tmp/$archive.sha256")"
if command -v sha256sum >/dev/null 2>&1; then
	actual="$(sha256sum "$tmp/$archive" | cut -d' ' -f1)"
elif command -v shasum >/dev/null 2>&1; then
	actual="$(shasum -a 256 "$tmp/$archive" | cut -d' ' -f1)"
else
	err "need sha256sum or shasum to verify the download"
fi
[ "$expected" = "$actual" ] || err "checksum mismatch (expected $expected, got $actual)"

# Extract and install.
tar -xzf "$tmp/$archive" -C "$tmp"
extracted="$tmp/mainstage-$version-$target"
mkdir -p "$BIN_DIR"
for b in mainstage mainstage-lsp; do
	install -m 0755 "$extracted/$b" "$BIN_DIR/$b" 2>/dev/null \
		|| { cp "$extracted/$b" "$BIN_DIR/$b" && chmod 0755 "$BIN_DIR/$b"; }
done

printf '\nInstalled mainstage and mainstage-lsp to %s\n' "$BIN_DIR"
case ":$PATH:" in
	*":$BIN_DIR:"*) ;;
	*) printf '\nAdd it to your PATH:\n  export PATH="%s:$PATH"\n' "$BIN_DIR" ;;
esac
