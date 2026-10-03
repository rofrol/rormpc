#!/usr/bin/env bash
# Install this checkout's release build as ~/.cargo/bin/rormpc, keeping the previous binary for rollback.
set -euo pipefail

usage="usage: rormpc_install.sh install|rollback|list|companions [--local] [--gap SECONDS]|status

  install     build the release binary, back up the installed one, install the new one
  rollback    put back the most recent backup (the current binary becomes a backup too)
  list        show the installed build and the backups, newest first
  companions  what rormpc's Hits pane, play counts and delete menu run, and services that work with it closed
              (see RORMPC.md): rormpc-tools (hits, musicdb, mpd-gap; uv tool install at a pinned tag), musicdb update
              hourly, the ro-listenbrainz-mpd scrobbler (cargo install at a pinned tag) and mpd-gap, seconds of
              silence between songs (--gap, default 3; 0 removes it). --local: both from checkouts in
              \$RORMPC_TOOLS_DIR and \$RO_LB_DIR (rormpc-tools editable). Needs uv and cargo.
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
RO_LB_TAG=v2.6.0-ro.4
RO_LB_DIR="${RO_LB_DIR:-$HOME/personal_projects/ro-listenbrainz-mpd}"
RORMPC_TOOLS_REPO=https://github.com/rofrol/rormpc-tools
RORMPC_TOOLS_TAG=v0.1.3
RORMPC_TOOLS_DIR="${RORMPC_TOOLS_DIR:-$HOME/personal_projects/rormpc-tools}"
if [ "$(uname)" = Darwin ]; then
  lb_config="$HOME/Library/Application Support/listenbrainz-mpd/config.toml"
else
  lb_config="${XDG_CONFIG_HOME:-$HOME/.config}/listenbrainz-mpd/config.toml"
fi
# my rule: a listen is 90% of the song played without a seek; pauses don't matter
listen_rule="listen_fraction = 0.9
listen_max_seconds = 0
listen_uninterrupted = true"

# service NAME INTERVAL COMMAND...: write and (re)start a background service that runs COMMAND, every INTERVAL
# seconds, or (INTERVAL "") all the time, restarted if it exits
service() {
  local name="$1" interval="$2"; shift 2
  local log="$HOME/Library/Logs/$name.log" args="" a
  if [ "$(uname)" = Darwin ]; then
    local label="io.github.rofrol.rormpc.$name" plist="$HOME/Library/LaunchAgents/io.github.rofrol.rormpc.$name.plist"
    for a in "$@"; do args="$args<string>$a</string>"; done
    local run="<key>KeepAlive</key><true/>"
    [ -n "$interval" ] && run="<key>StartInterval</key><integer>$interval</integer>"
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
	<key>EnvironmentVariables</key><dict><key>PATH</key><string>$HOME/.local/bin:$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string></dict>
	<key>StandardOutPath</key><string>$log</string>
	<key>StandardErrorPath</key><string>$log</string>
	<key>RunAtLoad</key><true/>
	$run
	<key>ThrottleInterval</key><integer>10</integer>
	<key>ProcessType</key><string>Standard</string>
</dict>
</plist>
PLIST
    mv "$plist.tmp" "$plist"
    launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
    # bootout returns before launchd has removed the job; bootstrapping it meanwhile fails with error 5
    while launchctl print "gui/$(id -u)/$label" >/dev/null 2>&1; do sleep 0.1; done
    launchctl bootstrap "gui/$(id -u)" "$plist"
  else
    local dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user" unit="rormpc-$name"
    mkdir -p "$dir"
    if [ -n "$interval" ]; then
      printf '[Unit]\nDescription=rormpc companion %s\n\n[Service]\nType=oneshot\nExecStart=%s\n' "$name" "$*" > "$dir/$unit.service.tmp"
      printf '[Unit]\nDescription=rormpc companion %s, every %s s\n\n[Timer]\nOnBootSec=60\nOnUnitActiveSec=%s\n\n[Install]\nWantedBy=timers.target\n' \
        "$name" "$interval" "$interval" > "$dir/$unit.timer.tmp" && mv "$dir/$unit.timer.tmp" "$dir/$unit.timer"
    else
      printf '[Unit]\nDescription=rormpc companion %s\nAfter=network-online.target\n\n[Service]\nExecStart=%s\nRestart=always\nRestartSec=10\n\n[Install]\nWantedBy=default.target\n' \
        "$name" "$*" > "$dir/$unit.service.tmp"
    fi
    mv "$dir/$unit.service.tmp" "$dir/$unit.service"
    systemctl --user daemon-reload
    if [ -n "$interval" ]; then
      systemctl --user enable --now "$unit.timer"
      systemctl --user start --no-block "$unit.service"
    else
      systemctl --user enable "$unit.service"
      systemctl --user restart "$unit.service"
    fi
  fi
  echo "started $name"
}

