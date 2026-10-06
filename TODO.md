# TODO

## Next: live Queue plan view (approved 2026-10-06)

- [ ] `o` toggles a view-only Queue plan; never sort or move the MPD queue to display it. Show the last two
      plays (`-2`, `-1`, dimmed), `0 ▶` current, `↑n` requests, all ten forecast slots, then an unselectable
      `unplanned · queue order` divider and the remaining songs in MPD order. Label the mode in the panel title.
- [ ] Preserve selection, scrolling and mouse targets by queue song ID across forecast refreshes; break ties
      by MPD position. Disable Next-header sorting in this view. Filtering keeps the original turn numbers.
- [ ] Dim stale forecast numbers and show `stale · Xm` when the daemon is absent or its state is old.
- [ ] J/K reorder requests in the request section; in the forecast they request a version-checked slot swap
      from mpd-player, which alone writes priorities and publishes confirmation. Never move across section
      boundaries or move past/current/unplanned rows. Document patch lifetime and clear it when a patched
      song plays or leaves the queue, or the daemon draws a new forecast.
- [ ] Verify with `cargo test`, `uv run pytest`, `uvx pyflakes src`, and TUI tests only on a scratch MPD.

## Done: Hits works like Queue, Up next as its own tab (2026-10-06)

- [x] Hits behaves like the Queue pane: e.g. Shift+C jumps to the currently playing song, if it is in the
      Hits table.
- [x] Hits is the first tab.
- [x] Up next is one top-level tab "Up next" (the menu action stays "Play next"), not a sub-tab of Hits and
      Queue (user decided 2026-10-06; Sol and MiMo: a view embedded twice means two cursors on one list). It
      lists the songs waiting in Up next: J/K reorder, remove, clear with a confirmation, Enter plays now; no
      "Play next" inside it. Hits and Queue get a key that opens it. Full plan: music-data TODO.md, entry
      "Playback annoyances".

Verified: `cargo test` (934 tests), `uv run pytest` (107 tests), pyflakes, and the TUI on a null-output scratch
MPD. Covered current-song jumps and custom bindings, search preservation, numeric tabs and `gu`, request
reorder/remove/confirmed clear, and failed `playid` while paused: requests remain, the error redraws without
another key, and a later explicit successful play clears it. No playback retry or added polling delay.

## Soon: playing-song indicator in Hits (design only)

- [ ] Consult the models and prepare visual mockups of playing-song indicators in Hits (for example, a
      `▶` beside `0` in the Next column). Show the variants to the user for review before implementing one.

## Soon: missing film score / soundtrack genre

- [ ] Investigate why the film score / soundtrack genre is missing from songs such as "Cast Away Theme"
      by Alan Silvestri, and make that classification available in Hits genre filters.

## "Mute for…" with a countdown

Asked 2026-10-06; planned with GPT-6.1 Sol and MiMo, done 2026-10-06 (rormpc-tools `player/mute.py`, rormpc
`ui/rormpc_mute.rs`, described in RORMPC.md). Mute, not pause: `setvol 0`, playback goes on (songs still count as
listens), the expiry restores the saved volume only if it is still 0 and never presses play. The timer is a module
of mpd-player with a wall-clock deadline in `mute.json`, so it works with rormpc closed, across sleep and across a
daemon restart. Commands on channel `rormpc`: `mute start|extend SECONDS`, `mute unmute` (now), `mute cancel`
(forget the timer, stay muted); every command bumps `generation` and leaves `error`, which rormpc waits for. A
volume set by anyone cancels the timer and stays; a stop or the end of the queue unmutes at once. UI: `om`
(ShowMuteMenu), the Queue menu, or a click on the slider while muted; the slider shows `muted · 12:34` and redraws
every second, also while paused. Tested on a scratch MPD: expiry, extend, unmute, custom `1h30`, `mpc volume`
during the mute, stop, a restart past the deadline.

- [ ] Not tested: a mute that spans an mpd-player gap silence (the gap pauses at 0:00; mute ignores pauses).
- [ ] A queue replaced by Sources… may pass through a stop, which unmutes. Watch whether that happens in use.
- [ ] Maybe later: "Pause for…" (now that one daemon owns the gap, it no longer fights mpd-gap).

## Test report of 2026-10-06 (scratch MPD, music-mpd-8e)

