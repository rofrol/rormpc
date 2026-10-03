#!/usr/bin/env bash
# Install this checkout's release build as ~/.cargo/bin/rormpc, keeping the previous binary for rollback.
set -euo pipefail

usage="usage: rormpc_install.sh install|rollback|list|companions [--local] [--gap SECONDS]|status

  install     build the release binary, back up the installed one, install the new one
  rollback    put back the most recent backup (the current binary becomes a backup too)
  list        show the installed build and the backups, newest first
  companions  optional background services that work with rormpc closed (see RORMPC.md):
              the ro-listenbrainz-mpd scrobbler (cargo install at a pinned tag; --local: from \$RO_LB_DIR)
              and mpd-gap, seconds of silence between songs (--gap, default 3; 0 removes it; needs uv).
              Writes launchd agents (macOS) or systemd user units (Linux) and (re)starts them.
  status      what of the above is installed and running, and the tools rormpc's features run

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

# ---------------------------------------------------------------- companions

RO_LB_REPO=https://github.com/rofrol/ro-listenbrainz-mpd
RO_LB_TAG=v2.6.0-ro.3
RO_LB_DIR="${RO_LB_DIR:-$HOME/personal_projects/ro-listenbrainz-mpd}"
bin_dir="${RORMPC_BIN:-$HOME/.local/bin}"
if [ "$(uname)" = Darwin ]; then
  lb_config="$HOME/Library/Application Support/listenbrainz-mpd/config.toml"
else
  lb_config="${XDG_CONFIG_HOME:-$HOME/.config}/listenbrainz-mpd/config.toml"
fi
# my rule: a listen is 90% of the song played without a seek; pauses don't matter
listen_rule="listen_fraction = 0.9
listen_max_seconds = 0
listen_uninterrupted = true"

# service NAME COMMAND...: write and (re)start a background service that runs COMMAND, restarted if it exits
service() {
  local name="$1"; shift
  local log="$HOME/Library/Logs/$name.log" args="" a
  if [ "$(uname)" = Darwin ]; then
    local label="io.github.rofrol.rormpc.$name" plist="$HOME/Library/LaunchAgents/io.github.rofrol.rormpc.$name.plist"
    for a in "$@"; do args="$args<string>$a</string>"; done
    mkdir -p "$HOME/Library/LaunchAgents" "$HOME/Library/Logs"
    # ProcessType Standard: Background lets macOS coalesce timers (mpd-gap's silence would stretch)
    cat > "$plist.tmp" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- Written by rormpc scripts/rormpc_install.sh companions; edits are overwritten. -->
<plist version="1.0">
<dict>
	<key>Label</key><string>$label</string>
	<key>ProgramArguments</key><array>$args</array>
	<key>EnvironmentVariables</key><dict><key>PATH</key><string>$bin_dir:$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string></dict>
	<key>StandardOutPath</key><string>$log</string>
	<key>StandardErrorPath</key><string>$log</string>
	<key>RunAtLoad</key><true/>
	<key>KeepAlive</key><true/>
	<key>ThrottleInterval</key><integer>10</integer>
	<key>ProcessType</key><string>Standard</string>
</dict>
</plist>
PLIST
    mv "$plist.tmp" "$plist"
    launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
    launchctl bootstrap "gui/$(id -u)" "$plist"
  else
    local unit="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/rormpc-$name.service"
    mkdir -p "$(dirname "$unit")"
    printf '[Unit]\nDescription=rormpc companion %s\nAfter=network-online.target\n\n[Service]\nExecStart=%s\nRestart=always\nRestartSec=10\n\n[Install]\nWantedBy=default.target\n' \
      "$name" "$*" > "$unit.tmp" && mv "$unit.tmp" "$unit"
    systemctl --user daemon-reload
    systemctl --user enable "rormpc-$name.service"
    systemctl --user restart "rormpc-$name.service"
  fi
  echo "started $name"
}

