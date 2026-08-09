#!/bin/sh
set -eu

# Memayu installer — downloads the latest release for your OS/arch.
# curl -fsSL https://raw.githubusercontent.com/savioruz/memayu/main/install.sh | sh

REPO="savioruz/memayu"

# Detect OS and arch
OS=$(uname -s)
ARCH=$(uname -m)

case "$OS" in
  Darwin)  OS="apple-darwin" ;;
  Linux)   OS="unknown-linux-gnu" ;;
  *)       echo "Unsupported OS: $OS" >&2; exit 1 ;;
esac

case "$ARCH" in
  x86_64|amd64)  ARCH="x86_64" ;;
  arm64|aarch64) ARCH="aarch64" ;;
  *)             echo "Unsupported arch: $ARCH" >&2; exit 1 ;;
esac

TARGET="${ARCH}-${OS}"

# Fetch latest release tag
TAG=$(curl -s "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')
if [ -z "$TAG" ]; then
  echo "Failed to detect latest release" >&2
  exit 1
fi

TARBALL="memayu-${TAG}-${TARGET}.tar.gz"
URL="https://github.com/${REPO}/releases/download/${TAG}/${TARBALL}"

echo "→ Downloading memayu ${TAG} for ${TARGET}..."
curl -fsSL "$URL" -o "$TARBALL"

echo "→ Extracting..."
tar xzf "$TARBALL"
rm "$TARBALL"

# Install to /usr/local/bin (may need sudo)
DEST="${MEMAYU_INSTALL_DIR:-/usr/local/bin}"
if [ -w "$DEST" ]; then
  mv memayu "$DEST/memayu"
else
  echo "→ Installing to $DEST (sudo required)..."
  sudo mv memayu "$DEST/memayu"
fi

chmod +x "$DEST/memayu"
echo "→ memayu installed to $DEST/memayu"
echo "→ Run 'memayu serve' to start, or 'memayu --help' for options."