Fixed 2026-10-06: Up next entries skipped past between two wakes of mpd-player (`mpc next` three times in a row,
random on) stayed waiting with a ↑1 badge; now a waiting entry whose MPD priority went back to 0 has started and
counts as played. The Queue menu keeps Weighted shuffle, Silence, Mute for… and Sources… with an empty queue;
"Clear queue…" asks first (Cancel is the default); Sources… leaves out empty playlists; menus are as wide as their
longest label (within the terminal).

- [ ] Same race with random off: entries are moved after the current song, so skipping past them leaves them
      before it. Telling that apart from the user jumping to a later song needs more than positions; not done.
- [ ] mpd-player down while an Up next song plays: it stays waiting. After an MPD restart the re-found entries
      carry no priority yet, so "priority 0 = started" cannot be used at startup.

## Media keys and Now Playing (outside the TUI)

Hardware media keys (play/pause/next/previous) and the system Now Playing widget work on macOS with
[mpd-now-playable](https://git.00dani.me/00dani/mpd-now-playable), a separate daemon that talks to MPD directly, so
it works while rormpc is closed. Nothing in this repository describes it yet.

Current macOS setup:

    uv tool install mpd-now-playable
    mpd-now-playable install-launchagent            # writes ~/Library/LaunchAgents/me.00dani.mpd-now-playable.plist
                                                    # (RunAtLoad, KeepAlive), bootout + bootstrap + kickstart
    mpd-now-playable uninstall-launchagent          # removes it

The plist runs the uv tool venv's Python (`sys.executable -m mpd_now_playable.cli`), so a reinstall of the tool
or a change of its Python can leave launchd restarting a dead path: rerun `install-launchagent --force` after
`uv tool upgrade` / reinstall.

Plan (2026-10-03, after asking GPT-6.1 Sol and MiMo; both said "document first"):

- [ ] Document it in RORMPC.md ("Media keys" section): what it is, the commands above, how to check it
      (`launchctl print gui/$UID/me.00dani.mpd-now-playable`), the upgrade caveat, and that it is independent
      of rormpc.
- [ ] Do not fold it into `rormpc_install.sh install`: that script installs and rolls back the binary, and a
      binary rollback must not touch the service. If a helper is still wanted, make it a separate opt-in
      subcommand (e.g. `rormpc_install.sh media-keys install|status|uninstall`) that only calls the tool's own
      `install-launchagent` / `uninstall-launchagent`, never writes its own plist, and refuses to run a second
      Now Playing source.
- [ ] Linux: MPD has no MPRIS of its own; a bridge exposes it on the session D-Bus. Options: this
      repository's `rmpcd` (Rust, MPRIS server via zbus, off by default: `config.mpris = true` in
      `~/.config/rmpcd/init.lua`; upstream calls rmpcd early stage), `mpd-mpris` (Go; Arch: `pacman -S
      mpd-mpris`, ships a systemd user unit: `systemctl --user enable --now mpd-mpris`) or `mpDris2`. Run
      exactly one of them. GNOME/KDE map media keys to MPRIS themselves; other WMs need `playerctl` bindings.
  - Omarchy (Arch + Hyprland): `default/hypr/bindings/media.lua` binds XF86AudioPlay/Pause/Next/Prev to
    `omarchy-shell media playPause|next|previous`, a Quickshell service that drives the active MPRIS player.
    Omarchy ships no MPD and no MPD bridge, so the keys reach MPD once one bridge above runs. A playing
    browser or mpv can become the active player and take the keys; for MPD-only keys override the bindings
    with `playerctl -p <name from playerctl -l> play-pause|next|previous`. Try rmpcd first, `mpd-mpris`
    as the fallback.
  - Roguix (Guix, Omarchy look): same Hyprland bindings and `playerctl` are installed; MPD and a bridge are
    not. Guix reportedly packages `mpdris2` (and recently `mpdris2-rs`): check with `guix show mpdris2
    mpdris2-rs` on the pinned channel. Otherwise package rmpcd in roguix-channel. Run it as a Guix Home
    Shepherd service (`home-shepherd-service-type`) and check that it lands on the Hyprland session's D-Bus.
