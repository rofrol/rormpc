#!/usr/bin/env bash
# Install this checkout's release build as ~/.cargo/bin/rormpc, keeping the previous binary for rollback.
set -euo pipefail

usage="usage: rormpc_install.sh install|rollback|list|companions [--local] [--gap SECONDS]|status

  install     build the release binary, back up the installed one, install the new one
  rollback    put back the most recent backup (the current binary becomes a backup too)
  list        show the installed build and the backups, newest first
  companions  what rormpc's Hits pane, play counts and delete menu run, and services that work with it closed
              (see RORMPC.md): rormpc-tools (hits, musicdb, mpd-player; uv tool install at a pinned tag), musicdb
              update hourly, the ro-listenbrainz-mpd scrobbler (cargo install at a pinned tag) and mpd-player, the
              playback daemon (silence between songs: --gap is the default until rormpc chooses one, 0 = none;
              Up next; prints its command socket and a Karabiner rule for media keys; MPD in Now Playing on macOS
              through mpd-player nowplaying, which retires mpd-now-playable, and MPRIS on Linux), and a daily check of Omarchy Radio (new songs wait for review, a notification). --local: both from checkouts in
              \$RORMPC_TOOLS_DIR and \$RO_LB_DIR (rormpc-tools editable). \$RORMPC_TOOLS_REF and \$RO_LB_REF
              (a tag or commit) replace the pinned tags, for CI to test a companion's commit. Needs uv and cargo.
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
RO_LB_TAG=v2.6.0-ro.6
RO_LB_DIR="${RO_LB_DIR:-$HOME/personal_projects/ro-listenbrainz-mpd}"
RORMPC_TOOLS_REPO=https://github.com/rofrol/rormpc-tools
RORMPC_TOOLS_TAG=v0.2.48
deps_url=$RORMPC_TOOLS_REPO#dependencies  # one table: program, what needs it, package per manager
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
# seconds, once a day (INTERVAL daily: at 10:00 on macOS, a run missed while asleep or off follows on wake or at
# login; OnCalendar=daily with Persistent=true on Linux), or (INTERVAL "") all the time, restarted if it exits
service() {
  local name="$1" interval="$2"; shift 2
  local log="$HOME/Library/Logs/$name.log" args="" a
  if [ "$(uname)" = Darwin ]; then
    local label="io.github.rofrol.rormpc.$name" plist="$HOME/Library/LaunchAgents/io.github.rofrol.rormpc.$name.plist"
    for a in "$@"; do args="$args<string>$a</string>"; done
    local run="<key>KeepAlive</key><true/>" priority="<key>ProcessType</key><string>Standard</string>"
    [ -n "$interval" ] && run="<key>StartInterval</key><integer>$interval</integer>"
    [ "$interval" = daily ] && run="<key>StartCalendarInterval</key><dict><key>Hour</key><integer>10</integer><key>Minute</key><integer>0</integer></dict>"
    # always-on services: ProcessType Standard, since Background lets macOS coalesce timers (mpd-player's silence
    # would stretch); periodic jobs (musicdb update) run at background priority with low-priority disk I/O
    [ -n "$interval" ] && priority="<key>ProcessType</key><string>Background</string>
	<key>LowPriorityIO</key><true/>"
    mkdir -p "$HOME/Library/LaunchAgents" "$HOME/Library/Logs"
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
	$priority
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
      # periodic jobs at background priority, like ProcessType Background on macOS
      printf '[Unit]\nDescription=rormpc companion %s\n\n[Service]\nType=oneshot\nExecStart=%s\nNice=10\nIOSchedulingClass=idle\n' "$name" "$*" > "$dir/$unit.service.tmp"
      local when="every $interval s" schedule="OnBootSec=60\nOnUnitActiveSec=$interval"
      [ "$interval" = daily ] && when="daily" schedule="OnCalendar=daily\nPersistent=true"
      printf '[Unit]\nDescription=rormpc companion %s, %s\n\n[Timer]\n%b\n\n[Install]\nWantedBy=timers.target\n' \
        "$name" "$when" "$schedule" > "$dir/$unit.timer.tmp" && mv "$dir/$unit.timer.tmp" "$dir/$unit.timer"
    else
      # dbus.socket: mpd-player's MPRIS needs the session bus at start (ignored where the unit does not exist)
      printf '[Unit]\nDescription=rormpc companion %s\nAfter=network-online.target dbus.socket\n\n[Service]\nExecStart=%s\nRestart=always\nRestartSec=10\n\n[Install]\nWantedBy=default.target\n' \
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
    if [ -f "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/rormpc-$1.timer" ]; then
      echo "timer $(systemctl --user is-active "rormpc-$1.timer" 2>/dev/null)"
    fi
  fi
}

