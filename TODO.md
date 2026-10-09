# TODO

## Next, in order

Triaged 2026-10-07 from the sections below; each item points at its section for the details.

- [x] Film score / soundtrack genre: find why "Cast Away Theme" (Alan Silvestri) has none and make the
      classification available in Hits genre filters ("Soon: missing film score / soundtrack genre").
- [x] Playing-song indicator in Hits: consult the models and prepare mockups of the variants, then ask in "Needs a
      decision" which one to build ("Soon: playing-song indicator in Hits").
- [x] Previous in the weighted shuffle: `shuffle prev` in mpd-player over the real playback history, neutral
      outcome, prev transitions logged for `musicdb import-skips`. Without the "restart in the first seconds" rule
      until it is decided ("Previous in the weighted shuffle"). Done in rormpc-tools 328cff3 (not released).
- [x] Previous: rormpc's Previous and Hammerspoon's media key send `shuffle prev` to mpd-player (channel `rormpc`)
      while it runs, `mpc prev` without it. Done: rormpc afc55ac, dotfiles 22e3b81 (Hammerspoon not reloaded). Both
      send it only when mpd-player is subscribed and its shuffle.json has `trail`, so until the rormpc-tools
      release they keep plain `mpc prev`. mpd-player's `playid` starts playback even when MPD was paused.
- [x] Versions: "Delete this file…" on a Versions file row (copy → merge, other recording → delete), the Queue
      `≋` marker, "Find versions…" with a key and Back to the Queue row ("Versions: delete a bad version…").
      Done in 9164204 (worker; checked on a scratch MPD without deleting or merging anything real).
- [x] Versions: chromaprint comparison within a group, the "Same recording? audio match" suggestion, default file
      to keep, rename "Remove duplicate entries" ("Versions: audio fingerprint finds copies").
      Done in rormpc 2b9b78c, rormpc-tools 21d17a0 (worker; not released).
- [x] Lyrics: Polish translation from tekstowo.pl next to the original, sidecar storage, layouts and highlighting.
      The LLM fallback waits for its decision below ("Lyrics: Polish translation next to the original").
      Done in rormpc 7e6b794, rormpc-tools 69d8201 (worker; tools not released: after v0.2.31).
- [x] Media keys: document mpd-now-playable in RORMPC.md ("Media keys and Now Playing", first item). Done in dacac50.
- [x] Mute: test a mute that spans an mpd-player gap silence on a scratch MPD ("Mute for…"). Done (rormpc-tools 529e2da).
- [x] Tests without CI: the missing rormpc-tools pytest cases and ro-listenbrainz-mpd listen-rule tests, run
      locally ("Tests and GitHub Actions…", first two items).
      Done: rormpc-tools e354b97 (179 tests), ro-listenbrainz-mpd 862c08a (12 tests); no bugs found.
- [x] Up next races: random off (entries skipped past end up before the current song) and mpd-player down while
      an Up next song plays ("Test report of 2026-10-06").
      Done in rormpc-tools 893091c (worker).
- [x] Release rormpc-tools v0.2.32 (Polish lyrics translations, new tests, the Up next fix) and install rormpc
      from master (decided 2026-10-07; the coordinator does it).
      Done: v0.2.32 pushed and installed with `companions`, rormpc 53751e0 installed.
- [x] Hits playing-song indicator, variant B: the playing row in `highlighted_item_style` and `▶0` in Next
      (decided 2026-10-07; mockups and pitfalls in "Soon: playing-song indicator in Hits"). Done in 81c9248 (worker; not yet installed).
- [x] CI: GitHub Actions for rormpc-tools (pytest), ro-listenbrainz-mpd (`cargo build` + `cargo test` on Ubuntu)
      and the rormpc installer smoke test, plus `push` in rormpc's ci.yml; triggers as in "Tests and GitHub
      Actions…" (decided 2026-10-07; a worker writes them, the coordinator pushes). Done and pushed: rormpc-tools and
      ro-listenbrainz-mpd `test.yml` green on the first run; rormpc ci.yml runs on push (Build & Test green on
      Ubuntu and macOS; Formatter only on pull requests, decided by the user, the fork is not rustfmt-clean);
      `installer_smoke.yml` is manual (workflow_dispatch) until it passes once; `RORMPC_TOOLS_REF` / `RO_LB_REF`
      override the pinned tags.
