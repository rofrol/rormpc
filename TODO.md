# TODO

## Next, in order

Triaged 2026-10-07 from the sections below; each item points at its section for the details.

- [x] Elapsed time runs fast (asked 2026-10-12: "serious matter. At 40 seconds in rormpc, the stopwatch on my phone [t-ryp5kjet]
      showed 37 s. Ask the models."): find whether MPD's elapsed, rormpc's display or the playback itself runs ~8%
      fast (40/37 = 1.081; 48000/44100 = 1.088), and fix it.
      Measured 2026-10-12 (coordinator, MPD status elapsed against the monotonic clock, 30 s each): 02:17, a 48 kHz mp3
      (Lady A, audio 48000:16:2, device 48000 per system_profiler): ratio 1.0896 (= 48000/44100 within 0.1%); a 44.1 kHz
      song (id 112) from its start to the next song's start: 239.2 s of wall time for 225.1 s (includes mpd-player's gap);
      02:26, a 48 kHz mp3 (id 6): ratio 1.0011, normal. So it is intermittent and follows the 44.1/48 ratio: the device
      may stay at the previous song's rate. MPD 0.24.15, osx output, no audio_output_format. Round
      20261012-021815-3d56 (sol f59daaf7 useful, MiMo e2962b72 and DeepSeek a6aedfe1 partial): an osx output / CoreAudio
      rate mix-up first, the mpg123 decoder second; separate on a second MPD with a null output (Sol; MiMo and DeepSeek
      claim null does not pace, which contradicts MPD's null `sync` option, to be checked); fixes: pin the output
      format or the device rate, or another decoder, verified by the same measurement and by the audible pitch.
      The user, 2026-10-12: "that speed-up is next in the TODO, do it" (moved to the top with the watchdog).
      Diagnosed 2026-10-12 (coordinator, a scratch MPD on port 6651 with a null output, sync yes): the same 48 kHz mp3
      runs at 0.9995-0.9997, so the mpg123 decoder and MPD's elapsed are right; the 1.09x comes from the osx output
      (CoreAudio / the device rate), intermittently. Proposed fix: `format "48000:16:2"` in ~/.mpdconf's audio_output
      (MPD resamples to the device's 48 kHz, so the device rate never switches), MPD restarted; the watchdog verifies.
      Done 2026-10-17 (asked again that day: "fix the faster playback of songs in rormpc now, ask the models"; this
      session, no worker: the change is host configuration in the dotfiles, ac09439 there). Cause, read in MPD's
      src/output/plugins/OSXOutputPlugin.cxx: on every Open the osx output sets the device stream's
      kAudioStreamPropertyPhysicalFormat to the format closest to the song (its sample rate too), then at once sets
      the audio unit's input format and calls AudioUnitInitialize; CoreAudio applies the rate change asynchronously,
      so the unit can be initialised against the old rate. Live check: on a 44.1 kHz song the speakers were at
      44100 Hz. Fix: `format "48000:*:*"` in ~/.mpdconf's osx audio_output and `resampler { plugin "soxr" quality
      "very high" }`, MPD restarted with `brew services restart mpd`. Verified: a 44.1 kHz song now leaves the
      speakers at 48000 Hz (the check that failed before), and the rest of that song (124.09 s) played in 123.5 s
      of monotonic time, from the status read to MPD's `idle player` at the song change (ratio 1.0048, against 1.088
      broken). Round 20261017-150106-b061: DeepSeek 7d19b7da useful (right that MPD changes the device rate), MiMo
      a445c4ec, Sol 1716163b and Argon a4fb21c8 partial (MiMo and Argon claimed MPD never sets the rate, wrong per
      the source). Follow-ups in "Proposed". The watchdog stays the long-run check.
      Measured again 2026-10-18 on scratch MPDs (the scripts, configs and logs are in
      ~/.local/share/rormpc-mpd-osx-race-2026-10-18): the 8.8% race did not reproduce in 28 output opens with
      44.1/48 kHz songs alternating, neither with the format unpinned (the speakers' rate switched on every song
      change) nor with `format "48000:*:*"`; the ratio of MPD's elapsed against the monotonic clock stayed within
      0.99-1.01. The earlier 1.033 of the seek-based script was its own artifact: it compares MPD's reported
      duration with wall time, and the same song gives 1.033 with and without the fixed format, so it is the file's
      duration estimate, not a rate. So the report to MPD still lacks a reproducer (see "Proposed").
      Found instead, also 2026-10-18: with `format "48000:*:*"` (soxr and libsamplerate alike) MPD 0.24.15 aborts
      (`Assertion failed: (nbytes > 0), function PlayChunk, file Thread.cxx, line 269`) when a seek into the last
      ~12 s is followed by a song change that changes the resampling ratio (44.1 -> 48 -> 44.1); reproduced in both
      configs, not when the same songs end naturally, and not with the format unpinned. New item in "Proposed".
      Fixed 2026-10-18 by moving the conversion into the decoder: `audio_output_format "48000:24:2"` in
      ~/.mpdconf instead of the per-output `format` (dotfiles dde312e), MPD restarted, queue and position restored.
      Isolated the trigger on scratch MPDs first: the `null` output with the same per-output `format` and the same
      seeks survived 8 rate changes (`output: converting ...` in its log too), so it is not the generic filter chain;
      the global `audio_output_format` puts the conversion in the decoder (`decoder: converting to 48000:24:2`) and
      survived 8 near-end seeks with 14 rate changes. Two generated 30 s sine files did not reproduce the abort even
      with the per-output `format`, so it also needs files whose length leaves a partial chunk (the real songs are
      204.157 s and 290.458 s). Verified the installed config on a scratch MPD holding a copy of it: no abort over
      8 near-end seeks with rate changes, MPD's elapsed against the monotonic clock 0.9962-1.009 over 6 song
      changes, speakers at 48000 Hz throughout. Round 20261018-015021-5614 asked (Sol failed on its usage limit;
      MiMo partial, Argon useful, and Argon's `audio_output_format` was the idea that made the config work).

- [x] A watchdog for playback speed (asked 2026-10-12: "some check has to be turned on: the song's time against the [t-s2qyloqw]
      real time that passed. Ask the models."). Round 20261012-024045-054e (sol d9ce8be0, MiMo e77e5e48, DeepSeek
      e154f8dd, all useful): in mpd-player (it runs with rormpc closed); samples on MPD's player idle events and the
      existing 30 s heartbeat, MPD status paired with the monotonic clock (never the wall clock); a fresh baseline on
      any player event, song change, pause (also the gap's 0:00 pause), seek, MPD restart or reconnect, a sample gap
      over 45 s or sleep; alarm when two consecutive 25-40 s windows are both above 1.03 or both below 0.97 (Sol; MiMo
      and DeepSeek 1-2% over longer windows); one macOS notification ("playback clock about 9% fast"), a state field
      the TUI shows, a JSON log line with the evidence (song, audio format, device nominal rate, samples, ratio);
      the device rate only as evidence, never an alarm alone; no automatic fix, at most a "reopen output"
      (disable/enable) action after the evidence is saved.
      Done 2026-10-12 (ca81fd2, installed with the companions tools@ca81fd21331c, pushed by the run) through herdr todo run r-m4vo3vt3 (pi-anthropic worker; reviewed by the coordinator):
      tools/src/rormpc_tools/player/rate.py samples MPD status on the daemon's existing wakes (monotonic midpoint,
      calls over 250 ms dropped), windows of 25-40 s, resets on song change, non-play state, seek (> 3 s off),
      sample gap over 45 s, clock discontinuity over 5 s, restart; two consecutive windows above 1.03 or below 0.97
      alarm: one notification, `rate_alarm` in player.json, a line in rate-alarms.jsonl with the samples and the
      device's nominal rate; cleared after two healthy windows or a reset. The rormpc status-bar display of
      rate_alarm is a later item.

- [x] After the move of rormpc-tools into tools/ (decided d-kgxuqbvh, pushed 2026-10-12 as 3358f5d): one repository, [t-6bzyvzb6]
      one coordinator. A worker: scripts/rormpc_install.sh installs the tools from the same ref as rormpc (`./tools`
      for a local install, `git+…/rormpc@REF#subdirectory=tools` otherwise; RORMPC_TOOLS_TAG goes, the companions
      record compares that ref); `.herdr/checks.toml` gains `tools` = `uv run --directory tools pytest -q`; a root
      `.github/workflows/tools.yml` on tools/** (pytest and the contract tests) replaces tools/.github; AGENTS.md
      "Coordination" says one coordinator for the whole repository and folds in tools/AGENTS.md's rules; tools/TODO.md
      and tools/.herdr removed (their open items are in this file now); docs that name rofrol/rormpc-tools point to
      tools/. The old repository is archived on GitHub (its 62 tags kept; copies here as tools-v0.2.x).
      Done 2026-10-12 (36dcf03, installed and pushed by the run; companions now tools@36dcf03f4c33) through herdr todo run r-2qfitnzb (worker; reviewed by the coordinator): the installer
      installs tools/ from the checkout (or tools/ at RORMPC_REF remotely); the companions record is the last
      commit that changed tools/ (+dirty hash); RORMPC_TOOLS_REPO/DIR/REF gone; checks.toml gains `tools`; root
      tools.yml (contract tests, pytest, pyflakes); installer_smoke tests github.sha; one coordinator in AGENTS.md
      with a tools/ section; tools/AGENTS.md, TODO.md, .github, .herdr and scripts/release.sh (its vX tags would
      trigger rormpc's release.yml) removed; docs point to tools/. RORMPC_TOOLS_TAG stays only because
      rmpc/src/shared/dependencies.rs reads it at build time (follow-up item).

- [x] Film score / soundtrack genre: find why "Cast Away Theme" (Alan Silvestri) has none and make the [t-2i7pvwhq]
      classification available in Hits genre filters ("Soon: missing film score / soundtrack genre").
- [x] Playing-song indicator in Hits: consult the models and prepare mockups of the variants, then ask in "Needs a [t-2e5e552e]
      decision" which one to build ("Soon: playing-song indicator in Hits").
- [x] Previous in the weighted shuffle: `shuffle prev` in mpd-player over the real playback history, neutral [t-bzr7nkwx]
      outcome, prev transitions logged for `musicdb import-skips`. Without the "restart in the first seconds" rule
      until it is decided ("Previous in the weighted shuffle"). Done in rormpc-tools 7bf33bb (not released).
- [x] Previous: rormpc's Previous and Hammerspoon's media key send `shuffle prev` to mpd-player (channel `rormpc`) [t-b3gjc7et]
      while it runs, `mpc prev` without it. Done: rormpc 54da00c, dotfiles 22e3b81 (Hammerspoon not reloaded). Both
      send it only when mpd-player is subscribed and its shuffle.json has `trail`, so until the rormpc-tools
      release they keep plain `mpc prev`. mpd-player's `playid` starts playback even when MPD was paused.
- [x] Versions: "Delete this file…" on a Versions file row (copy → merge, other recording → delete), the Queue [t-zypmsrmj]
      `≋` marker, "Find versions…" with a key and Back to the Queue row ("Versions: delete a bad version…").
      Done in f079b15 (worker; checked on a scratch MPD without deleting or merging anything real).
- [x] Versions: chromaprint comparison within a group, the "Same recording? audio match" suggestion, default file [t-jv3j5npa]
      to keep, rename "Remove duplicate entries" ("Versions: audio fingerprint finds copies").
      Done in rormpc 66d1b50, rormpc-tools ec3fe5f (worker; not released).
- [x] Lyrics: Polish translation from tekstowo.pl next to the original, sidecar storage, layouts and highlighting. [t-254mj4wn]
      The LLM fallback waits for its decision below ("Lyrics: Polish translation next to the original").
      Done in rormpc 1291c99, rormpc-tools 4cf9271 (worker; tools not released: after v0.2.31).
- [x] Media keys: document mpd-now-playable in RORMPC.md ("Media keys and Now Playing", first item). Done in 4f06a88. [t-5jz2sazq]
- [x] Mute: test a mute that spans an mpd-player gap silence on a scratch MPD ("Mute for…"). Done (rormpc-tools 4322e1a). [t-6mosa7ei]
- [x] Tests without CI: the missing rormpc-tools pytest cases and ro-listenbrainz-mpd listen-rule tests, run [t-wsuaycwe]
      locally ("Tests and GitHub Actions…", first two items).
      Done: rormpc-tools 060fa49 (179 tests), ro-listenbrainz-mpd 862c08a (12 tests); no bugs found.
- [x] Up next races: random off (entries skipped past end up before the current song) and mpd-player down while [t-bpeaqsb3]
      an Up next song plays ("Test report of 2026-10-06").
      Done in rormpc-tools 76bd09e (worker).
- [x] Release rormpc-tools v0.2.32 (Polish lyrics translations, new tests, the Up next fix) and install rormpc [t-ovz4sp6u]
      from master (decided 2026-10-07; the coordinator does it).
      Done: v0.2.32 pushed and installed with `companions`, rormpc d50147b installed.
- [x] Hits playing-song indicator, variant B: the playing row in `highlighted_item_style` and `▶0` in Next [t-i27yc5yb]
      (decided 2026-10-07; mockups and pitfalls in "Soon: playing-song indicator in Hits"). Done in 4b16420 (worker; not yet installed).
- [x] CI: GitHub Actions for rormpc-tools (pytest), ro-listenbrainz-mpd (`cargo build` + `cargo test` on Ubuntu) [t-ruly7mly]
      and the rormpc installer smoke test, plus `push` in rormpc's ci.yml; triggers as in "Tests and GitHub
      Actions…" (decided 2026-10-07; a worker writes them, the coordinator pushes). Done and pushed: rormpc-tools and
      ro-listenbrainz-mpd `test.yml` green on the first run; rormpc ci.yml runs on push (Build & Test green on
      Ubuntu and macOS; Formatter only on pull requests, decided by the user, the fork is not rustfmt-clean);
      `installer_smoke.yml` is manual (workflow_dispatch) until it passes once; `RORMPC_TOOLS_REF` / `RO_LB_REF`
      override the pinned tags.
- [x] Installer on macOS: `rormpc_install.sh companions` silently fails to add the listen rule to a fresh [t-al7sdhpj]
      scrobbler config (macOS awk rejects the newlines in `-v rule=...`; the `awk … && mv` hides it, prints "set
      the 90%-without-a-seek rule" anyway and leaves a .tmp file). Pass the rule with `\n` escapes, fail loudly,
      no .tmp left (found by the CI worker 2026-10-07; fix approved by the user). Done: the rule goes through ENVIRON.
- [x] "Pause for…" replaces "Mute for…" (asked 2026-10-07: "I don't want Mute for at all, only Pause for"): a timed [t-7tk6ukdy]
      pause owned by mpd-player with a wall-clock deadline (works with rormpc closed, across sleep and restarts);
      at the deadline it resumes only if still paused by this timer; a play by anyone cancels the timer. Remove
      Mute for… (module, commands, menu items, slider countdown, docs) in the same change; reuse its deadline and
      generation/error plumbing. Test on a scratch MPD. Done in rormpc 1c627ee, rormpc-tools 33f0cb9 (worker): `pause start|extend|
      resume|cancel`, pause.json; resumes only while MPD is still paused on the same song; the gap neither starts
      nor ends a silence while the timer holds the pause; an old mute.json restores its saved volume once. rormpc:
      `om` → ShowPauseMenu (`op` is ShowDecoders), `paused · 12:34` on the volume slider; `ShowMuteMenu` stays a
      serde alias until the dotfiles config says `"om": ShowPauseMenu` (switch it after installing). Known limits:
      play then pause on the same song while the daemon is down cannot be told apart and resumes; an MPD restart
      (new song ids) cancels the timer. Not installed yet.
- [x] Release rormpc-tools v0.2.33 (Pause for…), install rormpc (Pause for…, Hits ▶0) and switch the dotfiles [t-ttktfa4d]
      config to `"om": ShowPauseMenu` (decided 2026-10-07: now; the coordinator does it).
      Done: v0.2.33 pushed and installed, rormpc b230400 installed, dotfiles `om` → ShowPauseMenu (config loads).
- [x] Keys pasted or sent in one batch with `/` go to the filter, not to normal-mode commands, in the Queue [t-fr3ow6in]
      filter and the ordinary filters (approved 2026-10-07; see the item under "Done: live Queue plan view"). Done in
      13715f4 (worker): the key resolver sent the resolved action back through the app event channel, behind the
      keys already queued from the same batch; it now returns it and the event loop handles it before the next
      key. Checked on a scratch MPD (Queue and Directories filters, consume/single/random unchanged).
- [x] Live playlists, first version (decided 2026-10-07): public YouTube playlists only, manual check, first [t-pa3ol6v2]
      import reviewed, batch accept; the prerequisite `yt-mp3-mb --batch --json` in dotfiles, the `liveplaylist`
      CLI, the rormpc URL modal and pane ("Live playlists: paste a playlist URL…"). Done (worker): rormpc-tools e30be03
      (`yt-mp3-mb --batch --json`), bafa5ce..7615267 (`liveplaylist`), rormpc f3d0aa7 (`Pane(LivePlaylists())`, the URL
      input from its Enter menu). Not tested: a real download from a live playlist (the one allowed live listing
      returned 0 items); the UI was checked on a scratch MPD with hand-made items. Needs a release and a
      `Pane(LivePlaylists())` tab in the user's config (snippet in RORMPC.md).
- [x] Release rormpc-tools v0.2.34 (Live playlists), install rormpc (Live playlists pane, keys sent with `/`) and [t-w6hhhinf]
      add a "Live playlists" tab after Playlists in the dotfiles config (decided 2026-10-07: now).
      Done: v0.2.34 pushed and installed, rormpc 100c801 installed, dotfiles 9474d93 (tab + `gl`; the config loads).
- [x] Lyrics: machine translation with Claude through the Anthropic API when tekstowo.pl has none (decided [t-o4qhbf6g]
      2026-10-07): line by line with the stanza as context, labelled "machine translation", the key from the
      user's rormpc-tools config (never in the repo); the model id from the claude-api reference.
      Changed 2026-10-07 (the user: "ask the claude CLI, the way pi does it"): no API key; it calls the Claude
      Code CLI (`claude -p`, no tools or MCP, no session) like ~/.claude/skills/claude/ask_claude.py, on the user's
      Claude Code login. Done in rormpc-tools 181d233, rormpc 6c86ea1 (worker): `claude -p` (default
      claude-sonnet-5-5, `translate_model` in config.toml or RORMPC_TRANSLATE_MODEL), strict JSON per stanza, line
      counts checked with one retry, nothing stored on a mismatch; never replaces "mine" or a human translation.
      One live call on invented lyrics (4.4 s). Released 2026-10-07: v0.2.35 pushed and installed, rormpc 73bec6e
      installed; dotfiles be85114 renumbers the digit keys (1 Hits … 0 Live playlists; gy Lyrics, gd Deleted,
      gf Search).
- [x] The tab bar does not fit and cannot scroll (user, 2026-10-07, screenshot: "the tabs do not fit, [t-jpdlezoo]
      and mouse scroll does nothing; also 3 jumps to Directories"). The bar cuts "Ver…" at the right edge
      (Hits, Queue, Up next, Shuffle, Directories, Artists, Album Artists, Albums, Playlists, Lyrics, Deleted,
      Ver…); the wheel over it should scroll it or the bar should wrap/shrink; the digit keys should match the
      tabs' order (3 should be Up next, the third tab, not Directories). Reported to the herdr coordinator.
      Decided 2026-10-07: digits follow the tab order (1 Hits … 9 Playlists, 0 Live playlists); the coordinator
      changes the dotfiles keymap, a worker the bar. Done: dotfiles be85114 (digits), rormpc 692a803 (worker): the bar
      scrolls the active tab into view, ‹ › mark hidden tabs, the wheel over the bar and a click on ‹ › scroll it
      without switching, a click hits the tab drawn there. Not installed yet.
- [x] Asked 2026-10-07: "download the top 1% of music" (in Polish: "ściągnij top 1% muzyki"). Facts: Hits' Top % [t-qdbex3hq]
      is the share of a chart cohort (Billboard year-end, per year); `hits fetch` is a verified import queue for
      missing songs (rormpc: "Fetch missing…"). Scope decided 2026-10-07: Billboard year-end, all years, the top 1%
      of each year's chart, the missing songs through `hits fetch` (verified).
      Done 2026-10-07 (worker): `hits --years 1959-2025 --top 1-1` = 64 songs, 11 owned; of the 53 missing, 18
      promoted into the library, 28 wait in review (21 "other recording of the same song", 5 "different song", 2
      "no MusicBrainz match"), 7 failed (no YouTube result within 5 s of the chart recording's length: Macarena
      Bayside Boys Mix, Walk Like an Egyptian, Believe, Breathe, Happy, Low, Surfin' U.S.A.). Review: `hits fetch
      status`, or the Hits pane's "Fetch missing…".
- [x] Asked 2026-10-07: "in Hits, all my playlists as a source" (in Polish: "w Hits jako źródło moje playlisty [t-udmkxp36]
      wszystkie"). Facts: Hits' Source cycles Billboard US / my charts / whole library / my likes / recommended.
      Done in rormpc-tools bb4f9c2 (`hits --source playlists`), rormpc e2003ea (Source "my playlists" after "whole
      library") (worker): every stored playlist's owned songs once, ranked like the library, the playlists in Why;
      generated playlists left out (GENERATED_PLAYLISTS: Hits/My charts/Library/Likes/Recommendations/My playlists
      labels, LB, Folder, Skipped, Not finished; Tag … and Live playlists count). Real MPD: 268 songs from 4
      playlists. Not released yet.
- [x] Top 1% review (the user, 2026-10-07): accept the 21 "other recording of the same song" items (`hits fetch [t-bfeuks3n]
      accept`), reject the 5 "different song" items (`hits fetch reject`: the staged file is deleted and the
      rejection stays in the fetch queue state, $XDG_STATE_HOME/rormpc-tools/fetch/queue.json, so `hits fetch
      add` never queues them again; `hits fetch clear` keeps rejected ones; `hits fetch retry KEY` undoes it).
      The 2 "no MusicBrainz match" items stay in review. Done (worker): 21 accepted into Hits/<decade>s, 5 rejected,
      2 in review (Love Will Keep Us Together, Shadow Dancing); the fetch queue: 39 ok, 7 failed, 5 rejected.
- [x] Release rormpc-tools v0.2.36 (the "my playlists" Hits source) and install rormpc (decided 2026-10-07: now). [t-tt5krlur]
      Done: v0.2.36 pushed and installed, rormpc d6c0aad installed.
- [x] Add `"o": TogglePlanView` to the Queue keymap in the dotfiles config (approved 2026-10-07; from the [t-px26n5bu]
      music-mpd coordinator: the plan view was installed without it). Done: dotfiles 5c46056; `o` opens the plan view.
- [x] Leave evidence if the Previous forward jump repeats: MPD `log_level "verbose"` on the real MPD and a [t-fqlmsgoc]
      timestamped log line per media-key command in Hammerspoon (approved 2026-10-07; edit only, the user
      reloads Hammerspoon and restarts MPD). Done (worker): dotfiles 06fa6b7 logs each media-key event and mpc
      command to ~/.local/state/hammerspoon/media-keys.log (capped at 4000 lines); /opt/homebrew/etc/mpd/mpd.conf
      gets `log_file "~/.local/state/mpd/log"` and `log_level "verbose"` (backup mpd.conf.bak-2026-10-07; the log
      is not trimmed: turn it down after the investigation). Takes effect after the user restarts Hammerspoon and
      MPD (`brew services restart mpd`).
      Reverted 2026-10-08 (the user agreed): the cause was found (Karabiner), so mpd.conf was restored from the
      backup before MPD ever restarted with verbose logging.
- [x] Media keys through Karabiner, not Hammerspoon (found 2026-10-07: Hammerspoon's systemDefined tap never sees [t-ve5m6ji6]
      the MacBook's F7/F8/F9: Karabiner maps them to consumer keys from its virtual HID device, which go straight
      to Now Playing, so Previous went mpd-now-playable → MPD `previous` and counted a skip; the Input Monitoring
      permission did not change that). Decided by the user: a Karabiner complex modification catches F7/F8/F9
      without fn and runs a small script (prev: `shuffle prev` to mpd-player when it is subscribed and
      shuffle.json has `trail`, else `mpc prev`; play/pause: `mpc toggle`; next: `mpc next`), logging each command
      to ~/.local/state/hammerspoon/media-keys.log's successor; Shift+F7/F8/F9 pass through to the Now Playing app;
      fn+F7 stays F7. Remove the media-key code from Hammerspoon. Done (worker): dotfiles d0f17c6 (scripts/mpd-media-key,
      log ~/.local/state/media-keys/media-keys.log), 8b466bb (Karabiner rule), 268c9c5 (Hammerspoon override
      removed); rormpc 8af5c36 (RORMPC.md). Verified with a real F7 on 2026-10-07 18:40: `shuffle prev` sent, mpd-player
      went back to the song that really played before, recorded as "back" (no skip), prev.jsonl "confirmed".
- [x] Asked 2026-10-08: "can I see it in rormpc to decide? ask the models" (in Polish: "i mogę to jakoś zobaczyć [t-dh3qdczn]
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
- [x] Review fetch downloads in rormpc (approved 2026-10-08, the whole design from the consultation): `hits fetch accept --as-chart KEY` [t-fkmujm4p]
      (chart artist/title, YouTube source in a comment, no MBID; accept fails on a missing staged file; retry skips
      rejected video ids) in rormpc-tools; in the Hits pane a "Downloads (N to review)" list, expected vs
      downloaded details, Preview / Stop, Open on YouTube, Accept as the chart song, Accept with current tags,
      Reject (durable, confirmed), Try another candidate.

      Done in rormpc-tools 07055cd, rormpc ed3ccd1 (worker): `hits fetch accept --as-chart`, `hits fetch another`, rejected
      video ids skipped, accept fails on a missing file; Hits "Downloads (N to review)" first in the filter column,
      expected vs downloaded details, a shared preview player (rormpc_preview.rs, also used by Versions), and a
      retry race fixed (the worker starts only after the decision is written). Not released yet.
- [x] Release rormpc-tools v0.2.37 (Downloads review) and install rormpc (decided 2026-10-08: now). [t-won3gm2v]
      Done: v0.2.37 pushed and installed, rormpc 9bd3837 installed.
- [x] Installer smoke test (decided 2026-10-09): the coordinator dispatches `installer_smoke.yml` on rofrol/rormpc; [t-gd6dbzxj]
      when it passes, a worker adds push to master, pull_request, a weekly schedule and release tags like the other
      workflows (and drops the "until it has passed once" note), the coordinator pushes. A failure is fixed first.
      First run 2026-10-09 (run 37990844352) failed in "Fresh user with a lingering systemd user session" after
      36 s with no output: the journal shows ci's user manager up ("Startup finished in 98ms", dbus.socket
      listening) and two `systemctl` calls from `as-ci 'systemctl --user is-system-running --wait || systemctl
      --user --failed'` failing with "Failed to connect to bus: Permission denied"; the user generators also tried
      /home/runner/.config. Next: a worker reproduces it in an Ubuntu VM (OrbStack, as on 2026-10-03) and fixes
      the step; then the coordinator dispatches it again.
      Fixed in e8e42cf (worker, reproduced in an OrbStack Ubuntu 24.04 VM): the runner image's /etc/environment sets
      XDG_CONFIG_HOME=/home/runner/.config, which pam_env passed to ci; `as-ci` ran from the runner's work dir ci
      cannot read; the bus check now runs as ci; ffmpeg added (musicdb update needs it); and an installer bug: a
      failed `uv tool install` passed silently inside `if …; fi || {…}`. Second run dispatched: 37994088698.
      Second run green (degraded user manager handled). Triggers added in bac17c0 (worker): push to master and pull_request
      with a paths filter on the installer and the workflow, release tags, weekly "47 5 * * 1"; pushed.

- [x] Plan (asked 2026-10-09, design only): "there should be one combined view now: Queue, Hits, Shuffle. So I can [t-34iwvjvl]
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
      Done in dd9ce9d (worker): plans/combined-view.md, a "Play" tab with normal and weighted modes, preview + Apply,
      ± set chips, scoped pins/exclusions, smart lists, 6 mockups; consult rounds 20261009-230419-b57c and
      20261009-230647-3341. Its 12 open choices are in "Needs a decision" ("Combined view:").

- [x] Combined view, phase 1 (decided 2026-10-09, plans/combined-view.md): `hits` set chips (±, fixed rows [t-i2cikete]
      Billboard, my likes, my playlists, recommended), Rank by, Years of (default follows Rank by), CLI and Hits pane;
      `--source` kept as a shorthand; contract tests.
      Done in rormpc dce9c58, rormpc-tools 6e6670a (worker; pytest 261, cargo test 989; clippy shows only older
      warnings; not released). Behaviour changes by design: Top % is cut before genres and sets (Billboard rock
      Top 10% 50 → 45 rows), `-n` cuts after the artist filter; rows without a rank get population size +
      position; "of N" counts the songs after the set algebra, before period and filters; Billboard with Years of
      other than chart reads every chart year (6,333 songs, ~0.8 s). The installed hits 0.2.37 rejects the new
      arguments: rormpc and rormpc-tools must be released and installed together.
- [x] Combined view, phase 2: exceptions (pins and exclusions with a scope; default scope the open smart list, else [t-52rii5vk]
      library; any exclusion beats any pin, a pin beats `-` rules; hand edits in the queue are one-offs), CLI + log,
      marks and actions in Hits and Queue, "show excluded", the exceptions list; `hits hide` read as Billboard-scope
      exclusions.
      Done in rormpc b9de184, rormpc-tools 25a7580 (worker; pytest 277, cargo test 996; UI checked read-only with a
      temporary data dir). `hits except pin|exclude|remove`, `hits exceptions`, log exceptions.jsonl; PinSong (+) /
      ExcludeSong (-) in Hits and Queue with a scope menu; ✚ ⊘ in Hits, "show excluded (N)", "⋯ exceptions…".
      Settled by the coordinator: `hits hide` on a recommendation row counts as a Recommended-scope exclusion (else
      hiding recommendations would stop working). Noted for phase 3 (Play knows the rules): Queue takes the scopes
      from the default ~/.cache/rormpc/hits/current.json, not a pane's configured path, and its rows show no ✚ ⊘
      marks; `hits except --file` needs the file in songs.jsonl (`musicdb identity sync` first); `list:ID` scopes
      wait for phase 4 (default_scope()).
- [x] Release rormpc-tools v0.2.38 (phases 1 and 2: ± set chips, Rank by, Years of, exceptions) and install rormpc [t-o6yoho4z]
      from master (decided 2026-10-10: after phase 2; the coordinator does it as with earlier releases).
      Done 2026-10-10: v0.2.38 tagged and pushed (gated on pytest 277 + pyflakes), companions installed, rormpc
      743c541 installed and pushed; the config loads. Restart running rormpc instances to use it.
- [x] Combined view, plan addition (decided 2026-10-09: "only Play" among the browsing tabs): a worker extends [t-5fvst33c]
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
      Done in 5cea5bd (worker, second consult round): plans/combined-view.md "Play absorbs the browsing tabs":
      full-width bodies Queue | Browse | Live reusing the existing browsers, P/t/a actions, the playlist editor in
      Browse › Lists, the Live inbox with multi-select and a badge, a new phase 3b after phase 3. Found in the code:
      a song appended to a queue holding a Hits source with weighted on is never drawn today (shuffle.py draws only
      from source.json's members). Open choices 13-20 are in "Needs a decision".
- [x] Combined view, phase 3: the Play pane (normal mode: queue with the filter column collapsed to one line, `h` [t-mnrl3wsr]
      opens it; weighted mode: plan projection), preview + Apply with the queue version check, confirmation only on a
      source kind change or more than 25% of the queue; the default config switches to Play, old panes stay.
      Done in rormpc c642d4d, rormpc-tools 00ddc71 (worker; pytest 278, cargo test 1003; tested on a scratch MPD: preview,
      Apply with the 25% confirmation, the queue-changed refusal, Esc, w on/off keeps the queue, rules hash keeps or
      starts a round, 0 owned refused, + / - marks; the mouse was not tested). Not released. Settled by the
      coordinator: the queue version is taken when `a` is pressed, not when the preview was made (mpd-player's
      priority writes bump MPD's version on every song change, so a preview-time version would refuse almost every
      Apply; a song change while the confirmation is open refuses it); `w` (ToggleWeightedShuffle) added to the
      default keys; key 2 (old Queue tab) is unbound until phase 3b renumbers. Noted: the default Play tab is full
      width (no album art or lyrics as the old Queue tab had); ✚ ⊘ show for every exception on a file, applicable
      or not; Pane(Play()) has the hits command and ~/.cache/rormpc/hits/preview.json fixed; Enter in a preview row
      plays/appends it as in Hits; re-applying the same rules works only with that result on screen ("The queue
      already plays these filters"). Your config needs a Pane(Play()) tab to show it (choice 18).
- [x] Combined view, phase 4: smart lists (named "Smart list"): save, picker, load as preview, Previous sources, [t-xouyelmu]
      MPD export "Smart NAME" on by default; no freeze action.
      Done in rormpc fad602e, rormpc-tools 7820d3c (worker; pytest 300, cargo test 1011; tested on a scratch MPD: save and
      update, load as preview, Apply, rename, duplicate, delete, default pin scope, a blocked list, export after
      Apply; a full `musicdb update` and the mouse were not tested). `hits lists` + smartlists.jsonl, `hits --list /
      --rules / --open-list`, list:ID scopes, the "Smart NAME" export in musicdb update (exceptions.jsonl and
      smartlists.jsonl now committed with the data). Default keys: S SaveSmartList, L SmartLists, SelectAlbum moved
      from L to M (your config sets none of them, so it gets these). Also fixed: InputModal's initial value was
      ignored (renames started empty), a set-chip result without Top % reloaded with Top 10% ticked (the Proposed
      "-n 0" bug may be this; check before doing it). Settled by the coordinator: Duplicate copies the list's
      exceptions. Noted: the picker's r/u/c/d letters are found through the actions bound to them (Rate, Update,
      ToggleConsume, Delete): rebinding those breaks them, arrows and clicks always work; Apply with an open list
      re-exports the list's saved rules even when the filters on screen differ; the first key after closing a
      modal can be lost when sent in one batch with Esc (seen with Esc then L; the modal closes through the event
      channel, see AGENTS.md). Not released.
- [x] Combined view, phase 5: sets from tags, MPD playlists, Live playlists and smart lists through "+ set…", with [t-sakhxqgq]
      a cycle check.

      Done in rormpc b5129e6, rormpc-tools 33421e5 (worker; pytest 322, cargo test 1015; UI on a scratch MPD: picker
      sections, counts, `/`, [+] and [-], formula, × clear sets, a cycle refused; the mouse was not tested). `hits
      sets [--json]`, `--set ±tag:NAME|playlist:NAME|live:ID|list:ID`, scopes `set:<kind>:<name>`, cycles reported
      in `hits lists --json`, `hits sets` and the L picker. Noted: a smart list used as a set brings its missing
      chart songs only when the outer rules have chart rows too; the picker's count of a smart list is its last
      "Smart NAME" export (none before one); "× clear sets" also turns the fixed chips off (mockup 6); `set:list:ID`
      (a list used as a set) differs from `list:ID` (the open list). Not released.
- [x] Installer smoke test: "Units are enabled and active" proves the programs run: each unit's MainPID and its [t-vrs32msa]
      executable, and a log line from it (decided 2026-10-09; a Type=simple unit is "active" with a missing binary).
      Done in 2b23483 (worker; failing runs shown in an OrbStack VM with each program missing): the step
      waits for both daemons' MPD channel subscriptions (an idle event, at most 20 events of 60 s), checks each
      MainPID runs the installed program and runs `musicdb --version`. Also found: `is-active` with several units
      exited 0 when any one was active. Green on GitHub: run 38012260320.
- [x] rormpc-tools: `musicdb update` reads `api_url` from the scrobbler config like the scrobbler, so a custom or [t-jridiu23]
      fake API URL is used for ListenBrainz (decided 2026-10-09); adjust the smoke test's musicdb step if its
      expected exit changes.
      Done in rormpc-tools c6a3f04 (worker; pytest 332 offline): every token call (validate, lookup, listens import,
      feedback, submit, playlists, delete) goes to the scrobbler config's api_url, built like listenbrainz-mpd builds
      it; anonymous calls in hits.py stay on the public ListenBrainz; tests no longer read the real macOS config
      (~/Library/Application Support/listenbrainz-mpd). The smoke test's musicdb step now fails on "connection
      refused" from its fake api_url instead of "Token invalid" (its comment is outdated). Not released.
- [x] Missing ffmpeg: a readable error in rormpc-tools instead of a traceback, and ffmpeg in the installer's [t-7odtbbdz]
      `status` and dependency hint (decided 2026-10-09).

      Done in rormpc afa3fed, rormpc-tools 00354fa (worker; pytest 338): the four entry points print one line naming the
      missing program (ffmpeg, yt-dlp, fpcalc, mpc) with its brew/apt install and exit 1; `companions` stops without
      ffmpeg, `status` lists it, the apt hint includes it; the smoke test's musicdb comment updated. Not released.
- [x] Scrobble status (asked 2026-10-09: "I also want to see in how many seconds the entry will be sent to the [t-kuqrbhgk]
      scrobbler, and what percent of the required percent is done, and whether it is still possible since the song
      was scrolled, e.g. to 20%, and a manual button to send it to the scrobbler. It has to take the information
      how many percent is needed to send to the scrobbler from the ro mpd listenbrainz daemon. Ask the models.")
      A worker consults the models first (how rormpc gets the rule and the current listen's progress from
      ro-listenbrainz-mpd: its config's listen rule, a state file or a command; the seek rule makes a listen
      impossible; what the manual send does and how it avoids a double listen), then builds it; open choices go
      to "Needs a decision".


      Done in rormpc 7569f50, ro-listenbrainz-mpd cabb936 (worker, consulted Sol + MiMo; cargo test 20 and 1020; end to end
      on a scratch MPD, scratch scrobbler and a local API stub: countdown, seek → impossible, oL sends exactly one
      listen, no second automatic one, "scrobbler not running"). The daemon writes status.json next to
      listens.jsonl on every change (rule, counted/required, counting|impossible|never|sent); rormpc watches it and
      adds only MPD's elapsed time; a manual send is an MPD message `submit <instance> <play>` on channel
      listenbrainz_listen, logged with "manual": true. Also fixed a flaky scrobbler test (7/40 → 0/40). Not
      released: needs a ro-listenbrainz-mpd tag, RO_LB_TAG bump and `companions`. To see it, your theme needs
      `(kind: Property(Status(Scrobble)))` (RORMPC.md "Scrobble status"); oL works with your config already.
      Open choices are in "Needs a decision" ("Scrobble status:").
- [x] Hits "my plays" rank leaves out the weighted shuffle's own picks with every Years of, not only with listened [t-67ushfwe]
      (decided 2026-10-10; rormpc-tools hits_rules.score and where `plays` is gathered; tests; RORMPC.md wording).
      Done in rormpc-tools 4407f0c, rormpc c9332a3 (worker; a new test fails on the old score for release, chart and
      rediscover): my plays and rediscover count plays without the shuffle's own picks; the Plays column still
      counts every play; "not lately" (days since the last play) is unchanged. Not released.
- [x] Install hints (asked 2026-10-10: "a line with the brew or apt install command instead of a traceback - hmm. [t-fbrj2tut]
      odd, this hardcoding. And what about Arch, Guix, Windows? I don't know myself. Ask the models."): decide after
      a consult round how rormpc-tools and the installer name a missing program (ffmpeg, yt-dlp, fpcalc, mpc) on
      macOS, Debian/Ubuntu, Arch, Guix and Windows, then change it.
      Consulted 2026-10-10 by the coordinator (Sol 15375124, MiMo 6c88853c). Agreed: one small map program →
      package per manager (only fpcalc differs: libchromaprint-tools on Debian, chromaprint elsewhere) as the one
      source, the docs table made from it; Guix: `guix install` now, the package in home.scm / a manifest for
      good; Windows: say unsupported, guess no winget ids; the installer supports launchd and systemd user units,
      not Guix's Shepherd: say so before installing services. For optional programs say what stops working.
      Diverged: Sol = generic line + docs link, no detection; MiMo = detect the OS (/etc/os-release ID / ID_LIKE,
      sys.platform), print one matching command, generic fallback (open choice below). Checked: the catch is
      already narrow enough (it answers only when the error's filename is a known program, else re-raises).
      Decided by the user 2026-10-10: always a generic line ("<program> not found on PATH; <what stops working>")
      plus a link to one per-manager table in the docs (brew, apt, pacman, guix incl. the home.scm line; Windows
      unsupported); no OS detection; the installer says Shepherd is not supported.
      Done in rormpc-tools 80b34f4, rormpc fa38191 (worker; pytest 345): one generic line + link to the README's
      "Dependencies" table (brew, apt, pacman, guix with the home.scm note; Windows unsupported); optional programs
      say what stops working, once per run; `companions` stops on Linux without a systemd user session and names
      Shepherd as unsupported. Unverified package names are footnoted (rsgain on Arch/Guix, fpcalc in Guix's
      chromaprint); the coordinator doubts Guix's "mpclient" for mpc (Guix may call it mpd-mpc): check on the
      Guix machine. Not released.
- [x] Scrobble status follow-ups (decided 2026-10-10): the wording "62% counted · need 90% · in 1:24" (counted = [t-kk34m5dw]
      since the last seek; with a max-seconds cap lower than the fraction show the effective share, e.g. "need
      80%/4:00"; impossible: "no scrobble: seek to 20%; need 90%"; "no scrobble: song too short"; "scrobbling off";
      "scrobbled ✓"; "scrobbler not running"; Sol's table in the consult round), a "Send to ListenBrainz now" item in
      the Queue menu for the playing song next to oL, and `(kind: Property(Status(Scrobble)))` next to the progress
      bar before the time in your theme (dotfiles ~/.config/rormpc/themes/roman.ron).
      Done in rormpc 7395fbc, dotfiles 9c28609 (worker; cargo test 1021): the wording with conservative rounding
      (counted down, required and countdown up), an unknown length shows times ("0:30 counted · need 1:00"); the
      Queue menu item on the playing row; your theme has a 40-column status field between the progress bar and the
      time (it takes that width even when empty) and loads with the installed rormpc. The menu item was not clicked
      in a live TUI. Not released. The daemon reports no "too short" or "rule off" state today (it has neither
      rule); rormpc shows those texts once it does.
- [x] ro-listenbrainz-mpd remembers a sent listen (manual or automatic) across its own restart in the middle of a [t-k5z6ost3]
      song, in its SQLite database, so a later automatic listen of the same play is never sent twice (decided
      2026-10-10).
      Done in ro-listenbrainz-mpd d21d7a7 (worker; cargo test 27): a sent_listens table in the cache database (song id,
      file, length, listen time, manual or not, last position and state); after a restart the same id + file +
      a position reachable since the last observation is the same play: status "sent", no second listen, a manual
      submit gets "already sent"; a real replay counts again. Left (in README): a pause longer than the song while
      the daemon was down can look like a replay; a stop and restart of the same song then looks like the same
      play; enable_cache = false remembers nothing; a kill between sending and writing the row loses it. Not
      released.
- [x] Combined view, phase 3b (decided 2026-10-09 "only Play", choices 13-20 decided 2026-10-10): Play absorbs the [t-nnfklbn2]
      browsing tabs (plans/combined-view.md "Play absorbs the browsing tabs", but with the user's layout: Browse in
      the left column as Filters | Browse, the playlist editor and the Live inbox as overlays); groupings Artists,
      Album artists (separate), Albums, Folders, Lists; P plays replacing the queue with Apply's confirmation rule,
      t play next, a append (an appended song joins a Hits source's files and round, "+N added"); immediate
      playlist edits, confirmed delete; an unapplied preview stays and recomputes; Live inbox with multi-select and
      a pending badge; keys designed by the worker; the default config drops the six browsing tabs.
      Done in rormpc 02f56bd, rormpc-tools c030cf3 (worker; cargo test 1028, pytest 346; UI on a scratch MPD with a copy of
      your config, mouse on the switch, groupings, rows and the Live badge). Keys: B Filters | Browse, [ ] or 5-9
      groupings, P play replacing the queue (turns weighted and random off; album/folder/list order), t Up next,
      a/A append ("+N added" also with weighted off, so turning it on later still draws them), 0 / gl / a click on
      "Live 3 · ↓ 2" opens the Live overlay (Space marks, accept/reject the marked). Default tabs: Play, Up next,
      Search, Versions, Deleted, Lyrics. Your config needs `"P": PlayReplace` in the navigation map (your own "P":
      SortByColumn(4) in the queue map hides the built-in P). Not checked live: an unapplied preview surviving a
      Browse action (it writes ~/.cache/rormpc/hits/preview.json, the real one), Live multi-select with real items
      (unit test only). Noted: P's confirmation names the first song's file, not its title; t in the old browser
      tabs and in the queue now adds to Up next too. Not released.
- [x] After phase 3b: release and install, and change your config like the new default (Play, Up next, Search plus [t-ekdxwuli]
      Versions, Deleted, Lyrics; decided 2026-10-10; the coordinator does it).
      Done 2026-10-10: rormpc-tools v0.2.40 and ro-listenbrainz-mpd v2.6.0-ro.6 tagged and pushed (tests green),
      installer 66ac1f7, companions and rormpc 66ac1f7 installed; dotfiles e7b9272: tabs Play, Up next, Search,
      Versions, Deleted, Lyrics; digits 1-3 for them, 4-9 and 0 left to the built-in keys (Browse groupings, Live
      overlay), gs and gl removed, "P": PlayReplace in the navigation map; the config loads. Restart rormpc.
- [x] (approved 2026-10-10 from Proposed) rormpc-tools README: describe `hits --set`, `--rank`, `--years-of`, `hits except` and `hits exceptions` (the [t-my7xykcn]
      phase 1 and 2 workers left it out of scope).
      Done in rormpc-tools d66cd82 (worker; README only, pushed).
- [x] (approved 2026-10-10 from Proposed) AGENTS.md "Checking UI behaviour": a copy of the user's config for tests needs the themes/ directory next to [t-5wj2gkbl]
      it (reported by the phase 1 worker 2026-10-10).
      Done in 2fc7dcc (worker).
- [x] (approved 2026-10-10 from Proposed) The exceptions scope menu starts with the cursor on its title, not on the default scope (the fork's menus have [t-ur2lkhds]
      no unselectable header; phase 2 worker, 2026-10-10).
      Done in 7843217 (worker): MenuModal::start_at; tested.
- [x] (approved 2026-10-10 from Proposed) Hits: a result file written with `-n 0` and no `--top` loads into the filter column with Top 1-10% ticked [t-3olzl6sl]
      (older bug, found by the phase 2 worker 2026-10-10).
      Already fixed by phase 4; a test added in 1a72200 (worker).
- [x] Guix package names in the rormpc-tools dependency table (asked 2026-10-10: "check it yourself with the [t-wke6bw2e]
      models"): verified in Guix's source (codeberg master): mpc is `mpd-mpc` (mpd.scm; Guix's `mpc` is the GNU MPC
      library), `chromaprint` ships fpcalc (mp3.scm, -DBUILD_TOOLS=ON), `rsgain` exists (audio.scm). Sol had all
      three right, MiMo was wrong on rsgain. Fixed in rormpc-tools 9c6c888 (worker, pushed); Arch's rsgain stays
      unverified.
- [x] Deleted songs are not downloaded again (asked 2026-10-10: "important. when I delete a song from the library [t-7l4vfqk2]
      and ListenBrainz, it has to be recorded among the deleted, so that it is not downloaded again. ask the
      models"): a worker consults the models, checks what `musicdb delete` and the Deleted tab already record, and
      makes every downloader (hits fetch, yt-mp3-mb, liveplaylist, Versions) skip a deleted song.
      Done in rormpc-tools b9025ce, rormpc 6e0972d (worker, consulted Sol + MiMo; pytest 358, cargo test 1032; the UI not run).
      Before: the delete journal was read by no downloader; "Fetch missing…" after `hits fetch clear` would have
      fetched a deleted chart song again, and so could yt-mp3-mb and liveplaylist. Now the blocks come from the
      delete journal (old deletions included, 17 today; undo removes the block): the YouTube video id blocks in
      every downloader before downloading; the recording MBID and the chart key block unless the delete only removed
      a duplicate; the chart key gates only chart rows, never a URL you give; a download identified as a deleted
      recording waits for review (hits fetch, Live "blocked") or is dropped with the reason (yt-mp3-mb). Deliberate
      re-download: `musicdb deletions allow ID` (deletions/allowed.jsonl), the Deleted tab's and Hits' menus, or
      `yt-mp3-mb --allow-deleted`. Today's deletion, Beyoncé "Irreplaceable" (14:40, permanent, with history), is
      blocked by its video, recording and chart key. One old deletion (Bugi, Bugi (Dance RMX).mp3) has no id at all
      and blocks nothing. Not released.
- [x] Weighted mode without the "unplanned" rows (asked 2026-10-10 with a screenshot: "what is the point of showing [t-ueyk5vwo]
      unplanned? ask the models"). Consulted (Sol 75b2c1f0, MiMo 8d2faf34): the rows give access to the pool, but
      "queue order" misleads (the shuffle never plays it) and a big source buries the forecast; both proposed a
      collapsed line. Decided by the user 2026-10-10: remove them entirely; the pool stays reachable through `/`
      search and Browse. In Play's weighted mode and the Queue plan view: past plays, 0 ▶, Up next, the forecast,
      nothing after it; check what `/` and the row actions need when the matching song is outside the forecast.
      Done in 0ff3471 (worker; cargo test 1032; looked at read-only on the real MPD): the weighted view and the Queue plan
      view end at the forecast; a `/` match outside it shows after the forecast as a "·" row ("N · in the pool, not
      in the forecast") with Play now / Play next, pin/exclude and delete; J/K do not move it. `/` in the TUI was
      checked by a unit test only.
- [x] Release rormpc-tools v0.2.41 (deleted songs never downloaded again) and install rormpc (Deleted tab download [t-pjs4nypw]
      column, Hits ⌫ rows, scope-menu cursor, weighted view without unplanned rows) after the weighted-mode change
      (decided 2026-10-10; the coordinator does it).
      Done 2026-10-10: v0.2.41 tagged and pushed (tests green), installer d16f07e, companions and rormpc d16f07e
      installed and pushed; the config loads; `musicdb deletions` works. Restart rormpc.
- [x] Rename the "Play" tab to "Music" (asked 2026-10-10 with a screenshot: "it looks like a button. another name? ask [t-mnd5vcib]
      the models"; Sol 0eb6ec94 and MiMo 2f26e04d both put Music first, Player second; decided by the user
      2026-10-10: Music): the tab label in the built-in default and assets/example_config.ron, SwitchToTab names,
      RORMPC.md and plans; the pane type Pane(Play()) stays (explicit configs keep loading); your config's tab and
      its "1" key too.
      Done in 6c45715, dotfiles fd5e600 (worker; checked in a debug build with your config: the bar shows "Music").
      The label lives in your config, so a rormpc restart shows it with the installed binary too.
- [x] Deleted becomes an overlay in Music (asked 2026-10-10: "why a separate Deleted tab and not inside Music? ask [t-fhah4yiw]
      the models"; Sol 1092fd62: overlay like Live with a badge for failed steps only; MiMo aee8f69d: keep a tab;
      decided by the user 2026-10-10: an overlay in Music): the Deleted pane's table, restore/retry and download
      allow/block in an overlay opened with gd (and a click on its badge), the badge counting failed or unresolved
      steps only; the Deleted tab leaves the default tabs and your config; the pane stays loadable.
      Done in 0ef3b05, dotfiles 23175a9 (worker; cargo test 1033; gd and Esc checked read-only, the badge with a fake
      musicdb; the badge click untested): gd = ShowPlay(Deleted), badge "Deleted ! N" only while steps failed.
      Incident: the dotfiles commit used ShowPlay(Deleted), which the installed rormpc did not know, so your config
      failed to load ("Unexpected variant named `Deleted`") until the coordinator installed rormpc 0ef3b05 minutes
      later (running instances were not affected). Lesson queued for AGENTS.md.
- [x] Remove the Up next tab; its last bits move into Music (asked 2026-10-10: "not needed at all. does it duplicate [t-gaxmdikq]
      Music? ask the models"; Sol 54478bdf and MiMo 3e4eecf6: yes, a duplicate; Music already has the ↑ rows with
      play now, J/K, Make next, remove). Missing in Music and to be added: a header row "Up next · N" between the
      playing song and the forecast in both modes, J/K/D/Make next bounded to that block (no moving a forecast row
      by accident), "Clear Up next…" (confirmed) on it, the rejected-play error in Music's footer even when no
      request row is left, a key to jump to the block (gu). The Up next tab leaves the default tabs and your config;
      the pane stays loadable.
      Done in 0b84299 (worker; cargo test 1035; on a scratch MPD), installed, then dotfiles f6a62a4 (the coordinator, after
      the install): "Up next · N" header in both modes, J/K/D/Make next bounded to the requests, "Clear Up next (N)…"
      on the header (Enter, menu, D), the rejected-play error in Music's footer, 2 and gu = ShowPlay(UpNext); "New
      round" moved from the Up next tab's menu to Music's song and header menus.
- [x] AGENTS.md: a change to the user's config (dotfiles) that uses a new action, pane or key name is committed only [t-5uihvffo]
      together with the install of a rormpc that knows it (the coordinator installs, then commits the config), never
      ahead of it (2026-10-10: ShowPlay(Deleted) broke loading the config until the install).
      Done in c3342e3 (worker).
- [x] Media key Next lags (asked 2026-10-10: "the MacBook's Next media key works with some delay. ask the models"): [t-ti4mv6c7]
      a worker measures where the time goes (Hammerspoon's handler, `shuffle next` / mpc next through mpd-player,
      MPD, mpd-now-playable), consults the models, and fixes the cause (no added waits; AGENTS.md "Added delay is a
      bug signal"); live tests with the media key on the user's MPD go to "Needs a decision" first.
      Investigated 2026-10-10 (worker, consulted Sol + MiMo; no code changed): F9 goes Karabiner shell_command →
      ~/scripts/mpd-media-key (zsh -f) → mpc next. On a scratch MPD the whole script takes 26 ms to MPD's answer and
      47 ms to the new song (79 ms under background CPU policy); MPD restarting the CoreAudio output costs 60-125 ms
      on every next; crossfade 0, mpd-player and gap not involved. Today's real presses (system log): zsh start → mpc
      18-600 ms, mpc → MPD stop 20-30 ms, sometimes 90-430 ms: Karabiner's console user server has no launchd
      ProcessType, so its children are throttled and wait for CPU while agents run; mpc resolves "localhost" through
      mDNSResponder (up to 260 ms under load). Fix options in "Needs a decision".
- [x] Restore Bon Jovi "Livin' on a Prayer" and make a full restore possible (asked 2026-10-10, next: "I deleted [t-lamy5upz]
      Jon Bon Jovi - Livin' on a Prayer. Restore it. And give a way to restore, together with the ListenBrainz
      stats. Ask the models"). Checked: deletion 20261010-162151-140--Bon_Jovi--Bon_Jovi_-_Livin__On_A_Prayer--
      lDK9QqIzhwk--20090617.mp3 at 16:21:51, permanent (the file is gone, not in the Trash), ListenBrainz listens
      deleted, the YouTube playlist entry removed, so it is blocked from downloads. A worker consults the models,
      then: a "Restore…" for any deletion (trash or permanent) that brings the song back (from the Trash, else
      downloaded again from its video with the block lifted), resubmits its ListenBrainz listens from what the
      journal or the local listens log kept (timestamps preserved, no duplicates), puts back the playlist entries
      and play counts/stickers, and runs it for this song; anything it cannot restore is reported.
      Built in rormpc-tools 47c4a10, rormpc ed89d68 (worker, consulted Sol + MiMo; pytest 365, cargo test 1037):
      `musicdb restore ID` (dry run by default, --yes, each step journaled in deletions/restored.jsonl and resumed by
      a rerun or musicdb update; a re-download goes to a staging dir and enters the library only when video id,
      length and recording match; a deleted ListenBrainz listen is resubmitted once LB no longer lists it, with its
      original time, never twice; `yt-playlist add` skips playlists that already have the video); Deleted overlay
      "Restore…" shows the dry-run plan in its confirmation. Dry run for this song (coordinator, 2026-10-10): file
      re-downloaded from lDK9QqIzhwk to its old path (MPD playlists still list it), song id back, stickers
      playCount 2, 2 play events, the 2026-10-06 16:35:51 listen waits until LB has processed the deletion, the
      2026-10-10 16:15:05 listen was never deleted on LB (imported back hourly), back into the YouTube playlist
      (appended), download allowed again. Open choices in "Needs a decision".
      Restored 2026-10-10 16:47 by the coordinator (approved by the user), from the rormpc-tools checkout: re-downloaded
      (another encode, noted), song id, stickers (playCount 2), 2 play events, YouTube playlist (appended),
      downloads allowed; the 2026-10-06 16:35:51 listen waits for ListenBrainz to process its deletion. The hourly
      retry needs `musicdb update` from a release with `restore` (installed is 0.2.41): next release.

- [x] Apply fails with "MpdError: Cannot execute command: 'add'. Detail: 'No such directory'" (reported 2026-10-10 [t-pdj4bzlw]
      with a screenshot: "error. to the TODO"): Music, Hits 1980-1989 top 1-10% "(modified)", weighted, round done,
      the queue left with 2 songs. Likely a file in the result that is no longer in MPD's database (e.g. a song
      deleted today, still in hits' cache or current.json) aborts the whole add; the screenshot was taken with the
      old rormpc d16f07e still running. A worker reproduces it on a scratch MPD and makes Apply/Play skip files MPD
      does not know (reported in the status, never an empty or half-replaced queue), and checks hits marks such rows
      as not owned.
      Cause (coordinator, 2026-10-10): preview.json was computed at 15:30; between 15:55 and 16:31 the user deleted
      Bette Davis Eyes, Look Away, Physical, Call Me, Faith and I Want to Know What Love Is with Ctrl-x; Apply added
      the stale preview's files, 7 of its 16 no longer exist, MPD refused `add`, and the queue was left with 2 songs,
      so the weighted shuffle drew only from those 2 ("only picks from two songs all the time"). Also: Apply must
      recompute (or re-check) a preview older than the library's last change instead of trusting it.
      Done in ee49112 (worker; on a scratch MPD: 4 files with 2 missing → 2 queued and "2 songs no longer in the
      library were left out"; none existing → queue untouched; the outdated-preview recompute is unit-tested only):
      the replace adds first and deletes only after, skips files MPD lacks, refuses an empty result; source.json
      lists only the added files; a preview older than MPD's db_update or the newest deletion is recomputed before
      Apply ("Preview outdated, recomputing…", Esc cancels). Released with rormpc-tools v0.2.42 (also `musicdb
      restore` for the hourly retry), installer 98f134e, rormpc 98f134e installed. Noted: the status bar cut a long
      message after "in their order" (Proposed).
- [x] The table shows an old result as if current (asked 2026-10-10 with a screenshot: no decade ticked, the table [t-fpcuvhmu]
      still only 1980-1989: "I don't understand why there is a year limit here when it is not selected in the UI.
      ask the models"). Checked: the rows were the previous result (Period 1980-1989, chart years) while a newer
      hits run (`--rank plays --years-of listened -n 0`, started for an intermediate click) ran for minutes; only a
      footer said "the table shows the previous result". Consulted (Sol 3713451e, MiMo 83e1f64b), both: a banner
      pinned above the table's column headers naming the filters the shown rows were computed for and the ones
      being computed, with elapsed time ("Showing Period: 1980–1989 · updating for Period: All years, Rank by: my
      plays · 1:12"); an empty decade list reads "All years"; cancel a running hits run as soon as a newer one is
      queued (newest wins; a superseded result never replaces the table); Esc stops a run. Sol also dims the stale
      rows (MiMo against; the coordinator takes Sol's dim plus the banner, so it shows without color too). Also:
      why "Rank by: my plays" takes minutes (the play-history split per run): cache it across runs.
      Moved up 2026-10-10: "do it next".
      Done in rormpc 08d0201, rormpc-tools 9b56fbd (worker; cargo test 1048, pytest 366; the UI checked read-only),
      rormpc 472e06a installed (tools not released yet): a two-line banner above the headers ("Showing Rank by:
      Billboard, Years of: chart · updating for Rank by: my plays, Years of: listened · 0:00 · Esc stops"), stale rows
      dimmed; a newer run kills the running one (its exit awaited as an event), each run writes a temp file renamed
      only if still newest; Esc stops a run. The real cause of the screenshot: with Billboard chart years an empty
      decade list silently sent 1980-1989; now it means all chart years (1959-now) and reads "Period: ‹All years›".
      "My plays" runs take 0.6-2.9 s here; the minutes came from queued old runs finishing one by one; the split is
      cached anyway (second run 0.001 s instead of 0.25 s).
- [x] The filter column stays open in normal mode too (asked 2026-10-10 with a screenshot after `w` off: "the whole [t-u5f7g5q2]
      selection on the left disappeared. ask the models"; it was collapsed to one "Source: …" line by the plan's choice
      5). Consulted (Sol 8d6fdb94, MiMo 5e81fe67): both would keep it collapsed with a notice; MiMo's "filters only
      affect weighted playback" dismissed (Apply replaces the queue in both modes). Decided by the user 2026-10-10:
      always open, in both modes (`h` keeps moving focus between the column and the table; Browse B as now).
      Done in d802514 (worker; read-only check on the real MPD), rormpc 7d45293 installed.
- [x] Download from https://radio.omarchy.org/ (asked 2026-10-10: "download https://radio.omarchy.org/. ask the [t-6jdipai6]
      models"). Earlier (2026-10-03) radio.omarchy.com did not resolve and the Live playlists plan said radio only if
      the station publishes a track history (ICY metadata brings ads and talk). A worker looks at what the site
      offers (a playlist, a track history API, a stream with metadata), consults the models, and builds it as a
      Live playlists source if it fits (the same review, accept/reject, deleted-song blocks); open choices go to
      "Needs a decision".
      Moved up 2026-10-10: "radio next" (right after the item in progress).
      Done in rormpc-tools bc000c6, rormpc 90d661c (worker, consulted Sol + MiMo; pytest 383, cargo test 1049): the site is a
      static playlist.json (no stream); `liveplaylist add https://radio.omarchy.org/` makes the subscription
      omarchy-radio (33 items today, all pending review), MPD playlist "Omarchy Radio"; check is one conditional
      GET (ETag); malformed or duplicate entries are skipped and the list counts as partial (nothing marked removed
      upstream); deleted songs blocked by path before download and by audio md5 after; an item whose audio is
      already in the library refers to it; `hits --set +live:omarchy-radio` works. Also fixed: `add` did not mark
      deleted songs at the first review (YouTube too). Live overlay shows the artist and a "Source" link. Tools and
      rormpc must be released together. Dismissed: auto-accept (MiMo), blocking by artist+title alone.
      Released 2026-10-10: rormpc-tools v0.2.43 (also the my-plays cache), installer and rormpc installed; the
      coordinator ran `liveplaylist add https://radio.omarchy.org/`: subscription omarchy-radio, 33 items pending
      your review in the Live overlay (0 / gl), MPD playlist "Omarchy Radio".
- [x] Wrong release years (asked 2026-10-10: "Sweet Dreams (Are Made of This) released 21 January 1983. And we show [t-ax24akeu]
      2000. Investigate with the models. A repair plan for the whole library?"): a worker finds where 2000 comes from
      for that song (file tags: date vs originaldate, a compilation's date, MusicBrainz release vs recording first
      release, what rormpc and hits read), measures how many library songs show a compilation or reissue year
      instead of the first release, consults the models, and writes a repair plan (which year field each view uses,
      how tags are corrected, dry run, review, undo) into "Needs a decision" before any file is changed.
      Second example (2026-10-10): "Physical (Olivia Newton-John) released 1981. We show 2004. Ask the models where
      these errors come from."
      Checked by the coordinator 2026-10-10: both files are YouTube "official video" downloads tagged by
      rormpc-tools mbtag.write_year: TDRC = TDOR = the matched recording's first-release-date. Physical matched
      recording 0ef6e09f (disambiguation "music video"), first release 2004 ("The Best Of", then video DVDs, a 2021
      reissue); its release group "Physical" is 1981. Sweet Dreams matched bf8373da (date=2000; MusicBrainz was busy,
      not fetched). Consulted (Sol 50e36a78, MiMo f8ac5fa1): both say the cause is treating "first release of this
      recording" as "original release of the song"; MusicBrainz keeps music videos, edits, live and re-recordings
      as separate recordings (a plain remaster usually is not); works have no date. Both: keep TDRC (this
      recording) and TDOR (the song's original release) apart; penalize "music video" matches or send them to
      review; never use YouTube upload dates. Differ: MiMo computes TDOR as the earliest first release over
      recordings of the same work and artist credit, excluding video/live/remix/edit and requiring an Album or
      Single release group (re-recordings like "Taylor's Version": TDOR original, TDRC new); Sol warns this collapses
      demos, re-recordings and different performances and prefers the earliest release containing the verified
      audio recording, with review for uncertain ones. Repair plan (both): a no-network suspect scan first
      (video disambiguations, compilation/video-only releases, a year far from the album's other tracks), a dry-run
      report (current vs proposed, evidence, confidence), review, tags written atomically with the previous values
      kept for undo, MusicBrainz at 1 req/s with a cache (~3 h for the whole library).
      Measured and planned (worker, 2026-10-10; rormpc-tools 8b7d561 docs/release-years-plan.md, matcher fix b6bc349, pushed):
      817 files, 638 dated, 651 with a recording MBID; 407 of 408 dated YouTube downloads have TDRC = TDOR; 69 matched
      to video recordings (49 dated), 29 to DJ-mix segments, 13 to cast recordings. MusicBrainz sample (189
      recordings): of 616 dated matched files ~206 right, ~104 differ (69 by ≥2 years, 36 by ≥5), ~306 not checkable
      (mostly no work link); of 37 hand-checked proposals 24 right; later-year proposals almost always wrong (8/9),
      earlier ones right or closer 25/28. Sweet Dreams confirmed: a video recording on 2000/2005 video compilations,
      the audio recordings give 1983. Every view reads TDRC; nothing reads TDOR. Plan: a hybrid rule (remix/live/
      cover keep their own date; else the earliest official album/single release among audio recordings of the same
      work and artist within ±10 s), dry-run report, review, atomic writes keeping the old values, ~66 min at 1 req/s.
      The matcher now treats MusicBrainz's video flag like "(video)" and prefers an audio candidate (new downloads).
      Its 5 open choices are in "Needs a decision" ("Release years:").
- [x] Hover over the like heart no longer highlights it (asked 2026-10-10 with a screenshot of Music's weighted view: [t-xyvqib4m]
      "on hover the hearts don't light up like they used to, to show they can be clicked. ask the models"): a worker
      finds which change lost the hover highlight (the old Queue pane had it; Music / the plan projection may not
      pass hover state to the like column), consults the models on hover affordances for clickable cells in a TUI,
      and restores it in Music (both modes) and wherever else hearts are drawn.
      Done in 44e4cf0 (worker, consulted Sol + MiMo; cargo test 1051), installed: under the mouse ♥ and ✗ are bold and
      underlined, an unrated song shows a dim underlined ♥; Queue, Hits and the weighted view (hover pinned to the
      song from the last paint, cleared on scroll; Browse draws no hearts).
- [x] The forecast should always show 10 ahead (asked 2026-10-10 with a screenshot: -2, -1, 0 ▶ Eye of the Tiger, [t-qxldaudy]
      then nothing: "shouldn't there always be 10 ahead? ask the models"). Cause (checked): mpd-player's fill_plan
      draws only songs not heard in the current round; at the round's end it reports "round done: all N heard
      (shuffle newround starts another)" and plans nothing until a new round is started by hand (MPD's random keeps
      playing without the plan); resting songs shrink it further on a small source (16 songs here). Consulted (Sol
      2ba4d6cf, MiMo dc3cf48c): both want songs shown across the round boundary, marked "next round", without
      marking anything heard, and the reason shown when fewer than 10 ("4 ahead; 12 resting"); rest needs an escape
      on small sources (the least recently played eligible song, shown as relaxed). They differ on starting rounds:
      Sol an automatic continuous mode (manual rounds kept as an option), MiMo keep manual rounds and show the next
      round's picks dimmed as provisional. The choice is in "Needs a decision".
      Decided by the user 2026-10-10: automatic rounds (a continuous mode; manual rounds may stay an option): the
      forecast always aims at 10 playable songs, those of the next round marked "next round" and nothing marked
      heard by planning; the reason shown when fewer ("4 ahead; 12 resting"); rest relaxed on a small source to the
      least recently played eligible song, shown as relaxed.
      Done in rormpc-tools 963a373, rormpc 5d0f741 (worker; pytest 397, cargo test 1055; on a scratch MPD round 2
      started by itself and the plan never went empty: "1 ahead · 3 resting · rest relaxed"), released as v0.2.44:
      automatic rounds (manual behind `shuffle rounds manual`), next-round entries marked ↻, relaxed rest ≈, the
      reason in the header; J/K do not swap across the round boundary. Limit: a source smaller than 10 cannot show
      several rounds (one MPD priority per queue entry, 12 h rest): the plan shows this round and at most the next
      one with the reason (in "Needs a decision"). ↻/≈ are not in the old Queue's ShuffleNext column.
- [x] Year range: "to" below "from" and no open ends (asked 2026-10-10 with a screenshot "from ‹ 2000 › to ‹ 1991 ›": [t-5asvphuo]
      "illogical that this is smaller than from. ask the models. Besides, no option to turn off either this or from?").
      Consulted (Sol 3f2a69ae, MiMo fb2e7e9b): clamp while stepping (from stops at to and back, equal years allowed;
      no push, no swap). Decided by the user 2026-10-10 (Sol's way): Space on a bound row toggles it, the year is
      remembered and re-clamped when turned back on; labels "From: Any" / "From: ‹ 2000 ›", the period line "Any
      year", "Up to 1991", "From 2000", "1991–2000"; a footer hint "←/→ change year · Space toggle bound"; hits gets
      an open-ended period.
      Done: rormpc b1c6983, rormpc-tools c00c20f (worker); released v0.2.45 and installed 2026-10-10 (UI not
      checked live by the worker).
- [x] Rename the radio's MPD playlist "Omarchy Radio" to "radio.omarchy.org" (decided 2026-10-10; `liveplaylist`'s [t-5rrypivb]
      rename or --name, the playlist file moved, the subscription updated).
      Done in rormpc-tools c9dfbf3 (worker): `liveplaylist rename ID NAME` moves the .m3u and records the name
      (refuses a taken name, undoes the move if the save fails); new radio subscriptions default to
      "radio.omarchy.org". Released in v0.2.46. The live rename of the user's subscription was blocked by the auto-mode
      classifier (see "Needs a decision").
- [x] Instant filtering without Apply (asked 2026-10-10, next after the radio rename: "filtering must be instant, [t-vtmaq4gz]
      without apply. Work out with the models how to do it."): a worker studies how a filter change in the Music view
      reaches the table, the preview and the MPD queue today (Apply, preview.json, the weighted plan, the 3b
      decisions in plans/combined-view.md), consults the models on making every filter change take effect at once
      (never interrupting the playing song, coalescing fast key repeats by the latest generation, not by a delay),
      writes the plan with its open choices to "Needs a decision", then builds it once they are decided.
      Planned in 8ace736 plans/instant-filtering.md (worker, consult round 20261010-230739-29d8): the queue changes
      by a diff (shared songs keep their MPD ids, only entries the previous result added are removed), stale runs
      dropped by generation, rounds scoped to the playing source, undo instead of Apply; one hits run 0.19-0.31 s.
      Found: Replace::run writes source.json after changing the queue (mpd-player can draw from the old source for
      ~30 s); the plan fixes the order in its step 2. Eight open choices in "Needs a decision" ("Instant filtering:").
      All choices decided by the user 2026-10-11 (the "Instant filtering:" and "Banned:" entries in "Needs a
      decision"), with two additions: a hand removal from the queue bans the song into a separate Banned list (file
      kept, ListenBrainz untouched), and Rate moves to y so r can be ToggleRepeat and z / Z undo / redo everywhere.
      Stage 1 (plan step 1 and the round rules) done in rormpc-tools 06f1aab (worker): rounds keyed by source.json's
      `source_id` (older files keep the rules_hash key), `heard` kept across member changes, a song is heard only
      after a scrobble-qualifying play (played time, checked on MPD's idle events and the heartbeat, no timer);
      Banned: `banned.jsonl` in the data repo, `musicdb ban|unban|ban --undo|banned --json`, `hits` drops banned
      songs after the ranks (counts.banned), mpd-player never plans one. Released in v0.2.50 and installed
      2026-10-11 (547 tests pass). The plan's `removed` field is obsolete: a hand removal bans (the user's decision).
      Found at the install: the listen rule is hard-coded to ro-listenbrainz-mpd's defaults (0.5 / 240 s), but the
      user's scrobbler config says listen_fraction 0.9, no cap, uninterrupted (a seek restarts the count); "as in
      scrobble" means the user's rule, so mpd-player reads the scrobbler's config. Stage 1b done in rormpc-tools
      (worker, cherry-picked after 06f1aab): the [submission] listen_fraction / listen_max_seconds (0 = no cap) /
      listen_uninterrupted (a seek over 2 s restarts the count) from ro-listenbrainz-mpd's config.toml, re-read on
      its mtime; released in v0.2.51 and installed 2026-10-11 (561 tests). Noted: after a daemon restart mid-song the
      elapsed time counts as played, while the scrobbler starts from zero.
      Noted: mpd-player matches bans by file path only (hits also by YouTube id / MBID); the one-folder migration
      (music-data) must rewrite banned.jsonl's paths with the others.
      Stages 2-3 done in rormpc (worker, cherry-picked; installed 2026-10-11, 1071 tests): source.json is written
      before the queue changes, with `source_id` and `generation`; every finished newest `hits` run reconciles the
      queue by a diff in one MPD command list (shared songs keep their ids, the playing song and Up next stay, a
      phone's entries stay, normal mode in rank order, weighted mode only appends); Apply, the Preview banner and
      `[ Apply ]` are gone from Music; the first filter touch replaces another source; 0 owned songs empty the
      upcoming songs; a counts line ("16 owned · 177 missing · was 10"); `o` toggles rows ⇄ plan|queue; `a` says once
      "Filters apply at once; z undoes". Noted from the worker: with nothing playing a filter change starts playback
      (as Apply did); in normal mode result songs before the playing one move after it and can play again (as
      Replace did); mpd-player's round counter catches up only at its next plan fill. Until stage 5, Previous sources
      records every applied change and an open smart list exports on every change.
      Stage 4 done in rormpc (worker, cherry-picked; installed 2026-10-11, 1084 tests): global FilterUndo / FilterRedo
      on z / Z act on Music's filters from any tab (20 deep, one entry per generation that reached MPD, redo cleared
      by a new change); the first entry after leaving an album or playlist brings it back in its order around the
      playing song; the last-change line ("Filter: … · was 16 · z undoes"). Defaults: r ToggleRepeat, y Rate (the
      default y copy was dropped, Y still copies). The user's config and theme label (z rep → r rep) applied and
      committed in dotfiles 3b49821 right after the install. Found: the worker's scratch test wrote the user's
      ~/.cache/rormpc/hits/preview.json (a fixed path); stage 5's worker adds an AGENTS.md note to run scratch tests
      with their own XDG_CACHE_HOME.
      Stage 5 done in rormpc (worker, cherry-picked; installed 2026-10-11, 1091 tests): `d`, a middle click and the
      row menu ("Remove from queue and ban (keep file; z undoes)") take songs out of the queue and `musicdb ban` them
      in the background; `D`, an Up next request's removal and "Remove repeated entries" do not ban; a ban is an entry
      on the z / Z timeline (z runs `musicdb unban FILE...`, not `ban --undo`, so an unban in the panel meanwhile
      cannot misdirect it); the Banned panel (`ShowPlay(Banned)`, default `gB`; added to the user's config in dotfiles
      727433a after the install) unbans with Enter / u; Previous sources and an open smart list's export record a
      rule set once one of its songs starts or when the queue moves to another source; +/- on a queue row applies at
      once while filters own the queue; preview.json follows XDG_CACHE_HOME (AGENTS.md note). Noted: z after banning
      several songs, some already banned, unbans those too; the first song start after rormpc starts from a stop is
      not recorded in Previous sources. Proposed below: the smart list picker's "Load" and "a Apply" now do the same.
      Stage 6 done in rormpc-tools (worker, cherry-picked; v0.2.52 installed 2026-10-11, 575 tests): `hits` keeps the
      library index (MPD songs, play counts) in ~/.cache/hits/library-index.json, keyed by MPD's db_update and the
      play DB / path logs / deletions / banned.jsonl, written atomically; path tie-breaks give a stable order.
      Measured on the real library (10 runs each, median / p90): billboard warm 0.300 → 0.199 / 0.207 s, cold 0.327 s;
      library by plays warm 0.254 → 0.147 / 0.158 s, cold 0.347 s. Billboard is still over the 0.15 s line, and every
      play changes the play DB, so the run after a song is cold. Per the user's rule (hits serve only if still over
      0.15 s after caching), the cheaper caching the worker found comes first: next item, stage 6b.
      Stage 6b done in rormpc-tools (worker, cherry-picked; 586 tests): the library index no longer keys on the play
      DB (play counts recounted with an event→file memo, without MPD), chart matches cached per year list. Measured
      (median / p90): billboard warm 0.104 / 0.111 s, warm after a play 0.129 / 0.144 s, cold 0.245 s; library by
      plays warm 0.132 s, after a play 0.344 s (measured under heavy load; to be measured again on a quiet machine).
      Below 0.15 s for billboard: `hits serve` is not built (the user's rule).
- [x] Omarchy Radio: a daily check (decided 2026-10-10): a conditional GET once a day (launchd on macOS, a systemd [t-ia6hx3xl]
      user timer on Linux, installed by rormpc_install.sh companions) that only adds new items as pending and
      notifies; nothing is accepted or downloaded by itself.
      Done: rormpc-tools 7eb4ac0 (`liveplaylist check --kind omarchy --notify`), rormpc a94611e (companions: a
      LaunchAgent at 10:00, a missed run follows on wake; systemd OnCalendar=daily, Persistent=true; a status line)
      (worker). Released v0.2.46 (with the rename command) and installed 2026-10-10; the first run: 0 new.
- [x] Media keys through mpd-player (decided 2026-10-10, after the consult in "Media key Next lags"): a command socket [t-u2z73nhw]
      in mpd-player as the one control interface (next through the plan, previous through its history, toggle);
      the user's Karabiner F7/F8/F9 call it with send_user_command (no shell, mpc or DNS lookup per press; the
      dotfiles change goes in with the install, AGENTS.md "The user's config"); mpd-player registers as the macOS
      Now Playing provider and the Linux MPRIS player with the same commands, installed and started by
      rormpc_install.sh companions, replacing mpd-now-playable (its docs and the user's LaunchAgent removed after the
      switch). A worker plans it first with the models (macOS Now Playing from Python: PyObjC MediaPlayer or a small
      helper; MPRIS via D-Bus), open choices to "Needs a decision"; live key tests are the user's.
      Planned in rormpc-tools fe4515c docs/media-keys-plan.md (worker, consult round 20261010-193113-cc90): a datagram
      socket with a JSON allowlist (next, prev, toggle, play, pause, stop, seek) into mpd-player's command queue,
      each command reading MPD's state first; Karabiner send_user_command (not Karabiner's default socket path);
      Now Playing as a second process `mpd-player nowplaying` with its own launchd agent; MPRIS in the daemon via
      dbus-fast; companions stops mpd-now-playable before starting the new one; karabiner.json only suggested. Six
      open choices in "Needs a decision" ("Media keys:"); the build waits for them.
      Decided by the user 2026-10-10 (consult round 20261010-225609-d67e, Sol e510c27c, MiMo c6d56c6f): two
      stages, each committed, released and installed alone: (1) the command socket in mpd-player and the Karabiner
      switch to send_user_command, the old Karabiner shell script deleted with the switch; (2) the Now Playing and
      MPRIS provider with its units, retiring mpd-now-playable. The user tests the Control Center tile after the
      install (the worker never stops mpd-now-playable); the worker writes a short test and rollback procedure.
      Stage 1 done 2026-10-10: rormpc-tools d1df2c7 (mpd-player command socket: JSON datagrams, flock, 0600 socket,
      `mpd-player send`, `socket-path --check`, 70 tests; scratch MPD: 1-2 ms from receipt to MPD), rormpc a24fa20
      (companions print the socket and the Karabiner rule, `status` shows it bound); released v0.2.47, installed
      (socket bound); dotfiles 5d47ea3 (Karabiner F7/F8/F9 send_user_command, ~/scripts/mpd-media-key deleted;
      Karabiner reloaded it). The daemon makes ~/.local/state/rormpc 0700 where an older mpd-player made it 0755
      (the plan refused such a directory; accepted by the coordinator: only the same user reads it). Live key test
      is the user's: F8 toggle, F9 next (also from a pause), F7 prev ("confirmed" in prev.jsonl), a held F9 one
      next per press, F8 still MPD while a browser plays; the log is ~/Library/Logs/mpd-player.log, a failed send
      shows as "send_user_command: send_to failed" in Karabiner's console_user_server.log; rollback: revert
      dotfiles 5d47ea3. Left over: ~/.local/state/media-keys/ (the old script's log), for the user to delete.
      Stage 2 decides what `pause` does while "Pause for…" holds the pause (today: nothing, the timer runs on).
      Stage 2 done 2026-10-11: rormpc-tools fb16601 (`mpd-player nowplaying`: public MPNowPlayingInfoCenter and
      MPRemoteCommandCenter, every command through the socket, claims Now Playing on the first MPD play after it
      starts; MPRIS in the daemon via dbus-fast, on by default, `--no-mpris`; a Seeked signal when the same song is
      over 1 s off its predicted position, a tolerance chosen by the worker), rormpc 62dd86a (companions run
      `nowplaying --check`, retire mpd-now-playable with its own uninstall-launchagent, start
      mpd-player-nowplaying; Linux warns about mpd-mpris/mpDris2/rmpcd); released v0.2.48 and installed: the
      mpd-now-playable LaunchAgent is uninstalled (its program stays), Now Playing running. Not tested: the Control
      Center tile (the user's test), MPRIS on a real Linux desktop (only a private bus on macOS). Rollback:
      `launchctl bootout gui/$UID/io.github.rofrol.rormpc.mpd-player-nowplaying`, delete its plist,
      `mpd-now-playable install-launchagent`.
- [x] Installer: right after starting mpd-player-nowplaying, companions warned "mpd-player's Now Playing is [t-34a4whpn]
      xpcproxy" (2026-10-11) although it was running a moment later: the check reads launchd's state while the job
      is still spawning. Wait for the observable running state (a pid and `state = running`) or report a real
      failure (the job exited), never a sleep (found while installing stage 2; installer only, one revert).
      Done in rormpc abed203 (worker): `launchd_started` waits for `state = running` with a pid, or a real
      failure (exited since the bootstrap, not loaded), at most 100 polls of launchctl 0.1 s apart (launchd has no
      event or blocking form for a spawn; the delay accepted by the coordinator). Linux has no such check.
- [x] Release years: write the match's swap evidence (row["swap"]) into yt_mp3_mb's log.jsonl and its --batch [t-rkvdgpp6]
      proposal, so a swapped download shows from → to and why (found by the stage C worker 2026-10-11; today only
      the ">work" method suffix shows; logging only, one revert).
      Done in rormpc-tools 7b981d5 (worker): log.jsonl entries and --batch proposals carry "swap"; the
      "files" entries do not. Not released.
- [x] Instant filtering stage 6b (follows from stage 6): keep the play counts out of the library index's key (a [t-jbqu432z]
      play must not make the next run cold) and cache the chart matching (chart_candidates: name normalisation and
      matching of ~2000 chart rows), then measure again; `hits serve` only if the warm median is still over 0.15 s
      (the user's rule, 2026-10-11).
- [x] Smart list picker: "Load" and "a Apply" now do the same (filters apply at once); keep one button. Found by the [t-qjdzk34e]
      instant filtering stage 5 worker (2026-10-11). Cost: a small rormpc change, no build beyond the usual install.
      Decided by the user 2026-10-11: approved (d-bzzoexao).
      Done in rormpc (worker, cherry-picked; installed 2026-10-11, 1091 tests): one "Load" button (Enter; `a` an
      alias without a button), Pick::Apply removed, the hints and the save dialog's export note updated. Found: the
      user's config has no `L` (SmartLists) binding in its queue keys, so the picker may be unreachable there.
- [x] The status bar cuts long messages (seen by the Apply worker 2026-10-10: "… in their order" cut off): show [t-hewfzhd7]
      the whole text or wrap it, and keep it long enough to read.
      Decided by the user 2026-10-11: approved (d-bzzoexao).
      Done in rormpc (worker, cherry-picked; installed 2026-10-11, 1100 tests): a message wider than the bar wraps
      over the full width (the bar grows upward, up to 4 rows, over what is above it while shown) and stays
      clamp(2 s + columns / 15, 5 s, 20 s) (rmpc/src/ui/rormpc_status.rs `reading_time`; its `delay:` labels are
      a person's reading time, not a wait for an event: agreed by the coordinator). `rormpc remote status -t` keeps
      its own time. Also: the user's config got `L` (SmartLists) in its queue keys (decided d-bjtovhts; dotfiles
      02d3ee1). Note: from 2026-10-11 15:10 a herdr hook limits this coordinator tab to an allowlist; later items go
      through `herdr todo run`.
- [x] rormpc's own Next and Previous through mpd-player's command socket instead of the MPD channel (`shuffle [t-o4a6x3xr]
      prev`, MPD `next`) when it is bound (raised by the stage 1 worker 2026-10-10): one control path; behaviour
      is the same today. Cost: a small rormpc change, no build beyond the usual install.
      Decided by the user 2026-10-11: approved (d-bzzoexao).
      Done 2026-10-11 (4c0918c, installed and pushed by the run) through herdr todo run r-b67l5zil (worker; reviewed by the coordinator): rormpc's Next and Previous
      (keys and `rmpc next` / `rmpc prev`) send one non-blocking datagram to mpd-player's command socket (the same path rule
      as `mpd-player socket-path`); a failed send falls back at once (Previous: `shuffle prev`, then MPD `previous`; Next:
      MPD `next`) and the status bar says so once per outage; the rewind rule comes first as before. Not tried live: the
      worker's sandbox cannot bind sockets; one watcher test timed out in the sandbox (rormpc_upnext atomic replacements),
      left to the verify outside it.
- [x] Media keys: `pause` (key, Now Playing, MPRIS) while "Pause for…" holds the pause cancels the timer and stays [t-il2f5mdd]
      paused (decided by the user 2026-10-11, d-h2in7kd6). rormpc-tools player/control.py `_pause`.
      Moved to rormpc-tools' TODO (t-jeibd5n3) on 2026-10-12 (d-33kat4ps): it is rormpc-tools code only.
- [x] Media key Next in Music, weighted (asked 2026-10-11: "I am in Music, weighted, I press the media Next, it goes [t-a3axjgzk]
      to the next song, but the current one is not seen as the previous one, it is removed from the view. Ask the
      models."): find why the skipped song disappears from the plan view instead of showing as the last play.
      Checked 2026-10-11 (coordinator): afterwards the data is right: the skipped song (id 1893) is still queued with
      the same file, shuffle.json history ends with it as `early`, and the projection
      (rormpc_queue_plan.rs `project`) must then show it as -1; shuffle.json is watched by directory
      (rormpc_upnext.rs watch_path → invalidate + RequestRender). The user's rormpc ran since 12:17, before the
      three rormpc installs of the day (12:47, 13:17, 13:41); mpd-player 0.2.52 since 14:42. Round
      20261011-145943-d46f (Sol 772e3520 useful, MiMo df6b7c2c partial): both put the old running binary first, then
      a render between MPD's song change and mpd-player's history write; dismissed MiMo's "no redraw on shuffle.json"
      (the directory watcher exists). Next step for a worker: reproduce on a scratch MPD with its own mpd-player and
      the current build (Music, weighted, `mpd-player send next` to the scratch socket), read the view right after
      and once shuffle.json changes, log the projection's inputs for the skipped id (plan, history, requests,
      queue file); fix what it shows (Sol: decide whether Past should outrank Forecast; MiMo: one shuffle.json write
      per transition). The user restarts rormpc to rule out the old binary.
      Confirmed by the user 2026-10-12 with the current build: the skipped song disappears, then appears again as -1,
      so the view renders between MPD's song change and mpd-player's history write (the round's second cause).
      The fix goes in rormpc (a todo run): keep the song that was playing as -1 until shuffle.json has it.
      Fixed 2026-10-12 (9b2cb22, installed and pushed by the run) through herdr todo run r-kbykv266 (worker; reviewed by the coordinator): the plan view keeps the
      song MPD moved on from as -1 until shuffle.json's history has it or a newer entry, it plays again (Previous) or it left
      the queue; no timer; shuffle.json history entries' `t` now read to tell a repeat from an old entry. Not tried live
      (the sandbox). Not covered: the ShuffleNext column of the plain queue may still lose -1 for a moment.
- [x] Lyrics scroll wrong for "MONTERO (Call Me by Your Name)" by Lil Nas X (asked 2026-10-11: "the lyrics move [t-7cqbsitp]
      badly for MONTERO (Call Me by Your Name), Lil Nas X"): find whether the .lrc timing or rormpc's scrolling is off.
      Checked 2026-10-11 (coordinator): the file is the official video rip (189.8 s); LRCLIB entry 37216484 (chosen
      automatically, within 2 s) says 190 s but its lines run 0:06.77-2:13.72, the ~2:17 audio's timing, so every
      line comes early (the video's intro). Other candidates: 170, 147, 144, 143-138 s. rmpc honours `[offset:]`.
      Round 20261011-150637-ce29 (Sol 7981b1c7, MiMo ea11bfef, both useful), in agreement: an entry's declared
      duration is not proof; flag an entry whose last line is far before the end (Sol: >30 s and >20% of the file;
      MiMo: line span < 0.72 of the length or >45 s left, or 15% off the other candidates' median span) as "verify
      against the audio", never reject it; align with Whisper word timestamps of distinctive lines on the demucs
      vocal stem (the transcription pipeline), a robust offset fit (MiMo: also a slope for sped-up rips), piecewise
      only with >=3 anchors on each side of a >2 s jump; refuse and change nothing when the anchors are few or
      scattered (MiMo: <8 anchors, residual MAD >400 ms, |offset| >25% of the length). Store reversibly with
      provenance (entry id, file hash, method, residuals) and an undo; nudge keys (±0.1 / ±0.5 s, reset); check the
      sign of rmpc's `[offset:]` on a test file first. They differ on storage: Sol `[offset:]` for one offset and a
      derived .lrc for piecewise; MiMo always a generated .lrc beside a sidecar, the source untouched. First test:
      this MONTERO file (the first line on the first vocal onset ±0.3 s), then a mid-song skit, a long instrumental
      outro (must refuse).
      Decided by the user 2026-10-11 (d-g7ttw73b): now, the suspicious-entry flag and nudge keys (±0.1 / ±0.5 s,
      reset) whose offset is saved per song with an undo; the automatic alignment comes with the transcription build
      (the same Whisper and demucs), MONTERO as its first test.
      Nudge keys done 2026-10-12 (a07eabe, installed and pushed by the run) through herdr todo run r-mmkbts5b (worker; reviewed by the coordinator): in the Lyrics pane
      `}` / `{` shift the lines 0.1 s later / earlier, `)` / `(` 0.5 s, `=` resets; the shift is written into the song's
      .lrc as its `[offset:]` header tag (atomically, other lines byte for byte; rmpc's convention: a positive tag means
      earlier, so "+12.4 s later" is `[offset:-12400]`), the global lyrics_offset adds on top; the pane shows
      "lyrics +X s · =: reset". Refetching the lyrics overwrites the file and drops the shift. Not tried live (the sandbox
      cannot bind MPD's port). Still open in this item: the suspicious-entry flag (rormpc-tools) and the automatic alignment
      (with the transcription work).
      The rest (the suspicious-entry flag, the automatic alignment) moved to rormpc-tools' TODO (t-fb4wapwk) on
      2026-10-12 (d-33kat4ps); rormpc's nudge keys are done (a07eabe), so this item is done here.

- [x] Replace "Sweet Nothing" (Calvin Harris ft. Florence Welch), a 4:29 official-video rip with extra video parts [t-is2eqcck]
      (bit.do--spotify2015/364--Calvin_Harris--Calvin_Harris_-_Sweet_Nothing_(Official_Video)_ft._Florence_Welch--17ozSeGw-fY--20120920.mp3),
      with the clean 3:29 track from the official "Calvin Harris - Topic" channel (https://youtu.be/TawoXhEN2tE),
      reported by the music-mpd session (passed on by the user 2026-10-11). Decided by the user 2026-10-11: replace
      (d-hj7az67n); not D.O.D & Calvin Harris - Sweet Nothing (2025 Edit) (d-dse2zosm). How: `yt-mp3-mb --yes URL`,
      then `musicdb versions same NEW OLD` (the video goes to the quarantine with an alias, its plays and likes
      follow the new file); the coordinator runs it (a library change the user decided).
      Done by the coordinator 2026-10-11: downloaded as Singles/Calvin_Harris_feat._Florence_Welch--Sweet_Nothing--
      TawoXhEN2tE--20140324.mp3 (3:28, MusicBrainz recording 4cf00d80-35b4-49d6-a050-68ca435e694b, "auto"); `musicdb
      versions same` moved the like, wrote the alias and put the video rip in the quarantine (neither file had
      lyrics); its last step, `musicdb.sync`, failed with HTTP 404 (the hourly `musicdb update` runs it again).
      music-data's aliases.jsonl and likes.jsonl are left for the hourly export to commit.

- [x] AGENTS.md "Coordination": rormpc-tools has its own coordinator and TODO.md from 2026-10-12 (decided by the user, [t-ltsquwes]
      d-33kat4ps), replacing the 2026-10-07 rule that todo-rormpc owns both. Cross-repository work goes as a plain
      message to the rormpc-tools coordinator, which replies "landed <tag>"; the consuming item waits here as "Waiting
      on rormpc-tools"; rormpc's install script pins rormpc-tools' tag. Follows from that decision; text only.
      Done 2026-10-12 (642c138, through herdr todo run r-ykgns7lf; pushed by the run).

- [x] Repair the release years (decided 2026-10-10, rormpc-tools docs/release-years-plan.md and the "Release years:" [t-j74galug]
      decisions): views show TDOR (the song's original release), TDRC stays this recording's; the hybrid rule of
      the plan; a dry-run report; every row reviewed in a rormpc "Years to review" view (no automatic apply); atomic
      tag writes keeping the old values for undo; files without a MusicBrainz work link get an MBID picker; on
      download a video match without an audio alternative is swapped to the same-length audio recording through
      the work. MusicBrainz at 1 req/s (~66 min for the library, cached), run as a herdr-job.
      Stage A done 2026-10-11: rormpc-tools 00b5542 (worker): `musicdb years` (--dry-run report with stable ids,
      every row undecided; --accept/--reject/--undecide; --mbid ROW MBID; --apply only accepted rows whose audio md5
      and date tags are unchanged, years-backup.jsonl fsynced first, temp copy + os.replace, `mpc update --wait`;
      --rollback only when the file still has what that apply wrote; --json for rormpc); hits reads originaldate,
      falling back to date; downloads write TDRC = the recording's first release, TDOR = the rule. Values the worker
      chose (accepted by the coordinator): a proposal keeps the file's current TDRC (album rips keep their dates);
      a download never takes a TDOR later than its recording's first release (rule `first-release`); a row whose
      year stays but whose release group is earlier goes to review as `keeps-year`; at most 200 candidate
      recordings per work (`candidates-truncated`). 15-file sample: 3 right proposals, 2 wrong from MusicBrainz's
      own dates (one rated high, so the row-by-row review is needed), 6 "needs MBID". Not released. The full dry
      run: herdr-job 20261011-001830-7e3a. Stage B (rormpc view, MBID picker, views read originaldate) next;
      stage C (video to audio through the work on download) after it.
      Stage B1 done 2026-10-11: rormpc 4ed28ea (worker): "Years to review" panel over Music (`gY`, ShowPlay(Years);
      gy is Lyrics): counts, rows with evidence, a/D accept/reject (again: undecided), Space marks, h/l and menu
      filters, "Apply accepted…" and "Roll back…" with result dialogs; "needs MBID" rows read-only (the picker,
      stage B2, waits for its two decisions); the Year column and the album browser read OriginalDate, else Date
      (album_date_tags default [OriginalDate, Date]). Released with rormpc-tools v0.2.49 (stage A) and installed;
      dotfiles 22957db (gY, album_date_tags, the theme's Year column). Skipped-row messages go only to the status
      bar. Not seen on the full report yet (the dry run is still running).
      Stage C done 2026-10-11: rormpc-tools 43af761 (worker): resolve() swaps a video or DJ-mix match with no audio
      alternative to the work's same-length audio recording (years.audio_for, the same cache, 1 req/s), evidence in
      row["swap"] (from, to, rule, why), method ends ">work"; a failed lookup keeps the match. Not released. The
      MBID picker (stage B2) waits for its two decisions.
      Full dry run 2026-10-11 (herdr-job 20261011-001830-7e3a, nothing written): 517 rows in
      music-data/years-report.json: 59 high and 67 review proposals, 391 without one; 352 need an MBID (165
      no-recording, 187 no-work); classes: video 51, dj-mix 27, version 21, rg-earlier 21, later 14, undated 21.
      Sweet Dreams (row 348) 2000 → 1983 high, Livin' on a Prayer (row 362) 1988 → 1986 high. Ready for the
      user's review in `gY`.
      Waiting on rormpc-tools (2026-10-12, d-33kat4ps): its stage B2 backend is rormpc-tools t-bd3334hn (UFID rewrite,
      no-recording marking); rormpc's MBID picker in the Years panel follows its "landed <tag>".
      Landed 2026-10-12 (todo-rormpc-tools): rormpc-tools v0.2.54 (89f5c36, bbdf04c): `musicdb years --apply` writes a
      --mbid pick into the file's recording tag (UFID / MUSICBRAINZ_TRACKID) even when the year holds, rollback restores
      it, mb: keys follow; rows without a recording MBID carry class no-recording; the json only gained fields (file_mbid
      on rows, recording in the apply output); the contract test passes unchanged. Back in "Next, in order".
      Stage B2 rormpc side done 2026-10-12 (e67d2c3, installed and pushed by the run) through herdr todo run r-a7sefsnf (worker; reviewed by the coordinator):
      in the Years panel `a` on a needs-MBID row (or the menu's "Pick the recording (MBID)…") takes a MusicBrainz
      recording URL or MBID, validated, runs `musicdb years --mbid` in the background; picked rows can be accepted;
      the details, the Apply confirmation and the result show the recording-tag rewrite; RORMPC_TOOLS_TAG v0.2.54.
      rormpc-tools has no candidate list, so the pick is pasted. Not tried live (the sandbox). The companions
      install of 0.2.54 follows (the run installs rormpc only).

- [x] Years to review: the files without a recording MBID (about 166, class no-recording) listed under a separate [t-vssvn76c]
      filter, off by default, so the picker can give them one (decided by the user 2026-10-11, d-bzidm3vs).
      Waiting on rormpc-tools (2026-10-12, d-33kat4ps): the report's no-recording marking is part of rormpc-tools
      t-bd3334hn; rormpc's separate, off-by-default filter in the Years panel follows its "landed <tag>".
      Landed 2026-10-12 (todo-rormpc-tools): rormpc-tools v0.2.54 (89f5c36, bbdf04c): `musicdb years --apply` writes a
      --mbid pick into the file's recording tag (UFID / MUSICBRAINZ_TRACKID) even when the year holds, rollback restores
      it, mb: keys follow; rows without a recording MBID carry class no-recording; the json only gained fields (file_mbid
      on rows, recording in the apply output); the contract test passes unchanged. Back in "Next, in order".
      Done 2026-10-12 (fb79ea9, installed and pushed by the run) through herdr todo run r-er5n7oex (worker; reviewed by the coordinator): rows of class
      no-recording are hidden by default; the menu's "No recording MBID (N)" shows only them (or back), clearing
      the confidence and class filters; the summary counts "N without a recording MBID"; a picked recording moves
      the row back to the others.

- [x] `scripts/rormpc_install.sh install` keeps the companions at RORMPC_TOOLS_TAG by itself: compare the installed [t-oziokwmz]
      companions' tag with the pin, reinstall them and restart their LaunchAgents only on a mismatch, record the
      installed tag only after success, print both versions, fail loudly. Then every rormpc install (also herdr todo
      run's [install]) keeps mpd-player, musicdb and hits at the pin without a second command or an override. From
      herdr's round 20261012-020315-c8e8 (sol, MiMo; herdr DECISIONS 8a2e0ef1), which rejected multi-step installs in
      herdr, after the user's request to report the gap (d-b2ypouzq). Cost: a small script change and its smoke test.
      Decided by the user 2026-10-12: approved (d-nyb6ewr5).
      Done 2026-10-12 (22cfe45, installed and pushed by the run; the record now says rormpc-tools v0.2.54) through herdr todo run r-oswstluv (worker; reviewed by the coordinator): `install` compares
      the companions' recorded refs ($XDG_STATE_HOME/rormpc/companions, written atomically only after a successful
      companions step) with RORMPC_TOOLS_TAG and RO_LB_TAG and runs the companions step only on a mismatch or no
      record, printing both; a failed step exits non-zero; `status` shows record and pins; a stubbed test
      (scripts/test_rormpc_install.sh) runs in CI on Ubuntu and macOS. The first install after this has no record,
      so it reinstalls the companions once.

- [x] Should rormpc-tools live in the rormpc repository (asked 2026-10-12: "maybe rormpc-tools should be in the rormpc [t-wukv7ish]
      repo? ask the models?"): consult the models and record the outcome; a move needs the user's decision.
      Rounds 20261012-021857-908d (sol 4090d8c9, MiMo 1fa8a9a7, DeepSeek 4244b40f, all useful): move rormpc-tools into
      rormpc as tools/ with its history, install the tools from the same ref as rormpc (`./tools` or
      `git+…/rormpc@REF#subdirectory=tools`), Python CI in its own workflow on tools/**, archive (never delete) the old
      repository so old tags keep installing; ro-listenbrainz-mpd stays separate. Round 20261012-022654-56fc (sol
      50e2995f, MiMo 0de7d6cf, DeepSeek f9ed61b7, all useful): all three advise a linear append, not an interleave.
      Decided by the user 2026-10-12: move it (d-kgxuqbvh), interleaving both histories by author date with every
      rewritten commit's committer date = its author date (d-rjg7tv5p, against the three models).
      Plan (from the round): freeze both coordinators and pushes; in disposable clones, filter rormpc-tools to tools/,
      replay rormpc's 434 fork commits and the 191 tools commits on top of upstream's f677f67 sorted by author date,
      stopping if a repository's own order inverts by date, GIT_COMMITTER_DATE = author date, writing an old→new map;
      upstream's commits untouched; verify (upstream objects, 625 commits, final tree = old rormpc tree + tools/,
      cargo and pytest, a dry `git merge upstream/master`); retag tools-v0.2.x on the mapped commits; rewrite TODO.md
      and docs' SHAs from the map (commit messages stay); bundle backups; push with --force-with-lease; the old
      repository archived; installer and AGENTS.md moved to one repository and one coordinator.
      Done 2026-10-12 (coordinator, with overrides the user allowed, d-ddlh6iyt): both histories interleaved by author
      date on upstream's f677f67 (630 commits: 437 rormpc + 193 rormpc-tools under tools/; each repository's own
      order kept, 11 + 1 date inversions inside them), committer date = author date everywhere; checked: per-commit
      trees of both halves equal the originals, patch ids, authors and messages equal, no empty commits, fsck, final
      tree = old rormpc + tools/, cargo 1135 and pytest 658 pass, a dry merge of upstream/master gives the same result
      as before; review round 20261012-023558-f66a (sol, MiMo, DeepSeek). CI on rewrite-test: Build & Test pass;
      Formatter fails as before (pull requests only); the installer's companions-linux fails as on old master (item
      below). The user force-pushed master to 3358f5d; tags tools-v0.2.x (62) pushed; rormpc-tools' local main
      pushed and the repository archived. SHAs cited in this file were rewritten to the new history from the old→new map
      (2026-10-12, then the map was removed from the repository). Backups: git bundles in the coordinator's scratchpad (temporary).

- [x] Ctrl-y (undo the last deletion) also undoes a permanent deletion and one with the history, through [t-sea2trmx]
      `musicdb restore ID --yes` (download again, history, stickers, YouTube playlist), and the status bar after any
      deletion says "Ctrl-y undoes" (asked 2026-10-12: "and where is the undo of this deletion from disk and
      ListenBrainz?" after Mary J. Blige - Be Without You was deleted permanently with its history; the coordinator
      restored it with `musicdb restore`; today Ctrl-y only takes a song back from the Trash, the Deleted panel `gd`
      restores any deletion). Cost: a small rormpc change.
      Decided by the user 2026-10-12: approved (d-usvfh2ct).
      Done 2026-10-12 (e81d782, installed and pushed by the run) through herdr todo run r-m5abzkzg (2 attempts; reviewed by the coordinator): Ctrl-y undoes
      the newest deletion of any kind: the Trash through `musicdb undo --id`, a permanent one or one with the
      history through `musicdb restore --yes` in the background with its progress in the status bar (it downloads
      again); `musicdb undo --next` tells rormpc what comes next; every deletion's status line says what Ctrl-y
      does. A failed restore is retried from gd (the next Ctrl-y takes the older deletion). Also removed
      tools/tests/test_release_script.py, left behind when scripts/release.sh went.

- [ ] Transcribed lyrics (from tools/TODO.md after the move into rormpc, 2026-10-12). Plan: docs/transcribe-lyrics-plan.md, b2b2d5e. [t-jsjs5ngd]
      All seven choices were decided by the user 2026-10-11:
      - transcribe each new Omarchy Radio download automatically;
      - transcribe the ~274 library songs without lyrics once, overnight;
      - always separate the vocals (demucs);
      - show a transcript that passed the checks at once, labelled machine-transcribed;
      - model: Whisper large-v3, not the measured turbo;
      - a separate uv tool with an inline-dependency script;
      - try one commercial API on the whole evaluation set as an upper bound. This needs the user's API key: a
        worker without one reports it and never signs up.
      Stored as <song>.transcript.json, never as .lrc. Tell todo-rormpc "landed <tag>": rormpc's display of
      transcripts waits on it.
      Slices (coordinator 2026-10-12, by the plan's build order): 1. `transcribe.py`, the inline-dependency backend
      script, the sidecar and offline tests with a fake backend (done 2026-10-12: cd02be3, 1c5b7fb, model id fix
      ca369c6; smoke run on a library copy in temporary dirs: the backend installs, demucs + large-v3 ran, language
      voted en 5/5, the lines read well; a 14.5 min looped YouTube upload took 12 min and was withheld for
      words_outside_speech and high_word_density, with a repeated chorus loop and "Thank you." at the end); 2. the evaluation set
      and threshold calibration (needs the model downloads and library copies, outside the worker sandbox); 3. the
      radio auto-run and the overnight library batch; 4. the commercial API upper bound (the user's key); 5. Linux
      faster-whisper. rormpc's display and Enter-to-transcribe are rormpc's items.
      Evaluation 2026-10-12 (herdr-job 20261012-022855-363f, read-only; results outside the repository in
      ~/.local/share/rormpc-tools/transcribe-eval-2026-10-12/, rejudge with `musicdb lyrics transcribe-eval
      eval-list.tsv --from eval-out`): 10 songs with synced lyrics (pl, fr, 8 en) and 5 instrumentals, 41 min CPU with
      large-v3 + demucs. Language right 10/10; instrumentals 5/5 no_vocals (VAD 0.0-0.7 s of singing; lyrics songs 50-177
      s, ratio 0.24-0.85); 8/10 lyrics songs shown, mean WER 0.29 (0.09-0.59), median line sync 1.27 s. The 2 withheld
      fail only words_outside_speech (0.34, 0.47 > OUTSIDE_SPEECH_MAX 0.25: VAD misses part of the singing). Separating
      ranges: OUTSIDE_SPEECH_MAX >= 0.473 (or SPEECH_PAD >= 12.2 s); the other checks already separate. The rormpc-tools
      coordinator's reading for the next slice: raise OUTSIDE_SPEECH_MAX (e.g. 0.6), keep the rest, confirm on a larger
      set or the radio tracks before the overnight batch (10/5 is small).
      Asked by the user 2026-10-12 ("WER 0.29 and 1.3 s line sync: fix it across the whole library? ask the models";
      and on raising OUTSIDE_SPEECH_MAX to ~0.6: "I don't know, let him ask the models"). Round 20261012-032438-8fd4
      (Sol, MiMo, DeepSeek, run by the rormpc-tools session before it closed), checked against the code:
      all three: words_outside_speech must not withhold (speech VAD misses singing: 2/10 false rejections would be
      ~55 of 274); demote it to a review flag instead of raising the limit (DeepSeek: measure against vocal-stem
      activity); none proposes 0.6. Replace the sync metric in transcribe-eval (nearest line start rewards
      over-segmentation): monotonic word alignment to the reference, signed error p50/p90 per song, drift over the
      song, "is the right line highlighted at sampled times". Pilot forced alignment of the transcript's own words on
      the vocal stem (wav2vec2/MMS-CTC, whisperX-style) against stable-ts refinement, keeping Whisper's times where the
      alignment cost is high (pitfalls: sustained vowels, melisma, backing vocals; pl/fr support; alignment fixes
      times, never words). Consider: beam search and temperature fallback; the original mix against the stem on the
      worst songs. Expect worse on the 274 LRCLIB lacks. Dismissed: condition_on_previous_text on hard songs.
      Next slice: demote the check, the better metric, a forced-alignment pilot on ~20 songs, then the batch.
      Decided by the user 2026-10-12 (d-orwfxuit), replacing "show at once" of 2026-10-11: transcripts are generated
      but not shown or highlighted until the forced-alignment pilot succeeds.
      Slice landed 2026-10-18 (run r-alp4b557, worker pi on openai-codex/gpt-6.1-sol, commit 03ac0b3; the
      coordinator ran `uv run --directory tools pytest -q` in the run's worktree: 677 passed): `words_outside_speech`
      no longer withholds. `transcribe.review_flags()` records it in the sidecar's new `review` list, sidecar
      `version` is 2 and a version 1 record keeps its state until it is transcribed again (nothing rewrites existing
      sidecars), `describe` adds "; review: words_outside_speech", and `transcribe_eval` reports it as a review flag
      (`review_only` in the separating ranges: a review-only candidate bounds the lyrics flags but cannot catch an
      instrumental). The other four reasons withhold exactly as before. Next slice: the sync metric — monotonic word
      alignment to the reference, signed error p50/p90 per song and drift over the song, replacing the
      nearest-line-start distance — as its own run; then the forced-alignment pilot on ~20 songs, which needs the
      model downloads and so runs outside the worker sandbox.
      Slice 2 dispatched 2026-10-18 (run r-z4prcxfk): the sync metric by monotonic word alignment (signed p50, p90
      of the absolute error, drift per minute, the share of sampled times with the right line), check `tests tools`.
      Blocked 2026-10-18 04:17: worker w458 (pi, gpt-6.1-sol) was reattached after herdr's 0.9.3 install restarted
      the server and then its broker died; the worker record stays state=working with exited=0, `herdr worker
      interrupt|stop|kill` do not change it, the run sits at step attention with `runs.pending_event` null, `todo
      resume` has no event id to answer and `cancel-pending` refuses, so the run cannot be aborted by the documented
      path (herdr report r-105; asked the herdr coordinator for the recovery action and the fix). Two files are
      modified but uncommitted in the shared worker worktree (base 2e871e3); a fresh run for this slice needs that
      slot reverted first. Next: redispatch slice 2 once the stuck run is resolved or once the user says to force it.
      Resolved 2026-10-18 05:35 by herdr's rebuild (roherdr 6f35ab749): the orphaned worker w458 was reaped (state
      exited), the run's review event arrived and the coordinator aborted it; slice 2 was redispatched as run
      r-52mionae (agent pi). This session's lease went stale with the server handoff and was taken over again (pid
      97133).
      Slice 2 landed 2026-10-18 (run r-52mionae, worker pi/gpt-6.1-sol, commit 159af42; the coordinator ran
      `uv run --directory tools pytest -q` in the run's worktree: 685 passed): `transcribe_eval.sync()` now aligns the
      reference and transcript words monotonically (unit gap/substitution costs, a reference line matched when at
      least half its words match, the transcript line owning most matches, strictly increasing pairs) and reports
      `matched`, the signed `p50`, the absolute `p90`, `drift` in seconds per minute and `hit`, the share of 2 s
      samples where the line a player would show is the matched one; `within` counts matched pairs only. The summary
      keeps `median_sync` as a median magnitude and adds `median_drift` and `median_hit`; the table's columns are
      p50/p90/drift/hit. Tests cover the flaw that motivated this (over-segmentation no longer improves p50 or hit),
      a constant offset, clock drift (1.1x times give 6 s/min), absent words, and the table. Known limitation,
      documented in a test: when a later fragment of a line is what a player would display between two reference
      starts, `hit` counts it a miss, so fragmentation can lower `hit` even when the timing is right.
      Next: the forced-alignment pilot on ~20 songs (the plan's step: wav2vec2/MMS-CTC on the vocal stem against
      stable-ts, keeping Whisper's times where the alignment cost is high), which needs the model downloads and so
      runs outside a worker's sandbox — a herdr-job on this machine, read-only, results under
      ~/.local/share/rormpc-tools/.

- [ ] Lyrics timed for another cut (moved from rormpc's TODO t-7cqbsitp; MONTERO by Lil Nas X). [t-fb4wapwk]
      - Flag a suspicious LRCLIB entry, one whose last line is far before the file's end. The round of 2026-10-11
        proposed thresholds: Sol, more than 30 s and more than 20% of the file; MiMo, a line span below 0.72 of the
        length, more than 45 s left, or 15% off the other candidates' median span. Flag it for checking, never
        reject it.
        Decided by the coordinator 2026-10-12 after consult round 20261012-015328-4f97 (Sol, MiMo, DeepSeek; all
        three dropped the standalone span test: long intros make it false-flag and a uniform shift leaves the span
        unchanged). Flag reasons, each shown with its numbers:
        * `past-end`: a non-empty line's timestamp past the file's length + 2 s (all three);
        * `unordered`: timestamps that decrease in file order, after expanding multi-timestamp lines and applying
          `[offset:]` (all three);
        * `ends-early`: tail = length - the last non-empty line's start > clamp(0.20 x length, 25 s, 90 s) (Sol's
          20%, MiMo's and DeepSeek's absolute cap for long songs, Sol's lower floor for short ones);
        * supporting only, never a flag alone: with two or more other synced candidates that agree within 5 s,
          this entry's last line more than max(20 s, 10%) from them (Sol, MiMo).
        Thresholds are named, provisional constants; the label reads "lyrics end early, check timing". Calibration:
        the coordinator counts the flags over the library read-only after the build and expects at most about 5%
        of synced entries; more goes to "Needs a decision" with the numbers.
        Built 2026-10-12: e14608f (`musicdb lyrics check [FILE ...] [--dry-run]`, `suspect` in index.json and
        `status --json`). Counted over the library read-only: 83 of 444 synced flagged (53 ends-early, 30
        past-end): above 5%, see Needs a decision. The alignment part waits on the transcription.
      - Later, with the transcription: automatic alignment, using Whisper word timestamps on the vocal stem and a
        robust offset fit, refusing when the evidence is weak.
      Decided by the user 2026-10-11 (d-g7ttw73b): the flag now, the alignment with the transcription. rormpc
      already has the manual nudge keys (a07eabe), which write the song's `[offset:]` tag.
      Decided by the user 2026-10-12 (d-6l24gkjq): no fixed tail percentage; examine the audio file. Round
      20261012-030458 (sol 5dee12e6, MiMo facc92e2, DeepSeek 71eca818, all useful): the tail decides: vocal activity
      after the last lyric line (allowing the last line itself to finish; delay: not a wait, a window of song time)
      means the lyrics end early, a silent vocal tail is an instrumental outro; the start explains it (first vocal
      onset minus the first line's time = an offset to propose as a re-anchor through the nudge keys' [offset:] tag).
      Cheap pass: ffmpeg to mono 16 kHz, Silero VAD on the tail of the mix; escalate a speech-like or unclear tail to
      demucs on the tail window only (not the whole song) and VAD on the vocal stem; Whisper only to tell repeated or
      spoken words apart. Cache per decoded-audio hash (ffmpeg -f md5) and method version; the flag shows the evidence
      ("vocals at 3:42-4:09 after the last line at 3:20", "likely +34 s intro offset", "instrumental outro",
      "uncertain"). Validate on the 53 flagged files plus ~20 unflagged with a labelled tail, report precision and
      recall.
      Decided by the user 2026-10-12 (d-qf5urpbq): the reference labels come from Whisper large-v3 on the tails; the
      user listens only to the disputed ones.

- [x] CI: the installer smoke test's companions-linux job fails on master: "a daemon never subscribed to its MPD [t-kwqstgkt]
      channel" (MPD channels listenbrainz_feedback, listenbrainz_listen, rormpc; runs 38187215102, 38188905972);
      found while checking the rewritten history, failing before it.
      Root cause read 2026-10-18 from the job log: the step "Units are enabled and active, and their programs run"
      compares the channel list for exact equality with `want="listenbrainz_feedback rormpc"`, while
      ro-listenbrainz-mpd also subscribes to listenbrainz_listen, so the log shows
      "MPD channels: listenbrainz_feedback listenbrainz_listen rormpc" and fails on the extra channel. The fix is a
      containment test of the two channels that prove the daemons are connected. Worker task written
      (/tmp/task-t-kwqstgkt.md); dispatched 2026-10-18 as run r-cmzeb4fd once herdr cleared the stuck run.
      Done 2026-10-18 (run r-cmzeb4fd, commit f4d6523): the step now uses `has_channel()` for `rormpc` and
      `listenbrainz_feedback`, extra channels are allowed, a missing one fails with its name. Confirmed on master:
      the Installer smoke test of efedbc8 (run 39096017797) has **companions-linux: success** (it failed in every
      earlier run: 38256551254, 38190365753, 38189525687). Its other job, install-stubbed (macos-latest), still
      fails, but it failed in the previous run too, so it is a separate problem: "Proposed".
      Fix landed 2026-10-18 (run r-cmzeb4fd, worker pi/gpt-6.1-sol, commit 62aba8e; the coordinator checked the
      containment logic against three channel lists: the CI case passes, an extra channel passes, a missing channel
      fails naming it): the step "Units are enabled and active, and their programs run" no longer compares the
      channel list for exact equality. `has_channel()` tests that `rormpc` and `listenbrainz_feedback` are present,
      the wait loop breaks as soon as both are, extra channels are allowed, and a missing one fails with its name.
      The comment now records that the scrobbler subscribes to listenbrainz_listen as well.
      Open until the next CI run confirms it: the push that lands this touches this workflow, so the Installer smoke
      test starts on master by itself; close the item when its companions-linux job is green (this item), and note
      the run id.

- [x] The like column's hover cue (asked 2026-10-12, a screenshot: "what is this underline under the heart? ask the [t-jminob65]
      Dispatched 2026-10-18 as run r-eehcbr64 (worker pi, check `tests`; task in /tmp/task-t-jminob65.md): no
      underline, accent heart on hover, two-cell click target, bold for liked only, and the status-bar hint if the
      status bar can show it without a sticky message or a timer (otherwise the worker reports what it could not do).
      models"): the underline marks the heart under the mouse (rmpc/src/ui/rormpc_actions.rs `hovered_like`), and in
      the user's font the glyph is wider than its cell, so it looks like a glitch. Round 20261012-025148 (sol 124bcaab,
      MiMo 9800a64b, DeepSeek 2c338633): drop the underline. Decided by the user 2026-10-12 (d-njr573xw, after a
      mockup): no underline; while the mouse is over a heart the status bar says "click: like" / "click: unlike", the
      heart takes the accent colour (checked on the selected row too), and the column is two cells wide (heart + a
      blank, which is also the click target); bold stays for liked only.
      Landed 2026-10-18 (run r-lezhh63w, worker pi/gpt-6.1-sol, commit 5bcd297; check `tests` by the driver's
      verify): the underline is gone; a hovered heart takes the theme's label accent (`preview_label_style.fg`) and
      stays accent-coloured on a reversed selected row (the row repaints after the table so its
      `row_highlight_style` no longer overrides it), an unrated cell previews ♥ in that accent, and bold marks a
      liked heart only (`like_cell()` at rest, `hovered_like(line, accent)` on hover). The like column is two cells
      wide in the queue, the queue plan and hits, and both cells are the click target: hits derives it from the
      table's own column constraints (`columns()`, `like_area()`) instead of a hardcoded x, and the tests pin the
      geometry, the header exclusion, both cells hovering the same row and the hover clearing when the pointer
      leaves. Commit message: the subject carries "(AI-assisted)" so the driver's exact-message check and the
      repository's disclosure rule both hold.
      Next run of this item (approved by the user 2026-10-18, "tak"): the hover-scoped status API and then the
      "click: like"/"click: unlike" hint while the pointer is on the heart.
      Landed 2026-10-18 (run r-ich3gpku, worker pi/gpt-6.1-sol, commit c65d2eb; check `tests` by the driver's
      verify): the status bar now has a hover-scoped hint beside the timed messages. `Ctx` keeps the hint with the
      pane that owns it and the area it covers (`set_like_hint`, `clear_like_hint`, `clear_hover_outside`,
      `refresh_like_hint`, `set_hover_hint`), so a pane sets it when the pointer enters the like cell and clears it
      when the pointer leaves that area, another pane cannot clear it, and a repaint cannot resurrect a departed
      hover; only a real change sends `AppEvent::HoverHintChanged`, so no redraw is left pending (no timer
      anywhere). The progress bar shows the hint only while no timed status message is alive, so an error is never
      hidden. Hits, the queue and the queue plan show "click: like" / "click: unlike" from the song's like sticker
      ("2" liked, the values in rmpc/src/ui/rormpc_actions.rs:59-60; a ✗ becomes liked on a click, so it says
      "click: like"). Tests cover the ownership and leaving rules, the redraw bookkeeping, the timed-message
      priority and the rendered text.
      Also landed under this run (dispatched with this item's id by mistake): the AGENTS.md paragraph that
      reconciles the (AI-assisted) disclosure rule with herdr's exact-message verify — it belongs to item
      t-5n24p4bs, which is marked done with that commit.

- [x] The `(AI-assisted)` disclosure and herdr's exact-message verify (found 2026-10-18 while landing [t-5n24p4bs]
      t-jsjs5ngd's and t-jminob65's worker commits): two workers in a row were blocked by the conflict between
      AGENTS.md's "add an (AI-assisted) text to all messages" and the driver's requirement that a worker's commit
      subject equal the coordinator's `--message` exactly, with no body. The coordinator now puts "(AI-assisted)" in
      its `--message`, so the worker can obey both; write that into AGENTS.md "Coordination" (one line: the subject
      the driver is given carries the disclosure, a worker adds nothing to the body) and say so in the worker
      contract template. Parent: t-jminob65. Cost: a text change, no build.
      Done 2026-10-18 (commit d699dff, landed under run r-22dveye3): the paragraph is in AGENTS.md "Coordination".
- [x] Drop RORMPC_TOOLS_TAG from scripts/rormpc_install.sh: rmpc/src/shared/dependencies.rs reads it at build time for [t-z5e3iizs]
      the missing-tool hints (`cannot_run`, delete_menu.rs, `debuginfo`); point those hints to
      `uv tool install "rormpc-tools @ git+https://github.com/rofrol/rormpc@<build commit>#subdirectory=tools"` (or
      `scripts/rormpc_install.sh companions`) and remove the tag and its test. Found by the t-6bzyvzb6 worker.
      Landed 2026-10-18 (run r-f4ex4hch, worker pi/gpt-6.1-sol, commit 6fd29f8; check `tests` by the driver's
      verify): `RORMPC_TOOLS_TAG` is gone — the installer no longer defines it, the static and its parsing of
      `scripts/rormpc_install.sh` are removed from rmpc/src/shared/dependencies.rs, and `cannot_run()` now tells the
      user to `uv tool install "rormpc-tools @ git+https://github.com/rofrol/rormpc#subdirectory=tools"` or to run
      `scripts/rormpc_install.sh companions` for tools at the same commit as rormpc. The `musicdb delete --preview`
      version-mismatch message (rmpc/src/ui/modals/delete_menu.rs:99) keeps the companions hint without the tag,
      `rormpc debuginfo` prints the tools heading without an expected tag, and AGENTS.md and RORMPC.md say the same.
      A test pins the new hint and asserts the archived `rofrol/rormpc-tools` no longer appears. The first attempt
      (7d1967c) did not compile because delete_menu.rs stayed outside its allowed paths.
- [ ] YouTube deletion fails with HTTP 404 `videoNotFound` (user screenshot, 2026-10-13): add this to TODO and [t-pyr3xmsz]
      consult the models. `musicdb delete` reports "Deleted permanently" together with "failed, retried hourly".
      Diagnose playlist lookup/removal of unavailable videos, preserve deletion and restore evidence, classify
      terminal versus transient remote failures, and give a concise, accurate partial-success status. Test offline;
      do not delete anything from the user's playlists while investigating.
      Consult 2026-10-13, round 20261013-101517-e1fa (MiMo 0f81906c, DeepSeek 646f84aa; both useful;
      Sol excluded as the caller's family): both recommend complete unfiltered pagination, matching video IDs
      locally, not interpreting lookup 404 as absence; verify with an unavailable-video fixture and, only if
      separately approved, read-only live evidence. Classify structured reason AND operation (lookup/delete),
      preserve auth/playlist failures, and verify item absence before resolving delete-side not-found.
      Confirmed in code: all-or-nothing subprocess output loses successful removals on a later failure;
      restore.py uses journal `ytid` plus `youtube_removed` playlist IDs. Preserve per-item intent and confirmed
      outcomes durably, reconcile ambiguous outcomes at startup or explicit rerun, and test crashes and partial
      failure. Do not add video IDs redundantly just to repair restore (MiMo suggestion rejected: already stored).
      Timing: event-driven outcome reconciliation; existing hourly update is external YouTube reconciliation,
      not recovery of lost internal outcomes; no new timer, sleep, longer wait or blind resend.
      Landed 2026-10-18 (run r-xlhulvhi, worker pi/gpt-6.1-sol, commit f5916de; checks `tests tools` by the driver's
      verify): the YouTube step of `musicdb delete` no longer loses work or claims what it did not do.
      `yt_playlist.py`: `playlist_hits()` keeps the API's structured reason and, on a 404 `videoNotFound`, falls back to
      unfiltered pagination of that playlist and matches the video ID locally (`video_of()` reads `contentDetails` or
      `snippet.resourceId`), so an unavailable video that is still in a playlist is found; `find()` and `remove()`
      return one result per playlist (`present`/`absent`/`removed`/`failed: <reason>`, with the operation) instead of
      an all-or-nothing list; `remove()` journals an intent before each DELETE and a confirmation after it, reconciles
      a lost response by re-listing, and skips items already removed, so a retry never deletes twice.
      `musicdb.py`: `youtube_remove_record()` checkpoints `youtube_items`, `youtube_removed` and `youtube_results` in
      the journal (pending.jsonl, under the journal lock) before and after every write and marks `ops["youtube"]` as
      awaiting confirmation mid-flight, so the hourly update continues what is left; `delete()`'s single status line
      now separates the local result from the remote one (" · YouTube removal pending (retried hourly): 404
      videoNotFound" or " · YouTube: removed from N playlists", and the same for ListenBrainz), exits non-zero only
      when the local deletion failed, and notifies "Remote cleanup pending, retried hourly" instead of "FAILED".
      Tests: 25 focused (fake API), the full suite 683 passed (9 socket tests blocked in the sandbox).
      Open: the message and the fallback are verified offline only; confirming them on a real unavailable video needs
      the user's playlists (a live run is his call).

- [ ] Missing release years and uppercase metadata (user, 2026-10-13): "PILLOWTALK — ZAYN has no year. Review [t-dasiajfe]
      the others. Why the caps? Ask the models." Trace this song's file tags, MPD fields and displayed metadata;
      audit other missing years and uppercase titles/artists, distinguish missing source metadata from display/cache
      bugs, and report provenance. Follow the existing years review and rollback workflow; do not guess dates from
      YouTube uploads or blanket-title-case intentional artist/title styling.
      Read-only MPD check 2026-10-13: Title/Album PILLOWTALK, Artist ZAYN, recording MBID present,
      no Date or OriginalDate; hits.library_songs passes title/artist through unchanged. Actual file tags and
      official capitalization remain unverified; the filename's upload date is not release-year evidence.
      File-tag audit 2026-10-18 (read-only, mutagen over the library, herdr-job 20261018-112057-cc90): 806 audio
      files; **178 have no date tag at all** (so the missing year is source metadata, not a display or cache bug);
      PILLOWTALK's tags are title "PILLOWTALK", artist "ZAYN", album "PILLOWTALK", no date — and both the title and
      the artist are exactly the YouTube uploader's strings, which answers the "why the caps" question: the tags
      copy the source title, the caps are not introduced by rormpc (9 titles and 32 artist strings are all-caps;
      several are legitimate, e.g. M83, LMFAO, AWOLNATION, VA). Next: the year repair is the existing `musicdb
      years` review workflow (reviewed, backed-up, atomic apply and rollback) over the 178 files without a date; it
      needs the user's approval of the proposed years, and dates are never guessed from upload dates.
      Consult round 20261013-101517-e1fa (MiMo and DeepSeek): first compare file tags with MPD, then audit missing
      dates and capitalization provenance; reuse years.py's reviewed, freshness-guarded, backed-up atomic apply
      and rollback. Caps alone are not an error. Include unavailable release metadata, migration of upload dates,
      stale MPD, and version identity in test fixtures. MiMo flags the eight-digit-date heuristic as a possible
      ambiguity: verify provenance/format before treating an eight-digit value as upload metadata; not a confirmed
      cause here. Timing: no timing, on-demand audit and reviewed repair. No personal library inventory in git.

## Done: live Queue plan view (approved 2026-10-06, done 2026-10-07)

- [x] `o` toggles a view-only Queue plan; never sort or move the MPD queue to display it. Show the last two [t-hauxb32o]
      plays (`-2`, `-1`, dimmed), `0 ▶` current, `↑n` requests, all ten forecast slots, then an unselectable
      `unplanned · queue order` divider and the remaining songs in MPD order. Label the mode in the panel title.
- [x] Preserve selection, scrolling and mouse targets by queue song ID across forecast refreshes; break ties [t-6tkz6t5e]
      by MPD position. Disable Next-header sorting in this view. Filtering keeps the original turn numbers.
- [x] Dim stale forecast numbers and show `stale · Xm` when the daemon is absent or its state is old. [t-7ftn7rfa]
- [x] J/K reorder requests in the request section; in the forecast they request a version-checked slot swap [t-kv4kjvwe]
      from mpd-player, which alone writes priorities and publishes confirmation. Never move across section
      boundaries or move past/current/unplanned rows. Document patch lifetime and clear it when a patched
      song plays or leaves the queue, or the daemon draws a new forecast.
- [x] Verify with `cargo test`, `uv run pytest`, `uvx pyflakes src`, and TUI tests only on a scratch MPD. [t-khiaasvh]

Verified: `cargo test` (957 tests), `uv run pytest` (123 tests), pyflakes, and the TUI on a null-output scratch MPD
(port 6650): 20 confirmed forecast swaps with the MPD queue order unchanged, past/current/request/forecast
sections, request J/K, a filter that keeps slot 10 and plays its ID, stale state from an old, deleted or
59-second-old heartbeat while paused, `stale · 0m` at once when the daemon is killed (MPD sends no Subscription
event on a client disconnect, so rormpc waits on the daemon's process exit), and fresh again after a restart.

- [x] Approved and fixed 2026-10-07 (13715f4): keys that reach rormpc in the same input batch as `/` are resolved in normal mode (also in the [t-55qjjftx]
      ordinary Queue filter), so a paste or a scripted `send-keys / text` runs the letters as commands. Seen
      2026-10-07 on the scratch MPD (`c` turned consume on).

## Done: Hits works like Queue, Up next as its own tab (2026-10-06)

- [x] Hits behaves like the Queue pane: e.g. Shift+C jumps to the currently playing song, if it is in the [t-3mrqx4hb]
      Hits table.
- [x] Hits is the first tab. [t-rvllo3go]
- [x] Up next is one top-level tab "Up next" (the menu action stays "Play next"), not a sub-tab of Hits and [t-azdqvr3h]
      Queue (user decided 2026-10-06; Sol and MiMo: a view embedded twice means two cursors on one list). It
      lists the songs waiting in Up next: J/K reorder, remove, clear with a confirmation, Enter plays now; no
      "Play next" inside it. Hits and Queue get a key that opens it. Full plan: music-data TODO.md, entry
      "Playback annoyances".

Verified: `cargo test` (934 tests), `uv run pytest` (107 tests), pyflakes, and the TUI on a null-output scratch
MPD. Covered current-song jumps and custom bindings, search preservation, numeric tabs and `gu`, request
reorder/remove/confirmed clear, and failed `playid` while paused: requests remain, the error redraws without
another key, and a later explicit successful play clears it. No playback retry or added polling delay.

## Soon: playing-song indicator in Hits (design only)

- [x] Consult the models and prepare visual mockups of playing-song indicators in Hits (for example, a [t-ue3ntd5o]
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

- [x] Investigate why the film score / soundtrack genre is missing from songs such as "Cast Away Theme" [t-x7xs2ppp]
      by Alan Silvestri, and make that classification available in Hits genre filters.

Done 2026-10-07 (rormpc-tools 1f26bfe, not released yet): MusicBrainz has no soundtrack genre, only tags; Silvestri
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

- [x] Tested 2026-10-07 on a scratch MPD (rormpc-tools 4322e1a pins it): a mute that spans an mpd-player gap silence. [t-kfmmtztt]
      They leave each other alone: an expiry inside the silence restores the volume and stays paused until the gap
      presses play; a gap while muted pauses and resumes at volume 0 and the mute's deadline restores it later; a
      mute started during the silence works (a pause is not a stop).
- [x] A queue replaced by Sources… may pass through a stop, which unmutes. Watch whether that happens in use. [t-kmhfnth6]
- [x] Maybe later: "Pause for…" (now that one daemon owns the gap, it no longer fights mpd-gap). [t-ydqho7yh]
- Closed 2026-10-09: "Mute for…" was removed and "Pause for…" built instead (see "Next, in order").

## Test report of 2026-10-06 (scratch MPD, music-mpd-8e)

Fixed 2026-10-06: Up next entries skipped past between two wakes of mpd-player (`mpc next` three times in a row,
random on) stayed waiting with a ↑1 badge; now a waiting entry whose MPD priority went back to 0 has started and
counts as played. The Queue menu keeps Weighted shuffle, Silence, Mute for… and Sources… with an empty queue;
"Clear queue…" asks first (Cancel is the default); Sources… leaves out empty playlists; menus are as wide as their
longest label (within the terminal).

- [x] Same race with random off: entries are moved after the current song, so skipping past them leaves them [t-e37djosr]
      before it. Telling that apart from the user jumping to a later song needs more than positions; not done.
- [x] mpd-player down while an Up next song plays: it stays waiting. After an MPD restart the re-found entries [t-pokofslm]
      carry no priority yet, so "priority 0 = started" cannot be used at startup.

Fixed 2026-10-07 (rormpc-tools 76bd09e): MPD resets a song's priority to 0 whenever it starts, random on or off, and a
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

- [x] Document it in RORMPC.md ("Media keys" section): what it is, the commands above, how to check it [t-7udt3tch]
      (`launchctl print gui/$UID/me.00dani.mpd-now-playable`), the upgrade caveat, and that it is independent
      of rormpc.
- [x] Do not fold it into `rormpc_install.sh install`: that script installs and rolls back the binary, and a [t-7dcqfxzv]
      binary rollback must not touch the service. If a helper is still wanted, make it a separate opt-in
      subcommand (e.g. `rormpc_install.sh media-keys install|status|uninstall`) that only calls the tool's own
      `install-launchagent` / `uninstall-launchagent`, never writes its own plist, and refuses to run a second
      Now Playing source.
- [x] Linux: MPD has no MPRIS of its own; a bridge exposes it on the session D-Bus. Options: this [t-hmqhlfug]
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
- [x] Extend rmpcd instead (asked 2026-10-07: "maybe extend the rmpc daemon? ask the models"). Consulted [t-r5nbxrzk]
      2026-10-07 (GPT-6.1 Sol, MiMo), both rank it last today: keep mpd-now-playable (+ mpd-mpris on Linux) first,
      a standalone crate second, rmpcd only as a deliberate commitment to run rmpcd (unused here, early-stage MPRIS,
      tokio + Lua + zbus; edits near its startup re-conflict on every upstream bump). If rmpcd: AppKit's
      NSApplication on the main thread, the tokio runtime on a worker thread, `CFRunLoopStop` from the signal path
      for shutdown, features `now-playing` (macOS) and `mpris` (Linux) so zbus never builds on macOS, one cfg-gated
      module and a small spawn hook. Both: AirPods / Now Playing Previous should also go to mpd-player's `shuffle
      prev` (mpd-now-playable's handler, or the port's), so every Previous walks the same history.
      Decided 2026-10-07 (the user): leave it as it is; no Bluetooth headphones or Now Playing widget are used to
      switch songs, so mpd-now-playable stays unchanged and rmpcd is not extended.
- [x] Rust instead of the Python tool on macOS. Asked GPT-6.1 Sol and MiMo twice on 2026-10-03; both say keep [t-hnddajm5]
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
- [x] Windows: no plan unless MPD on Windows is actually used (SMTC, e.g. via souvlaki; needs a hidden HWND with a [t-4be65hpj]
      message pump).
- [x] Whatever is documented, check: works with rormpc closed, survives MPD restart and sleep/wake, clears [t-5hkpfm6j]
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

- [x] Menu: hover selects the entry under the cursor, a single left click confirms it. One hit-test function shared by [t-44dzvz5w]
      hover and click (headers, separators, borders, scrolling); a click on blank space never runs the selected entry.
      Redraw only when the selection changes.
- [x] `Moved`: pass it on only while a modal that wants hover is open; drop it early otherwise, so motion causes no [t-a5ddt7ve]
      redraws or wakeups elsewhere. Check that nothing treats any mouse event as activity.
- [x] Click-through: double clicks are synthesised from two left clicks, so once one click closes the menu the second [t-ki4qcclm]
      one arrives as `DoubleClick` on the pane underneath and can play/add a song. Reset the double-click tracker
      when a modal closes on a click (or swallow the next click at that position).
- [x] Outside click: an opt-in per modal (e.g. `fn layout(&self, frame: Rect) -> Option<Rect>` computed the same way [t-3xqdfi5o]
      as in `render`, no stored rect, so it is right before the first render and after a resize; `None` means "not
      dismissible"). Menu, select, info, keybinds, outputs, decoders: close (same path as Esc). Input: cancel like Esc,
      never submit. Confirm / destructive modals: never close on an outside click. Swallow the click and wheel events
      outside instead of passing them through.
- [x] Backdrop: switch `modal_backdrop` to `Modifier::DIM` like herdr and enable it in the `roman` theme; decide [t-5mu5o62z]
      whether the context menu dims (herdr does not). Popups must start from `Clear` or they inherit DIM. Check in
      Ghostty and kitty (`faint-opacity` / `dim_opacity` decide how strong it is) and with bold/reversed selections.
- [x] Order: menu hover + single click with the click-through fix first, outside click second, backdrop last. [t-ccfiqjgp]

Done 2026-10-03: `MouseEventKind::Moved`, `Section::item_at`, `Modal::area` (every modal except the confirmation
one), `Ui::modal_click`; the menu ignores `DoubleClick`; `QueueFindModal::destroy` leaves insert mode. The context
menu is dimmed behind as well: it is a centred popup here, not a menu at the pointer as in herdr. Checked in the
TUI through herdr: hover highlight, one click opens "Show info", a double click outside closes the menu without
playing the row underneath, the backdrop cells carry DIM and the popup does not. Still to check by eye: how strong
DIM looks in Ghostty.

- [x] Hover lag reported 2026-10-03 (the user is not sure it is real). The user runs rormpc in a Ghostty quick [t-nfpnkfhh]
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

- [x] Prerequisite: a non-interactive mode for dotfiles `yt-mp3-mb` (e.g. `--batch --json`): no prompts, prints [t-odozsiae]
      the produced paths and the unresolved matches as JSON, resumable without duplicate downloads. Uncertain
      matches stay "needs review" instead of being asked about inside a child process the TUI cannot answer.
- [x] A new CLI in dotfiles (like `musicdb` / `hits`), e.g. `liveplaylist add|check|accept|reject|list --json`. [t-nlr65hdi]
      It owns state, source adapters, matching, downloads and writing the MPD playlist; rormpc owns presentation
      and decisions only (a URL input modal and a "Live playlists" pane with accept / reject / accept all), runs
      it with argv (no shell), never blocks the UI thread, can cancel it. Progress as JSONL or a status file
      written atomically (temp + rename), as the Hits pane does.
- [x] State per subscription in music-data (secrets outside it, logs/progress in ~/.cache): url, kind, MPD [t-ebce5ppk]
      playlist name, a stable target dir (not the playlist title, which can change), schema version, lock file.
      Per source item: the decision (pending/accepted/rejected, rejects are durable) separate from the job state
      (queued/downloading/needs_match/ready/failed), source position and last-seen time, the local path.
- [x] Order lives in the .m3u (playlist_directory), not in `NNN` file names. Publish only ready files. Songs [t-saehdg3c]
      already in the library are referenced, not downloaded again, but only on a confirmed recording match
      (MBID), never on a loose title match.
- [x] Removals and reorders upstream: never delete local files and never infer a removal from a failed or partial [t-r67d6t3y]
      check (yt-dlp YouTube extraction breaks, bot checks, 403s); an id that reappears is reactivated.
- [x] First version: public YouTube playlists only (`yt-dlp --flat-playlist -J` to list ids cheaply), manual [t-tm6p4zkx]
      check, first import reviewed, batch accept. No timer: nobody answers "add these?" at 4 am; later a launchd
      check may only add pending items and notify.
- [ ] Later, maybe, Spotify. Verified 2026-10-03 in Spotify's February 2026 migration guide: playlist items are [t-i3kf2nsf]
      readable only for playlists the user owns or collaborates on (not arbitrary public URLs), Development Mode
      needs the app owner on Premium and allows 5 users; since Nov 2024 algorithmic and Spotify editorial
      playlists are off-limits to new apps. So: user OAuth (PKCE), own playlists only. Matching to YouTube: ISRC
      or artist + title + duration + version words, always reviewed, never the first `ytsearch` hit (covers,
      live, nightcore, loops); a YouTube rip often fails AcoustID, so "downloaded, unidentified" is a real state.
- [ ] Radio: only if a station publishes a track history (an API or page); ICY metadata brings ads and DJ talk. [t-j2jzqsov]
      Out of scope until there is a concrete station to look at.

## Tests and GitHub Actions for rormpc, rormpc-tools and ro-listenbrainz-mpd

Today nothing guards these three repositories: rormpc's CI comes from upstream and runs only on pull requests
and by hand (pushes to master run nothing), rormpc-tools has no tests, and the scrobbler fork's deltas are not
tested anywhere (upstream's CI is on Codeberg). Bugs found by hand on 2026-10-03 that tests would have caught:
KeyError 'user_name' on an invalid ListenBrainz token, an uncaught SystemExit that stopped the hourly sync,
the launchd bootout/bootstrap race (error 5), missing libsqlite3-dev on Linux.

Plan (2026-10-03, after asking GPT-6.1 Sol and MiMo; both: tests first, CI second):

- [x] (tests done locally 2026-10-07, 060fa49; the workflow waits for the CI decision) rormpc-tools: pytest without network, on a temporary DB/data dir and mocked HTTP: invalid token and a [t-abkgrqxq]
      ListenBrainz outage still let `update` sync and export; play counts, and a local listen counted once with its
      ListenBrainz copy (same timestamp); skips import and the Skipped playlist; delete/undo with the journal;
      mpd-gap state transitions on a fake clock and fake MPD status. Workflow on Ubuntu, ~1-2 min.
- [x] (tests done locally 2026-10-07, 862c08a; the workflow waits for the CI decision) ro-listenbrainz-mpd: `cargo build` on Ubuntu (catches the apt build dependencies), tests for the listen rule [t-5c5awwvf]
      (fraction, max seconds, uninterrupted: seek restarts, pause neutral) and the local log lines. ~2-4 min.
- [ ] rormpc: installer smoke test on Ubuntu: fresh user, `loginctl enable-linger`, `XDG_RUNTIME_DIR` and the user [t-6vqhpuoq]
      D-Bus, MPD with a generated tone, fake token and API URL, `rormpc_install.sh companions`, then assert units and
      the musicdb timer are active, the listen_* lines are in the config and the token untouched, `status` output,
      a reinstall. The installer pins released tags, so a smoke test would pass on a broken branch: first add
      overrides (e.g. `RORMPC_TOOLS_REF`, `RO_LB_REF`, or reuse `--local` with checkouts) so CI tests the commit
      under test. ~5-8 min. This is what was done by hand in an OrbStack Ubuntu VM on 2026-10-03.
- [x] (left out, decided by the user 2026-10-09) macOS: first a throwaway probe that `launchctl bootstrap gui/$UID` works on hosted runners, then the same [t-bdru7wdf]
      smoke test with launchd.
- [x] Triggers: push to master, pull_request, workflow_dispatch, weekly schedule (toolchain, uv and runner-image [t-bnzdfzn6]
      drift; GitHub disables schedules after 60 days without repository activity), and on release tags. Add
      `push` to the upstream ci.yml here too.
- Not automated: live ListenBrainz, MusicBrainz, Billboard and YouTube (OAuth) calls, and exact gap timing on
  shared runners.

## Versions: delete a bad version, find versions from the Queue

- [x] Versions: delete a bad version, and find versions from the Queue (asked 2026-10-06: "in Versions I can't [t-t7djeefl]
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
  - Done 2026-10-07 (f079b15): `Versions()` song property (`≋`), cached from one `musicdb versions --json --all`
    and read again after a delete, merge or library update; "Find versions…" in the Queue menu and `V`
    (QueueActions::FindVersions); the delete menu re-checks the preview and reports musicdb's last line when
    done; the Versions pane now lists resolved groups too (dimmed). Your explicit keymap and theme need
    `"V": FindVersions` and the column `(prop: (kind: Property(Versions()), default: (kind: Text(""))),
    label_prop: (kind: Text("≋")), width: "1")`; a column before Year/Plays shifts their SortByColumn numbers.

## Lyrics: Polish translation next to the original

- [x] Asked 2026-10-07: show a Polish translation in the Lyrics tab, the original on the left and Polish on the [t-suqwi72j]
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
  - Done 2026-10-07 (rormpc 1291c99, rormpc-tools 4cf9271): `musicdb lyrics translate FILE|--current` (one song, on
    demand; the usual URL first, else the site search, at most 3 requests 2 s apart, a clear User-Agent), accepted
    only when the page's original overlaps ours by word (Jaccard ≥ 0.45); `<stem>.pl.json` sidecars; language
    detected once with langdetect (new dependency), `musicdb lyrics lang FILE CODE|auto` overrides it. tekstowo.pl's
    robots.txt allows song and search pages (its `Disallow: /` names only AI crawler user agents) and its terms
    allow private use, commercial use is banned. The Lyrics pane: Enter looks the translation up (the pane gets
    mapped actions, not raw keys, so not `t`), h/l switches original / translation below 100 columns, a stale
    translation is hidden. Live check: Viva la Vida and Skyfall (their sidecars are in the real lyrics_dir).

## Previous in the weighted shuffle

- [x] Asked 2026-10-07: with the weighted shuffle on, the Previous media key bounced between unrelated songs and [t-po7ptgcs]
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
  - Done 2026-10-07 (rormpc-tools 7bf33bb, by a worker): `shuffle prev [CMD_ID]`, a trail of really played queue
    ids with a cursor in shuffle.json, a 0.25 s debounce, prev.jsonl (fsynced before `playid`, then confirmed or
    failed), `import-skips` drops a matching skip within 2 s. The worker's own choice: a song already played to
    80% (FINISHED_SHARE) keeps its finished outcome when left with Previous. The rormpc history shows the neutral
    case as kind "back".

## Versions: audio fingerprint finds copies

- [x] Asked 2026-10-07 (screenshot): "Love Story (Disco Lines remix)" (official upload, 139 s) and "Love Story (Disco [t-65ymtdhz]
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
  - Done 2026-10-07 (rormpc 66d1b50, rormpc-tools ec3fe5f): `audiomatch.py`; `musicdb versions --json` reads only the
    fingerprint cache (~/.cache/rormpc-tools/fingerprints.json) and lists missing files; `musicdb versions
    fingerprint` fills it (the Versions pane in the background, the hourly `update`; 130 files took 3.9 s). On the
    library: 32 groups suggest "Same recording?", 6 pairs "similar". The worker's own choices: length does not pick
    the file to keep when lengths differ by more than 25% (a loop or extended mix: only the start was compared, the
    suggestion says to listen first); files labelled as different versions are never suggested. `a` in Versions
    merges the selected audio match after a confirmation ("Keep another file…" picks another); not clicked through
    in the TUI because Merge would move a real file to the quarantine.

## Proposed

- [ ] The installer smoke test's macOS job fails on master: the `install-stubbed (macos-latest)` job of [t-23vr4rs4]
      .github/workflows/installer_smoke.yml fails in `scripts/test_rormpc_install.sh` (the log ends after
      "restart running rormpc instances to use it" with exit code 1; it also failed in run 38256551254 of
      2026-10-12, before the companions-linux fix, so it is not caused by it). Needs a run of the workflow with the
      failing assertion read out of the full log, then the fix, and the same kind of review as t-kwqstgkt. Cost:
      one worker run plus a CI run to confirm.

- [x] Report the osx output rate race to MPD upstream (from the fast-playback fix of 2026-10-17, round [t-hxhcbpqm]
      20261017-150106-b061, DeepSeek; after the 2026-10-18 round 20261018-015021-5614): OSXOutputPlugin sets the
      device's physical format and initialises the audio unit without waiting for
      kAudioDevicePropertyNominalSampleRate to change, so 44.1/48 kHz switches can play at 48000/44100. Needs the
      user's go (an outward action). The 2026-10-18 measurements did not reproduce the race (see t-ryp5kjet), so
      either find a reproducer (MiMo: a stress loop over 200+ output opens capturing the device's PhysicalFormat at
      every Open, which no-ops now that the config keeps the device at 48 kHz; Argon: unpinned with mpd-player's gap
      and another process holding a 48 kHz stream) or send the source argument as a comment on
      https://github.com/MusicPlayerDaemon/MPD/issues/1005 with the two scratch-MPD logs. Cost: a short report, no
      build.
      Done 2026-10-18 (the user's go, "1 i 2"): a comment on
      https://github.com/MusicPlayerDaemon/MPD/issues/1005#issuecomment-6243838710 with the source reading (Open sets
      kAudioStreamPropertyPhysicalFormat at OSXOutputPlugin.cxx:426, then AudioUnitSetInputStreamFormat:721 and
      AudioUnitInitialize:729 without waiting), the 1.0896 and the fixed-rate measurements, and an offer to prepare
      a patch that waits for the device to report the requested rate. The issue stays closed; watch for a reply.
- [x] Report the osx output abort with a per-output `format` to MPD upstream (2026-10-18, t-ryp5kjet, after round [t-73voypj6]
      20261018-015021-5614): MPD 0.24.15, per-output `format "48000:*:*"` on the osx output (soxr or libsamplerate),
      a seek into the last ~12 s, then a song change 44.1 -> 48 -> 44.1:
      `Assertion failed: (nbytes > 0), function PlayChunk, file Thread.cxx, line 269`. Isolation done: the `null`
      output with the same format and seeks is fine, so it needs the osx output; the decoder-side
      `audio_output_format` is fine; two generated 30 s sine files do not trigger it, the real songs (204.157 s and
      290.458 s with mpg123) do. The user's config no longer uses that option (dde312e), so nothing breaks for him;
      the report protects everyone else, and MiMo, Argon and the repo rule 6 all point the same way. What a good
      report needs: the two mp3 files or a licence-clean reproducer that also triggers it, `log_level "verbose"`,
      the config, the seek commands, and an lldb `thread apply all bt` of the abort. Needs the user's go (an
      outward action) and then a fork with the regression test if upstream wants a patch. Cost: a report, no build.
      Done 2026-10-18 (the user's go, "1 i 2"): https://github.com/MusicPlayerDaemon/MPD/issues/2601, with a
      licence-clean reproducer instead of the songs: two generated 30.337 s sine mp3s (ffmpeg commands in the
      report), the seek sequence, the verbose log excerpt and the macOS crash report's backtrace of the output thread
      (`__assert_rtn` below `AudioOutputControl::PlayChunk`); 30.000 s files do not trigger it, so the leftover chunk
      matters. A fork with a patch and a regression test follows only if upstream asks (rule 6).
- [x] Pin the osx output to the built-in speakers' device UID in ~/.mpdconf (from the same round, MiMo, DeepSeek, [t-7xd7nok4]
      Argon): with the default device MPD follows a switch to another output (Teams' virtual device, headphones).
      Changes behaviour when the user picks another output on purpose, so it is the user's choice. Cost: a config
      line and an MPD restart, no build.
      Done 2026-10-17 in dotfiles 8e9e5ee: `device "MacBook Pro Speakers"` (MPD matches the CoreAudio device name);
      kept in the 2026-10-18 rewrite (dde312e).
- [x] Pin the osx output to the built-in speakers' device UID in ~/.mpdconf (from the same round, MiMo, DeepSeek, [t-3etxycyf]
      Argon): with the default device MPD follows a switch to another output (Teams' virtual device, headphones).
      Changes behaviour when the user picks another output on purpose, so it is the user's choice. Cost: a config
      line and an MPD restart, no build.
      Done 2026-10-17 in dotfiles 8e9e5ee: `device "MacBook Pro Speakers"` (MPD matches the CoreAudio device name);
      kept in the 2026-10-18 rewrite (dde312e).

## Needs a decision

- [x] The like column's hover hint in the status bar: the rest of d-njr573xw landed 2026-10-18 (no underline, accent [t-yepz4aez]
      heart, two-cell click target, bold for liked only; run r-eehcbr64, commit 393c76f), but the "click: like" /
      "click: unlike" text while the pointer is on the heart could not be built. The status bar is fed by
      `AppEvent::Status` messages that the event loop removes on a timer, so a hint would be sticky or need a timer,
      and the worker stopped rather than invent one. Build a hover-scoped way for a pane to show and clear a status
      line, drop the hint and rely on the accent alone, or put the hint somewhere that is redrawn every frame?
      Options: a hover-scoped status API (the pane sets and clears it on the same mouse events that set its hover
      state; no timer) | drop the hint, the accent and the two-cell target are enough | show the hint in the like
      column's header row instead ("♥ click")
      Checked: worker w471's WORKER-BLOCKED report of run r-eehcbr64 and rmpc/src/shared/macros.rs:36-47 (the
      `status_info!`/`status_error!` log targets) plus the status-bar event handling in rmpc/src/ui.
      Decided by the user 2026-10-18 ("tak", to the recommended option): build a hover-scoped status API — a pane
      sets and clears the status line in the same mouse events that set and clear its hover state, with no timer —
      and then show "click: like" / "click: unlike" while the pointer is on the heart. Approved work: the next run
      of t-jminob65 after the styling one lands.

- [x] Previous in the weighted shuffle: restart the current song (seek 0, no outcome) when Previous is pressed in [t-4rtdcdqz]
      its first ~3 s? Decided 2026-10-07: no, Previous always goes back.
      Checked: Sol says leave it out (it fights walking back), MiMo says restart; the TODO marks it undecided.
- [x] Previous: may an agent reproduce the burst with your Hammerspoon and mpd-now-playable pointed at a scratch [t-7szvmown]
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
- [x] Lyrics: which model and account should the machine-translation fallback use (the lyrics leave the machine)? Decided 2026-10-07: Claude through the Anthropic API (queued in Next). [t-oxpnfol4]
      Checked: the plan says "an LLM" without naming one; no API key for it is configured in rormpc-tools.
- [x] Media keys: do you still want an opt-in `rormpc_install.sh media-keys install|status|uninstall` helper? Answered 2026-10-07: "not sure; maybe extend the rmpc daemon (rmpcd)? ask the models" (consulted; decided: leave it as it is, no helper). [t-iznpgwli]
      Checked: the plan makes it conditional ("if a helper is still wanted").
- [x] Media keys on Linux: set up a bridge (rmpcd, mpd-mpris) on Omarchy or Roguix now? Closed 2026-10-07: the user chose to leave media keys as they are (keep mpd-now-playable). [t-ytb5fnr5]
      Checked: needs those machines and a live test of the keys; nothing on this Mac to verify it.
- [x] Start the Rust Now Playing port (`rormpc-now-playable`), or keep mpd-now-playable? Closed 2026-10-07: the user chose to leave media keys as they are (keep mpd-now-playable). [t-ledvb2uw]
      Checked: Sol and MiMo both said keep the Python tool; the plan is complete if it is wanted.
- [x] Live playlists: start building them (yt-mp3-mb batch mode in dotfiles, the `liveplaylist` CLI, the rormpc [t-ahsljup3]
      pane)? Decided 2026-10-07: yes, the first version (queued in Next).
      Checked: a multi-repository feature with downloads; the plan is agreed but not ordered.
- [x] CI: push GitHub Actions workflows (rormpc-tools, ro-listenbrainz-mpd, rormpc installer smoke test, `push` [t-yg2242o4]
      trigger in ci.yml)? Decided 2026-10-07: yes (queued in Next).
      Checked: they only matter once pushed to GitHub; the local tests are in "Next, in order".
- [x] Mute: build "Pause for…"? Decided 2026-10-07: the user wants no "Mute for…" at all, only "Pause for…" (queued in Next). [t-tbcoe5mj]
      Checked: listed as "maybe later".
- [x] Mute: has a queue replaced by Sources… unmuted you in use? Closed 2026-10-07: moot, "Mute for…" is being replaced by "Pause for…". [t-qpxqgpgu]
      Checked: only observable in your use; the code path passes through a stop, which unmutes.
- [x] Does the modal DIM backdrop look right in Ghostty? Answered 2026-10-07: yes, it looks right. [t-7x4nu2lj]
      Checked: the DIM cells were verified through herdr; the strength in Ghostty needs your eyes.
- [x] Release rormpc-tools (push main and tag v0.2.31, bump `RORMPC_TOOLS_TAG`, run `companions`)? Decided 2026-10-07: yes; done 2026-10-07 (rormpc-tools v0.2.31 pushed and installed with `companions`, rormpc fa344da installed, dotfiles 7e15624 adds `V` and the `≋` column; Hammerspoon picks up `shuffle prev` on its next restart). [t-rwa3xbsg]
      Checked: main is 6 commits ahead of origin (the plan view's mpd-player swaps, the soundtrack genre, `shuffle
      prev`); the installed tools are v0.2.30, so none of them works in the installed rormpc until a release.
- [x] Hits playing-song indicator: build variant A, B (my pick), C or D (B with ⏸ when paused)? Decided 2026-10-07: B (queued in Next). [t-7xxrkifk]
      Checked: mockups and MiMo's ranking in "Soon: playing-song indicator in Hits"; Sol was at its usage limit.
- [x] Install rormpc from master and add `"V": FindVersions` and the `≋` Versions() column to your config and theme? Decided 2026-10-07: yes; done together with the release above. [t-k4jbx4mj]
      Checked: the installed binary does not know either name, so adding them now would break loading the
      config; the exact lines are in "Versions: delete a bad version…".
- [x] Release rormpc-tools v0.2.32 with the Polish lyrics translations and install rormpc from master? Decided 2026-10-07: yes, after the Up next item (queued in Next). [t-qqjebrcv]
      Checked: the Lyrics pane runs `musicdb lyrics translate`, which only exists in rormpc-tools after v0.2.31.
- [x] Up next after an MPD restart: keep re-found entries waiting (a rare request plays twice), trust restored [t-ftx32cmj]
      priorities when the queue's files look unchanged (can drop a request after an MPD crash), or match the
      scrobbler's listens.jsonl / skips.jsonl? Decided 2026-10-07: keep them waiting (a request may play twice, none is lost).
      Checked: the worker's analysis (MPD's state file restores priorities after a clean restart, but a replaced
      queue also starts at 0); it recommends keeping them waiting, which is what the code does now. Recorded
      earlier as the user's choice by mistake: that text was a Claude Code input suggestion in the worker's pane.

- [x] CI: dispatch `installer_smoke.yml` on rofrol/rormpc once now and, when it passes, run it on push to master [t-vcdynvn3]
      and weekly like the other workflows? Decided by the user 2026-10-09: yes (queued in Next).
      Options: run it once, then add push + weekly triggers | run it once, keep it manual | leave it
      Checked 2026-10-09: `gh run list -R rofrol/rormpc --workflow installer_smoke.yml` shows no run yet; ci.yml
      runs on push; rormpc-tools and ro-listenbrainz-mpd test.yml already have push, tags and a weekly schedule.
- [x] CI on macOS: build the throwaway `launchctl bootstrap gui/$UID` probe on a hosted runner, then a launchd [t-t6o7suqf]
      smoke test ("Tests and GitHub Actions…")? Decided by the user 2026-10-09: leave it.
      Options: leave it (the macOS install is exercised on this Mac at every release) | build the probe first
      Checked 2026-10-09: nothing for macOS exists in installer_smoke.yml; it is the last open CI plan point.
- [x] Live playlists: test a real download with a public YouTube playlist you choose (the one live listing so far [t-r6brdw3r]
      returned 0 items)? Decided by the user 2026-10-09: leave it until it is used.
      Options: you give a playlist URL and a worker tests it on a scratch MPD | leave it until you use it
      Checked 2026-10-09: the first version is released (v0.2.34) and checked only with hand-made items.
- [x] Combined view: What is a saved filter set called in the UI? Decided by the user 2026-10-09: Smart list. [t-oyyvfsl2]
      Options: Smart list | Live list (your word) | Preset
      Checked: plans/combined-view.md "Open choices" (1); the first option is the plan's recommendation.
- [x] Combined view: Does a filter change wait for Apply, or replace the queue at once? Decided by the user 2026-10-09: preview, Apply plays. [t-lz5gab5a]
      Options: Preview, Apply plays | Replace at once when weighted is on | Replace at once always
      Checked: plans/combined-view.md "Open choices" (2); the first option is the plan's recommendation.
- [x] Combined view: When does Apply ask for a confirmation? Decided by the user 2026-10-09: only when the source kind changes or more than 25% of the queue goes. [t-uvxh36f4]
      Options: Only when the source kind changes or more than 25% of the queue goes | Always | Never
      Checked: plans/combined-view.md "Open choices" (3); the first option is the plan's recommendation.
- [x] Combined view: What happens to the old Queue, Hits and Shuffle tabs? Decided by the user 2026-10-09: Play replaces them in the default config, the panes stay. [t-yujuc6nl]
      Options: Play replaces them in the default config, the panes stay | Keep all four tabs | Remove the old panes
      Checked: plans/combined-view.md "Open choices" (4); the first option is the plan's recommendation.
- [x] Combined view: Where is the filter column in normal mode? Decided by the user 2026-10-09: collapsed to one source line, h opens it. [t-k66v6n6t]
      Options: Collapsed to one source line, h opens it | Always open | Hidden
      Checked: plans/combined-view.md "Open choices" (5); the first option is the plan's recommendation.
- [x] Combined view: Does a pin beat a `-` set or `-` genre? Decided by the user 2026-10-09: yes, an exception beats every rule (an exclusion still beats a pin). [t-we7smoub]
      Options: Yes, an exception beats every rule | No, a `-` rule beats pins (Sol) | Only a library-scope pin beats it
      Checked: plans/combined-view.md "Open choices" (6); the first option is the plan's recommendation.
- [x] Combined view: What do a hand removal and a hand addition in the queue do by default? Decided by the user 2026-10-09: nothing lasting, pin/exclude only on + / -. [t-aixzd3hn]
      Options: Nothing lasting, pin/exclude only on + / - | Ask each time | Always record an exception in the open smart list
      Checked: plans/combined-view.md "Open choices" (7); the first option is the plan's recommendation.
- [x] Combined view: What is the default scope of a new pin or exclusion? Decided by the user 2026-10-09: the open smart list, else library. [t-fcfzrzhs]
      Options: The open smart list, else library | Always library | The first + set
      Checked: plans/combined-view.md "Open choices" (8); the first option is the plan's recommendation.
- [x] Combined view: Are smart lists exported as MPD playlists "Smart NAME"? Decided by the user 2026-10-09: yes, per list, on by default. [t-kp7fyuch]
      Options: Yes, per list, on by default | Only when asked | Never
      Checked: plans/combined-view.md "Open choices" (9); the first option is the plan's recommendation.
- [x] Combined view: Which sets get a fixed chip row? Decided by the user 2026-10-09: Billboard, my likes, my playlists, recommended; the rest via "+ set…". [t-763grm5f]
      Options: Billboard, my likes, my playlists, recommended; the rest via "+ set…" | Every tag and playlist as a row | Only Billboard
      Checked: plans/combined-view.md "Open choices" (10); the first option is the plan's recommendation.
- [x] Combined view: Which years does Period filter by default? Decided by the user 2026-10-09: follow Rank by. [t-lzrwsqqz]
      Options: Follow Rank by (chart, listened or release year) | Always release year | Always ask
      Checked: plans/combined-view.md "Open choices" (11); the first option is the plan's recommendation.
- [x] Combined view: Is there a "freeze as static list" action? Decided by the user 2026-10-09: no, the MPD export is the static copy. [t-og7nah72]
      Options: No, the MPD export is the static copy | Yes, into a tag list | Yes, into an MPD playlist
      Checked: plans/combined-view.md "Open choices" (12); the first option is the plan's recommendation.
- [x] Combined view: build it now, in the plan's five phases (plans/combined-view.md "Build order"), one worker per [t-byft6qz6]
      phase with a release after each?
      Options: yes, all five phases in order | only phase 1 (± set chips, Rank by, Years of in Hits) first | not now
      Checked: all 12 open choices are decided (2026-10-09); every phase ships on its own.
      Decided by the user 2026-10-09: yes, all five phases in order (queued in Next).
- [x] Installer smoke test: make "Units are enabled and active" prove the programs run (a Type=simple unit is [t-5x7wdmxk]
      "active" even when its binary is missing; the first VM run passed it without rormpc-tools)?
      Options: yes, check each unit's MainPID/exec and a log line | leave it
      Checked: reported by the smoke-fix worker 2026-10-09; the later steps (listen, status) catch some of it.
      Decided by the user 2026-10-09: yes, check each unit's MainPID/exec and a log line (queued in Next).
- [x] rormpc-tools: `musicdb update` ignores the scrobbler config's `api_url` and calls the real ListenBrainz (the [t-foii52mn]
      smoke test's fake token gets "Token invalid", exit 1); make it use the same API URL?
      Options: yes, read api_url like the scrobbler | leave it, the test expects exit 1
      Checked: reported by the smoke-fix worker 2026-10-09.
      Decided by the user 2026-10-09: yes, read api_url like the scrobbler (queued in Next).
- [x] Missing ffmpeg: a readable error in rormpc-tools instead of a traceback, and ffmpeg in the installer's [t-yw2piruo]
      `status` / dependency hint?
      Options: yes, both | only the installer hint | leave it
      Checked: reported by the smoke-fix worker 2026-10-09 (FileNotFoundError: 'ffmpeg' in a fresh VM).
      Decided by the user 2026-10-09: yes, both (queued in Next).
- [x] Combined view: which browsing tabs stay once Play exists (asked 2026-10-09: "with Play, are Artists, Albums, [t-3ij6nebz]
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
- [x] Release rormpc-tools v0.2.38 and install rormpc with phase 1 (± set chips, Rank by, Years of) now, or after [t-ph5bjtp3]
      more phases?
      Options: now (the installed Hits keeps working only with matching tools) | after phase 2 | after phase 3
      Checked: rormpc dce9c58 needs hits ≥ the new tools; the installed 0.2.37 rejects --set/--rank/--years-of.
      Decided by the user 2026-10-10: after phase 2 (queued in Next right after phase 2).
- [x] Hits "my plays" rank: with Years of release it counts every play, with Years of listened it leaves out the [t-vn2wpfkg]
      weighted shuffle's own picks (as the old sources did); make both the same?
      Options: both leave out the shuffle's own picks | both count every play | keep it as it is
      Checked: reported by the phase 1 worker 2026-10-10 (hits_rules.score).
      Decided by the user 2026-10-10: both leave out the shuffle's own picks ("probably"; queued in Next).
- [x] Add `"+": PinSong` and `"-": ExcludeSong` to the queue keybinds in your dotfiles rormpc config? Closed 2026-10-10 by the coordinator: the question was wrong; your config does not set `clear`, so the built-in keys (+, - since b9de184) merge with yours and already work. Your config [t-xruvl3bd]
      has its own queue keymap, so the new keys do nothing there until added; the Queue menu's "Pin in Hits
      results…" / "Exclude from Hits results…" work without them.
      Options: yes, the coordinator adds both | other keys (name them) | no, the menu is enough
      Checked: assets/example_config.ron binds them (b9de184); ~/.config/rormpc/config.ron line ~140 has an explicit
      queue map without them; neither key is bound there.
- [x] Combined view: Where do Browse and Live go inside Play? Decided by the user 2026-10-10: Browse in the left column (Filters | Browse), the playlist editor and Live as overlays (Sol's layout, not the plan's recommendation). [t-wyqtzdky]
      Options: full-width bodies Queue | Browse | Live, B and the digits switch (reuses the browsers unchanged) | Browse in the left column, editor and Live as overlays (Sol) | right-pane modes Sources | Queue | Live (MiMo)
      Checked: plans/combined-view.md "Open choices" (13); the first option is the plan's recommendation.
- [x] Combined view: Does `P` (play an album, folder, artist or playlist, replacing the queue) ask for a confirmation? Decided by the user 2026-10-10: Apply's rule (source kind change or more than 25% of the queue). [t-lc7xn3ho]
      Options: Apply's rule: only on a source kind change or more than 25% of the queue | never, Previous sources undoes it | always
      Checked: plans/combined-view.md "Open choices" (14); the first option is the plan's recommendation.
- [x] Combined view: A song appended (`a`) to a queue holding a Hits source while weighted is on: what happens to it? Decided by the user 2026-10-10: it joins the source's files and the round, the source shows "+N added". [t-j6my7vij]
      Options: it joins the source's files and the round, the source shows "+N added" | it becomes an Up next request | it stays outside the round (today: never drawn)
      Checked: plans/combined-view.md "Open choices" (15); the first option is the plan's recommendation.
- [x] Combined view: Stored playlist editor: immediate edits or a save buffer? Decided by the user 2026-10-10: immediate MPD edits as today, a confirmation before deleting a whole playlist. [t-t6uiz7e6]
      Options: immediate MPD edits as today, a confirmation before deleting a whole playlist | edit a copy, explicit Save and Discard
      Checked: plans/combined-view.md "Open choices" (16); the first option is the plan's recommendation.
- [x] Combined view: Keys for Browse? Decided by the user 2026-10-10: the worker designs them for the left-column layout after the plan's pattern (P play, t play next, a append), checked against your config. [t-twt35smc]
      Options: B body, [ ] grouping, P play replacing the queue, t play next, digits 5..9 groupings and 0/gl Live | the same without the digits | other keys
      Checked: plans/combined-view.md "Open choices" (17); the first option is the plan's recommendation.
- [x] Combined view: Change your own ~/.config/rormpc/config.ron (explicit tabs and digits) when phase 3b ships? Decided by the user 2026-10-10: yes, the coordinator mirrors the new default (Play, Up next, Search plus Versions, Deleted, Lyrics). [t-vca5bprt]
      Options: yes, the coordinator mirrors the new default (Play, Up next, Search plus Versions, Deleted, Lyrics; digits to ShowPlay) | keep your tabs | remove only the six tabs, keep your digits
      Checked: plans/combined-view.md "Open choices" (18); the first option is the plan's recommendation.
- [x] Combined view: An unapplied preview when a Browse action changes the queue? Decided by the user 2026-10-10: the preview stays, its counts are recomputed, Apply judges the confirmation then. [t-qq7djvph]
      Options: the preview stays, its counts are recomputed, Apply judges the confirmation then (Sol) | the preview is dropped with a note (MiMo)
      Checked: plans/combined-view.md "Open choices" (19); the first option is the plan's recommendation.
- [x] Combined view: Artists and Album artists in Browse? Decided by the user 2026-10-10: two separate groupings. [t-s6iei64h]
      Options: two separate groupings (MiMo; the code has both) | one grouping with a toggle
      Checked: plans/combined-view.md "Open choices" (20); the first option is the plan's recommendation.
- [x] Scrobble status: What does "62% of 50%" mean? Answered 2026-10-10: "62% listened. required 90%. ask the models" (consult, then decide). Consulted (Sol 8ca9ba0f, MiMo 2bf1c6a3): "listened" is not honest under the uninterrupted rule. Decided by the user 2026-10-10: "62% counted · need 90% · in 1:24" (queued in Next). [t-2du6g3na]
      Options: progress toward the required share (62% of the way to the threshold, as built) | the share of the song heard ("31% of 50%")
      Checked: reported by the scrobble status worker 2026-10-10; the first option is what was built or recommended.
- [x] Scrobble status: Which key sends to ListenBrainz now? Decided by the user 2026-10-10: oL plus an item in the Queue menu for the playing song (queued in Next). [t-ec262gyd]
      Options: oL (as built) | oL plus an item in the Queue menu for the playing song | another key
      Checked: reported by the scrobble status worker 2026-10-10; the first option is what was built or recommended.
- [x] Scrobble status: When does a manual send ask for a confirmation? Decided by the user 2026-10-10: when the rule is not met yet (as built). [t-4ig3kaur]
      Options: when the rule is not met yet (as built) | always | never
      Checked: reported by the scrobble status worker 2026-10-10; the first option is what was built or recommended.
- [x] Scrobble status: Which timestamp does a manual send carry after a seek under the uninterrupted rule? Decided by the user 2026-10-10: the seek (as built). [t-vlzn7sp7]
      Options: the seek (as built, the same as an automatic listen) | the song's start
      Checked: reported by the scrobble status worker 2026-10-10; the first option is what was built or recommended.
- [x] [t-tmx6yi4w] Scrobble status: Should the scrobbler remember a send across its own restart in the middle of a song (today a later automatic listen could go a second time)?
      Options: leave it (rare) | store it in the daemon's SQLite database
      Decided by the user 2026-10-10: store it in the daemon's SQLite database (queued in Next).
      Checked: reported by the scrobble status worker 2026-10-10; the first option is what was built or recommended.
- [x] Scrobble status: Add Status(Scrobble) to your theme in dotfiles? Decided by the user 2026-10-10: next to the progress bar, before the time (queued in Next; the coordinator edits the theme). [t-hobwuxhg]
      Options: next to the progress bar, before the time | in the header | no
      Checked: reported by the scrobble status worker 2026-10-10; the first option is what was built or recommended.
- [x] Release now: rormpc-tools v0.2.39 (smart lists, named sets, api_url, missing-program errors, my plays rank, [t-j7pbp2wc]
      Play's round key), a ro-listenbrainz-mpd tag (scrobble status), bump both tags in the installer, run
      `companions`, install rormpc (Play, smart lists, scrobble status)?
      Options: yes, now | after phase 3b | not yet
      Checked 2026-10-10: all of it is in the mains with tests green; nothing of it is installed (installed: tools
      0.2.38, rormpc 743c541). Play shows only with a Pane(Play()) tab in your config (choice 18).
      Decided by the user 2026-10-10: yes, now; and add Play to the config as the first tab, the other tabs stay for
      now, digits shifted by one (the coordinator does both).
      Done 2026-10-10: rormpc-tools v0.2.39 and ro-listenbrainz-mpd v2.6.0-ro.5 tagged and pushed (tests green),
      installer c91c116 pins both, companions installed, rormpc c91c116 installed and pushed; dotfiles b6720d3 adds
      the Play tab first (1 Play, 2 Hits, 3 Queue, 4 Up next, 5 Shuffle, 6-9 browsers, 0 Playlists; Live playlists
      by gl); the config loads. Restart running rormpc instances.
- [x] Install hints: how does a missing program's message name the install? Decided by the user 2026-10-10: always a generic line + a docs link (Sol). [t-ebrhnj6i]
      Options: detect the OS and print the one matching command (brew, apt, pacman, guix), generic line + docs link
      otherwise (MiMo) | always a generic line + a docs link to the per-manager table (Sol) | keep brew + apt
      Checked: consult round 2026-10-10 (see the "Install hints" item in Next); you run macOS, Omarchy (Arch) and
      Guix.
- [x] MPD (asked 2026-10-10: "do I have the impression that this MPD daemon, to which files have to be sent again [t-kcdinapa]
      and again, is very limiting? ask the models to analyze, not change"): plan a change, or keep things as they are?
      Options: keep MPD as is; only fix the cheap self-inflicted parts later (e.g. the round draws from queue ∩
      source, so hand-appended songs count) | plan option (a): mpd-player owns what plays next, MPD keeps only the
      current song + 1-2 (first test phone clients on a tiny queue) | look at Mopidy or an own libmpv player |
      propose small patches to upstream MPD (idle on client disconnect, plan/edit versions), fork only if refused
      Checked: consult round 2026-10-10 (Sol d359f945, MiMo bcad472d), analysis only. Both: partly right; sending
      files is not a cost (MPD gets file references, 8,300 songs is small), the friction is the API shape. Inherent
      to MPD: the queue is the only playback state, priorities are the only scheduling hook (they work only with
      random on) and share the one queue version, random order cannot be walked back, no disconnect event.
      Self-inflicted: whole-source queues, the plan published as priorities (hence the useless queue version),
      rounds read a file list instead of the queue (hence appended songs never drawn). MPD gives what is costly to
      lose: phone clients, gapless, outputs, database, stickers, idle events, headless. Verdict of both: keep MPD;
      Sol: option (a) only if the friction justifies it, after deciding who owns phone edits; MiMo: (a) plus a
      read-only "Up next" web page for phones; Mopidy's MPD frontend lacks priorities and stickers (a poor trade);
      libmpv only if MPD itself blocks a feature.
      Asked 2026-10-10: "why not fork MPD so it works great with our architecture while clients stay compatible? ask
      the models". Consulted (Sol 1ea0e3a1, MiMo 7d561134): both say don't fork now. Opt-in extensions keep phone
      clients working (unknown commands and status keys are ignored), but the big win, an external "next song"
      provider with the queue as a window, puts a foreign process in the playback path (audio stalls if it hangs)
      and breaks what phone clients assume the queue is (reorder/delete of planned songs); small parts (separate
      plan/edit versions ~30 lines, an idle event on disconnect ~20 lines) help only mpd-player; each fork costs a
      rebase per MPD release and own packages for Homebrew, Arch and Guix. Upstream chances: an idle event on client
      disconnect 20-30% (MiMo), conditional priority updates the strongest general case (Sol), an external sequencer
      ~0. Flip to a fork only if upstream refuses and a no-fork prototype still fails a real phone workflow.
      (MiMo also suggested polling `listclients`: dismissed, MPD has no such command.) Fourth option added below.
      Decided by the user 2026-10-10: keep MPD, only cheap fixes (the main one, appended songs joining the round,
      is done in phase 3b); no fork, no short-queue rebuild.
- [x] Deleted songs: should an undo of a deletion also leave a "restored" event in the log, for history (today the [t-dctwvze7]
      journal record just disappears with its block)?
      Options: no, as it is | yes, append "restored" to deletions/allowed.jsonl
      Checked: MiMo's point in the deleted-songs consult round 2026-10-10; the worker left it as it is.
      Asked 2026-10-10; the user did not understand it ("I delete something and you write restored?"): it is about
      undoing a deletion (Ctrl-y); explained in plain text, recommended to leave it as it is.
      Decided by the user 2026-10-10: leave it as it is.
- [x] Deleted songs: should deleting only a duplicate (another file of the recording stays) block the whole [t-z2lxe7ho]
      recording too?
      Options: only that file's YouTube video (as built) | the whole recording
      Checked: the deleted-songs worker 2026-10-10.
      Decided by the user 2026-10-10: only that file's YouTube video (as built).
- [x] Release now: rormpc-tools v0.2.41 (deleted songs never downloaded again) and install rormpc (Deleted tab [t-vqzg2efo]
      download column, Hits ⌫ rows, the scope-menu cursor)?
      Options: yes, now (the block only works once installed) | together with the weighted-mode change
      Checked: tests green; installed are tools 0.2.40 and rormpc 66ac1f7.
      Decided by the user 2026-10-10: after the weighted-mode change, one release (queued in Next).
- [x] [t-s6vvhvta] Forecast across rounds: when a round ends, start the next one automatically, or keep starting rounds by hand?
      Options: automatic: the next round starts when one ends, the forecast shows its songs marked "next round"
      (Sol) | by hand as now, the forecast shows the next round's picks dimmed as provisional (MiMo)
      Checked: consult round 2026-10-10 (see "The forecast should always show 10 ahead" in Next).
      Decided by the user 2026-10-10: automatic, the next round starts when one ends (Sol).
- [x] [t-47mq65ez] Media key lag: which fix?
      Options: plan it whole: an mpd-player command socket, Karabiner send_user_command, and mpd-player as the Now
      Playing / MPRIS provider installed by rormpc_install.sh (replacing mpd-now-playable) | only the socket +
      Karabiner for this Mac | only 127.0.0.1 in ~/scripts/mpd-media-key now (a stopgap)
      Decided by the user 2026-10-10: plan it whole (queued in Next).
      Checked: the media key worker's measurements 2026-10-10 (see "Media key Next lags" in Next); a live test with
      F9 and `log stream` is yours after the fix.
      Answered 2026-10-10: "what is going on here? and what if someone installs rormpc and has no Karabiner? ask the
      models". Consulted (Sol 81f389a3, MiMo c846b96d): rormpc's installer sets up no media keys at all; the
      documented macOS default (mpd-now-playable) and a Linux MPRIS bridge must be installed by hand, and both send
      plain MPD next/previous, which skips the weighted shuffle's plan and history; and the OS hands the keys to the
      current Now Playing / MPRIS player, so a browser can take them either way (why this setup uses Karabiner).
      Both recommend: mpd-player gets a command socket as the one control interface (next through the plan,
      previous through its history), Karabiner calls it with send_user_command (no shell, no mpc, no DNS lookup);
      for users without Karabiner, mpd-player registers as the Now Playing provider (macOS) / MPRIS player (Linux)
      using the same commands, installed by rormpc_install.sh; 127.0.0.1 in the script is only a stopgap.
      (MiMo's "prev sends both mpc prev and shuffle prev" was dismissed: the script sends one of them.)
- [x] [t-didszvcn] Restore: a re-downloaded file whose audio MD5 differs from the deleted one (a new encode of the same video)?
      Options: restore it and note the difference (as built) | keep it in staging for a decision | restore only identical audio
      Checked: the restore worker 2026-10-10.
      Decided by the user 2026-10-10: restore it and note the difference (as built).
- [x] [t-tacsoont] Restore: when the tagger does not recognise the re-download, write MBID, artist and title from the journal?
      Options: yes, from the journal (as built; the same video was recognised before) | keep it in staging for a decision
      Checked: the restore worker 2026-10-10.
      Decided by the user 2026-10-10: yes, from the journal (as built).
- [x] Radio: Should Omarchy Radio be checked on a schedule? Decided by the user 2026-10-10: a daily conditional GET that only adds pending items and notifies (queued in Next). [t-etpilz4j]
      Options: manual check only, as today (Sol) | a daily conditional GET that only adds pending items and notifies (MiMo)
      Checked: the radio worker 2026-10-10: the site changes rarely (last push 2026-10-07); an unchanged check costs one 304.
- [x] Radio: Which MPD playlist name for the station? Answered 2026-10-10: "omarchy.radio.org" (the site is radio.omarchy.org: asked which one is meant). Decided by the user 2026-10-10: "radio.omarchy.org" (queued in Next). [t-35gfgmhs]
      Options: "Omarchy Radio" (from playlist.json, the default) | "Live Radio Omarchy"
      Checked: the radio worker 2026-10-10: YouTube subscriptions are named after the playlist title; --name overrides it.
- [x] Radio: Fetch a radio track's lyrics (/tracks/lyrics/<file>.lrc) into lyrics_dir? Answered 2026-10-10: "extract them with whisper or something? see ~/scripts/yt-wh" (queued in Next). [t-iwlmeuuo]
      Options: later, once a track has lyrics | now, one extra request per track
      Checked: the radio worker 2026-10-10: no track in today's playlist.json has a lyrics field.
- [x] Radio: Detect an MP3 replaced under the same file name on the station? Decided by the user 2026-10-10: no, accept the limitation. [t-jpcjeixe]
      Options: no, accept the limitation (rare) | a HEAD per accepted track on check, comparing its ETag
      Checked: the radio worker 2026-10-10: the playlist.json ETag does not change when only an MP3 is replaced.
- [x] Release years: Which year do the views show? Decided by the user 2026-10-10: TDOR (original release), TDRC kept per recording. [t-zi36ueif]
      Options: TDOR (original release), TDRC kept per recording | TDRC = TDOR = the original release, as today | TDOR, and TDRC only for versions
      Checked: rormpc-tools docs/release-years-plan.md (2026-10-10): every view reads TDRC today; MPD exposes TDOR as `originaldate`.
- [x] Release years: May high-confidence rows be applied without row-by-row review? Decided by the user 2026-10-10: no, review every row. [t-gipr7rh2]
      Options: yes, earlier-only same-length rows with a matching release-group year | no, review every row | yes, every earlier proposal
      Checked: rormpc-tools docs/release-years-plan.md (2026-10-10): 12 of 15 high rows right in the sample, the other 3 off by one year; earlier-only proposals 25 of 28 right or closer.
- [x] Release years: Where is the review done? Decided by the user 2026-10-10: a rormpc "Years to review" view. [t-g3mcydg4]
      Options: the Markdown report with --accept/--reject ids | a rormpc "Years to review" view | both, the view later
      Checked: rormpc-tools docs/release-years-plan.md (2026-10-10): an earlier review of undated files used the report-and-ids flow.
- [x] Release years: Should a video match with no audio alternative be swapped to the audio recording through the work on download? Decided by the user 2026-10-10: yes, look the work up and swap to the same-length audio recording. [t-5y3q7ir4]
      Options: yes, look the work up and swap to the same-length audio recording | send it to review instead | keep the video recording and fix only the year
      Checked: rormpc-tools docs/release-years-plan.md (2026-10-10): at least 24 of 60 earlier video matches had an audio alternative; the rest need a work lookup.
- [x] Release years: What happens to files without any MusicBrainz work link? Decided by the user 2026-10-10: ask for an MBID through the picker. [t-ttkklnsa]
      Options: keep the current year, list only suspect classes | clear the year when it is a video recording's | ask for an MBID through the picker
      Checked: rormpc-tools docs/release-years-plan.md (2026-10-10): 46 of 189 sampled recordings have no work relationship.
- [x] Forecast on a source smaller than 10 songs: show the rest of this round and the next one with the reason (as Decided by the user 2026-10-10: as built. [t-5sxhjovy]
      built), or add a display-only preview of later rounds?
      Options: as built (one MPD priority per queue entry; a played song rests ≥ 12 h) | a display-only preview of
      later rounds without MPD priorities | duplicate entries in the queue (changes your queue)
      Checked: the forecast worker 2026-10-10.
- [x] Media keys: Now Playing on macOS: a second process, an in-daemon provider, or a Swift helper? Decided by the user 2026-10-10: a second process with its own launchd agent. [t-vgmdwrnc]
      Options: a second process from the same package with its own launchd agent (both models) | inside the mpd-player daemon on a CoreFoundation loop | a small Swift helper
      Checked: rormpc-tools docs/media-keys-plan.md (2026-10-10).
- [x] Media keys: When should the provider claim Now Playing? Decided by the user 2026-10-10: on the first MPD play after it starts, then the true state. [t-vhlbg34j]
      Options: on the first MPD play after it starts, then the true state (paused stays paused) | at startup as Playing like mpd-now-playable (takes the slot from a browser) | only while MPD plays
      Checked: rormpc-tools docs/media-keys-plan.md (2026-10-10).
- [x] Media keys: May a worker stop your mpd-now-playable for about 15 minutes for a Control Center test on a scratch MPD? Decided by the user 2026-10-10: no, the user runs that test after the install. [t-elg66ftq]
      Options: no, you run that test after the install | yes, the worker stops it and starts it again
      Checked: rormpc-tools docs/media-keys-plan.md (2026-10-10).
- [x] Media keys: Next (key, Now Playing, MPRIS) while you have paused MPD: play the next song, or stay paused on it? Decided by the user 2026-10-10: play it. [t-wzebwbr5]
      Options: play it, as Previous already does | stay paused, as MPD's own next does
      Checked: rormpc-tools docs/media-keys-plan.md (2026-10-10).
- [x] Media keys: Keep the old Karabiner shell script after the rule moves to send_user_command? Decided by the user 2026-10-10: delete it with the switch. [t-hdsvqnfy]
      Options: keep it as a manual fallback for a week, then delete | delete it with the switch
      Checked: rormpc-tools docs/media-keys-plan.md (2026-10-10).
- [x] Media keys: Linux MPRIS: on by default whenever a session bus is present, or opt-in? Decided by the user 2026-10-10: on by default, warn when another MPD MPRIS bridge runs. [t-bqrzg3rl]
      Options: on by default, warn when another MPD MPRIS bridge runs | opt-in with `mpd-player --mpris`
      Checked: rormpc-tools docs/media-keys-plan.md (2026-10-10).
- [x] Radio rename: the worker's `uv run liveplaylist rename omarchy-radio radio.omarchy.org` on your library was [t-l2luj3wm]
      Decided by the user 2026-10-11: the coordinator runs the rename (d-ku6vswnr).
      Done by the coordinator 2026-10-11: `liveplaylist rename omarchy-radio radio.omarchy.org`; MPD lists
      "radio.omarchy.org", no "Omarchy Radio" left.
- [x] [t-dipotnvv] Instant filtering: What does removing a source song from the queue in Music do, now that the next click would bring it back?
      Asked 2026-10-11; the user did not know what removing a song from the source means and said "ultimately there
      is to be only one folder"; asked again in plain words, the user answered: "but there will be no sources any
      more. There is to be one folder. So what is the source then? YouTube playlists or radio.omarchy.org? Ask the
      models." Round 20261011-113547-9d23 (Sol 53901ace useful, MiMo 9e5e17b1 partial), checked against
      plans/instant-filtering.md: both drop the word "source" from the UI (one folder says where files live, not
      what plays; the UI shows the current selection: the chips, or "Playing: <playlist> · list order" /
      "Playing: <album> · track order"); a YouTube playlist or radio.omarchy.org is then either played as itself
      (its order) or included as a + chip in the filters (Sol: two separate actions). Hand deletion (Sol, accepted):
      left out of this listening session by song ID, kept across filter clicks until a new Play of an album or
      playlist; "Omitted: N" shown next to the summary, opening it restores one or all; deletions share the z / Z
      history with filter clicks. Dismissed: MiMo's "cleared on any chip edit" (brings the song back on the next
      click, the problem asked about) and "the playing song is yanked" (the plan already keeps it, marked
      off-source). Already in the plan: rounds survive filter clicks (source_id).
      Options: Left out until the next Play of an album or playlist, "Omitted: N" with restore, z undoes it | Lasting exclusion, the same as - | It comes back on the next filter click
      Asked 2026-10-11 again; the user answered: "if permanent, there would be two lists: deleted and banned? Banned
      still on disk and in ListenBrainz? Ask the models." Round 20261011-115011-5f47 (Sol a0bd9992, MiMo 4432b4d8,
      both useful) agree: no separate "banned" list. Three verbs: thumbs-down = less (weight 0.1, stays), the planned
      whole-library exclusion ⊘ = never (file stays on disk, ListenBrainz listens stay, no automatic LB "hate",
      listed under "show excluded (N)", undone with z), delete = gone (Deleted panel, Ctrl-y). A hand removal from
      the queue is a session-only omission, never a lasting decision. They differ on its lifetime: Sol until rormpc
      closes or an album/playlist is played (announced "Temporary omissions cleared"); MiMo until rormpc closes or the
      user restores it, never cleared by playing an album. Consider (Sol): move hits-hidden.jsonl into
      Billboard-scoped exclusions; an excluded song played on purpose plays once. Consider (MiMo): name them
      Less · Never · Gone in the UI.
      Decided by the user 2026-10-11, against both models: a hand removal from the queue is lasting (d-6xjfvyp3),
      and lasting removals go to a separate Banned list, like Deleted but the file stays on disk (d-x3g5wgkv).
      Banned songs never come back through filters or the shuffle; the Banned list restores them; z undoes a ban.
- [x] [t-u7mcb2xp] Banned: what happens in ListenBrainz when a song is banned?
      Decided by the user 2026-10-11: nothing; listens stay, no feedback (d-a7xehj4c).
- [x] [t-lpowr4tg] Instant filtering: Does the first filter touch replace another kind of source (an album played with P, a playlist, the whole library)?
      Decided by the user 2026-10-11: at once; the old source is the first undo entry (d-s6fcqo5u).
- [x] [t-ofvymmok] Instant filtering: In weighted mode, what does the table show while filtering?
      Decided by the user 2026-10-11: the forecast, with the counts in the header and a rows ⇄ plan toggle for the
      full result (d-eaccjsts).
- [x] [t-cqi53tl3] Instant filtering: Which keys undo and redo a filter change?
      Decided by the user 2026-10-11: z undo, Shift+Z redo, as in the ki editor (d-7x5qkzms).
- [x] [t-4eyutus4] Instant filtering: `z` is the user's global ToggleRepeat; where do z / Z undo and redo filter changes?
      Decided by the user 2026-10-11: z / Z everywhere, ToggleRepeat moves to r ("repeat na r?", d-3wvtn5zn). Open:
      r is Rate() in the navigation keys of the user's config; see the question below.
- [x] [t-sf7al4v4] Instant filtering: r is Rate() in the lists' navigation keys; with ToggleRepeat on r, where does Rate go?
      Decided by the user 2026-10-11: Rate moves to y, r is ToggleRepeat (d-bcn65ven, d-wsmj4hvj).
      Options: Rate moves to a free key, e.g. y (name it) | Repeat goes to a free key instead and Rate keeps r | r is repeat only outside lists
      Checked: ~/.config/rormpc/config.ron navigation "r": Rate(), "R": AddRandom; rmpc/src/ui/mod.rs handle_action
      gives the pane the key first and claims the global binding only after, so a global r would toggle repeat
      almost nowhere (2026-10-11). Free single keys: B E F H I L M O Q S T W y.
- [x] [t-jusw7yz7] Instant filtering: A filter with 0 owned songs?
      Decided by the user 2026-10-11: the upcoming songs go, it stops after the playing song (Sol; d-sy7ma764).
- [x] [t-blsr4dqn] Instant filtering: When does Previous sources record a rule set, now that every click applies?
      Decided by the user 2026-10-11: when the rule set is left, or once a song of it has played (d-m7t2ldzp).
- [x] [t-iyyq2is3] Instant filtering: Should a round count a song as heard only after a scrobble-qualifying play instead of on start (Sol)?
      Decided by the user 2026-10-11: yes, heard only after a scrobble-qualifying play ("as in scrobble"), in step 1
      (d-yxxlysnq).
- [x] [t-rezfoev3] Instant filtering: Keep `hits` warm beyond the cached library index?
      Decided by the user 2026-10-11: only if the end-to-end time after the cache is over 0.15 s (d-ed7ivr5u).
- [x] [t-ccwpr5yr] Media keys: What should `pause` (key, Now Playing, MPRIS) do while "Pause for…" already holds the pause?
      Decided by the user 2026-10-11: cancel the timer and stay paused (d-h2in7kd6); queued in "Next, in order".
- [x] Release years: About 166 files have no recording MBID at all ("needs MBID", class no-recording); should the [t-kbrwabr2]
      Decided by the user 2026-10-11: list them under a separate filter, off by default (d-bzidm3vs); queued in "Next, in order".
- [x] Release years: When you pick a recording MBID for a file in the picker, should it also rewrite the file's [t-mk3klt4c]
      Decided by the user 2026-10-11: yes, the picked recording also rewrites the file's UFID (d-hkqe4mfl).
- [x] Release years: A video match whose work has same-length audio recordings, none on an official Album, Single [t-ygvgseeh]
      Decided by the user 2026-10-11: swap, as built (d-6bdujk63).
- [x] [t-g3benoav] Transcribed lyrics: Transcribe new Omarchy Radio downloads automatically, or only on request?
      Decided by the user 2026-10-11: automatically after each radio download (d-k5ct3563).
- [x] [t-nrrenexw] Transcribed lyrics: Run the whole library's songs without lyrics (about 274, about 10 hours of CPU)?
      Decided by the user 2026-10-11: yes, once overnight (d-ofgw3a5v).
- [x] [t-pcvo242u] Transcribed lyrics: May a single-song `--current` request skip vocal separation to answer in about 20 s instead of about 2 min?
      Decided by the user 2026-10-11: no, always separate the vocals (d-ihaam37d).
- [x] [t-lpkmaejb] Transcribed lyrics: Show a transcript that passed the checks at once, or only after the user accepts it?
      Decided by the user 2026-10-11: show at once with the machine-transcribed label (d-m3sz7ubx).
- [x] [t-t3l657ag] Transcribed lyrics: Default Whisper model?
      Decided by the user 2026-10-11: large-v3 (d-crma4ykk), not the measured large-v3-turbo.
- [x] [t-gm7vuxkv] Transcribed lyrics: Where do the heavy dependencies live?
      Decided by the user 2026-10-11: a separate uv tool with an inline-dependency script (d-5z76bdt3).
- [x] [t-a66m24n7] Transcribed lyrics: Try one commercial transcription API on the evaluation set as an upper bound (the audio leaves the machine)?
      Decided by the user 2026-10-11: yes, the whole evaluation set (d-j6gatiuc); it needs an API key the user has.

- [ ] rormpc-tools items while herdr todo run cannot serve them (reports r-8, r-9; d-rp6cp5mq): herdr's round of [t-lwdzxibp]
      2026-10-12 (sol, MiMo, DeepSeek; queued in herdr TODO 7e8d6163) plans `todo run --repo <path>` (the item stays
      here, slot/checks/commit/install/push from the target, default branch read from git), release recipes in the
      target's .herdr run by the driver (staged, resumable), and a two-repository change as two chained items (the
      dependency first). Meanwhile it recommends (sol, MiMo; DeepSeek only once): start each blocked rormpc-tools item
      the old way with the allowlist override, one start per item, rormpc-tools' own `uv run pytest -q`, the one commit
      verified before landing on `main`; releases by hand with a checklist; split two-repository features now.
      Should the coordinator do that (the user said "what overrides?" before the round and chose none meanwhile)?
      Options: accept the round's meanwhile plan | wait for herdr's `todo run --repo`
      Checked: herdr-85's message of 2026-10-12 (d-s2mkmlzc).

- [ ] Worker dispatch is blocked: neither subscription has capacity, so "Next, in order" cannot run (herdr's quota [t-jvfoarip]
      gate refused the first item's worker). Wait for the window, override the gate once, or stop until capacity
      returns?
      Options: wait for the window (recommended: herdr's usage poller publishes a measurement and wakes the pending
      start when the reading drops below 80%, no timer of ours) | allow `--ignore-usage` for this one item now (the
      user's explicit permission for this launch: `todo cancel-pending r-6fh3o42n`, then a new run with the flag) |
      stop coordinating and leave the items for later
      Checked: `herdr todo run t-jsjs5ngd` on 2026-10-18 02:47 created run r-6fh3o42n (attempt 1, step preflight,
      status waiting) and the gate refused it: "OpenAI (ChatGPT plan) window five_hour (5h): 100% used (the gate
      closed at a refusal reopens only below 80%)"; the Claude Code subscription reported its monthly limit on
      2026-10-17 21:51 ("your weekly limit resets Oct 22 at 2am"). No run of this repository is in flight; the
      predecessor's 278 runs are all terminal.
      Moved on 2026-10-18: herdr 0.9.3 (roherdr 2785ab6eb) gained `--model`, so a pi worker runs on another
      provider; the same item dispatched with `--agent pi --model deepseek/deepseek-chat` became run r-iigptjsv
      and the gate refused that too: "no new worker while 2 DeepSeek workers are admitted and not exited, at most 2
      ([usage] gate_max_workers), across all repositories" (r-3rdihbgn, agentic-workflow-bend-claude; r-6bvnzguw,
      Robokill). Decided by the user 2026-10-18 ("jak to limit 2 równoległych workerów w całym herdrze? miało być
      na koordynatora"): the cap belongs per coordinator, not across all repositories; no `--ignore-usage`
      override; the herdr coordinator was asked to consult the models and change the gate (message sent 03:0x to
      the agent in pane wR:p2WN of /Users/romanfrolow/personal_projects/herdr, whose repository lease is stale and
      handed over to a note). r-iigptjsv stays waiting at preflight: it starts when a slot frees or after herdr's
      fix, whichever comes first; if herdr says the waiting run cannot start, redispatch the same task and paths
      (its task file is /tmp/task-t-jsjs5ngd-a.md).
      Resumed 2026-10-18 03:47 (the user: "już sol dostępny do konsultacji, wznów"): the DeepSeek cap had cleared
      and the run was blocked at preflight on disk instead ("0.4 GiB free ..., less than 15 GiB"; 28 GiB free by then,
      it had been transient), so run r-iigptjsv was aborted (event 220484, the only action it offered) and the item
      redispatched with `--agent pi` on the ChatGPT plan: run r-alp4b557 started (attempt 1, step start). Wait with
      `herdr-job` + `herdr todo wait r-alp4b557`.

## Waiting on rormpc-tools

Items that consume rormpc-tools work and wait on its coordinator's "landed <tag>" (cross-repository rule in AGENTS.md "Coordination", 2026-10-12). They move back to "Next, in order" when it replies.

- [ ] Lyrics for songs without any, transcribed (asked 2026-10-10 about the radio tracks: "extract them with whisper or [t-dmpfe6gj]
      something? see ~/scripts/yt-wh"): a worker looks at ~/scripts/yt-wh (yt-dlp + wh-speakers-diarize), consults
      the models on transcribing sung lyrics (whisper models and their accuracy on music, vocal separation first,
      timestamps into .lrc, language detection, cost per song), and plans or builds "Transcribe lyrics" for a song
      with no lyrics (radio tracks first), labelled as machine-transcribed like the machine translations.
      Planned in rormpc-tools b2b2d5e docs/transcribe-lyrics-plan.md (worker, 7 songs measured, Sol + MiMo consulted):
      mlx_whisper large-v3-turbo on a demucs vocal stem (Polish WER 0.05 with the language detected from the first
      vocal entry, English 0.23-0.28; line timing 0.4-1.2 s off, enough to highlight lines), demucs on CPU ~2 min per
      song (~10.5 h for the ~274 songs without lyrics); instrumentals detected by VAD on the stem (whisper writes
      "Thank you." there); stored as <song>.transcript.json, never as .lrc, shown in rormpc as machine-transcribed.
      Seven open choices in "Needs a decision" ("Transcribed lyrics:"); the build waits for them.
      All seven decided by the user 2026-10-11: transcribe each new Omarchy Radio download automatically, and the ~274
      library songs without lyrics once overnight; always separate the vocals; show a transcript that passed the
      checks at once, labelled machine-transcribed; model large-v3 (not the measured turbo); a separate uv tool with an
      inline-dependency script; one commercial API tried on the whole evaluation set as an upper bound (needs the
      user's API key: a worker that has none reports it, never signs up).
      Waiting on rormpc-tools (2026-10-12, d-33kat4ps): the tool is rormpc-tools t-jsjs5ngd; rormpc's display of
      <song>.transcript.json (labelled machine-transcribed) follows its "landed <tag>".

## Waiting on herdr

Items that wait on a herdr change (its coordinator, herdr-85, says when it is installed).