# mpd_player_env MPD_PLAYER ARGS...: run mpd-player with the XDG variables its service gets, so a path it prints is
# the service's: launchd agents get none, systemd user units the user manager's
mpd_player_env() {
  local unset=(-u XDG_STATE_HOME -u XDG_RUNTIME_DIR) set=() line
  if [ "$(uname)" != Darwin ]; then
    while IFS= read -r line; do
      case "$line" in XDG_STATE_HOME=*|XDG_RUNTIME_DIR=*) set+=("$line") ;; esac
    done < <(systemctl --user show-environment 2>/dev/null)
  fi
  env "${unset[@]}" ${set[@]+"${set[@]}"} "$@"
}

# the bare media keys straight to mpd-player's command socket (no process start, mpc or DNS lookup per press), also
# while another app holds macOS's Now Playing; the installer never edits karabiner.json
media_key_hint() {
  local sock
  sock="$(mpd_player_env "$1" socket-path 2>/dev/null)" || {
    echo "mpd-player has no command socket (rormpc-tools older than this script expects)" >&2
    return 0
  }
  echo "mpd-player's command socket: $sock (mpd-player send next|prev|toggle|play|pause|stop)"
  [ "$(uname)" = Darwin ] || return 0
  local key sep="" manipulators=""
  for key in f7:prev f8:toggle f9:next; do
    manipulators="$manipulators$sep
        { \"type\": \"basic\", \"from\": { \"key_code\": \"${key%%:*}\" },
          \"to\": [{ \"send_user_command\": { \"endpoint\": \"$sock\", \"payload\": { \"command\": \"${key#*:}\" } } }] }"
    sep=","
  done
  cat <<HINT
Media keys through Karabiner-Elements (16.0 or newer), if you want the bare F7/F8/F9 to reach MPD even while another
app is the Now Playing app: add this rule to "complex_modifications" > "rules" in ~/.config/karabiner/karabiner.json
(Shift+F7/F8/F9 still go to the Now Playing app):
    { "description": "F7/F8/F9 control MPD through mpd-player's command socket",
      "manipulators": [$manipulators
      ] }
HINT
}

NOW_PLAYABLE_LABEL=me.00dani.mpd-now-playable
# other MPD MPRIS bridges: beside mpd-player's MPRIS both would answer the keys
MPRIS_BRIDGES="mpd-mpris mpDris2 rmpcd"

# macOS: mpd-player nowplaying becomes the Now Playing provider. mpd-now-playable, the provider before it, is retired
# first with its own command (two providers flap the metadata and both answer the keys); the installer never edits
# its plist and never brings it back on its own
now_playing() {
  local player="$1" target plist="$HOME/Library/LaunchAgents/$NOW_PLAYABLE_LABEL.plist" npp
  target="gui/$(id -u)/$NOW_PLAYABLE_LABEL"
  if ! "$player" nowplaying --check >/dev/null 2>&1; then
    echo "warning: '$player nowplaying --check' fails (rormpc-tools older than this script expects, or PyObjC does" \
      "not load); mpd-player's Now Playing is not started and mpd-now-playable is left as it is" >&2
    return 0
  fi
  if launchctl print "$target" >/dev/null 2>&1 || [ -e "$plist" ]; then
    npp="$(command -v mpd-now-playable || echo "$(uv tool dir --bin)/mpd-now-playable")"
    if [ ! -x "$npp" ]; then
      echo "warning: mpd-now-playable's LaunchAgent ($NOW_PLAYABLE_LABEL) is there but not its program; remove it" \
        "('launchctl bootout $target', then delete $plist) and run this again. mpd-player's Now Playing is not" \
        "started beside it." >&2
      return 0
    fi
    "$npp" uninstall-launchagent
    # bootout returns before launchd has removed the job: wait until it is gone
    local _
    for _ in $(seq 100); do
      launchctl print "$target" >/dev/null 2>&1 || break
      sleep 0.1  # delay: polling launchd, which has no event for a removed job; at most 10 s
    done
    if launchctl print "$target" >/dev/null 2>&1 || [ -e "$plist" ]; then
      echo "warning: mpd-now-playable is still loaded or its plist is still there; mpd-player's Now Playing is not" \
        "started beside it. Check with 'launchctl print $target'." >&2
      return 0
    fi
    echo "retired mpd-now-playable (its LaunchAgent is removed; 'uv tool uninstall mpd-now-playable' removes the" \
      "program; 'mpd-now-playable install-launchagent' brings it back)"
  fi
  service mpd-player-nowplaying "" "$player" nowplaying
  local state; state="$(service_state mpd-player-nowplaying)"
  if [ "$state" != running ]; then
    echo "warning: mpd-player's Now Playing is $state, see ~/Library/Logs/mpd-player-nowplaying.log. To go back:" \
      "'launchctl bootout gui/$(id -u)/io.github.rofrol.rormpc.mpd-player-nowplaying'," \
      "then 'mpd-now-playable install-launchagent'" >&2
  fi
}

# Linux: the bridges running now, one per line (mpDris2 is a Python script: matched in the command line, the others
# by program name)
mpris_bridges() {
  local b
  for b in $MPRIS_BRIDGES; do
    if [ "$b" = mpDris2 ]; then
      pgrep -u "$(id -u)" -f -- "/$b( |$)" >/dev/null 2>&1 && echo "$b"
    else
      pgrep -u "$(id -u)" -x -- "$b" >/dev/null 2>&1 && echo "$b"
    fi
  done
  return 0
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
  # before installing anything: the services are launchd agents or systemd user units, nothing else
  if [ "$(uname)" != Darwin ] && ! systemctl --user show-environment >/dev/null 2>&1; then
    echo "companions: supports launchd (macOS) and systemd user units (Linux) only, and finds no systemd user session" \
      "here (Guix System's Shepherd is not supported). With systemd: log in normally, or run" \
      "'sudo loginctl enable-linger \$USER' once on a machine nobody logs into." >&2
    exit 1
  fi
  command -v uv >/dev/null || { echo "needs uv: https://docs.astral.sh/uv/" >&2; exit 1; }
  command -v ffmpeg >/dev/null || {
    echo "companions: ffmpeg not found on PATH; musicdb update hashes the audio with it, yt-mp3-mb converts with it. Install: $deps_url" >&2
    exit 1
  }
  if [ -n "$local_build" ]; then
    uv tool install --force --editable "$RORMPC_TOOLS_DIR"
  else
    uv tool install --force "rormpc-tools @ git+$RORMPC_TOOLS_REPO@${RORMPC_TOOLS_REF:-$RORMPC_TOOLS_TAG}"
  fi
  # apart from uv: set -e is off inside an if that || tests, so a failed uv install went unnoticed there
  if [ -n "$local_build" ]; then
    cargo install --locked --path "$RO_LB_DIR"
  elif [ -n "${RO_LB_REF:-}" ]; then
    cargo install --locked --git "$RO_LB_REPO" --rev "$RO_LB_REF"
  else
    cargo install --locked --git "$RO_LB_REPO" --tag "$RO_LB_TAG"
  fi || {
    echo "companions: building ro-listenbrainz-mpd failed; on Linux it needs a C compiler, pkg-config, OpenSSL and SQLite headers. Install: $deps_url" >&2
    exit 1
  }
  local tools; tools="$(uv tool dir --bin)"
  service musicdb 3600 "$tools/musicdb" update
  # new Omarchy Radio songs only wait for review (pending) and notify; nothing is accepted or downloaded. A failed
  # check is retried by the next day's run. Without an Omarchy Radio subscription it does nothing.
  service omarchy-radio daily "$tools/liveplaylist" check --kind omarchy --notify
  [ -f "$lb_config" ] || "$HOME/.cargo/bin/ro-listenbrainz-mpd" --create-default-config
  if ! grep -qE '^listen_(fraction|max_seconds|uninterrupted)' "$lb_config"; then
    # into [submission], i.e. before the [mpd] table; the token and the rest stay as they are. The rule goes in
    # through the environment: macOS awk rejects a newline in -v
    if rule="$listen_rule" awk 'BEGIN { rule = ENVIRON["rule"] }
        /^\[mpd\]/ && !done { print rule "\n"; done = 1 } { print } END { if (!done) print rule }' \
        "$lb_config" > "$lb_config.tmp" && mv "$lb_config.tmp" "$lb_config"; then
      echo "set the 90%-without-a-seek rule in $lb_config (edit the listen_* lines to change it)"
    else
      rm -f "$lb_config.tmp"
      echo "error: could not add the listen rule to $lb_config; add these lines before [mpd] by hand:" >&2
      echo "$listen_rule" >&2
      exit 1
    fi
  fi
  if grep -qE '^[[:space:]]*token(_file)?[[:space:]]*=' "$lb_config" || [ -n "${LISTENBRAINZ_TOKEN:-}" ]; then
    service ro-listenbrainz-mpd "" "$HOME/.cargo/bin/ro-listenbrainz-mpd"
  else
    echo "not starting the scrobbler: put your ListenBrainz token (listenbrainz.org/settings) into $lb_config, then run this again" >&2
  fi
  for other in $(cargo install --list | sed -n 's/^\(listenbrainz-mpd\) .*/\1/p'); do
    echo "warning: upstream $other is installed too; if it runs, every listen is sent twice" >&2
  done
  # mpd-player replaced mpd-gap (it does the silence and more); --gap is only its default until the length is
  # chosen in rormpc, which it remembers
  remove_service mpd-gap
  service mpd-player "" "$tools/mpd-player" --seconds "$gap"
  media_key_hint "$tools/mpd-player"
  if [ "$(uname)" = Darwin ]; then
    now_playing "$tools/mpd-player"
  else
    local b
    for b in $(mpris_bridges); do
      echo "warning: $b runs; with MPRIS on, it and mpd-player both answer the media keys: stop it" >&2
    done
  fi
}

status() {
  [ -x "$installed" ] && echo "rormpc: $("$installed" version | head -1)" || echo "rormpc: not installed"
  uv tool list 2>/dev/null | grep '^rormpc-tools' || echo "rormpc-tools: not installed"
  cargo install --list 2>/dev/null | grep -E '^(ro-listenbrainz-mpd|listenbrainz-mpd) ' || echo "ro-listenbrainz-mpd: not installed"
  echo "musicdb update service: $(service_state musicdb)"
  echo "Omarchy Radio daily check: $(service_state omarchy-radio)"
  echo "scrobbler service: $(service_state ro-listenbrainz-mpd)"
  echo "mpd-player service: $(service_state mpd-player)"
  local player sock; player="$(uv tool dir --bin 2>/dev/null)/mpd-player"
  if [ -x "$player" ]; then
    sock="$(mpd_player_env "$player" socket-path --check 2>/dev/null)" || true
    echo "mpd-player command socket: ${sock:-none (this mpd-player has no command socket)}"
  fi
  if [ "$(uname)" = Darwin ]; then
    echo "Now Playing (mpd-player nowplaying): $(service_state mpd-player-nowplaying)"
    launchctl print "gui/$(id -u)/$NOW_PLAYABLE_LABEL" >/dev/null 2>&1 &&
      echo "mpd-now-playable: loaded (a second Now Playing provider; companions retires it)"
  else
    local mpris="not owned"
    if ! command -v busctl >/dev/null; then
      mpris="unknown (no busctl)"
    elif busctl --user status org.mpris.MediaPlayer2.mpd_player >/dev/null 2>&1; then
      mpris="owned"
    fi
    echo "mpd-player MPRIS (org.mpris.MediaPlayer2.mpd_player): $mpris"
    local b
    for b in $(mpris_bridges); do echo "other MPD MPRIS bridge running: $b"; done
  fi
  grep -qE '^[[:space:]]*token(_file)?[[:space:]]*=' "$lb_config" 2>/dev/null && echo "ListenBrainz token: set" || echo "ListenBrainz token: missing in $lb_config"
  echo "listen rule: $(grep -E '^listen_' "$lb_config" 2>/dev/null | tr '\n' ' ')"
  local tool
  for tool in hits musicdb mpc uv ffmpeg; do printf '%s: %s\n' "$tool" "$(command -v "$tool" || echo 'not on PATH')"; done
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