- [ ] Rust instead of the Python tool on macOS. Asked GPT-6.1 Sol and MiMo twice on 2026-10-03; both say keep
      mpd-now-playable (it works, 50 MB RSS is not a real cost), and if it is replaced, by a standalone port,
      not inside rmpcd (rmpcd is the Linux zbus/MPRIS daemon; a macOS stack in it doubles its platforms).
      - What there is to port: mpd-now-playable 1.6.2 (MIT) is ~1900 lines of Python, but the macOS part is
        ~110 (`receivers/cocoa/now_playing.py`): `NSApplication` with the accessory activation policy,
        `MPRemoteCommandCenter` handlers for play/pause/toggle/stop/next/previous and
        `changePlaybackPosition` (seek), rate/skip commands disabled, `MPNowPlayingInfoCenter` info and
        playback state, and state forced to Playing at startup so the keys can resume a paused MPD. No app
        bundle, no Info.plist, no audio of its own.
      - Its threading answer: one thread. `corefoundationasyncio.CoreFoundationEventLoop` is an asyncio loop
        that runs on the main thread's CFRunLoop, so MPD I/O and the Cocoa callbacks share it. Rust has no
        such tokio loop, but none is needed: `rmpc-mpd` is a blocking client (std `TcpStream`; `idle`,
        `read_picture`, `albumart`). Main thread: AppKit run loop. One std thread: MPD idle loop, sending
        snapshots to the main queue (GCD `dispatch` main queue); remote command handlers return at once and
        send the command to a second MPD connection.
      - Bindings: `objc2`, `objc2-app-kit`, `objc2-media-player`, `block2` directly, for exact control of
        commands and playback state (souvlaki/playwire hide part of it).
      - Keep: transport commands, seek, title/artist/album/duration/elapsed (on idle events and seeks, no
        1 Hz ticking), artwork with a size cap, reconnect with backoff and a full refresh after reconnect and
        wake, a tiny config (MPD address). Drop: redis/memcached, websockets, MusicBrainz.
      - Watch: forcing Playing at startup can take the Now Playing slot from Music/Spotify/a browser while
        MPD is paused (maybe claim it on the first MPD activity instead); ad-hoc sign the binary for local use.
      - Where it lives (asked Sol and MiMo again, both chose this): a new crate in this workspace, e.g.
        `rormpc-now-playable` (thin macOS binary, target-gated objc2 deps; MPD-state mapping in its own
        module that rmpcd could use later), reusing `rmpc-mpd`. Not a dependency of the TUI: the keys must
        work while rormpc is closed. Not inside rmpcd for now: rmpcd is unused here, early stage, tokio + Lua,
        and edits there would conflict on every rebase. Only the workspace `members` line touches upstream.
      - Daemons on the Mac stay at three: mpd, listenbrainz-mpd, the port (it replaces mpd-now-playable;
        never run both). Do not start rmpcd on macOS and do not move scrobbling into it (its plugin is
        Last.fm, not ListenBrainz).
      - Install: an opt-in `rormpc_install.sh now-playable install|uninstall|rollback`, separate from the
        TUI install; on install it uninstalls mpd-now-playable's LaunchAgent, rollback restores it.
      - The idle thread blocks in `idle` and MPD accepts only `noidle` then, so commands from the keys go
        through a second connection (a command worker), never the idle one.
      - Estimates from the models: a proof of concept in a day or two, as reliable as the Python tool in
        one to two weeks. Spike first: a launchd-started binary that receives play/pause and shows a title.
- [ ] Windows: no plan unless MPD on Windows is actually used (SMTC, e.g. via souvlaki; needs a hidden HWND with a
      message pump).
- [ ] Whatever is documented, check: works with rormpc closed, survives MPD restart and sleep/wake, clears
      stale metadata when playback stops, uninstall leaves MPD alone.

## Modals and the context menu: mouse

Reported 2026-10-03. Causes found in the code:

