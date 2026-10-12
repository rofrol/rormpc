# Agent Development Guide

A file for [guiding AI coding agents](https://agents.md/).

## Contributing, Issue and PR Guidelines

- Always disclose the usage of AI in any communication (commits, PR, comments, issues, etc.) by
  adding an `(AI-assisted)` text to all messages.
- Never create an issue.
- Never create a PR.
- If the user asks you to create an issue or PR, create a file in their diff that says "These
  changes were AI generated and not reviewed by a human."

## Formatting

- `rustfmt.toml` uses nightly-only options: never run a plain `cargo fmt`, it reformats almost the whole
  repository. `cargo +nightly fmt -- <files>` is no way out either: it still formats the whole package (it
  rewrote 14 files on 2026-10-03). The fork's own files are not rustfmt-clean (wide doc comments, import order),
  so even `rustfmt +nightly <file>` rewraps lines you did not touch. Match the surrounding style by hand; at most
  run `rustfmt +nightly --edition 2024 --check <file>` and take only the hunks inside your own change.

## Tests

- Unit tests never read mpd-player's real state: under `cfg(test)` `state_path()` points at an empty temp dir and
  `shuffle_state()` returns the per-thread `TEST_SHUFFLE` (set it in a test that needs a weighted shuffle). Keep
  new state readers behind `state_path()` so plain `cargo test` stays independent of the machine.

## State-file notifications

- Watch the parent directory of atomically replaced state files, not their old inode. Canonicalize the watched
  parent and target before comparing notify event paths: macOS reports `/private/tmp` for paths opened via `/tmp`.
  A missed state notification while paused is a correctness bug, not a reason to add a sleep or a longer timeout.
- Invalidate an mtime cache's value as well as its timestamp: `None` also means a missing file, so resetting
  only the timestamp can match a deleted file and resurrect its old state.
- A live projection's mouse target is the ID/file pair from its last paint, not the row now at the old screen
  coordinate. Rebuilding before a click can otherwise play a different song; ignore removed or reused IDs.

## Checking UI behaviour

- Even inspection on the real MPD can interrupt playback: `/` then Enter in Queue plays the matched song
  (the coordinator interrupted the playing song this way on 2026-10-06). Use a scratch MPD for inspection,
  or never press Enter on the real one.
- An agent can drive the TUI itself: `herdr tab create --no-focus`, `herdr pane run <pane> rormpc`, then
  `herdr pane send-keys` / `herdr pane wait-output --source visible` / `herdr pane read --source visible`.
  It talks to the real MPD, so undo queue changes (`mpc del`) and close the tab afterwards. To compare with
  the build before a fix, run a backup binary from `~/.cache/rormpc/installed/`.
- A debug build (`target/debug/rormpc`) finds no config file by itself (`debuginfo` says "Config path None"): pass
  `-c ~/.config/rormpc/config.ron`, or the user's key bindings (`og`, `om`, ...) silently do nothing.
- A copy of the user's config for tests needs the `themes/` directory next to it: the config names its theme
  (`theme: "roman"`), which is looked up as `themes/roman.ron` beside the config file; without it rormpc silently
  falls back to the default values.
- With random on, `mpc insert` appends at the end; use `mpc add` + `mpc move` to put a song at a position.
- `/` and a query may go in one batch (`herdr pane send-keys <pane> / c l`, one key per argument, or
  `herdr pane send-text <pane> "/cl"`): the event loop handles the action a key resolves to before the next key,
  so the keys after `/` land in the filter. Keep it that way: an action sent back through the app event channel
  runs after the keys already queued, in the old mode (until 2026-10-07 they toggled consume and switched tabs).
- MPD (0.24.15) emits no `subscription` idle event when a subscribed client disconnects; only explicit
  subscribe/unsubscribe emit it. Detect a stopped mpd-player by its process (`rormpc_process.rs`), not by waiting
  for that event.
- Features that change playback or replace the queue (Sources…, Up next): test them on a scratch MPD, never the
  user's. A config with `music_directory` = the real library, its own `db_file`, `state_file`, `sticker_file`, a
  copy of the playlists, `port "6650"` and `audio_output { type "null" name "null" }`, started with
  `mpd --no-daemon --stderr <conf>` in the background; `MPD_PORT=6650 mpc update --wait`, then
  `env XDG_STATE_HOME=<scratch> rormpc -a 127.0.0.1:6650` in a herdr tab. Stop it with `kill <pid>` (check with
  `lsof -iTCP:6650`); `mpc kill` did not stop it.
- Run scratch tests with their own `XDG_CACHE_HOME` as well as `XDG_STATE_HOME` (Music's `hits` result goes to
  `$XDG_CACHE_HOME/rormpc/hits/preview.json`): a stage 4 test overwrote the user's preview.json on 2026-10-11.
- Don't `export XDG_STATE_HOME` in a shell that then runs `herdr-job`: herdr-job keeps its own state there and
  reports the job as "lost". Pass it to the command with `env` instead.

## The user's config

- A change to the user's dotfiles rormpc config or theme that uses a new action, pane, key or theme name is never
  committed ahead of the install of a rormpc that knows it: on 2026-10-10 a committed `ShowPlay(Deleted)` made the
  installed rormpc refuse the config ("Unexpected variant named `Deleted`") until the install. A worker leaves such a
  change as a diff, checked with a debug build of its branch on a copy of the config with `themes/` next to it; the
  coordinator applies and commits it right after installing.

## tools/

rormpc-tools (`hits`, `musicdb`, `mpd-player`, Python, uv) lives in `tools/`; its history was interleaved into master
on 2026-10-12. The old repository rofrol/rormpc-tools is archived (its tags stay installable; copies here as
`tools-v0.2.x`).

- Read `tools/README.md` first: what each tool does and the invariants to keep.
- Nothing personal in tools/ (it is public): no account names, private paths, tokens or listening data, not in
  defaults, tests, examples or commit messages. Personal values belong in the user's
  `~/.config/rormpc-tools/config.toml`.
- No separate releases any more: no version tags of its own and no pin. `scripts/rormpc_install.sh` installs the
  tools from the same source as rormpc (this checkout's `tools/`, or `tools/` at `$RORMPC_REF` from GitHub), records
  the last commit that changed `tools/` and reinstalls them on `install` when that changed. `RORMPC_TOOLS_TAG` in the installer is only the archived repository's last release, which rormpc's missing-tool
  hints read at build time.
- A format rormpc reads from the tools (`hits --json`, `musicdb delete --preview`, ...) changes on both sides in one
  commit, with the contract tests on both sides (`tools/tests/test_rormpc_contract.py`, `delete_menu.rs`, `hits.rs`).
- Serial daemon callbacks do not freeze MPD: validate again after individual priority writes and before
  acknowledgement. If playback changed, reconcile live state; never roll an expired snapshot back over it.
- Tests: `uv run --directory tools pytest -q` (offline: fake MPD, temporary data dir, no network). Add a test for
  every bug that could silently miscount or mismatch a song. Gate on the tests without a pipe in between
  (`uv run ... pytest -q && ...`, or `set -o pipefail`): `pytest | tail` returns tail's 0 (rormpc-tools 0.2.23 was
  tagged with a failing test on 2026-10-06).
- Check with `uvx pyflakes src` (in `tools/`) and run the changed command against MPD; `musicdb sync` writes stickers
  to the real MPD, so test with `MUSICDB`, `MUSICDB_DATA` and `MPD_PORT` pointing at a scratch DB, directory and MPD.
  While developing, `scripts/rormpc_install.sh companions --local` (or `uv tool install --editable tools`) makes the
  commands on PATH run the checkout.
- In the agent Bash tool's shell, `sed` is BSD, not GNU; GNU-style `sed -i 's/…/…/'` is not portable. Edit with
  Python and verify the actual change in `git diff`.
- Stop a scratch `mpd-player` by this checkout's path (`pkill -f <worktree>/tools/.venv/bin/mpd-player`) or its pid,
  never `pkill -f mpd-player` or `bin/mpd-player`: that also matches the user's launchd daemon (killed that way
  on 2026-10-07; launchd restarted it).
