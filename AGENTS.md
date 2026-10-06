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

## Checking UI behaviour

- Even inspection on the real MPD can interrupt playback: `/` then Enter in Queue plays the matched song
  (the coordinator interrupted the playing song this way on 2026-10-06). Use a scratch MPD for inspection,
  or never press Enter on the real one.
- An agent can drive the TUI itself: `herdr tab create --no-focus`, `herdr pane run <pane> rormpc`, then
  `herdr pane send-keys` / `herdr pane wait-output --source visible` / `herdr pane read --source visible`.
  It talks to the real MPD, so undo queue changes (`mpc del`) and close the tab afterwards. To compare with
  the build before a fix, run a backup binary from `~/.cache/rormpc/installed/`.
- With random on, `mpc insert` appends at the end; use `mpc add` + `mpc move` to put a song at a position.
- Features that change playback or replace the queue (Sources…, Up next): test them on a scratch MPD, never the
  user's. A config with `music_directory` = the real library, its own `db_file`, `state_file`, `sticker_file`, a
  copy of the playlists, `port "6650"` and `audio_output { type "null" name "null" }`, started with
  `mpd --no-daemon --stderr <conf>` in the background; `MPD_PORT=6650 mpc update --wait`, then
  `env XDG_STATE_HOME=<scratch> rormpc -a 127.0.0.1:6650` in a herdr tab. Stop it with `kill <pid>` (check with
  `lsof -iTCP:6650`); `mpc kill` did not stop it.
- Don't `export XDG_STATE_HOME` in a shell that then runs `herdr-job`: herdr-job keeps its own state there and
  reports the job as "lost". Pass it to the command with `env` instead.