remove_service() {
  if [ "$(uname)" = Darwin ]; then
    launchctl bootout "gui/$(id -u)/io.github.rofrol.rormpc.$1" 2>/dev/null || true
    while launchctl print "gui/$(id -u)/io.github.rofrol.rormpc.$1" >/dev/null 2>&1; do sleep 0.1; done
    rm -f "$HOME/Library/LaunchAgents/io.github.rofrol.rormpc.$1.plist"
  else
    systemctl --user disable --now "rormpc-$1.timer" "rormpc-$1.service" 2>/dev/null || true
    rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/rormpc-$1".{service,timer}
  fi
}

service_state() {
  if [ "$(uname)" = Darwin ]; then
    launchctl print "gui/$(id -u)/io.github.rofrol.rormpc.$1" 2>/dev/null | sed -n 's/^\tstate = //p' | grep . || echo "not installed"
  else
    systemctl --user is-active "rormpc-$1.service" 2>/dev/null || true
    systemctl --user is-active "rormpc-$1.timer" 2>/dev/null | sed 's/^/timer /' || true
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
  command -v uv >/dev/null || { echo "needs uv: https://docs.astral.sh/uv/" >&2; exit 1; }
  if [ -n "$local_build" ]; then
    uv tool install --force --editable "$RORMPC_TOOLS_DIR"
    cargo install --locked --path "$RO_LB_DIR"
  else
    uv tool install --force "rormpc-tools @ git+$RORMPC_TOOLS_REPO@$RORMPC_TOOLS_TAG"
    cargo install --locked --git "$RO_LB_REPO" --tag "$RO_LB_TAG"
  fi
  local tools; tools="$(uv tool dir --bin)"
  service musicdb 3600 "$tools/musicdb" update
  [ -f "$lb_config" ] || "$HOME/.cargo/bin/ro-listenbrainz-mpd" --create-default-config
  if ! grep -qE '^listen_(fraction|max_seconds|uninterrupted)' "$lb_config"; then
    # into [submission], i.e. before the [mpd] table; the token and the rest stay as they are
    awk -v rule="$listen_rule" '/^\[mpd\]/ && !done { print rule "\n"; done = 1 } { print } END { if (!done) print rule }' \
      "$lb_config" > "$lb_config.tmp" && mv "$lb_config.tmp" "$lb_config"
    echo "set the 90%-without-a-seek rule in $lb_config (edit the listen_* lines to change it)"
  fi
  if grep -qE '^[[:space:]]*token(_file)?[[:space:]]*=' "$lb_config" || [ -n "${LISTENBRAINZ_TOKEN:-}" ]; then
    service ro-listenbrainz-mpd "" "$HOME/.cargo/bin/ro-listenbrainz-mpd"
  else
    echo "not starting the scrobbler: put your ListenBrainz token (listenbrainz.org/settings) into $lb_config, then run this again" >&2
  fi
  for other in $(cargo install --list | sed -n 's/^\(listenbrainz-mpd\) .*/\1/p'); do
    echo "warning: upstream $other is installed too; if it runs, every listen is sent twice" >&2
  done
  if [ "$gap" = 0 ]; then
    remove_service mpd-gap; echo "removed mpd-gap"
  else
    service mpd-gap "" "$tools/mpd-gap" --seconds "$gap"
  fi
}

status() {
  [ -x "$installed" ] && echo "rormpc: $("$installed" version | head -1)" || echo "rormpc: not installed"
  uv tool list 2>/dev/null | grep '^rormpc-tools' || echo "rormpc-tools: not installed"
  cargo install --list 2>/dev/null | grep -E '^(ro-listenbrainz-mpd|listenbrainz-mpd) ' || echo "ro-listenbrainz-mpd: not installed"
  echo "musicdb update service: $(service_state musicdb)"
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