remove_service() {
  if [ "$(uname)" = Darwin ]; then
    launchctl bootout "gui/$(id -u)/io.github.rofrol.rormpc.$1" 2>/dev/null || true
    rm -f "$HOME/Library/LaunchAgents/io.github.rofrol.rormpc.$1.plist"
  else
    systemctl --user disable --now "rormpc-$1.service" 2>/dev/null || true
    rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/rormpc-$1.service"
  fi
}

service_state() {
  if [ "$(uname)" = Darwin ]; then
    launchctl print "gui/$(id -u)/io.github.rofrol.rormpc.$1" 2>/dev/null | sed -n 's/^\tstate = //p' | grep . || echo "not installed"
  else
    systemctl --user is-active "rormpc-$1.service" 2>/dev/null || true
  fi
}

companions() {
  local local_build="" gap=3
  while [ $# -gt 0 ]; do
    case "$1" in
      --local) local_build=1 ;;
      --gap) gap="$2"; shift ;;
      *) echo "$usage" >&2; exit 2 ;;
    esac
    shift
  done
  if [ -n "$local_build" ]; then
    cargo install --locked --path "$RO_LB_DIR"
  else
    cargo install --locked --git "$RO_LB_REPO" --tag "$RO_LB_TAG"
  fi
  [ -f "$lb_config" ] || "$HOME/.cargo/bin/ro-listenbrainz-mpd" --create-default-config
  if ! grep -qE '^listen_(fraction|max_seconds|uninterrupted)' "$lb_config"; then
    # into [submission], i.e. before the [mpd] table; the token and the rest stay as they are
    awk -v rule="$listen_rule" '/^\[mpd\]/ && !done { print rule "\n"; done = 1 } { print } END { if (!done) print rule }' \
      "$lb_config" > "$lb_config.tmp" && mv "$lb_config.tmp" "$lb_config"
    echo "set the 90%-without-a-seek rule in $lb_config (edit the listen_* lines to change it)"
  fi
  if grep -qE '^[[:space:]]*token(_file)?[[:space:]]*=' "$lb_config" || [ -n "${LISTENBRAINZ_TOKEN:-}" ]; then
    service ro-listenbrainz-mpd "$HOME/.cargo/bin/ro-listenbrainz-mpd"
  else
    echo "not starting the scrobbler: put your ListenBrainz token (listenbrainz.org/settings) into $lb_config, then run this again" >&2
  fi
  for other in $(cargo install --list | sed -n 's/^\(listenbrainz-mpd\) .*/\1/p'); do
    echo "warning: upstream $other is installed too; if it runs, every listen is sent twice" >&2
  done
  if [ "$gap" = 0 ]; then
    remove_service mpd-gap; echo "removed mpd-gap"
  elif command -v uv >/dev/null; then
    mkdir -p "$bin_dir"
    install -m 755 "$repo/scripts/mpd-gap" "$bin_dir/mpd-gap"
    service mpd-gap "$bin_dir/mpd-gap" --seconds "$gap"
  else
    echo "not installing mpd-gap: it needs uv (https://docs.astral.sh/uv/)" >&2
  fi
}

status() {
  [ -x "$installed" ] && echo "rormpc: $("$installed" version | head -1)" || echo "rormpc: not installed"
  cargo install --list 2>/dev/null | grep -E '^(ro-listenbrainz-mpd|listenbrainz-mpd) ' || echo "ro-listenbrainz-mpd: not installed"
  echo "scrobbler service: $(service_state ro-listenbrainz-mpd)"
  echo "mpd-gap service: $(service_state mpd-gap)"
  grep -qE '^[[:space:]]*token(_file)?[[:space:]]*=' "$lb_config" 2>/dev/null && echo "ListenBrainz token: set" || echo "ListenBrainz token: missing in $lb_config"
  echo "listen rule: $(grep -E '^listen_' "$lb_config" 2>/dev/null | tr '\n' ' ')"
  local tool
  for tool in hits musicdb mpc uv; do printf '%s: %s\n' "$tool" "$(command -v "$tool" || echo 'not on PATH')"; done
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
  companions) shift; companions "$@" ;;
  status) status ;;
  *) echo "$usage" >&2; exit 2 ;;
esac