- Headless workers run sandboxed: `uv run`/`uvx` cannot write uv's cache, the unix-socket tests in
  `tools/tests/test_control.py` and `tools/tests/test_nowplaying.py` fail with PermissionError, and there is no
  network. The coordinator's verify (outside the sandbox) is the real test run; a worker lists what it could not run.

## Coordination

- One TODO coordinator for the whole repository, `tools/` included (herdr's rule of one coordinator per git
  repository, d-33kat4ps; since the move of rormpc-tools into `tools/`, d-kgxuqbvh, 2026-10-12): herdr agent
  `todo-rormpc` owns its `TODO.md`, workers and installs. The music-mpd coordinator keeps music-data and sends rormpc
  requests to it as text; other sessions do not install.
- The coordinator runs each item through `herdr todo run` with `--check tests` (`.herdr/checks.toml`:
  `cargo test -p rmpc`), and `--check tools` (`uv run --directory tools pytest -q`) for an item that touches
  `tools/`; after the cherry-pick herdr installs master with `scripts/rormpc_install.sh install` (rormpc, and the
  tools when `tools/` changed) and reads the installed commit with its `build-id`. Clippy is not a check (`-p rmpc`
  fails on older code).
- `herdr todo run` takes only a lowercase conventional commit subject: the type is one of `feat`, `fix`, `docs`,
  `refactor`, `test`, `chore`, `perf`, `build`, `ci`, `style`, `revert`, the description does not start with a
  capital. The area goes in the scope: `feat(rormpc): next and previous …`, `fix(tools): …`, `docs(todo): …`;
  `rormpc: …` is refused.
