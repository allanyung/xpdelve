#!/bin/sh

set -eu

repo="https://github.com/allanyung/xpdelve"
latest_url="$(curl -fsSL -o /dev/null -w '%{url_effective}' "$repo/releases/latest")"
tag="${latest_url##*/}"
version="${tag#v}"

case "$tag" in
  v[0-9]*) ;;
  *)
    printf 'Could not determine the latest xpdelve version from %s\n' "$latest_url" >&2
    exit 1
    ;;
esac

case "$(uname -s)" in
  Linux) os="Linux" ;;
  Darwin) os="Darwin" ;;
  *)
    printf 'Unsupported operating system: %s\n' "$(uname -s)" >&2
    exit 1
    ;;
esac

case "$(uname -m)" in
  x86_64|amd64) arch="x86_64" ;;
  arm64|aarch64) arch="arm64" ;;
  *)
    printf 'Unsupported architecture: %s\n' "$(uname -m)" >&2
    exit 1
    ;;
esac

archive="xpdelve_${version}_${os}_${arch}.tar.gz"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

printf 'Downloading xpdelve %s for %s/%s...\n' "$version" "$os" "$arch"
curl -fsSL "$repo/releases/download/$tag/$archive" -o "$tmpdir/$archive"
tar -xzf "$tmpdir/$archive" -C "$tmpdir"
mkdir -p "$HOME/.local/bin"
install -m 0755 "$tmpdir/xpdelve" "$HOME/.local/bin/xpdelve"

if [ "$os" = "Darwin" ]; then
  xattr -d com.apple.quarantine "$HOME/.local/bin/xpdelve" 2>/dev/null || true
fi

printf 'Installed xpdelve %s to %s/.local/bin/xpdelve\n' "$version" "$HOME"