- [x] Installer on macOS: `rormpc_install.sh companions` silently fails to add the listen rule to a fresh
      scrobbler config (macOS awk rejects the newlines in `-v rule=...`; the `awk … && mv` hides it, prints "set
      the 90%-without-a-seek rule" anyway and leaves a .tmp file). Pass the rule with `\n` escapes, fail loudly,
      no .tmp left (found by the CI worker 2026-10-07; fix approved by the user). Done: the rule goes through ENVIRON.
- [x] "Pause for…" replaces "Mute for…" (asked 2026-10-07: "I don't want Mute for at all, only Pause for"): a timed
      pause owned by mpd-player with a wall-clock deadline (works with rormpc closed, across sleep and restarts);
      at the deadline it resumes only if still paused by this timer; a play by anyone cancels the timer. Remove
      Mute for… (module, commands, menu items, slider countdown, docs) in the same change; reuse its deadline and
      generation/error plumbing. Test on a scratch MPD. Done in rormpc d29d92c, rormpc-tools 731768f (worker): `pause start|extend|
      resume|cancel`, pause.json; resumes only while MPD is still paused on the same song; the gap neither starts
      nor ends a silence while the timer holds the pause; an old mute.json restores its saved volume once. rormpc:
      `om` → ShowPauseMenu (`op` is ShowDecoders), `paused · 12:34` on the volume slider; `ShowMuteMenu` stays a
      serde alias until the dotfiles config says `"om": ShowPauseMenu` (switch it after installing). Known limits:
      play then pause on the same song while the daemon is down cannot be told apart and resumes; an MPD restart
      (new song ids) cancels the timer. Not installed yet.
- [x] Release rormpc-tools v0.2.33 (Pause for…), install rormpc (Pause for…, Hits ▶0) and switch the dotfiles
      config to `"om": ShowPauseMenu` (decided 2026-10-07: now; the coordinator does it).
      Done: v0.2.33 pushed and installed, rormpc d553c48 installed, dotfiles `om` → ShowPauseMenu (config loads).
- [x] Keys pasted or sent in one batch with `/` go to the filter, not to normal-mode commands, in the Queue
      filter and the ordinary filters (approved 2026-10-07; see the item under "Done: live Queue plan view"). Done in
      4257576 (worker): the key resolver sent the resolved action back through the app event channel, behind the
      keys already queued from the same batch; it now returns it and the event loop handles it before the next
      key. Checked on a scratch MPD (Queue and Directories filters, consume/single/random unchanged).
- [x] Live playlists, first version (decided 2026-10-07): public YouTube playlists only, manual check, first
      import reviewed, batch accept; the prerequisite `yt-mp3-mb --batch --json` in dotfiles, the `liveplaylist`
      CLI, the rormpc URL modal and pane ("Live playlists: paste a playlist URL…"). Done (worker): rormpc-tools 9212507
      (`yt-mp3-mb --batch --json`), ffa991f..e7a6212 (`liveplaylist`), rormpc aef0abc (`Pane(LivePlaylists())`, the URL
      input from its Enter menu). Not tested: a real download from a live playlist (the one allowed live listing
      returned 0 items); the UI was checked on a scratch MPD with hand-made items. Needs a release and a
      `Pane(LivePlaylists())` tab in the user's config (snippet in RORMPC.md).
- [x] Release rormpc-tools v0.2.34 (Live playlists), install rormpc (Live playlists pane, keys sent with `/`) and
      add a "Live playlists" tab after Playlists in the dotfiles config (decided 2026-10-07: now).
      Done: v0.2.34 pushed and installed, rormpc 1cb1b3b installed, dotfiles 9474d93 (tab + `gl`; the config loads).
- [x] Lyrics: machine translation with Claude through the Anthropic API when tekstowo.pl has none (decided
      2026-10-07): line by line with the stanza as context, labelled "machine translation", the key from the
      user's rormpc-tools config (never in the repo); the model id from the claude-api reference.
      Changed 2026-10-07 (the user: "ask the claude CLI, the way pi does it"): no API key; it calls the Claude
      Code CLI (`claude -p`, no tools or MCP, no session) like ~/.claude/skills/claude/ask_claude.py, on the user's
      Claude Code login. Done in rormpc-tools 6ac635f, rormpc e621bef (worker): `claude -p` (default
      claude-sonnet-5-5, `translate_model` in config.toml or RORMPC_TRANSLATE_MODEL), strict JSON per stanza, line
      counts checked with one retry, nothing stored on a mismatch; never replaces "mine" or a human translation.
      One live call on invented lyrics (4.4 s). Released 2026-10-07: v0.2.35 pushed and installed, rormpc 6474e92
      installed; dotfiles be85114 renumbers the digit keys (1 Hits … 0 Live playlists; gy Lyrics, gd Deleted,
      gf Search).
- [x] The tab bar does not fit and cannot scroll (user, 2026-10-07, screenshot: "the tabs do not fit,
      and mouse scroll does nothing; also 3 jumps to Directories"). The bar cuts "Ver…" at the right edge
      (Hits, Queue, Up next, Shuffle, Directories, Artists, Album Artists, Albums, Playlists, Lyrics, Deleted,
      Ver…); the wheel over it should scroll it or the bar should wrap/shrink; the digit keys should match the
      tabs' order (3 should be Up next, the third tab, not Directories). Reported to the herdr coordinator.
      Decided 2026-10-07: digits follow the tab order (1 Hits … 9 Playlists, 0 Live playlists); the coordinator
      changes the dotfiles keymap, a worker the bar. Done: dotfiles be85114 (digits), rormpc 7249c32 (worker): the bar
      scrolls the active tab into view, ‹ › mark hidden tabs, the wheel over the bar and a click on ‹ › scroll it
      without switching, a click hits the tab drawn there. Not installed yet.
- [x] Asked 2026-10-07: "download the top 1% of music" (in Polish: "ściągnij top 1% muzyki"). Facts: Hits' Top %
      is the share of a chart cohort (Billboard year-end, per year); `hits fetch` is a verified import queue for
      missing songs (rormpc: "Fetch missing…"). Scope decided 2026-10-07: Billboard year-end, all years, the top 1%
      of each year's chart, the missing songs through `hits fetch` (verified).
      Done 2026-10-07 (worker): `hits --years 1959-2025 --top 1-1` = 64 songs, 11 owned; of the 53 missing, 18
      promoted into the library, 28 wait in review (21 "other recording of the same song", 5 "different song", 2
      "no MusicBrainz match"), 7 failed (no YouTube result within 5 s of the chart recording's length: Macarena
      Bayside Boys Mix, Walk Like an Egyptian, Believe, Breathe, Happy, Low, Surfin' U.S.A.). Review: `hits fetch
      status`, or the Hits pane's "Fetch missing…".
- [x] Asked 2026-10-07: "in Hits, all my playlists as a source" (in Polish: "w Hits jako źródło moje playlisty
      wszystkie"). Facts: Hits' Source cycles Billboard US / my charts / whole library / my likes / recommended.
      Done in rormpc-tools cd664bb (`hits --source playlists`), rormpc 9e56cc4 (Source "my playlists" after "whole
      library") (worker): every stored playlist's owned songs once, ranked like the library, the playlists in Why;
      generated playlists left out (GENERATED_PLAYLISTS: Hits/My charts/Library/Likes/Recommendations/My playlists
      labels, LB, Folder, Skipped, Not finished; Tag … and Live playlists count). Real MPD: 268 songs from 4
      playlists. Not released yet.
- [x] Top 1% review (the user, 2026-10-07): accept the 21 "other recording of the same song" items (`hits fetch
      accept`), reject the 5 "different song" items (`hits fetch reject`: the staged file is deleted and the
      rejection stays in the fetch queue state, $XDG_STATE_HOME/rormpc-tools/fetch/queue.json, so `hits fetch
      add` never queues them again; `hits fetch clear` keeps rejected ones; `hits fetch retry KEY` undoes it).
      The 2 "no MusicBrainz match" items stay in review. Done (worker): 21 accepted into Hits/<decade>s, 5 rejected,
      2 in review (Love Will Keep Us Together, Shadow Dancing); the fetch queue: 39 ok, 7 failed, 5 rejected.
- [x] Release rormpc-tools v0.2.36 (the "my playlists" Hits source) and install rormpc (decided 2026-10-07: now).
      Done: v0.2.36 pushed and installed, rormpc 22deb27 installed.
- [x] Add `"o": TogglePlanView` to the Queue keymap in the dotfiles config (approved 2026-10-07; from the
      music-mpd coordinator: the plan view was installed without it). Done: dotfiles 5c46056; `o` opens the plan view.
- [x] Leave evidence if the Previous forward jump repeats: MPD `log_level "verbose"` on the real MPD and a
      timestamped log line per media-key command in Hammerspoon (approved 2026-10-07; edit only, the user
      reloads Hammerspoon and restarts MPD). Done (worker): dotfiles 06fa6b7 logs each media-key event and mpc
      command to ~/.local/state/hammerspoon/media-keys.log (capped at 4000 lines); /opt/homebrew/etc/mpd/mpd.conf
      gets `log_file "~/.local/state/mpd/log"` and `log_level "verbose"` (backup mpd.conf.bak-2026-10-07; the log
      is not trimmed: turn it down after the investigation). Takes effect after the user restarts Hammerspoon and
      MPD (`brew services restart mpd`).
      Reverted 2026-10-08 (the user agreed): the cause was found (Karabiner), so mpd.conf was restored from the
      backup before MPD ever restarted with verbose logging.
- [x] Media keys through Karabiner, not Hammerspoon (found 2026-10-07: Hammerspoon's systemDefined tap never sees
      the MacBook's F7/F8/F9: Karabiner maps them to consumer keys from its virtual HID device, which go straight
      to Now Playing, so Previous went mpd-now-playable → MPD `previous` and counted a skip; the Input Monitoring
      permission did not change that). Decided by the user: a Karabiner complex modification catches F7/F8/F9
      without fn and runs a small script (prev: `shuffle prev` to mpd-player when it is subscribed and
      shuffle.json has `trail`, else `mpc prev`; play/pause: `mpc toggle`; next: `mpc next`), logging each command
      to ~/.local/state/hammerspoon/media-keys.log's successor; Shift+F7/F8/F9 pass through to the Now Playing app;
      fn+F7 stays F7. Remove the media-key code from Hammerspoon. Done (worker): dotfiles d0f17c6 (scripts/mpd-media-key,
      log ~/.local/state/media-keys/media-keys.log), 8b466bb (Karabiner rule), 268c9c5 (Hammerspoon override
      removed); rormpc 6c430c6 (RORMPC.md). Verified with a real F7 on 2026-10-07 18:40: `shuffle prev` sent, mpd-player
      went back to the song that really played before, recorded as "back" (no skip), prev.jsonl "confirmed".
- [x] Asked 2026-10-08: "can I see it in rormpc to decide? ask the models" (in Polish: "i mogę to jakoś zobaczyć
      w rormpc, żeby zdecydować? pytaj modeli"), about the 2 top 1% downloads waiting in `hits fetch` review
      (Love Will Keep Us Together, Shadow Dancing; reason "no MusicBrainz match"). Facts: the Hits pane marks such
      rows `?` in the `✗` column, its menu has Accept / Reject the download, the details say why; the row is only
      visible in a Hits result that contains it (Billboard US, its year), and there is no preview of the staged
      file. Consult the models (consult skill) on how to review fetch items in rormpc (e.g. a list of everything in
      review across results, listening to the staged file before deciding), record the outcome here.
      Consulted 2026-10-08 by a worker (GPT-6.1 Sol, MiMo, round 20261008-024751-c31f); both agreed on the first
      points. Verified in the code: a "no MusicBrainz match" file is tagged from YouTube (artist = the uploader
      channel), and `hits fetch accept` moves it as is, so the chart row stays missing and the library gains a
      song by the channel; `retry` would download the same top video again. Act on: "Accept as the chart song"
      (the chart's artist and title, the YouTube source in a comment, never the chart's MBID) beside "Accept with
      current tags"; a "Downloads (N to review)" list in the Hits filter column for all review and failed items;
      details with expected vs downloaded (length difference highlighted, channel · video title, reason in plain
      words); a preview outside MPD reusing the Versions player (stopped before Accept/Reject); "Try another
      candidate" skipping rejected video ids; "Open on YouTube". Consider: a tag edit while staged; accept failing
      when the staged file is gone. Today the two items can be heard with `mpv` on the staged files in
      ~/.local/state/rormpc-tools/fetch/staging/ and are best not accepted as they are.
- [x] Review fetch downloads in rormpc (approved 2026-10-08, the whole design from the consultation): `hits fetch accept --as-chart KEY`
      (chart artist/title, YouTube source in a comment, no MBID; accept fails on a missing staged file; retry skips
      rejected video ids) in rormpc-tools; in the Hits pane a "Downloads (N to review)" list, expected vs
      downloaded details, Preview / Stop, Open on YouTube, Accept as the chart song, Accept with current tags,
      Reject (durable, confirmed), Try another candidate.

      Done in rormpc-tools 609d8dc, rormpc c1db024 (worker): `hits fetch accept --as-chart`, `hits fetch another`, rejected
      video ids skipped, accept fails on a missing file; Hits "Downloads (N to review)" first in the filter column,
      expected vs downloaded details, a shared preview player (rormpc_preview.rs, also used by Versions), and a
      retry race fixed (the worker starts only after the decision is written). Not released yet.
- [x] Release rormpc-tools v0.2.37 (Downloads review) and install rormpc (decided 2026-10-08: now).
      Done: v0.2.37 pushed and installed, rormpc 059a876 installed.

## Done: live Queue plan view (approved 2026-10-06, done 2026-10-07)

- [x] `o` toggles a view-only Queue plan; never sort or move the MPD queue to display it. Show the last two
      plays (`-2`, `-1`, dimmed), `0 ▶` current, `↑n` requests, all ten forecast slots, then an unselectable
      `unplanned · queue order` divider and the remaining songs in MPD order. Label the mode in the panel title.
- [x] Preserve selection, scrolling and mouse targets by queue song ID across forecast refreshes; break ties
      by MPD position. Disable Next-header sorting in this view. Filtering keeps the original turn numbers.
- [x] Dim stale forecast numbers and show `stale · Xm` when the daemon is absent or its state is old.
- [x] J/K reorder requests in the request section; in the forecast they request a version-checked slot swap
      from mpd-player, which alone writes priorities and publishes confirmation. Never move across section
      boundaries or move past/current/unplanned rows. Document patch lifetime and clear it when a patched
      song plays or leaves the queue, or the daemon draws a new forecast.
- [x] Verify with `cargo test`, `uv run pytest`, `uvx pyflakes src`, and TUI tests only on a scratch MPD.

Verified: `cargo test` (957 tests), `uv run pytest` (123 tests), pyflakes, and the TUI on a null-output scratch MPD
(port 6650): 20 confirmed forecast swaps with the MPD queue order unchanged, past/current/request/forecast
sections, request J/K, a filter that keeps slot 10 and plays its ID, stale state from an old, deleted or
59-second-old heartbeat while paused, `stale · 0m` at once when the daemon is killed (MPD sends no Subscription
event on a client disconnect, so rormpc waits on the daemon's process exit), and fresh again after a restart.

- [x] Approved and fixed 2026-10-07 (4257576): keys that reach rormpc in the same input batch as `/` are resolved in normal mode (also in the
      ordinary Queue filter), so a paste or a scripted `send-keys / text` runs the letters as commands. Seen
      2026-10-07 on the scratch MPD (`c` turned consume on).

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

- [x] Consult the models and prepare visual mockups of playing-song indicators in Hits (for example, a
      `▶` beside `0` in the Next column). Show the variants to the user for review before implementing one.

Mockups 2026-10-07 (consulted MiMo; GPT-6.1 Sol was at its usage limit). Today the playing song shows only `0` in
Next; the Queue paints its row in `highlighted_item_style` (#7aa0cd bold), the cursor row is black on #7aa0cd.
`»` marks the row painted in #7aa0cd bold, `[...]` the cursor row:

    A  row only (as in the Queue)            B  row + glyph                       C  glyph only
     Rank  %  ✓ ♥ Next Artist   Title        Rank  %  ✓ ♥ Next Artist   Title     Rank  %  ✓ ♥ Next Artist   Title
     #3   97% ✓   -1   Toto     Africa        #3   97% ✓   -1   Toto     Africa     #3   97% ✓   -1   Toto     Africa
    »#4   96% ✓ ♥ 0    Queen    Flash        »#4   96% ✓ ♥ ▶0   Queen    Flash      #4   96% ✓ ♥ ▶0   Queen    Flash
     #5   95% ✓   ↑1   Adele    Skyfall       #5   95% ✓   ↑1   Adele    Skyfall    #5   95% ✓   ↑1   Adele    Skyfall

    D  B with a paused glyph                 E  cursor on the playing row (B)
    »#4   96% ✓ ♥ ⏸0   Queen    Flash        [#4   96% ✓ ♥ ▶0   Queen    Flash]   the glyph keeps it visible

  - MiMo ranks A first (same model as the Queue, no column change), B second (survives no-colour terminals and
    the cursor sitting on the row, where A's colour disappears under the cursor style). My pick: B.
  - Pitfalls: `▶` and `⏸` are East Asian ambiguous width (2 cells in some fonts) and would shift `↑10`/`-2`;
    a hidden (dimmed) owned row can play, so the paint must override DIM; a playing song outside the filter
    gets no phantom row (Shift+C and the footer already cover it); the same file in two rows lights both.

## Soon: missing film score / soundtrack genre

- [x] Investigate why the film score / soundtrack genre is missing from songs such as "Cast Away Theme"
      by Alan Silvestri, and make that classification available in Hits genre filters.

Done 2026-10-07 (rormpc-tools f2f26c0, not released yet): MusicBrainz has no soundtrack genre, only tags; Silvestri
carries "soundtrack" (3 votes) and the genre "classical" (2), and only genres were read. The tags soundtrack, film
score, score, film soundtrack and original soundtrack now count as the genre "soundtrack": on a recording always,
on an artist only with at least the votes of the artist's best genre (otherwise the Beatles' tag made "Hey Jude"
a soundtrack). "soundtrack" is a default Hits checkbox in rormpc and rormpc-tools, "film score" an alias of it.
On the library: 23 songs, e.g. Cast Away, Requiem for a Dream, Jesus Christ Superstar, Skyfall, Eye of the Tiger.

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

- [x] Tested 2026-10-07 on a scratch MPD (rormpc-tools 529e2da pins it): a mute that spans an mpd-player gap silence.
      They leave each other alone: an expiry inside the silence restores the volume and stays paused until the gap
      presses play; a gap while muted pauses and resumes at volume 0 and the mute's deadline restores it later; a
      mute started during the silence works (a pause is not a stop).
- [x] A queue replaced by Sources… may pass through a stop, which unmutes. Watch whether that happens in use.
- [x] Maybe later: "Pause for…" (now that one daemon owns the gap, it no longer fights mpd-gap).
- Closed 2026-10-09: "Mute for…" was removed and "Pause for…" built instead (see "Next, in order").

## Test report of 2026-10-06 (scratch MPD, music-mpd-8e)

Fixed 2026-10-06: Up next entries skipped past between two wakes of mpd-player (`mpc next` three times in a row,
random on) stayed waiting with a ↑1 badge; now a waiting entry whose MPD priority went back to 0 has started and
counts as played. The Queue menu keeps Weighted shuffle, Silence, Mute for… and Sources… with an empty queue;
"Clear queue…" asks first (Cancel is the default); Sources… leaves out empty playlists; menus are as wide as their
longest label (within the terminal).

- [x] Same race with random off: entries are moved after the current song, so skipping past them leaves them
      before it. Telling that apart from the user jumping to a later song needs more than positions; not done.
- [x] mpd-player down while an Up next song plays: it stays waiting. After an MPD restart the re-found entries
      carry no priority yet, so "priority 0 = started" cannot be used at startup.

Fixed 2026-10-07 (rormpc-tools 893091c): MPD resets a song's priority to 0 whenever it starts, random on or off, and a
`playid` jump resets only the song jumped to. Every waiting entry now gets 255, 254, … also with random off (MPD
ignores them there), so "priority 0 = started" holds in both modes and at startup (checked before any write;
`"marked": true` in upnext.json guards state from older versions). Priority writes skip the playing song. Verified
on a scratch MPD. Open: after an MPD restart, entries re-found by file under new ids stay waiting (a restored 0 is
ambiguous with a replaced queue); the cost is a rare request that plays twice (see "Needs a decision"). While
cleaning up, the worker's `pkill -f bin/mpd-player` also killed the real daemon (launchd restarted it); the lesson
is in rormpc-tools' AGENTS.md.

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

Closed 2026-10-07 by the user: media keys stay as they are (mpd-now-playable, no helper, no Linux bridge,
no Rust port); the unchecked points below were closed with it on 2026-10-09.

- [x] Document it in RORMPC.md ("Media keys" section): what it is, the commands above, how to check it
      (`launchctl print gui/$UID/me.00dani.mpd-now-playable`), the upgrade caveat, and that it is independent
      of rormpc.
- [x] Do not fold it into `rormpc_install.sh install`: that script installs and rolls back the binary, and a
      binary rollback must not touch the service. If a helper is still wanted, make it a separate opt-in
      subcommand (e.g. `rormpc_install.sh media-keys install|status|uninstall`) that only calls the tool's own
      `install-launchagent` / `uninstall-launchagent`, never writes its own plist, and refuses to run a second
      Now Playing source.
- [x] Linux: MPD has no MPRIS of its own; a bridge exposes it on the session D-Bus. Options: this
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
- [x] Extend rmpcd instead (asked 2026-10-07: "maybe extend the rmpc daemon? ask the models"). Consulted
      2026-10-07 (GPT-6.1 Sol, MiMo), both rank it last today: keep mpd-now-playable (+ mpd-mpris on Linux) first,
      a standalone crate second, rmpcd only as a deliberate commitment to run rmpcd (unused here, early-stage MPRIS,
      tokio + Lua + zbus; edits near its startup re-conflict on every upstream bump). If rmpcd: AppKit's
      NSApplication on the main thread, the tokio runtime on a worker thread, `CFRunLoopStop` from the signal path
      for shutdown, features `now-playing` (macOS) and `mpris` (Linux) so zbus never builds on macOS, one cfg-gated
      module and a small spawn hook. Both: AirPods / Now Playing Previous should also go to mpd-player's `shuffle
      prev` (mpd-now-playable's handler, or the port's), so every Previous walks the same history.
      Decided 2026-10-07 (the user): leave it as it is; no Bluetooth headphones or Now Playing widget are used to
      switch songs, so mpd-now-playable stays unchanged and rmpcd is not extended.
- [x] Rust instead of the Python tool on macOS. Asked GPT-6.1 Sol and MiMo twice on 2026-10-03; both say keep
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
- [x] Windows: no plan unless MPD on Windows is actually used (SMTC, e.g. via souvlaki; needs a hidden HWND with a
      message pump).
- [x] Whatever is documented, check: works with rormpc closed, survives MPD restart and sleep/wake, clears
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

- [x] Prerequisite: a non-interactive mode for dotfiles `yt-mp3-mb` (e.g. `--batch --json`): no prompts, prints
      the produced paths and the unresolved matches as JSON, resumable without duplicate downloads. Uncertain
      matches stay "needs review" instead of being asked about inside a child process the TUI cannot answer.
- [x] A new CLI in dotfiles (like `musicdb` / `hits`), e.g. `liveplaylist add|check|accept|reject|list --json`.
      It owns state, source adapters, matching, downloads and writing the MPD playlist; rormpc owns presentation
      and decisions only (a URL input modal and a "Live playlists" pane with accept / reject / accept all), runs
      it with argv (no shell), never blocks the UI thread, can cancel it. Progress as JSONL or a status file
      written atomically (temp + rename), as the Hits pane does.
- [x] State per subscription in music-data (secrets outside it, logs/progress in ~/.cache): url, kind, MPD
      playlist name, a stable target dir (not the playlist title, which can change), schema version, lock file.
      Per source item: the decision (pending/accepted/rejected, rejects are durable) separate from the job state
      (queued/downloading/needs_match/ready/failed), source position and last-seen time, the local path.
- [x] Order lives in the .m3u (playlist_directory), not in `NNN` file names. Publish only ready files. Songs
      already in the library are referenced, not downloaded again, but only on a confirmed recording match
      (MBID), never on a loose title match.
- [x] Removals and reorders upstream: never delete local files and never infer a removal from a failed or partial
      check (yt-dlp YouTube extraction breaks, bot checks, 403s); an id that reappears is reactivated.
- [x] First version: public YouTube playlists only (`yt-dlp --flat-playlist -J` to list ids cheaply), manual
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

- [x] (tests done locally 2026-10-07, e354b97; the workflow waits for the CI decision) rormpc-tools: pytest without network, on a temporary DB/data dir and mocked HTTP: invalid token and a
      ListenBrainz outage still let `update` sync and export; play counts, and a local listen counted once with its
      ListenBrainz copy (same timestamp); skips import and the Skipped playlist; delete/undo with the journal;
      mpd-gap state transitions on a fake clock and fake MPD status. Workflow on Ubuntu, ~1-2 min.
- [x] (tests done locally 2026-10-07, 862c08a; the workflow waits for the CI decision) ro-listenbrainz-mpd: `cargo build` on Ubuntu (catches the apt build dependencies), tests for the listen rule
      (fraction, max seconds, uninterrupted: seek restarts, pause neutral) and the local log lines. ~2-4 min.
- [ ] rormpc: installer smoke test on Ubuntu: fresh user, `loginctl enable-linger`, `XDG_RUNTIME_DIR` and the user
      D-Bus, MPD with a generated tone, fake token and API URL, `rormpc_install.sh companions`, then assert units and
      the musicdb timer are active, the listen_* lines are in the config and the token untouched, `status` output,
      a reinstall. The installer pins released tags, so a smoke test would pass on a broken branch: first add
      overrides (e.g. `RORMPC_TOOLS_REF`, `RO_LB_REF`, or reuse `--local` with checkouts) so CI tests the commit
      under test. ~5-8 min. This is what was done by hand in an OrbStack Ubuntu VM on 2026-10-03.
- [x] (left out, decided by the user 2026-10-09) macOS: first a throwaway probe that `launchctl bootstrap gui/$UID` works on hosted runners, then the same
      smoke test with launchd.
- [x] Triggers: push to master, pull_request, workflow_dispatch, weekly schedule (toolchain, uv and runner-image
      drift; GitHub disables schedules after 60 days without repository activity), and on release tags. Add
      `push` to the upstream ci.yml here too.
- Not automated: live ListenBrainz, MusicBrainz, Billboard and YouTube (OAuth) calls, and exact gap timing on
  shared runners.

## Versions: delete a bad version, find versions from the Queue

- [x] Versions: delete a bad version, and find versions from the Queue (asked 2026-10-06: "in Versions I can't
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
  - Done 2026-10-07 (9164204): `Versions()` song property (`≋`), cached from one `musicdb versions --json --all`
    and read again after a delete, merge or library update; "Find versions…" in the Queue menu and `V`
    (QueueActions::FindVersions); the delete menu re-checks the preview and reports musicdb's last line when
    done; the Versions pane now lists resolved groups too (dimmed). Your explicit keymap and theme need
    `"V": FindVersions` and the column `(prop: (kind: Property(Versions()), default: (kind: Text(""))),
    label_prop: (kind: Text("≋")), width: "1")`; a column before Year/Plays shifts their SortByColumn numbers.

## Lyrics: Polish translation next to the original

- [x] Asked 2026-10-07: show a Polish translation in the Lyrics tab, the original on the left and Polish on the
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
  - Done 2026-10-07 (rormpc 7e6b794, rormpc-tools 69d8201): `musicdb lyrics translate FILE|--current` (one song, on
    demand; the usual URL first, else the site search, at most 3 requests 2 s apart, a clear User-Agent), accepted
    only when the page's original overlaps ours by word (Jaccard ≥ 0.45); `<stem>.pl.json` sidecars; language
    detected once with langdetect (new dependency), `musicdb lyrics lang FILE CODE|auto` overrides it. tekstowo.pl's
    robots.txt allows song and search pages (its `Disallow: /` names only AI crawler user agents) and its terms
    allow private use, commercial use is banned. The Lyrics pane: Enter looks the translation up (the pane gets
    mapped actions, not raw keys, so not `t`), h/l switches original / translation below 100 columns, a stale
    translation is hidden. Live check: Viva la Vida and Skyfall (their sidecars are in the real lyrics_dir).

## Previous in the weighted shuffle

- [x] Asked 2026-10-07: with the weighted shuffle on, the Previous media key bounced between unrelated songs and
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
  - Done 2026-10-07 (rormpc-tools 328cff3, by a worker): `shuffle prev [CMD_ID]`, a trail of really played queue
    ids with a cursor in shuffle.json, a 0.25 s debounce, prev.jsonl (fsynced before `playid`, then confirmed or
    failed), `import-skips` drops a matching skip within 2 s. The worker's own choice: a song already played to
    80% (FINISHED_SHARE) keeps its finished outcome when left with Previous. The rormpc history shows the neutral
    case as kind "back".

## Versions: audio fingerprint finds copies

- [x] Asked 2026-10-07 (screenshot): "Love Story (Disco Lines remix)" (official upload, 139 s) and "Love Story (Disco
      Lines full remix)" (a re-upload by a "Central Bass Boost" channel, 137 s) are not detected as a copy. Facts:
      they share a Versions group with no decision; dedupe only merges the same md5 or YouTube id; no MBIDs; the
      Queue's "Remove duplicate entries (N)" only finds the same file queued twice. fpcalc is already used by
      mbtag (AcoustID), not to compare files with each other.
      Measured 2026-10-07: raw chromaprint (`fpcalc -raw`, first 120 s), share of equal bits at the best offset
      within ±60 frames (~±7 s), over the 72 file pairs of the Versions groups: the same recording 0.87-0.99 (Love
      Story 0.929, Nightcall 0.989, Take Five 0.979, 30 pairs at 0.88 or more); live, remix, edit and other
      performances 0.51-0.69 (Rolling in the Deep live 0.53, Somebody That I Used to Know live 0.69); 0.75-0.83 in
      between (Lucy Pearl, Oh Laura, Marvin Gaye, Wyclef Jean: maybe another master or edit). A bass-boosted
      re-upload still matched; sped-up / nightcore (pitch and tempo change) is expected to break chromaprint and
      is out of scope.
      Consulted 2026-10-07 (MiMo; GPT-6.1 Sol was at its usage limit):
  - In `musicdb versions`: compare files within a group only (never library-wide), fingerprints cached per (path,
    size, mtime). A pair at 0.88 or more gets the suggestion "Same recording? audio match 93%" with the reason; one
    key confirms the existing "Same recording: keep this file, merge the others…"; never merged by itself. 0.72-0.88:
    "similar audio (another master or edit?)", no suggestion to merge. Recalibrate when the library grows.
  - Default file to keep: the official channel / an MBID, then the longer (untrimmed) one, then the higher bitrate,
    then more plays; shown with the reason, changeable before merging.
  - Compare only the overlapping part at the best offset; a short intro or a trimmed end must not lower the score.
  - The Queue item "Remove duplicate entries (N)" stays about the same file queued twice; rename it to "Remove
    repeated entries (N)" so it is not read as "copies of a recording".
  - Done 2026-10-07 (rormpc 2b9b78c, rormpc-tools 21d17a0): `audiomatch.py`; `musicdb versions --json` reads only the
    fingerprint cache (~/.cache/rormpc-tools/fingerprints.json) and lists missing files; `musicdb versions
    fingerprint` fills it (the Versions pane in the background, the hourly `update`; 130 files took 3.9 s). On the
    library: 32 groups suggest "Same recording?", 6 pairs "similar". The worker's own choices: length does not pick
    the file to keep when lengths differ by more than 25% (a loop or extended mix: only the start was compared, the
    suggestion says to listen first); files labelled as different versions are never suggested. `a` in Versions
    merges the selected audio match after a confirmation ("Keep another file…" picks another); not clicked through
    in the TUI because Merge would move a real file to the quarantine.

- [x] Installer smoke test (decided 2026-10-09): the coordinator dispatches `installer_smoke.yml` on rofrol/rormpc;
      when it passes, a worker adds push to master, pull_request, a weekly schedule and release tags like the other
      workflows (and drops the "until it has passed once" note), the coordinator pushes. A failure is fixed first.
      First run 2026-10-09 (run 37990844352) failed in "Fresh user with a lingering systemd user session" after
      36 s with no output: the journal shows ci's user manager up ("Startup finished in 98ms", dbus.socket
      listening) and two `systemctl` calls from `as-ci 'systemctl --user is-system-running --wait || systemctl
      --user --failed'` failing with "Failed to connect to bus: Permission denied"; the user generators also tried
      /home/runner/.config. Next: a worker reproduces it in an Ubuntu VM (OrbStack, as on 2026-10-03) and fixes
      the step; then the coordinator dispatches it again.
      Fixed in 9ff4029 (worker, reproduced in an OrbStack Ubuntu 24.04 VM): the runner image's /etc/environment sets
      XDG_CONFIG_HOME=/home/runner/.config, which pam_env passed to ci; `as-ci` ran from the runner's work dir ci
      cannot read; the bus check now runs as ci; ffmpeg added (musicdb update needs it); and an installer bug: a
      failed `uv tool install` passed silently inside `if …; fi || {…}`. Second run dispatched: 37994088698.
      Second run green (degraded user manager handled). Triggers added in 75cada9 (worker): push to master and pull_request
      with a paths filter on the installer and the workflow, release tags, weekly "47 5 * * 1"; pushed.

- [x] Plan (asked 2026-10-09, design only): "there should be one combined view now: Queue, Hits, Shuffle. So I can
      choose to prepare weighted. If I change the year in a filter as in Hits, a new list is prepared. If I turn
      weighted off, the normal view comes back. The things chosen in Hits, like year and genres, can be saved as a
      live list, like collections on Steam or in Calibre. Show visualizations. Ask the models." A worker consults
      the models and writes the plan with mockups to plans/combined-view.md; the coordinator shows it and asks
      the open choices in "Needs a decision".
      Added 2026-10-09: "of course I can also remove or add songs by hand. Then something has to be done with that.
      I don't know what. A dynamic list but from a limited set? Ask the models."
      Added 2026-10-09: "it can be exceptions. Pins and exclusions on the whole library or on a chosen subset such
      as Billboard 100. And those subsets like Billboard should be made the way genres are. They can be - or +.
      Ask the models."
      Done in 68939c9 (worker): plans/combined-view.md, a "Play" tab with normal and weighted modes, preview + Apply,
      ± set chips, scoped pins/exclusions, smart lists, 6 mockups; consult rounds 20261009-230419-b57c and
      20261009-230647-3341. Its 12 open choices are in "Needs a decision" ("Combined view:").

- [x] Combined view, phase 1 (decided 2026-10-09, plans/combined-view.md): `hits` set chips (±, fixed rows
      Billboard, my likes, my playlists, recommended), Rank by, Years of (default follows Rank by), CLI and Hits pane;
      `--source` kept as a shorthand; contract tests.
      Done in rormpc af62e2e, rormpc-tools 06ecfee (worker; pytest 261, cargo test 989; clippy shows only older
      warnings; not released). Behaviour changes by design: Top % is cut before genres and sets (Billboard rock
      Top 10% 50 → 45 rows), `-n` cuts after the artist filter; rows without a rank get population size +
      position; "of N" counts the songs after the set algebra, before period and filters; Billboard with Years of
      other than chart reads every chart year (6,333 songs, ~0.8 s). The installed hits 0.2.37 rejects the new
      arguments: rormpc and rormpc-tools must be released and installed together.
- [ ] Combined view, phase 2: exceptions (pins and exclusions with a scope; default scope the open smart list, else
      library; any exclusion beats any pin, a pin beats `-` rules; hand edits in the queue are one-offs), CLI + log,
      marks and actions in Hits and Queue, "show excluded", the exceptions list; `hits hide` read as Billboard-scope
      exclusions.
- [ ] Combined view, plan addition (decided 2026-10-09: "only Play" among the browsing tabs): a worker extends
      plans/combined-view.md, after a consult round, with how Play covers what Artists/Album Artists, Albums,
      Directories, Playlists and Live playlists do today: play one album in track order, add one song or album
      without Apply (Play next / Append), edit a stored playlist's order and contents, review Live playlist
      downloads with a visible pending count, folder (filename) order; keys and click targets for each; the default
      config without those tabs (old panes stay for explicit configs). Open choices go to "Needs a decision".
      Do it before phase 3.
      Consulted 2026-10-09 by the coordinator (Sol a91fcbfc, MiMo 0fd6ccf8), input for the worker: both put a
      Browse/Sources mode into Play (grouping Artists / Album artists / Albums / Folders / Lists, drill down, `/`
      search; p play, n play next, a append on any row, no Apply) and an explicit Order (Source = disc/track,
      filename or list order · Ranked · Weighted; playing an album picks Source order by itself); a stored-playlist
      editor (reorder, remove, rename, delete, explicit save) and a Live playlists inbox (refresh, download status,
      accept/reject with multi-select, a pending-count badge in Play) that never replace the queue. Diverged on
      where: Sol puts Browse in the left column (Build | Browse) and the editor and inbox in overlays; MiMo cycles
      right-pane modes (Sources | Queue | Live). MiMo: keep Artists and Album artists apart (different grouping).
      Nothing needs its own tab, but filters alone do not replace browsing, editing or the download review.
- [ ] Combined view, phase 3: the Play pane (normal mode: queue with the filter column collapsed to one line, `h`
      opens it; weighted mode: plan projection), preview + Apply with the queue version check, confirmation only on a
      source kind change or more than 25% of the queue; the default config switches to Play, old panes stay.
- [ ] Combined view, phase 4: smart lists (named "Smart list"): save, picker, load as preview, Previous sources,
      MPD export "Smart NAME" on by default; no freeze action.
- [ ] Combined view, phase 5: sets from tags, MPD playlists, Live playlists and smart lists through "+ set…", with
      a cycle check.

- [ ] Installer smoke test: "Units are enabled and active" proves the programs run: each unit's MainPID and its
      executable, and a log line from it (decided 2026-10-09; a Type=simple unit is "active" with a missing binary).
- [ ] rormpc-tools: `musicdb update` reads `api_url` from the scrobbler config like the scrobbler, so a custom or
      fake API URL is used for ListenBrainz (decided 2026-10-09); adjust the smoke test's musicdb step if its
      expected exit changes.
- [ ] Missing ffmpeg: a readable error in rormpc-tools instead of a traceback, and ffmpeg in the installer's
      `status` and dependency hint (decided 2026-10-09).

- [ ] Scrobble status (asked 2026-10-09: "I also want to see in how many seconds the entry will be sent to the
      scrobbler, and what percent of the required percent is done, and whether it is still possible since the song
      was scrolled, e.g. to 20%, and a manual button to send it to the scrobbler. It has to take the information
      how many percent is needed to send to the scrobbler from the ro mpd listenbrainz daemon. Ask the models.")
      A worker consults the models first (how rormpc gets the rule and the current listen's progress from
      ro-listenbrainz-mpd: its config's listen rule, a state file or a command; the seek rule makes a listen
      impossible; what the manual send does and how it avoids a double listen), then builds it; open choices go
      to "Needs a decision".

- [ ] rormpc-tools README: describe `hits --set`, `--rank`, `--years-of` (phase 1 worker left it out of scope).
- [ ] AGENTS.md "Checking UI behaviour": a copy of the user's config for tests needs the themes/ directory next to
      it (reported by the phase 1 worker 2026-10-10).

## Proposed

## Needs a decision

- [x] Previous in the weighted shuffle: restart the current song (seek 0, no outcome) when Previous is pressed in
      its first ~3 s? Decided 2026-10-07: no, Previous always goes back.
      Checked: Sol says leave it out (it fights walking back), MiMo says restart; the TODO marks it undecided.
- [x] Previous: may an agent reproduce the burst with your Hammerspoon and mpd-now-playable pointed at a scratch
      MPD, or will you test the fix live with the media keys? Answered 2026-10-07: "I pressed the back key and it jumped as if forward, in Weighted mode, the Shuffle tab"
      (rormpc open on the Shuffle view). Not reproduced; since `shuffle prev` (rormpc-tools 0.2.31) Previous no
      longer sends MPD `previous` in weighted mode once Hammerspoon has restarted with dotfiles 22e3b81. Watch
      whether it happens again; if it does, look at mpd-player's handling of a song it did not start (origin
      "other") right after a Previous, and at the Shuffle view's keys.
      From the music-mpd coordinator 2026-10-07 (music-mpd-b8 on a scratch MPD 0.24.15, mpd-player 0.2.30, 53
      previous / 48 next stress run): no song change without a command; the daemon only sends prioid, single
      oneshot and the gap's play, never next/playid; MPD's `previous` plays current-1 in its random order; after
      `previous` the song left stays right after the current one with priority 0, so the next `next` replays it
      before the plan. The 00:15 forward jump was not reproduced.
      Checked: both talk to the real MPD on this Mac; AGENTS.md forbids playback tests on the user's MPD.
- [x] Lyrics: which model and account should the machine-translation fallback use (the lyrics leave the machine)? Decided 2026-10-07: Claude through the Anthropic API (queued in Next).
      Checked: the plan says "an LLM" without naming one; no API key for it is configured in rormpc-tools.
- [x] Media keys: do you still want an opt-in `rormpc_install.sh media-keys install|status|uninstall` helper? Answered 2026-10-07: "not sure; maybe extend the rmpc daemon (rmpcd)? ask the models" (consulted; decided: leave it as it is, no helper).
      Checked: the plan makes it conditional ("if a helper is still wanted").
- [x] Media keys on Linux: set up a bridge (rmpcd, mpd-mpris) on Omarchy or Roguix now? Closed 2026-10-07: the user chose to leave media keys as they are (keep mpd-now-playable).
      Checked: needs those machines and a live test of the keys; nothing on this Mac to verify it.
- [x] Start the Rust Now Playing port (`rormpc-now-playable`), or keep mpd-now-playable? Closed 2026-10-07: the user chose to leave media keys as they are (keep mpd-now-playable).
      Checked: Sol and MiMo both said keep the Python tool; the plan is complete if it is wanted.
- [x] Live playlists: start building them (yt-mp3-mb batch mode in dotfiles, the `liveplaylist` CLI, the rormpc
      pane)? Decided 2026-10-07: yes, the first version (queued in Next).
      Checked: a multi-repository feature with downloads; the plan is agreed but not ordered.
- [x] CI: push GitHub Actions workflows (rormpc-tools, ro-listenbrainz-mpd, rormpc installer smoke test, `push`
      trigger in ci.yml)? Decided 2026-10-07: yes (queued in Next).
      Checked: they only matter once pushed to GitHub; the local tests are in "Next, in order".
- [x] Mute: build "Pause for…"? Decided 2026-10-07: the user wants no "Mute for…" at all, only "Pause for…" (queued in Next).
      Checked: listed as "maybe later".
- [x] Mute: has a queue replaced by Sources… unmuted you in use? Closed 2026-10-07: moot, "Mute for…" is being replaced by "Pause for…".
      Checked: only observable in your use; the code path passes through a stop, which unmutes.
- [x] Does the modal DIM backdrop look right in Ghostty? Answered 2026-10-07: yes, it looks right.
      Checked: the DIM cells were verified through herdr; the strength in Ghostty needs your eyes.
- [x] Release rormpc-tools (push main and tag v0.2.31, bump `RORMPC_TOOLS_TAG`, run `companions`)? Decided 2026-10-07: yes; done 2026-10-07 (rormpc-tools v0.2.31 pushed and installed with `companions`, rormpc 1a691fc installed, dotfiles 7e15624 adds `V` and the `≋` column; Hammerspoon picks up `shuffle prev` on its next restart).
      Checked: main is 6 commits ahead of origin (the plan view's mpd-player swaps, the soundtrack genre, `shuffle
      prev`); the installed tools are v0.2.30, so none of them works in the installed rormpc until a release.
- [x] Hits playing-song indicator: build variant A, B (my pick), C or D (B with ⏸ when paused)? Decided 2026-10-07: B (queued in Next).
      Checked: mockups and MiMo's ranking in "Soon: playing-song indicator in Hits"; Sol was at its usage limit.
- [x] Install rormpc from master and add `"V": FindVersions` and the `≋` Versions() column to your config and theme? Decided 2026-10-07: yes; done together with the release above.
      Checked: the installed binary does not know either name, so adding them now would break loading the
      config; the exact lines are in "Versions: delete a bad version…".
- [x] Release rormpc-tools v0.2.32 with the Polish lyrics translations and install rormpc from master? Decided 2026-10-07: yes, after the Up next item (queued in Next).
      Checked: the Lyrics pane runs `musicdb lyrics translate`, which only exists in rormpc-tools after v0.2.31.
- [x] Up next after an MPD restart: keep re-found entries waiting (a rare request plays twice), trust restored
      priorities when the queue's files look unchanged (can drop a request after an MPD crash), or match the
      scrobbler's listens.jsonl / skips.jsonl? Decided 2026-10-07: keep them waiting (a request may play twice, none is lost).
      Checked: the worker's analysis (MPD's state file restores priorities after a clean restart, but a replaced
      queue also starts at 0); it recommends keeping them waiting, which is what the code does now. Recorded
      earlier as the user's choice by mistake: that text was a Claude Code input suggestion in the worker's pane.

- [x] CI: dispatch `installer_smoke.yml` on rofrol/rormpc once now and, when it passes, run it on push to master
      and weekly like the other workflows? Decided by the user 2026-10-09: yes (queued in Next).
      Options: run it once, then add push + weekly triggers | run it once, keep it manual | leave it
      Checked 2026-10-09: `gh run list -R rofrol/rormpc --workflow installer_smoke.yml` shows no run yet; ci.yml
      runs on push; rormpc-tools and ro-listenbrainz-mpd test.yml already have push, tags and a weekly schedule.
- [x] CI on macOS: build the throwaway `launchctl bootstrap gui/$UID` probe on a hosted runner, then a launchd
      smoke test ("Tests and GitHub Actions…")? Decided by the user 2026-10-09: leave it.
      Options: leave it (the macOS install is exercised on this Mac at every release) | build the probe first
      Checked 2026-10-09: nothing for macOS exists in installer_smoke.yml; it is the last open CI plan point.
- [x] Live playlists: test a real download with a public YouTube playlist you choose (the one live listing so far
      returned 0 items)? Decided by the user 2026-10-09: leave it until it is used.
      Options: you give a playlist URL and a worker tests it on a scratch MPD | leave it until you use it
      Checked 2026-10-09: the first version is released (v0.2.34) and checked only with hand-made items.
- [x] Combined view: What is a saved filter set called in the UI? Decided by the user 2026-10-09: Smart list.
      Options: Smart list | Live list (your word) | Preset
      Checked: plans/combined-view.md "Open choices" (1); the first option is the plan's recommendation.
- [x] Combined view: Does a filter change wait for Apply, or replace the queue at once? Decided by the user 2026-10-09: preview, Apply plays.
      Options: Preview, Apply plays | Replace at once when weighted is on | Replace at once always
      Checked: plans/combined-view.md "Open choices" (2); the first option is the plan's recommendation.
- [x] Combined view: When does Apply ask for a confirmation? Decided by the user 2026-10-09: only when the source kind changes or more than 25% of the queue goes.
      Options: Only when the source kind changes or more than 25% of the queue goes | Always | Never
      Checked: plans/combined-view.md "Open choices" (3); the first option is the plan's recommendation.
- [x] Combined view: What happens to the old Queue, Hits and Shuffle tabs? Decided by the user 2026-10-09: Play replaces them in the default config, the panes stay.
      Options: Play replaces them in the default config, the panes stay | Keep all four tabs | Remove the old panes
      Checked: plans/combined-view.md "Open choices" (4); the first option is the plan's recommendation.
- [x] Combined view: Where is the filter column in normal mode? Decided by the user 2026-10-09: collapsed to one source line, h opens it.
      Options: Collapsed to one source line, h opens it | Always open | Hidden
      Checked: plans/combined-view.md "Open choices" (5); the first option is the plan's recommendation.
- [x] Combined view: Does a pin beat a `-` set or `-` genre? Decided by the user 2026-10-09: yes, an exception beats every rule (an exclusion still beats a pin).
      Options: Yes, an exception beats every rule | No, a `-` rule beats pins (Sol) | Only a library-scope pin beats it
      Checked: plans/combined-view.md "Open choices" (6); the first option is the plan's recommendation.
- [x] Combined view: What do a hand removal and a hand addition in the queue do by default? Decided by the user 2026-10-09: nothing lasting, pin/exclude only on + / -.
      Options: Nothing lasting, pin/exclude only on + / - | Ask each time | Always record an exception in the open smart list
      Checked: plans/combined-view.md "Open choices" (7); the first option is the plan's recommendation.
- [x] Combined view: What is the default scope of a new pin or exclusion? Decided by the user 2026-10-09: the open smart list, else library.
      Options: The open smart list, else library | Always library | The first + set
      Checked: plans/combined-view.md "Open choices" (8); the first option is the plan's recommendation.
- [x] Combined view: Are smart lists exported as MPD playlists "Smart NAME"? Decided by the user 2026-10-09: yes, per list, on by default.
      Options: Yes, per list, on by default | Only when asked | Never
      Checked: plans/combined-view.md "Open choices" (9); the first option is the plan's recommendation.
- [x] Combined view: Which sets get a fixed chip row? Decided by the user 2026-10-09: Billboard, my likes, my playlists, recommended; the rest via "+ set…".
      Options: Billboard, my likes, my playlists, recommended; the rest via "+ set…" | Every tag and playlist as a row | Only Billboard
      Checked: plans/combined-view.md "Open choices" (10); the first option is the plan's recommendation.
- [x] Combined view: Which years does Period filter by default? Decided by the user 2026-10-09: follow Rank by.
      Options: Follow Rank by (chart, listened or release year) | Always release year | Always ask
      Checked: plans/combined-view.md "Open choices" (11); the first option is the plan's recommendation.
- [x] Combined view: Is there a "freeze as static list" action? Decided by the user 2026-10-09: no, the MPD export is the static copy.
      Options: No, the MPD export is the static copy | Yes, into a tag list | Yes, into an MPD playlist
      Checked: plans/combined-view.md "Open choices" (12); the first option is the plan's recommendation.
- [x] Combined view: build it now, in the plan's five phases (plans/combined-view.md "Build order"), one worker per
      phase with a release after each?
      Options: yes, all five phases in order | only phase 1 (± set chips, Rank by, Years of in Hits) first | not now
      Checked: all 12 open choices are decided (2026-10-09); every phase ships on its own.
      Decided by the user 2026-10-09: yes, all five phases in order (queued in Next).
- [x] Installer smoke test: make "Units are enabled and active" prove the programs run (a Type=simple unit is
      "active" even when its binary is missing; the first VM run passed it without rormpc-tools)?
      Options: yes, check each unit's MainPID/exec and a log line | leave it
      Checked: reported by the smoke-fix worker 2026-10-09; the later steps (listen, status) catch some of it.
      Decided by the user 2026-10-09: yes, check each unit's MainPID/exec and a log line (queued in Next).
- [x] rormpc-tools: `musicdb update` ignores the scrobbler config's `api_url` and calls the real ListenBrainz (the
      smoke test's fake token gets "Token invalid", exit 1); make it use the same API URL?
      Options: yes, read api_url like the scrobbler | leave it, the test expects exit 1
      Checked: reported by the smoke-fix worker 2026-10-09.
      Decided by the user 2026-10-09: yes, read api_url like the scrobbler (queued in Next).
- [x] Missing ffmpeg: a readable error in rormpc-tools instead of a traceback, and ffmpeg in the installer's
      `status` / dependency hint?
      Options: yes, both | only the installer hint | leave it
      Checked: reported by the smoke-fix worker 2026-10-09 (FileNotFoundError: 'ffmpeg' in a fresh VM).
      Decided by the user 2026-10-09: yes, both (queued in Next).
- [x] Combined view: which browsing tabs stay once Play exists (asked 2026-10-09: "with Play, are Artists, Albums,
      Directories, Playlists, Live playlists still needed?")?
      Options: keep Albums, Live playlists, Playlists, Search; merge Artists + Album Artists into one; Directories
      off the tab bar with a key | the same, but Playlists only behind the L picker | keep all tabs as they are
      Checked: consult round (Sol 3c079666, MiMo 1fb3e8db). Both: Albums keep (play one album in track order, which
      Play's rank/Apply does not do), Live playlists keep (an inbox of downloads to review, with a pending count),
      Artists and Album Artists merge (same songs, a grouping toggle), Directories off the default bar (imports,
      box sets, filename-order mixes), browsers keep Play next / Append so one song never needs Apply. Diverged:
      Playlists (MiMo keep the tab, an editing workspace; Sol behind L with Open/Edit). Both: hide before
      removing, keep keys and a click target; digit keys follow the tab order, so removing a tab renumbers them.
      Decided by the user 2026-10-09: "only Play" (none of the offered options): Artists, Album Artists, Albums,
      Directories, Playlists and Live playlists leave the tab bar; their tasks move into Play (queued in Next as a
      plan addition before phase 3). Search, Up next, Lyrics, Deleted and Versions were not part of the question.
- [ ] Release rormpc-tools v0.2.38 and install rormpc with phase 1 (± set chips, Rank by, Years of) now, or after
      more phases?
      Options: now (the installed Hits keeps working only with matching tools) | after phase 2 | after phase 3
      Checked: rormpc af62e2e needs hits ≥ the new tools; the installed 0.2.37 rejects --set/--rank/--years-of.
- [ ] Hits "my plays" rank: with Years of release it counts every play, with Years of listened it leaves out the
      weighted shuffle's own picks (as the old sources did); make both the same?
      Options: both leave out the shuffle's own picks | both count every play | keep it as it is
      Checked: reported by the phase 1 worker 2026-10-10 (hits_rules.score).
