#!/usr/bin/env bash
# Test of `rormpc_install.sh install` keeping the companions at the pinned tags, with nothing real touched: a copy
# of the installer in a scratch repository, a fake HOME and XDG directories, and stubs for cargo, uv, ffmpeg,
# launchctl and systemctl that log their calls. Runs on macOS and Linux (CI: installer_smoke.yml).
set -euo pipefail

src="$(cd "$(dirname "$0")" && pwd)/rormpc_install.sh"
work="$(mktemp -d "${TEST_TMPDIR:-${TMPDIR:-/tmp}}/rormpc-install-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
repo="$work/repo" stubs="$work/stubs" calls="$work/calls"
mkdir -p "$repo/scripts" "$stubs" "$work/home/.cargo/bin" "$work/loaded" "$work/tools"
cp "$src" "$repo/scripts/rormpc_install.sh"
pin="$(sed -n 's/^RORMPC_TOOLS_TAG=//p' "$src")"
lb_pin="$(sed -n 's/^RO_LB_TAG=//p' "$src")"

stub() { printf '#!/usr/bin/env bash\necho "%s $*" >> "%s"\n%s\n' "$1" "$calls" "$2" > "$stubs/$1"; chmod +x "$stubs/$1"; }
# cargo build writes a fake rormpc; cargo install puts a fake scrobbler in place
stub cargo "case \"\$1\" in
  build) mkdir -p '$repo/target/release'
         printf '#!/bin/sh\necho \"rormpc 0.1 (git abcdef1)\"\n' > '$repo/target/release/rormpc'
         chmod +x '$repo/target/release/rormpc' ;;
  install) [ \"\${2:-}\" = --list ] && exit 0
           mkdir -p \"\$HOME/.cargo/bin\"
           printf '#!/bin/sh\n[ \"\$1\" = --create-default-config ] && mkdir -p \"\$(dirname \"\$LB_CONFIG\")\" && printf \"[submission]\\\\n\\\\n[mpd]\\\\n\" > \"\$LB_CONFIG\"\nexit 0\n' > \"\$HOME/.cargo/bin/ro-listenbrainz-mpd\"
           chmod +x \"\$HOME/.cargo/bin/ro-listenbrainz-mpd\" ;;
esac"
# uv tool install fails when FAIL_UV is set; the tools are stubs that do what the installer asks of them
stub uv "case \"\$1 \$2\" in
  'tool install') [ -n \"\${FAIL_UV:-}\" ] && { echo 'uv: install failed' >&2; exit 3; }
    for t in musicdb liveplaylist mpd-player hits; do
      printf '#!/bin/sh\n[ \"\$1\" = socket-path ] && echo /fake/socket\nexit 0\n' > '$work/tools/'\$t; chmod +x '$work/tools/'\$t
    done ;;
  'tool dir') echo '$work/tools' ;;
  'tool list') exit 0 ;;
esac"
stub ffmpeg ""
# launchctl: a job is loaded from bootstrap until bootout, and then runs at once
stub launchctl "case \"\$1\" in
  bootstrap) touch '$work/loaded/'\"\$(basename \"\$3\" .plist)\" ;;
  bootout) rm -f '$work/loaded/'\"\${2##*/}\" ;;
  print) [ -e '$work/loaded/'\"\${2##*/}\" ] || exit 113; printf '\tstate = running\n\tpid = 1\n' ;;
esac"
stub systemctl "exit 0"
stub pgrep "exit 1"

if [ "$(uname)" = Darwin ]; then
  lb_config="$work/home/Library/Application Support/listenbrainz-mpd/config.toml"
else
  lb_config="$work/xdg/config/listenbrainz-mpd/config.toml"
fi
run() {  # run ARGS...: the copied installer with the fake environment; output in $work/out
  env -i PATH="$stubs:$(dirname "$(command -v bash)"):/usr/bin:/bin:/usr/sbin:/sbin" HOME="$work/home" \
    XDG_STATE_HOME="$work/xdg/state" XDG_CACHE_HOME="$work/xdg/cache" XDG_CONFIG_HOME="$work/xdg/config" \
    LB_CONFIG="$lb_config" ${FAIL_UV:+FAIL_UV=1} \
    bash "$repo/scripts/rormpc_install.sh" "$@" > "$work/out" 2>&1
}
fail() { echo "FAIL: $*" >&2; echo "--- output:" >&2; cat "$work/out" >&2; exit 1; }
expect() { grep -qxF -- "$1" "$work/out" || fail "expected the line '$1'"; }
record="$work/xdg/state/rormpc/companions"
uv_installs() { grep -c '^uv tool install' "$calls" || true; }

# 1. nothing recorded: install runs the companions step and records the pins
run install || fail "first install exited $?"
expect "companions: none recorded -> $pin, ro-listenbrainz-mpd none recorded -> $lb_pin"
[ "$(uv_installs)" = 1 ] || fail "first install: $(uv_installs) uv installs, want 1"
[ "$(cat "$record")" = "$(printf 'rormpc-tools %s\nro-listenbrainz-mpd %s' "$pin" "$lb_pin")" ] ||
  fail "record after the first install: $(cat "$record")"
[ -x "$work/home/.cargo/bin/rormpc" ] || fail "rormpc not installed"

# 2. at the pin: install touches nothing
: > "$calls"
run install || fail "second install exited $?"
expect "companions: $pin, at the pin"
grep -qE '^(uv tool install|cargo install|launchctl|systemctl)' "$calls" && fail "second install touched the companions: $(cat "$calls")"

# 3. status shows the record and the pins
run status || fail "status exited $?"
expect "companions recorded: rormpc-tools $pin, pin $pin"
expect "companions recorded: ro-listenbrainz-mpd $lb_pin, pin $lb_pin"

# 4. a new pin: install reinstalls and records it
sed -i.bak "s/^RORMPC_TOOLS_TAG=.*/RORMPC_TOOLS_TAG=v9.9.9/" "$repo/scripts/rormpc_install.sh"
: > "$calls"
run install || fail "install after the pin bump exited $?"
expect "companions: $pin -> v9.9.9"
grep -qF "uv tool install --force rormpc-tools @ git+https://github.com/rofrol/rormpc-tools@v9.9.9" "$calls" ||
  fail "no uv install at the new pin: $(cat "$calls")"
grep -qx "rormpc-tools v9.9.9" "$record" || fail "record after the bump: $(cat "$record")"

# 5. a failing companions step: install exits non-zero with the step's error, and the old record stays
sed -i.bak "s/^RORMPC_TOOLS_TAG=.*/RORMPC_TOOLS_TAG=v9.9.10/" "$repo/scripts/rormpc_install.sh"
before="$(cat "$record")"
FAIL_UV=1 run install && fail "install with a failing uv exited 0"
expect "companions: v9.9.9 -> v9.9.10"
expect "uv: install failed"
grep -q "^install: the companions step failed (exit 3" "$work/out" || fail "no failure message"
[ "$(cat "$record")" = "$before" ] || fail "record changed by a failed step: $(cat "$record")"
[ ! -e "$record.tmp" ] || fail "a temporary record was left"

# 6. after the failure, the next install runs the step again
run install || fail "install after the failure exited $?"
expect "companions: v9.9.9 -> v9.9.10"
grep -qx "rormpc-tools v9.9.10" "$record" || fail "record after the retry: $(cat "$record")"

echo "ok: rormpc_install.sh install keeps the companions at the pin"
