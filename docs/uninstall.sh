#!/bin/sh
# Uninstalls DepRadar on macOS or Linux, however it was installed, and
# removes the data it keeps (saved projects, window state).
#
# Piped straight into `sh` from the landing page one-liner, so this must
# stay POSIX sh — no bashisms (no `pipefail`).
set -eu

ID="com.sikander.depradar"
os="$(uname -s)"

case "$os" in
  Darwin)
    if command -v brew >/dev/null 2>&1 && brew list --cask depradar >/dev/null 2>&1; then
      brew uninstall --cask depradar
    else
      # Older builds shipped as depradar.app, current ones as DepRadar.app.
      for app in "/Applications/DepRadar.app" "/Applications/depradar.app"; do
        [ -d "$app" ] && rm -rf "$app"
      done
    fi
    rm -rf "$HOME/Library/Application Support/$ID" \
           "$HOME/Library/Caches/$ID" \
           "$HOME/Library/Saved Application State/$ID.savedState" \
           "$HOME/Library/WebKit/$ID"
    ;;
  Linux)
    if command -v dpkg >/dev/null 2>&1 && dpkg -s depradar >/dev/null 2>&1; then
      sudo dpkg -r depradar
    elif [ -f "$HOME/.local/bin/depradar" ]; then
      rm -f "$HOME/.local/bin/depradar"
    else
      echo "No DepRadar installation found via dpkg or ~/.local/bin/depradar." >&2
      exit 1
    fi
    # Saved projects, cached webview data, window state.
    rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/$ID" \
           "${XDG_CACHE_HOME:-$HOME/.cache}/$ID" \
           "${XDG_CONFIG_HOME:-$HOME/.config}/$ID"
    ;;
  *)
    echo "Unsupported OS: $os. See https://github.com/depradar-app/depradar#uninstall for Windows." >&2
    exit 1
    ;;
esac

echo "DepRadar uninstalled."
