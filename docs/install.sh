#!/bin/sh
# Installs DepRadar on macOS or Linux. Always fetches whatever release is
# currently "latest" on GitHub, so this script doesn't need editing when a
# new version ships. Linux downloads are verified against the release's
# SHA256SUMS file.
#
# Piped straight into `sh` from the landing page one-liner, so this must
# stay POSIX sh — no bashisms (no `pipefail`; the explicit checks below
# already catch a failed curl).
#
# GITHUB_TOKEN (optional) raises the GitHub API rate limit for the lookup.
set -eu

REPO="depradar-app/depradar"
API="${DEPRADAR_RELEASE_API:-https://api.github.com/repos/${REPO}/releases/latest}"

fail() {
  echo "$1" >&2
  exit 1
}

# Escapes nothing fancy: `$1` is always a literal suffix such as `_amd64.deb`.
asset_url() {
  # $1: the END of the asset's file name (so `x.deb` never matches `x.deb.sig`).
  printf '%s' "$release_json" \
    | grep -o "\"browser_download_url\": *\"[^\"]*${1}\"" \
    | head -n1 \
    | sed -E 's/.*"(https?:[^"]+)"/\1/'
}

fetch_release() {
  if [ -n "${GITHUB_TOKEN:-}" ]; then
    curl -fsSL -H "Authorization: Bearer ${GITHUB_TOKEN}" "$API"
  else
    curl -fsSL "$API"
  fi
}

# Checks $1 (a downloaded file) against the release's SHA256SUMS entry for
# the asset name $2. Older releases have no SHA256SUMS: warn and go on.
verify() {
  sums_url="$(asset_url "/SHA256SUMS")"
  if [ -z "$sums_url" ]; then
    echo "Warning: this release has no SHA256SUMS file; the download could not be verified." >&2
    return 0
  fi
  expected="$(curl -fsSL "$sums_url" | awk -v n="$2" '$2 == n || $2 == "*" n { print $1; exit }')"
  [ -n "$expected" ] || fail "No checksum listed for $2 in SHA256SUMS."
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$1" | cut -d' ' -f1)"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$1" | cut -d' ' -f1)"
  else
    echo "Warning: no sha256sum/shasum available; the download could not be verified." >&2
    return 0
  fi
  [ "$actual" = "$expected" ] || fail "Checksum mismatch for $2 (expected $expected, got $actual). Aborting."
  echo "Checksum OK."
}

os="$(uname -s)"

case "$os" in
  Darwin)
    if command -v brew >/dev/null 2>&1; then
      echo "Installing via Homebrew..."
      brew install depradar-app/tap/depradar
    else
      echo "Homebrew not found. Install it from https://brew.sh, then run:" >&2
      echo "  brew install depradar-app/tap/depradar" >&2
      exit 1
    fi
    ;;
  Linux)
    case "$(uname -m)" in
      x86_64 | amd64) deb_arch="amd64"; appimage_arch="amd64" ;;
      aarch64 | arm64) deb_arch="arm64"; appimage_arch="aarch64" ;;
      *) fail "Unsupported CPU architecture: $(uname -m). See https://github.com/${REPO}/releases/latest" ;;
    esac

    release_json="$(fetch_release)" \
      || fail "Could not query GitHub for the latest release (rate limit? set GITHUB_TOKEN, or download it from https://github.com/${REPO}/releases/latest)."

    tmp_dir="$(mktemp -d)"
    trap 'rm -rf "$tmp_dir"' EXIT
    if command -v dpkg >/dev/null 2>&1; then
      url="$(asset_url "_${deb_arch}.deb")"
      [ -n "$url" ] || fail "Could not find a ${deb_arch} .deb asset in the latest release."
      name="$(basename "$url")"
      echo "Downloading $url"
      curl -fsSL "$url" -o "$tmp_dir/$name"
      verify "$tmp_dir/$name" "$name"
      if command -v apt-get >/dev/null 2>&1; then
        sudo apt-get install -y "$tmp_dir/$name"
      else
        sudo dpkg -i "$tmp_dir/$name"
      fi
    else
      url="$(asset_url "_${appimage_arch}.AppImage")"
      [ -n "$url" ] || fail "Could not find an ${appimage_arch} AppImage asset in the latest release."
      name="$(basename "$url")"
      mkdir -p "$HOME/.local/bin"
      echo "Downloading $url"
      curl -fsSL "$url" -o "$tmp_dir/$name"
      verify "$tmp_dir/$name" "$name"
      mv "$tmp_dir/$name" "$HOME/.local/bin/depradar"
      chmod +x "$HOME/.local/bin/depradar"
      echo "Installed to ~/.local/bin/depradar — make sure that directory is on your PATH."
    fi
    ;;
  *)
    fail "Unsupported OS: $os. See https://github.com/${REPO}#install for Windows."
    ;;
esac

echo "DepRadar installed."
