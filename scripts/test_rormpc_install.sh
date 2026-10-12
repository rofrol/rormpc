#!/usr/bin/env bash
# Test of `rormpc_install.sh install` keeping the companions in step with tools/ and the scrobbler's pin, with nothing
# real touched: a copy of the installer in a scratch git repository with a tools/ of its own, a fake HOME and XDG
# directories, and stubs for cargo, uv, ffmpeg, launchctl and systemctl that log their calls. Runs on macOS and Linux
# (CI: installer_smoke.yml).
set -euo pipefail

src="$(cd "$(dirname "$0")" && pwd)/rormpc_install.sh"
work="$(mktemp -d "${TEST_TMPDIR:-${TMPDIR:-/tmp}}/rormpc-install-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
repo="$work/repo" stubs="$work/stubs" calls="$work/calls"
mkdir -p "$repo/scripts" "$repo/tools" "$stubs" "$work/home/.cargo/bin" "$work/loaded" "$work/tools"
cp "$src" "$repo/scripts/rormpc_install.sh"
lb_pin="$(sed -n 's/^RO_LB_TAG=//p' "$src")"
git() { command git -C "$repo" -c user.name=test -c user.email=test@example.invalid "$@"; }
git init -q
printf '[project]\nname = "rormpc-tools"\n' > "$repo/tools/pyproject.toml"
git add -A && git commit -qm tools
tools_at() { echo "tools@$(git log -1 --format=%H -- tools | cut -c1-12)"; }  # the source the installer names

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
    LB_CONFIG="$lb_config" ${FAIL_UV:+FAIL_UV=1} ${RORMPC_REF:+RORMPC_REF="$RORMPC_REF"} \
    bash "$script" "$@" > "$work/out" 2>&1
}
fail() { echo "FAIL: $*" >&2; echo "--- output:" >&2; cat "$work/out" >&2; exit 1; }
expect() { grep -qxF -- "$1" "$work/out" || fail "expected the line '$1'"; }
record="$work/xdg/state/rormpc/companions"
script="$repo/scripts/rormpc_install.sh"
uv_installs() { grep -c '^uv tool install' "$calls" || true; }

# 1. nothing recorded: install runs the companions step from the checkout's tools/ and records its commit
first="$(tools_at)"
run install || fail "first install exited $?"
expect "companions: none recorded -> $first, ro-listenbrainz-mpd none recorded -> $lb_pin"
[ "$(uv_installs)" = 1 ] || fail "first install: $(uv_installs) uv installs, want 1"
grep -qxF "uv tool install --force --reinstall-package rormpc-tools $repo/tools" "$calls" ||
  fail "tools not installed from the checkout: $(cat "$calls")"
[ "$(cat "$record")" = "$(printf 'rormpc-tools %s\nro-listenbrainz-mpd %s' "$first" "$lb_pin")" ] ||
  fail "record after the first install: $(cat "$record")"
[ -x "$work/home/.cargo/bin/rormpc" ] || fail "rormpc not installed"

# 2. tools/ unchanged: install touches nothing, also after a commit outside tools/
echo change > "$repo/README.md"; git add -A; git commit -qm readme
: > "$calls"
run install || fail "second install exited $?"
expect "companions: $first, up to date"
grep -qE '^(uv tool install|cargo install|launchctl|systemctl)' "$calls" && fail "second install touched the companions: $(cat "$calls")"

# 3. status shows the record and what install would install
run status || fail "status exited $?"
expect "companions recorded: rormpc-tools $first, want $first"
expect "companions recorded: ro-listenbrainz-mpd $lb_pin, want $lb_pin"

# 4. a commit that changes tools/: install reinstalls them and records the new commit
echo '# changed' >> "$repo/tools/pyproject.toml"; git commit -qam "tools change"
second="$(tools_at)"
: > "$calls"
run install || fail "install after the tools/ commit exited $?"
expect "companions: $first -> $second"
[ "$(uv_installs)" = 1 ] || fail "install after the tools/ commit: $(uv_installs) uv installs, want 1"
grep -qx "rormpc-tools $second" "$record" || fail "record after the tools/ commit: $(cat "$record")"

# 5. uncommitted changes in tools/ count as a new source, and each different state as another
echo '# dirty' >> "$repo/tools/pyproject.toml"
run install || fail "install with a dirty tools/ exited $?"
grep -q "^companions: $second -> $second+dirty\.[0-9a-f]\{12\}$" "$work/out" || fail "dirty tools/ not reinstalled"
dirty="$(sed -n 's/^rormpc-tools //p' "$record")"
run install || fail "install with the same dirty tools/ exited $?"
expect "companions: $dirty, up to date"
echo new > "$repo/tools/untracked.py"
run install || fail "install with an untracked file in tools/ exited $?"
grep -qx "rormpc-tools $dirty" "$record" && fail "an untracked file in tools/ did not change the source"
dirty="$(sed -n 's/^rormpc-tools //p' "$record")"
rm "$repo/tools/untracked.py"; git checkout -q -- tools
run install || fail "install with a clean tools/ again exited $?"
expect "companions: $dirty -> $second"
grep -qx "rormpc-tools $second" "$record" || fail "record after cleaning tools/: $(cat "$record")"

# 6. $RORMPC_REF: tools/ of that rormpc commit from GitHub
: > "$calls"
RORMPC_REF=v9.9.9 run install || fail "install with RORMPC_REF exited $?"
expect "companions: $second -> rormpc@v9.9.9"
grep -qxF "uv tool install --force rormpc-tools @ git+https://github.com/rofrol/rormpc@v9.9.9#subdirectory=tools" "$calls" ||
  fail "no uv install of tools/ at the ref: $(cat "$calls")"
grep -qx "rormpc-tools rormpc@v9.9.9" "$record" || fail "record after RORMPC_REF: $(cat "$record")"

# 7. a failing companions step: install exits non-zero with the step's error, and the old record stays
before="$(cat "$record")"
FAIL_UV=1 run install && fail "install with a failing uv exited 0"
expect "companions: rormpc@v9.9.9 -> $second"
expect "uv: install failed"
grep -q "^install: the companions step failed (exit 3" "$work/out" || fail "no failure message"
[ "$(cat "$record")" = "$before" ] || fail "record changed by a failed step: $(cat "$record")"
[ ! -e "$record.tmp" ] || fail "a temporary record was left"

# 8. after the failure, the next install runs the step again
run install || fail "install after the failure exited $?"
expect "companions: rormpc@v9.9.9 -> $second"
grep -qx "rormpc-tools $second" "$record" || fail "record after the retry: $(cat "$record")"

# 9. without a checkout (the script alone, as CI's companions job runs it): tools/ of master from GitHub
mkdir -p "$work/alone"; cp "$src" "$work/alone/rormpc_install.sh"
: > "$calls"
script="$work/alone/rormpc_install.sh" run companions || fail "companions without a checkout exited $?"
grep -qxF "uv tool install --force rormpc-tools @ git+https://github.com/rofrol/rormpc@master#subdirectory=tools" "$calls" ||
  fail "no uv install of master's tools/: $(cat "$calls")"
grep -qx "rormpc-tools rormpc@master" "$record" || fail "record without a checkout: $(cat "$record")"

echo "ok: rormpc_install.sh install keeps the companions in step with tools/ and the pin"
