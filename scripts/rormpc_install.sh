#!/usr/bin/env bash
# Install this checkout's release build as ~/.cargo/bin/rormpc, keeping the previous binary for rollback.
set -euo pipefail

usage="usage: rormpc_install.sh install|rollback|list

  install   build the release binary, back up the installed one, install the new one
  rollback  put back the most recent backup (the current binary becomes a backup too)
  list      show the installed build and the backups, newest first

Installed binary: \$RORMPC_INSTALLED (default ~/.cargo/bin/rormpc).
Backups: ~/.cache/rormpc/installed/<time>_<revision>, the last 5 are kept.
A backup restores the binary only: config written for newer fork features may need the newer binary."

installed="${RORMPC_INSTALLED:-$HOME/.cargo/bin/rormpc}"
backups="${XDG_CACHE_HOME:-$HOME/.cache}/rormpc/installed"
repo="$(cd "$(dirname "$0")/.." && pwd)"

revision_of() { # "rormpc 3c6f5fa Subject" -> 3c6f5fa
  "$1" version 2>/dev/null | sed -n 's/.*git \(v[^ ]*-g\)\{0,1\}\([0-9a-f]\{7,\}\).*/\2/p' | head -1
}

backup_current() {
  [ -x "$installed" ] || return 0
  mkdir -p "$backups"
  local rev; rev="$(revision_of "$installed")"
  cp -p "$installed" "$backups/$(date +%Y%m%d-%H%M%S)_${rev:-unknown}"
  ls -1t "$backups" | tail -n +6 | while read -r old; do rm -f "$backups/$old"; done
}

case "${1:-}" in
  install)
    (cd "$repo" && cargo build --release -p rmpc)
    backup_current
    install -m 755 "$repo/target/release/rormpc" "$installed"
    echo "installed $("$installed" version | head -1) -> $installed"
    echo "restart running rormpc instances to use it"
    ;;
  rollback)
    latest="$(ls -1t "$backups" 2>/dev/null | head -1)"
    [ -n "$latest" ] || { echo "no backups in $backups" >&2; exit 1; }
    tmp="$(mktemp)"; cp -p "$backups/$latest" "$tmp"; rm -f "$backups/$latest"
    backup_current
    install -m 755 "$tmp" "$installed"; rm -f "$tmp"
    echo "rolled back to $latest"
    ;;
  list)
    [ -x "$installed" ] && echo "installed: $("$installed" version | head -1)"
    ls -1t "$backups" 2>/dev/null | sed 's/^/backup: /'
    ;;
  *) echo "$usage" >&2; exit 2 ;;
esac