- No hover in the context menu: `shared/mouse_event.rs` maps `CTMouseEventKind::Moved` to `None`, so motion never
  reaches widgets (crossterm's `EnableMouseCapture` already requests any-motion reports, ?1003).
- A menu entry needs a double click: `MenuModal::handle_mouse_event` only selects on `LeftClick` and confirms on
  `DoubleClick`.
- A click outside a modal does nothing: `Ui::handle_mouse_event` hands every event to `modals.last_mut()` without a
  hit test, and the `Modal` trait has no area.
- No dimmed background: `Ui::render` already has `theme.modal_backdrop` (sets `fg(DarkGray)` on the whole buffer, which
  flattens all colours); the `roman` theme has it off. herdr adds `Modifier::DIM` to every cell instead (colours kept),
  and dims dialogs but not its context menu or navigator.

Plan (asked GPT-6.1 Sol and MiMo on 2026-10-03; both agreed on the points below):

- [x] Menu: hover selects the entry under the cursor, a single left click confirms it. One hit-test function shared by
      hover and click (headers, separators, borders, scrolling); a click on blank space never runs the selected entry.
      Redraw only when the selection changes.
- [x] `Moved`: pass it on only while a modal that wants hover is open; drop it early otherwise, so motion causes no
      redraws or wakeups elsewhere. Check that nothing treats any mouse event as activity.
- [x] Click-through: double clicks are synthesised from two left clicks, so once one click closes the menu the second
      one arrives as `DoubleClick` on the pane underneath and can play/add a song. Reset the double-click tracker
      when a modal closes on a click (or swallow the next click at that position).
- [x] Outside click: an opt-in per modal (e.g. `fn layout(&self, frame: Rect) -> Option<Rect>` computed the same way
      as in `render`, no stored rect, so it is right before the first render and after a resize; `None` means "not
      dismissible"). Menu, select, info, keybinds, outputs, decoders: close (same path as Esc). Input: cancel like Esc,
      never submit. Confirm / destructive modals: never close on an outside click. Swallow the click and wheel events
      outside instead of passing them through.
- [x] Backdrop: switch `modal_backdrop` to `Modifier::DIM` like herdr and enable it in the `roman` theme; decide
      whether the context menu dims (herdr does not). Popups must start from `Clear` or they inherit DIM. Check in
      Ghostty and kitty (`faint-opacity` / `dim_opacity` decide how strong it is) and with bold/reversed selections.
- [x] Order: menu hover + single click with the click-through fix first, outside click second, backdrop last.

Done 2026-10-03: `MouseEventKind::Moved`, `Section::item_at`, `Modal::area` (every modal except the confirmation
one), `Ui::modal_click`; the menu ignores `DoubleClick`; `QueueFindModal::destroy` leaves insert mode. The context
menu is dimmed behind as well: it is a centred popup here, not a menu at the pointer as in herdr. Checked in the
TUI through herdr: hover highlight, one click opens "Show info", a double click outside closes the menu without
playing the row underneath, the backdrop cells carry DIM and the popup does not. Still to check by eye: how strong
DIM looks in Ghostty.

- [x] Hover lag reported 2026-10-03 (the user is not sure it is real). The user runs rormpc in a Ghostty quick
      terminal, not in herdr, so the chain is rormpc frame pacing + Ghostty only. Measured by driving it through
      herdr (inject SGR motion, poll `herdr pane read --ansi`): highlight 14-40 ms after one motion event, 28-57 ms
      after a burst of 301, so no backlog. The 30 fps cap delays only renders closer than one frame apart
      (continuous movement); taps 50 ms apart cannot show it. Done: `max_fps: 60` in the user's config, and
      `core/event_loop.rs` draws at once after a key or mouse event once the event queue is drained (`user_input`);
      `max_fps` still paces background renders. Second move 8 ms after the first at 30 fps: median 29 -> 22 ms
      (about 15 ms of that is the herdr send/poll overhead of the test). Sol and MiMo agreed on the cause; MiMo's
      claim of a `Duration` underflow panic in the pacing is wrong (`checked_sub` / `saturating_sub`).

## Live playlists: paste a playlist URL, download it, check for new tracks

Idea (2026-10-03): a button/modal in rormpc where I paste a playlist URL (YouTube, Spotify, radio.omarchy.com) and
its songs are downloaded; locally I get an MPD playlist. It is "live": it can check the source for new tracks and
ask whether to add them. radio.omarchy.com did not resolve (DNS) on 2026-10-03, so its format is unknown.

Plan after asking GPT-6.1 Sol and MiMo (both agreed on the shape and the first version):

- [ ] Prerequisite: a non-interactive mode for dotfiles `yt-mp3-mb` (e.g. `--batch --json`): no prompts, prints
      the produced paths and the unresolved matches as JSON, resumable without duplicate downloads. Uncertain
      matches stay "needs review" instead of being asked about inside a child process the TUI cannot answer.
- [ ] A new CLI in dotfiles (like `musicdb` / `hits`), e.g. `liveplaylist add|check|accept|reject|list --json`.
      It owns state, source adapters, matching, downloads and writing the MPD playlist; rormpc owns presentation
      and decisions only (a URL input modal and a "Live playlists" pane with accept / reject / accept all), runs
      it with argv (no shell), never blocks the UI thread, can cancel it. Progress as JSONL or a status file
      written atomically (temp + rename), as the Hits pane does.
- [ ] State per subscription in music-data (secrets outside it, logs/progress in ~/.cache): url, kind, MPD
      playlist name, a stable target dir (not the playlist title, which can change), schema version, lock file.
      Per source item: the decision (pending/accepted/rejected, rejects are durable) separate from the job state
      (queued/downloading/needs_match/ready/failed), source position and last-seen time, the local path.
- [ ] Order lives in the .m3u (playlist_directory), not in `NNN` file names. Publish only ready files. Songs
      already in the library are referenced, not downloaded again, but only on a confirmed recording match
      (MBID), never on a loose title match.
- [ ] Removals and reorders upstream: never delete local files and never infer a removal from a failed or partial
      check (yt-dlp YouTube extraction breaks, bot checks, 403s); an id that reappears is reactivated.
- [ ] First version: public YouTube playlists only (`yt-dlp --flat-playlist -J` to list ids cheaply), manual
      check, first import reviewed, batch accept. No timer: nobody answers "add these?" at 4 am; later a launchd
      check may only add pending items and notify.
- [ ] Later, maybe, Spotify. Verified 2026-10-03 in Spotify's February 2026 migration guide: playlist items are
      readable only for playlists the user owns or collaborates on (not arbitrary public URLs), Development Mode
      needs the app owner on Premium and allows 5 users; since Nov 2024 algorithmic and Spotify editorial
      playlists are off-limits to new apps. So: user OAuth (PKCE), own playlists only. Matching to YouTube: ISRC
      or artist + title + duration + version words, always reviewed, never the first `ytsearch` hit (covers,
      live, nightcore, loops); a YouTube rip often fails AcoustID, so "downloaded, unidentified" is a real state.
- [ ] Radio: only if a station publishes a track history (an API or page); ICY metadata brings ads and DJ talk.
      Out of scope until there is a concrete station to look at.

## Tests and GitHub Actions for rormpc, rormpc-tools and ro-listenbrainz-mpd

Today nothing guards these three repositories: rormpc's CI comes from upstream and runs only on pull requests
and by hand (pushes to master run nothing), rormpc-tools has no tests, and the scrobbler fork's deltas are not
tested anywhere (upstream's CI is on Codeberg). Bugs found by hand on 2026-10-03 that tests would have caught:
KeyError 'user_name' on an invalid ListenBrainz token, an uncaught SystemExit that stopped the hourly sync,
the launchd bootout/bootstrap race (error 5), missing libsqlite3-dev on Linux.

Plan (2026-10-03, after asking GPT-6.1 Sol and MiMo; both: tests first, CI second):

- [ ] rormpc-tools: pytest without network, on a temporary DB/data dir and mocked HTTP: invalid token and a
      ListenBrainz outage still let `update` sync and export; play counts, and a local listen counted once with its
      ListenBrainz copy (same timestamp); skips import and the Skipped playlist; delete/undo with the journal;
      mpd-gap state transitions on a fake clock and fake MPD status. Workflow on Ubuntu, ~1-2 min.
- [ ] ro-listenbrainz-mpd: `cargo build` on Ubuntu (catches the apt build dependencies), tests for the listen rule
      (fraction, max seconds, uninterrupted: seek restarts, pause neutral) and the local log lines. ~2-4 min.
- [ ] rormpc: installer smoke test on Ubuntu: fresh user, `loginctl enable-linger`, `XDG_RUNTIME_DIR` and the user
      D-Bus, MPD with a generated tone, fake token and API URL, `rormpc_install.sh companions`, then assert units and
      the musicdb timer are active, the listen_* lines are in the config and the token untouched, `status` output,
      a reinstall. The installer pins released tags, so a smoke test would pass on a broken branch: first add
      overrides (e.g. `RORMPC_TOOLS_REF`, `RO_LB_REF`, or reuse `--local` with checkouts) so CI tests the commit
      under test. ~5-8 min. This is what was done by hand in an OrbStack Ubuntu VM on 2026-10-03.
- [ ] macOS: first a throwaway probe that `launchctl bootstrap gui/$UID` works on hosted runners, then the same
      smoke test with launchd.
- [ ] Triggers: push to master, pull_request, workflow_dispatch, weekly schedule (toolchain, uv and runner-image
      drift; GitHub disables schedules after 60 days without repository activity), and on release tags. Add
      `push` to the upstream ci.yml here too.
- Not automated: live ListenBrainz, MusicBrainz, Billboard and YouTube (OAuth) calls, and exact gap timing on
  shared runners.

## Versions: delete a bad version, find versions from the Queue

- [ ] Versions: delete a bad version, and find versions from the Queue (asked 2026-10-06: "in Versions I can't
  delete a version of a song if I decide it is bad. Or in the Queue right-click find versions, or some marker on the
  song that versions exist"). Facts: Versions file rows offer Preview, Label and "Same recording: keep this file,
  merge the others…" (quarantine + aliases); deleting exists only in Queue/Hits ("Delete library file…", Ctrl-x:
  `musicdb delete --preview`, then `musicdb delete`, Ctrl-y undo, Deleted pane). 63 groups today.
  Consulted 2026-10-06 (GPT-6.1 Sol, MiMo; agreed unless noted):
  - Delete: a "Delete this file…" item on a Versions file row that opens the same Delete library file… modal
    (same preview, Trash, undo, Deleted pane; no second delete path), with its four items: Trash / Trash + delete
    the history / Delete permanently / … + delete the history, where history = the ListenBrainz listens
    (irreversible), the video in my YouTube playlists and the local plays. Plain Trash stays the default. Copy
    keeps the two verbs apart: delete = an unwanted recording, merge = the same recording.
  - Decided 2026-10-06: "Delete this file…" first asks what the file is. "A copy of <other file> (same recording)"
    runs the existing merge for that pair: the copy goes to the quarantine with an alias, and its plays and
    decisions move to the file that stays. "A different recording I don't want" runs the delete; its plays and
    decisions stay with the deleted file, shown as "previously owned, deleted", never moved to a sibling (that
    would claim it is the same recording); with "+ delete the history" they go too (asked 2026-10-06: "maybe ask
    whether it should disappear from ListenBrainz and other places": that is this choice). Undo restores file,
    references and group together, not deleted history.
  - Deleting the last file of a group is allowed; the group leaves the active list (its history stays unless
    "+ delete the history" was chosen); the cursor moves to the next group.
  - Queue: both a context menu item "Find versions…" (plus a key) and a quiet marker. The marker shows only when
    the song's group has more than one owned file (not on every name collision); blank otherwise; membership cached
    once, not queried per row. Decided 2026-10-06: a narrow column with `≋`, blank when there are no versions.
  - The jump opens Versions with the group and the originating file selected; Back/Esc returns to the Queue row
    and scroll position. No autoplay.
  - Pitfalls: re-check the preview before running (the queue or library may have changed), stale group
    membership after a delete, the same file queued twice, background failures shown, never "done" early.

## Lyrics: Polish translation next to the original

- [ ] Asked 2026-10-07: show a Polish translation in the Lyrics tab, the original on the left and Polish on the
      right, both left-aligned; what if there is no translation, the song is Polish, or in another language (e.g.
      Czech)? Facts: lyrics come only from LRCLIB (`musicdb lyrics`, `.lrc`/`.txt` + `index.json` in `lyrics_dir`),
      which has no translations; the pane shows one column, plain `.txt` scrolled by progress.
      Consulted 2026-10-07 (GPT-6.1 Sol, MiMo); decided by the user: source tekstowo.pl first, then an LLM;
      literal, line-by-line translation (for understanding, no rhyme); the current line highlighted.
  - Layout: original left, Polish right, both left-aligned, for any non-Polish original (English, Czech, Italian
    …). A Polish original: one full-width column, no translation. No translation yet: the original full width and
    a short status with the action ("no Polish translation · t: translate"). Narrow terminal (below ~100
    columns): one column and a key that switches original / translation.
  - Current line highlighted in both columns. `.lrc`: the translation inherits the original's timestamps through
    line ids. `.txt`: the line is estimated from the song's progress, as the scroll already is; say it is estimated.
    Only highlight a paired line when the pairing is trustworthy (1:1); a translation whose lines do not match
    (merged or reordered verses, typical for human translations) is aligned by stanza and shown without the
    line highlight on the right.
  - Sources: tekstowo.pl (human translations, no API: an HTML scraper, fragile, check its terms; personal use
    only, never commit the fetched text to a public repo); when it has none, an LLM translates line by line with
    the stanza as context (the lyrics go to an external model; label it "machine translation"); a translation I
    paste or import wins over both. Musixmatch (partner API) and Genius (annotations, not translations) are out.
  - Storage: keep the `.lrc`/`.txt` untouched; a sidecar per song and language, e.g. `<stem>.pl.json`, with the
    source (tekstowo URL / model and version / mine), human or machine, fetch date, a hash of the original lyrics
    (stale when the original changes) and per-line units pointing at the original's line ids (one-to-many
    allowed). Never overwrite my own edits. Language of the original: detected once (whatlang/lingua) and stored,
    with a manual override; short or mixed-language lyrics fool detection.
  - When: on demand, asynchronously, when the Lyrics tab shows a song without one; cached for offline use;
    batch only as an explicit command. Pitfalls: wrong song or version matched on tekstowo, instrumental tracks,
    repeated choruses, timestamp offsets, invented lines from the LLM, and `index.json` having one writer
    (`musicdb lyrics`); keep translations out of it or behind the same writer.

## Previous in the weighted shuffle

- [ ] Asked 2026-10-07: with the weighted shuffle on, the Previous media key bounced between unrelated songs and
      each change was recorded as an early skip. Facts: Hammerspoon runs `mpc prev`; with random on, MPD's
      `previous` goes to the previous song in its own random order, not the one that played before; mpd-player
      records every change of song as finished / early (48 h rest) / late, and ro-listenbrainz-mpd logs the same
      changes in skips.jsonl (imported hourly). A burst of 11 such skips (2026-10-07 00:15-00:17) was removed by
      hand from skips.jsonl and shuffle.json.
      Experiment 2026-10-07 (scratch MPD 0.24.15, 12 songs, random on): plain `previous` walks back exactly through
      the songs that played, also when priorities are reset after every song the way mpd-player does (3 runs). So
      MPD itself is not the cause and a fork of MPD would change nothing (asked "what about forking MPD?",
      consulted Sol and MiMo; Sol leaned to a small MPD patch, MiMo to a protocol proxy; both said test first).
      In the real burst the first second moved forward through three songs (Jennifer Lopez, 34 s → Kings of Leon
      0.2 s → Czesław Śpiewa 0.4 s → No No No), as if something sent `next`/`play` right after the key; only then
      did Previous walk back. Next: reproduce on the scratch MPD with mpd-player running (upnext, shuffle, gap)
      and Hammerspoon + mpd-now-playable, logging every MPD command (MPD `log_level "verbose"`), to find what
      moves forward. M.A.L.P. and any other client send plain `previous`, so the fix must work without the daemon
      seeing the key.
      Consulted 2026-10-07 (GPT-6.1 Sol, MiMo; both agreed unless noted):
  - `shuffle prev` in mpd-player (rormpc-tools); in weighted mode nothing calls MPD's `previous`. Hammerspoon and
    rormpc send it through the daemon's existing command channel; without the daemon, fall back to `mpc prev`
    (knowing it reopens the bug).
  - A cursor over the real playback history: each press goes one song further back; songs reached with Previous
    are not added to that trail; normal forward play starts from the cursor again. By queue id; an entry no longer
    in the queue is skipped over, never re-added; at the start of the history nothing happens. The song starts at
    0:00.
  - "Press within the first seconds restarts the current song": Sol says leave it out at first (it fights
    predictable walking back), MiMo says restart (seek 0, no outcome) when under ~3 s. Undecided.
  - The plan: the song gone back to leaves the plan; the rest keeps its order and is topped up; the song that was
    left is not put back. Up next requests stay first on forward play.
  - Leaving a song with Previous is a neutral outcome: no skip, no rest, no weight change; listening credit already
    earned stays.
  - The scrobbler is a separate process: the daemon logs each prev transition (from, to, time, command id) before
    acting, and `musicdb import-skips` drops a skip that matches one (MiMo: within ±2 s); the daemon's own outcome
    uses the same record. No blanket "ignore the next change" flag: a natural end, Next or a queue edit can race
    with Previous. Persist the intent, then confirm the observed transition before marking it neutral; debounce
    key repeat.
