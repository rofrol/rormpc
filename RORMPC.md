# rormpc

A personal fork of [rmpc](https://github.com/mierak/rmpc). Fork code lives in its own files; upstream files
only get the few lines that register new panes, so rebasing on upstream stays mechanical.

The binary is `rormpc` and its config lives in `~/.config/rormpc/`, so upstream `rmpc` can stay installed
next to it. Install with `scripts/rormpc_install.sh install` (keeps the previous binary;
`rollback` and `list` too). Commits carry "(AI-assisted)" (see AGENTS.md).

## Companion tools

rormpc alone needs only MPD. The Hits pane, the play-count and skip columns and the delete menu run
[rormpc-tools](https://github.com/rofrol/rormpc-tools) (`hits`, `musicdb`), and some services work with rormpc closed:

    scripts/rormpc_install.sh companions [--gap SECONDS]   # install / update / restart all of it (needs uv, cargo)
    scripts/rormpc_install.sh status                       # what is installed and running

- rormpc-tools at a pinned tag (`uv tool install`), and `musicdb update` every hour: play counts, skips and likes
  into MPD stickers. Settings (music directory, where the history goes, ListenBrainz user) in
  `~/.config/rormpc-tools/config.toml`, see its README.
- [ro-listenbrainz-mpd](https://github.com/rofrol/ro-listenbrainz-mpd), my fork of the listenbrainz-mpd scrobbler,
  installed with cargo at a pinned tag. `companions` adds my rule to its config (a listen is 90% of the song played
  without a seek; the `listen_*` lines, change them there) and starts it once the ListenBrainz token is in the config.
  It also records skipped songs for musicdb.
- `mpd-player` (rormpc-tools, replaced `mpd-gap`): the playback daemon. Seconds of silence between songs, chosen in
  rormpc (`og`, ShowGapMenu, or the Queue menu "Silence between songs…") and remembered; `--gap` is only the
  default before that. rormpc sends it commands as MPD messages on channel `rormpc` (`gap set 5`) and reads its
  state from `$XDG_STATE_HOME/rormpc/gap.json`; a change counts once that file shows it.
  "Pause for…" (`om`, ShowPauseMenu, the Queue menu, or a click on the volume slider during a timed pause):
  presets 1-60 min or a custom duration (`5`, `90s`, `1h30`, `1:30`); mpd-player pauses MPD and plays on at a
  wall-clock deadline, also with rormpc closed, after the Mac slept past it or after a restart of mpd-player. The
  slider shows `paused · 12:34` meanwhile; the menu then offers play now, +5/15/30 min, or cancel the timer and stay
  paused. It plays on only if MPD is still paused on the same song: a play, stop, another song or a replaced queue
  by anyone cancels the timer. The gap never presses play during a timed pause. State in `pause.json`. (`op` is
  ShowDecoders upstream, so the pause menu keeps the `o m` slot of the "Mute for…" it replaced; an old config's
  ShowMuteMenu opens it too.)

The companions need ffmpeg (`musicdb update` hashes the audio with it): `companions` stops without it. On Linux,
building the scrobbler needs a C toolchain, `pkg-config`, OpenSSL and SQLite headers. The packages for Homebrew,
Debian/Ubuntu, Arch and Guix are in rormpc-tools' [dependency table](https://github.com/rofrol/rormpc-tools#dependencies);
Windows is not supported. The services are launchd agents or systemd user units only: on Linux `companions` stops
before installing anything without a systemd user session (Guix System's Shepherd is not supported); log in
normally, or on a machine nobody logs into run `sudo loginctl enable-linger $USER` once.

They run as launchd agents `io.github.rofrol.rormpc.*` (logs in `~/Library/Logs/`) on macOS, systemd user units
`rormpc-*.service` (musicdb with a `.timer`) on Linux; the installer writes them, so edits there are overwritten.
The hourly `musicdb update` runs at background priority (launchd `ProcessType Background` + `LowPriorityIO`;
systemd `Nice=10`, `IOSchedulingClass=idle`): about a second of CPU per run. The always-on services stay at
standard priority, since background priority lets macOS stretch timers (mpd-player's silence).
`companions --local` installs both from checkouts instead (`$RORMPC_TOOLS_DIR`, `$RO_LB_DIR`), rormpc-tools editable.
Without rormpc-tools those features say the command was not found and print the `uv tool install` command for
the expected tag (`RORMPC_TOOLS_TAG` in `scripts/rormpc_install.sh`, which rormpc reads at build time); the rest
works. `rormpc debuginfo` lists `musicdb` and `hits` with their `--version` and the expected tag. JSON that rormpc
reads carries a version it checks: `hits --json` files and `musicdb delete --preview` (`{"version": 1, "songs":
[...]}`; the older bare list is still read). Change such a format in both repositories together, with the tests on
both sides (rormpc-tools `tests/test_rormpc_contract.py`, rormpc `delete_menu.rs`).

## Media keys

On macOS the hardware media keys (play/pause, next, previous) and the system Now Playing widget come from
[mpd-now-playable](https://git.00dani.me/00dani/mpd-now-playable), a separate daemon that talks to MPD directly, so
they work with rormpc closed. It is independent of rormpc: `rormpc_install.sh` neither installs nor touches it.

    uv tool install mpd-now-playable
    mpd-now-playable install-launchagent      # ~/Library/LaunchAgents/me.00dani.mpd-now-playable.plist, started now
    mpd-now-playable uninstall-launchagent    # stops and removes it
    launchctl print gui/$UID/me.00dani.mpd-now-playable   # check: state = running

The plist runs the uv tool venv's Python, so after `uv tool upgrade mpd-now-playable` or a reinstall rerun
`mpd-now-playable install-launchagent --force`, or launchd keeps restarting a dead path.

On this setup Karabiner-Elements (`~/.config/karabiner/karabiner.json`, profile ISO) overrides the MacBook's
F7/F8/F9, because a browser playing media takes Now Playing and with it the keys: a complex modification runs
`~/scripts/mpd-media-key prev|toggle|next`, which sends `mpc prev`/`toggle`/`next`. It has to be Karabiner:
its `fn_function_keys` turns F7/F8/F9 into consumer keys from its virtual keyboard, which go straight to the Now
Playing app, so an event tap (Hammerspoon's, until 2026-10-07) never sees them. Shift+F7/F8/F9 still send the
media key to the Now Playing app; fn+F7/F8/F9 stay plain F-keys. Previous sends `shuffle prev` to mpd-player
(channel `rormpc`) when mpd-player is subscribed and its `shuffle.json` has `trail`, else plain `mpc prev`. Each
press appends a line (key, command, exit status) to `~/.local/state/media-keys/media-keys.log`. Volume and mute
are left alone; AirPods/Bluetooth buttons and external keyboards' media keys still go to the Now Playing app.

On Linux MPD has no MPRIS of its own: the keys need an MPRIS bridge (rmpcd, mpd-mpris or mpDris2; see TODO.md).

## Music tab (the Play pane)

`Pane(Play())`, the first tab of the default config (`1`, named "Music"), is Queue, Hits and Shuffle in one tab
(plans/combined-view.md, phase 3). The old `Queue`, `Hits()` and `Shuffle()` panes still load from an explicit
config. Up next lives in it too (its own tab went on 2026-10-10, see Up next in Music). A header line always says
what plays and how:
`▶ Playing from: Hits · 1980s … · weighted · round 12/84` or `… · in queue order`.
The tab was called "Play" until 2026-10-10; the pane type `Pane(Play())`, `ShowPlay(...)` and the action names
(PlayReplace, PlayNext) keep their names, so explicit configs keep loading, a tab named "Play" included.

- The left column (Filters | Browse) is always open, in both modes (decided by the user 2026-10-10; until then
  normal mode collapsed it to a `Source: …` line). `h` gives it the keys, `l` gives them back to the table.
- **Normal mode** (weighted off): the table is MPD's queue in its order, as in the Queue pane.
- **Weighted mode** (`w`, ToggleWeightedShuffle, now also in the default keys): the table is the plan projection of
  the Queue plan view (past plays, `0 ▶`, `↑n` requests and the forecast, nothing after it). `o` does nothing
  here: the table follows `w`. Turning weighted off keeps the queue as it is.
- **A filter change prepares**: `hits` runs at once (one run at a time, as Hits' Apply) into its own file,
  `~/.cache/rormpc/hits/preview.json` (`current.json` stays the Hits pane's), and the table shows that result under
  a banner: `Preview · <label> · 312 matched · 287 owned · 25 missing · playing song: outside  [ a Apply ]`. MPD is
  not touched. Esc drops the preview and brings the filters back to the rules being played.
- **Apply plays**: `a`, the banner's button, the column's `[ Apply ]` or the row menu's "Apply" replace the queue
  with the preview's owned songs like "Play these N songs": the playing song goes on (outside the new source it
  plays on and is not in the round), Up next is kept, and `source.json` gets the result's `args` as `rules` and
  their canonical hash `rules_hash` (sets, Rank by, Years of, period, Top %, genres and artists in any order,
  owned; not "show excluded"). mpd-player keys its round by that hash: the same rules keep the round, other rules
  start a new one. Apply asks first only when the queue holds another kind of source (the whole library, a
  playlist, nothing known) or more than a quarter of the queue (the playing song and Up next not counted) would
  go. With 0 owned songs it is off ("widen the filter"); the queue is never emptied.
- **The queue version**: the counts and the confirmation are judged on the queue as it is when `a` is pressed;
  the replace carries MPD's `playlist` version of that moment and refuses with "The queue changed, preview again"
  when MPD's differs (a phone, mpc, another rormpc changed it meanwhile). No retry, no delay: `a` again judges the
  new queue. mpd-player's priority updates count as changes too, so a confirmation left open across a song change
  is refused.
- Pins and exclusions: `+` / `-` on a row (queue, plan or preview) open the scope menu with the `+` sets of the
  rules being played; queue and plan rows (in the Queue pane too) show `✚` / `⊘` for a file an exception names
  (`hits exceptions --json`, read in the background and again after `+` / `-` or the Exceptions list).

### Browse, the playlist editor and the Live inbox

Music also took over the browsing tabs (plans/combined-view.md, phase 3b, with the user's layout of 2026-10-10):
Directories, Artists, Album Artists, Albums, Playlists and Live playlists are no longer default tabs; their panes
still load from an explicit config.

- **The left column switches Filters | Browse**: `B` (ToggleBrowse), or a click on `Filters` / `Browse` at its top.
  Browse is about half of Music's width (the three-column browser needs it); the queue or plan stays on the right.
- **Groupings**: Artists · Album artists · Albums · Folders · Lists, in a chip row (click), `[` / `]`
  (PreviousGrouping / NextGrouping), or from anywhere with the digits below. Each grouping is the old tab's pane
  (navigation, `/` search, Space marks, the context menu), created on first use and kept, so it remembers its path,
  cursor and marks; each gets its own MPD query target (`PaneType::PlayBrowse`), so a reply that arrives after a
  switch lands in its own list. Lists are the stored MPD playlists, the smart lists' "Smart NAME" exports and the
  Live playlists' `.m3u` among them.
- **Actions on any row**: Enter plays a song now (the queue stays) or opens a container; `P` (PlayReplace) plays
  the selection replacing the queue and starts it at once, in its order (an album in disc/track order, an artist by
  album date then disc/track, a folder in filename order, a list in its order; on a song, its album/folder/list
  from that song), with weighted shuffle and random off; it asks first under Apply's rule (another source kind, or
  more than a quarter of the queue would go) and refuses when the queue changed meanwhile. `source.json` gets the
  kind (`album`, `artist`, `directory`, `playlist`, `selection`), a name and the files. `t` (PlayNext) puts the
  selection into Up next; `a` / `A` append. Never Apply: in Browse `a` appends, elsewhere in Music it applies (the
  header says `Browse › Albums (a appends)` while Browse has the keys).
- **Appending to a Hits source**: songs appended with `a` / `A` while the queue holds a Hits result join its files:
  `source.json` lists them in `added` (the rules hash stays), the source line says `+N added`, and mpd-player
  (rormpc-tools newer than 0.2.39) draws them in the same round (a round already done opens again for them). The
  next Apply drops them like any hand edit. It is recorded with weighted off too, so turning it on later still
  draws them.
- **An unapplied preview stays** when a Browse action changes the queue: the banner keeps its counts (they do not
  depend on the queue) and Apply judges its confirmation against the queue as it is when `a` is pressed.
- **The playlist editor** opens as a panel over Music when a playlist is entered in Lists (`l` on it):
  `Editing "melancholic" · changes are saved at once`. J/K move the song (marked songs together), `D` removes it,
  Ctrl-r renames (at the Lists level); every edit is an immediate MPD command, as before. `D` at the Lists level
  deletes whole playlists only after a confirmation naming them. Generated playlists ("Tag …", "Smart …", the
  `hits` exports such as "Hits …", "Not finished", "Skipped", "Folder …", and the Live playlists' MPD playlists)
  are read-only here: D, J/K and Ctrl-r say who writes them. `P`, `t`, `a` work in the panel; Esc (with nothing
  marked) or a click outside closes it.
- **The Live inbox** is the Live playlists pane as a panel over Music: `0` or `gl` (ShowPlay(Live)), or a click on
  the `Live` badge at the right of Music's header, which shows `Live 3` while items wait for a decision and `↓ 2`
  while a download runs. Space marks items; `a`, `D` and the menu's "Accept marked" / "Reject marked" act on the
  marked items (else the item under the cursor). Esc clears the marks, then closes the panel.
- **Deleted** is the Deleted pane (see Deleted pane) as a panel over Music: `gd` (ShowPlay(Deleted)), or a click on
  the `Deleted ! 2` badge left of `Live`, shown only while deletions have failed or unresolved steps (an error, or a
  step whose outcome is "failed"). The panel has the pane's keys (Enter restores, retries, allows or blocks
  downloading it again); Esc or a click outside closes it. It replaced the Deleted tab (decided by the user
  2026-10-10).
- **Keys** (built-in and `assets/example_config.ron`; `ShowPlay(...)` opens the Music tab from anywhere):

  | Key | Map | Action |
  |---|---|---|
  | `1` / `3` | global | Music / Search tabs; `gv` Versions, `gy` Lyrics |
  | `2`, `gu` | global | `ShowPlay(UpNext)`: the cursor on Music's "Up next · N" row ("Up next is empty" without one) |
  | `5` `6` `7` `8` `9` | global | `ShowPlay(Browse(Folders))`, `(Artists)`, `(AlbumArtists)`, `(Albums)`, `(Lists)` |
  | `0`, `gl` | global | `ShowPlay(Live)`: the Live inbox (`ShowPlay(Queue)` brings the filters back) |
  | `gd` | global | `ShowPlay(Deleted)`: the Deleted panel |
  | `B` | queue | ToggleBrowse: the left column, Filters or Browse |
  | `[` / `]` | queue | PreviousGrouping / NextGrouping in Browse |
  | `P` | navigation | PlayReplace: play the selection replacing the queue (Browse) |
  | `t` | navigation | PlayNext: the selection into Up next (Browse, the browser tabs, the Queue) |
  | `a` / `A` | navigation | append (Browse); Apply elsewhere in Music |
  | Esc | navigation | Browse: marks first, then the editor panel, then back to the filters |

  A config that binds `P` in its queue map (e.g. `"P": SortByColumn(4)`) drops the built-in navigation `P`: a key
  the user binds replaces the built-in binding in every map. Add `"P": PlayReplace` to its `navigation` map; both
  then reach Music, the queue claims its sort and Browse its PlayReplace. Configs with their own digits keep them;
  bind `ShowPlay(...)` to reach Browse, Live and Deleted by key (`B` and the badges work without it).

### Smart lists

A smart list is Music's filters saved under a name (plans/combined-view.md, phase 4), kept by `hits lists` in the
data repo as the event log `<data_dir>/smartlists.jsonl` (semantic rules, not `hits` arguments).

- `S` (QueueActions `SaveSmartList`) saves the filters on screen: a name prompt with the rules and the preview's
  counts shown. A name that exists asks "Update it" (with the filters on screen) or "Another name". The saved list
  becomes the open one.
- `L` (`SmartLists`) opens the picker: Smart lists, Previous sources, then MPD playlists and Live playlists in their
  own sections (the "Smart …" exports are left out of the MPD section). Enter loads a list's rules into the filter
  column as a preview: nothing plays until Apply. On a list row `a` applies it (the preview is made, then played as
  Apply plays it), `r` renames, `u` updates it with the filters on screen, `c` duplicates it (with its exceptions),
  `d` deletes it after a confirmation (its exceptions and its "Smart NAME" playlist go with it); Left/Right and a
  click pick the same buttons. The letters are matched by what they are bound to (Add, Rate, Update,
  ToggleConsume/ConsumeOff, Delete), since a modal sees actions, not keys. Enter on an MPD or Live playlist plays it
  as the source, after the same confirmation as "Sources…". "× Close the smart list" keeps the filters and leaves
  the list.
- The open smart list is part of the filters: Music runs `hits --open-list ID`, so the list's own exceptions
  (scope `list:ID`) apply, and its id is part of the rules hash. The header names it (`Smart list: 80s party`,
  "(changed)" once the filters differ from its rules), and a list applied as saved plays as "Hits · smart list 80s party". A new pin or exclusion defaults to the open
  list ("in the smart list 80s party only (while it is open)"), else library.
- Export: Apply of a list writes it as the MPD playlist "Smart NAME" (`hits lists export ID` in the background) and
  `musicdb update` writes every list hourly; the picker shows "287 songs · exported 3 h ago". The export is a
  snapshot for phones, never read back as rules, and `hits`' "my playlists" set leaves "Smart " playlists out.
- Previous sources: each Apply in Music is remembered in `$XDG_STATE_HOME/rormpc/previous-sources.json` (the last 10
  rule sets, the same rules once; local state, not the data repo). Enter loads one as a preview, `a` applies it, so
  "weighted off, now give me the old queue back" is one explicit Apply.
- A list whose rules or events this version cannot read shows `! NAME · made by a newer rormpc-tools, update it`
  and is never loaded or exported (its last export stays).
- A smart list can use another one as a set ("+ set…"); updating a list so that it leads back to itself is
  refused, and a cycle that arrives anyway (two machines' logs merged) shows `· ! smart list cycle: A → B → A` on
  its row; it still loads, so the filters can be changed, but runs and exports stop with that error.
- `SelectAlbum`, `L` before, is `M` in the default keys and the example config.

## Hits pane

Ranked chart hits produced by the `hits` CLI ([rormpc-tools](https://github.com/rofrol/rormpc-tools)), e.g.

    hits --years 1985-1992 --top 11-20 -g "+rock +pop -country" --json ~/.cache/rormpc/hits/current.json

A table (rank, percentile, ✓/✗ owned, artist, title, year, plays) with details for the selected row and a
status line (label, counts, when `hits` ran). Missing songs are dimmed rows with nothing to play. The song
playing (or paused) is painted like the Queue's playing row (`highlighted_item_style`, also when the row is hidden)
and its Next cell reads `▶0`, which stays visible under the cursor; every row with its file lights up, and a song
outside the filter gets no extra row. Enter or a
double click plays the selected owned song, `a` appends it; the queue is never replaced. A file already in
the queue is not appended again (from any pane: Directories, Search, Find too): Enter plays its existing entry (the playing one is not restarted, a paused
one resumes), `a` only says so; the row's menu has "Add another copy" for a deliberate duplicate. The pane re-reads the file when it changes and when the MPD database changes.

The Queue's `JumpToCurrent` action (`Shift+C` by default, including custom bindings) also works in Hits:
if the playing song's file is in the visible table, it selects that row and focuses the table; a second press
centres it. A search that hides the song stays intact, and a missing match changes neither selection nor
playback. Queue-only actions such as removing or reordering queue entries are not applied to chart rows.

The default and example configurations have the tabs Music, Search, Versions and Lyrics (Music replaced their
Hits, Queue, Up next and browsing tabs and holds Deleted as a panel, see Music tab). `1` opens Music, `3` Search,
`2` and `gu` Music's Up next block, `5`-`9` and `0` Music's Browse groupings and Live inbox, `gd` its Deleted panel.
Existing explicit configurations
are not rewritten; the Hits pane works there as before. The last active tab is still restored on startup.

The filter column on the left (h/l moves between it and the table) builds the `hits` arguments. "Sets" are
chips like the genre rows (click or Space cycles off → + → − → off): Billboard US (year-end charts), my likes, my
playlists (the songs of all my stored MPD playlists together, each once; "Why" names the playlists a song is on;
the playlists the tools write themselves — `hits --playlist`'s, "LB …", "Folder …", "Skipped", "Not finished" —
are left out, and the details line names the ones skipped; "Tag …" and Live playlists count) and recommended (songs
of artists similar to the ones I play most, from ListenBrainz Radio; no years, the details say which artists led to
each song). "+ set…" adds any other set as a row under these (plans/combined-view.md, phase 5): a tag list ("Tag
God", `musicdb tag`), a stored MPD playlist (the generated ones, "Tag …" and the Live playlists' own are offered in
their own sections or not at all), a followed Live playlist (its accepted, downloaded songs) or a smart list (what
it selects, with its own exceptions). The picker (`hits sets --json`, read in the background) has a section per kind
with the sizes and `/` searches it; a pick is added as `+`, and its row then cycles like a chip, staying while off;
"× clear sets" turns every chip off and drops the added rows (dim while there is nothing to clear). A smart list
that leads back to itself through the lists it uses (a cycle) is listed with the error and not added, the open
smart list is not offered as its own set, and a run that meets a cycle or a missing tag list or playlist stops
with that error under the filters, never with an empty set. Named sets are passed as `--set +tag:God`, `--set
-playlist:NAME`, `--set +live:ID`, `--set +list:ID` (a name may hold spaces, colons and commas), stored that way
in smart lists, and named in the formula ("Tag God", "Playlist NAME", "Live NAME", "Smart NAME"). The selection
is (union of the + sets, or the whole library when none is +) − (union of the − sets) ∩ period ∩ genres ∩
artists ∩ Top % ∩ owned (`hits --set ±KIND`). "Rank by" cycles Billboard (best year-end
position) / my plays / rediscover (often played, not lately) / none (my plays and rediscover leave out the weighted
shuffle's own picks, with every Years of; the Plays column counts every play); Top % is cut in the rank's own population
(the chart songs of the period, or the library songs of the period), before the sets, genres and artists, so a song's
rank never depends on which chips are on, and a song outside that population shows "—" and stays only with no Top %
box ticked. With Rank by none the Top % rows are dim and do nothing. "Years of" says what the period means: chart
year, release year or listened year (the period row then reads "Listened:", the year in progress included; "thin
data" under 30 plays); "auto" follows Rank by (Billboard → chart,
my plays → listened, else release), cycling it picks one. A period with no decade ticked means every year, except
for Billboard chart years. Under the rows, above Apply, the rule formula is printed, with the result's counts once
Apply ran with these filters: "(Billboard ∪ Likes) − Recommended ∩ 1980-1989 ∩ Top 1-10% ∩ rock · 87 of 1,056"
(rows shown of the songs the sets leave). The old Source choices are these combinations: my charts = no set, my
plays, listened; whole library = no set, my plays or rediscover; `hits --source` still maps them, and result files
written before the chips load into the matching rows. Apply takes ~0.3 s once the chart songs were looked up: `hits` keeps its MusicBrainz lookups and
matches and the ListenBrainz popularity (30 days; ListenBrainz down = not asked for 20 min) in one SQLite file,
`~/.cache/hits/cache.sqlite3` (raw search results compressed; `hits compact` reclaims space). A new install
starts from the seed in rormpc-tools (the matches of every chart year, ~0.8 MB), and the hourly `musicdb update`
fills gaps such as a new chart year, 30 MusicBrainz lookups per run. `musicdb chart` draws my yearly top 10 as an animated bar chart race. "+ other genre…"
under the genre checkboxes asks for genres that have no checkbox (`italo-disco, -schlager`: no sign includes,
`-` excludes) and adds them as rows. The genre checkboxes are the ones pinned with `hits genres pin` (rormpc-tools,
`~/.config/rormpc-tools/hits-genres.json`). "⋯ explore genres…" lists every genre of the library with its song
count (how many from the recording's own tags) and plays; picking one filters by it alone, on the chart or
among my liked songs, or pins/unpins its checkbox. "Pin genre in Hits…" in the Queue menu and on Hits rows does
the same from a song: it lists the song's genres (`hits genres of`: the recording's MusicBrainz genres, else the
artist's, plus my `musicdb genre` corrections; a missing chart row uses the row's genres) with 📌 on the pinned
ones; a newly pinned genre gets its checkbox on the next render, an unpinned one keeps its box until restart.
"× clear genres" and "× clear artists" are dim, and do nothing, while no box is + or -. Under the mouse, action
rows and the label of a checkbox (never the box) are underlined and bold; that is only a look, the "›" cursor
stays where it was. Artists work like genres (`hits --artist "+Queen; -Madonna"`):
"+ artist…" lists the artists of the result's whole cohort (before the Top % cut) with their song counts, `/`
searches the list; a picked artist is a three-state row (+ / - / off), "× clear artists" drops them. The artist
filter applies after the Top % cut, so "Queen, 1980s, top 10%" is Queen's songs in the decade's top 10% with their
ranks in the decade, and matches the whole credit or any artist in it ("A feat. B", "A & B"), case and diacritics
ignored. (`/` in any menu now also ignores case and diacritics and takes words in any order.) `[ Apply ]` stays at the bottom of the column however far it is scrolled, and
says "• changed" when the filters differ from the result on screen, "running…" while `hits` runs.

Missing songs can be fetched from a missing row's menu: "Fetch this song", "Fetch the first 10 missing" or
"Fetch all N missing…" (confirmed). This queues them in `hits fetch` (rormpc-tools), a verified import queue that
keeps running when rormpc closes: it downloads one song at a time with pauses, and only an exact match of the
chart's recording goes into `Hits/<decade>s`; anything else waits for review. The `✗` column then shows the state:
`…` queued, `↓` fetching, `?` review, `!` failed (the details say why), `+` arrived (the result reruns to show it as
owned); the status line counts them.

"Downloads (N to review)", the first row of the filter column, lists every download in review (`?`) or failed (`!`)
across all results, not only the one on screen; queued and fetching ones are a count in its status line. The cursor
stays on the same song when the queue changes. The details put the chart song (expected) beside the staged file
(downloaded): artist, title and length, a length off by more than the fetch's 5 s highlighted, "uploader ≠ artist"
when the file was tagged from the YouTube channel (MusicBrainz knew no recording for it), and the reason in plain
words. Enter (or the menu) decides; the same items are in a `?` / `!` chart row's menu:

- Preview the download / Stop the preview: the staged file in mpv, else ffplay, outside MPD (the Versions pane's
  player); nothing plays until asked, one preview at a time, and it stops before any decision, when the view is
  left and when the pane hides.
- Open on YouTube.
- Accept as the chart song (`hits fetch accept --as-chart`): the chart's artist and title, the YouTube channel,
  video title and URL in a comment, never the chart's recording MBID (the file was not identified as it); Hits then
  matches it by name. Accept with current tags moves it as it is. Either fails, and the item stays in review, when
  the staged file is gone.
- Reject the download… (confirmed): the file is deleted and the song is not fetched again; Retry in the song's menu
  undoes it.
- Try another candidate (`hits fetch another`): the upload's video id is stored as rejected and the worker takes the
  next YouTube search result; a failed item offers it too, beside Retry. Retry also skips rejected videos.

A song deleted from the library before (Ctrl-x, any mode) is never fetched again: its missing row shows `⌫` with
the date in the details, Fetch missing… leaves it out, and its menu offers "Allow downloading again" (`musicdb
deletions allow ID`, then Hits reruns). A queued one becomes `blocked` (`⌫`, "deleted" in Downloads; Retry after
allowing it), and a download that MusicBrainz identifies as a deleted recording waits in review ("deleted before").

Esc or the Downloads row again goes back to the chart; Apply does too.

Exceptions (pins and exclusions with a scope, `hits except`, rormpc-tools): `+` on a Hits or Queue row pins the
song ✚, `-` excludes it ⊘ (QueueActions `PinSong` / `ExcludeSong`; the row menus have "Pin in results…" and
"Exclude from results…"). A small menu asks the scope, the default first: library (every result) or one of the
`+` sets of the filters, an added one included (`set:tag:God`, `set:playlist:NAME`, `set:live:ID`,
`set:list:ID`; the Queue offers the `+` sets of the Hits result file), which applies only while that set is `+`, or
the smart list open in Music, which applies only while it is open and is the default then (see Smart
lists). A pin puts the song in whatever the filters
say (a `-` set, genre or artist included); it needs an owned file, has no rank ("—") and sits after the ranked rows,
outside the ranking and the Top % cut. An exclusion takes the song out; any applicable exclusion beats any pin.
`hits hide` is the same as an exclusion scoped to Billboard (keyed by the chart song, so it covers missing rows) and
keeps its own log. Exceptions apply after the Top % cut, so no rank moves. The mark column shows ✚ / ⊘ and the
details list each exception with its scope ("excluded · Billboard US (hits hide)"; a set-scoped one that does not
apply now is dim). "show excluded (N)" in the filter column (the old "show hidden") puts the excluded rows back,
dim and marked; "⋯ exceptions…" lists every exception by scope (`hits exceptions --json`; a pinned song whose file
is gone shows `!`) and Enter removes the one under the cursor (a hide is unhidden). In the filter column the same
`+` / `-` set the set, genre or artist row under the cursor (pressed again: off). Only these keys record an
exception: "Remove from queue" and a song added by hand stay one-offs, and the queue itself never changes when an
exception is recorded; the next "Play these" (or Apply in Music) takes it into account. The log is
`<data_dir>/exceptions.jsonl`, keyed by music-data's song id (songs.jsonl), so a pin follows a moved or merged file.

```ron
(name: "Hits", pane: Split(size: "100%", direction: Vertical, panes: [
    (pane: Pane(Hits()), size: "100%", borders: "ALL", border_symbols: Rounded), // path: "~/.cache/rormpc/hits/current.json"
])),
```

## Queue filter

`/` in the Queue (QueueActions `Find`) filters it as you type, inline, no window: the header line shows
`FILTER / query  12/779`, rows that don't match are hidden and the rest keep their queue order. Every typed word
must appear in artist, title, album or file name, in any order, diacritics folded ("zolw" finds "Żółw", "lodz"
finds "Łódź"). When nothing matches exactly, typos are forgiven: the header says `Close matches (N)` and
`0 exact · N close`, and each typed word may be a few edits (Damerau-Levenshtein, a swap of two letters counts as
one) away from a word of the row: none for words of up to 3 letters, 1 for 4-7, 2 for 8 and more, at most 2 in the
whole query; the last word may be unfinished. "beyonse" finds Beyoncé, "nigthcall" Nightcall; close matches are
ranked by edits, then queue order. While typing: ↑/↓ or Ctrl-n/Ctrl-p move among the shown rows (the cursor starts on the first match
after each keystroke), Backspace, Ctrl-w and Ctrl-u edit. Enter plays the selected song (by MPD song id), clears
the filter and shows it in the whole queue; Esc clears the filter and puts the cursor and the scroll back where
they were. The filtered rows are a view rebuilt from MPD's queue by song id whenever it changes, so nothing acts
on a filtered position: while filtered, moving, sorting and the context menu wait for the filter to be cleared
(the status bar says so); delete works on the selected or marked rows by id.

## Context menus

Queue (ContextMenu, e.g. Ctrl-z): besides upstream's items, Like ♥ / Dislike ✗ / Clear like (rmpc's like
sticker) and, in its own section, "Move library file to Trash…" with a confirmation (the same as Ctrl-x,
`musicdb delete`); "Remove" is renamed "Remove from queue (keep file)", and "Remove repeated entries (N)…"
collapses a file queued several times to one entry (the playing one, else the first), after a confirmation (such
entries carry a dim `⧉` in the first column);
library files stay. Hits rows: Play now, Add to queue, like
items (owned songs), "Hide song across charts" / "Unhide" for every chart song (`hits hide|unhide`, an
append-only log in the private data repo; hidden songs keep their place in the ranking and are dropped after
the Top % cut), "Pin in results…" / "Exclude from results…" / "Exceptions…" (see Hits pane), and "Move library
file to Trash…" for owned songs. "show excluded (N)" in the filter column lists hidden and excluded songs (marked
`⊘`) to review them. Menu items show the key that does the same thing directly.

Mouse in menus and modals: the menu item under the pointer is selected on hover and one click runs it (upstream
needs a double click). A click outside a modal closes it like Esc and does not reach the pane underneath, even
as the second click of a double click; confirmation dialogs stay open. With `modal_backdrop: true` in the theme
the screen behind a modal is dimmed with the terminal's faint attribute (as herdr does), keeping its colours.

## Deleted pane

Songs deleted with Ctrl-x, newest first, from `musicdb deletions --json --all` (rormpc-tools): when, whether the
file went to the Trash or was deleted permanently, whether its history was kept or deleted, and failed steps
(`!`). The details show the ListenBrainz listens deleted, the YouTube playlists the video was removed from and
each step's outcome. Enter (or the context menu) retries failed steps or offers "Restore…" for any deletion,
Trash or permanent: `musicdb restore ID --json` plans it without changing anything (the file from the Trash or
downloaded again from its video and checked, its song id, stickers, plays, the ListenBrainz listens the deletion
deleted with their original time, the YouTube playlist entries), the confirmation lists each step (`+` will be
done, `✓` done, `…` waits and is retried hourly, `✗` cannot), and Restore runs `musicdb restore ID --yes` in the
background. A restored row says `restored` in the File column; one with a step still waiting offers "Restore…
(continue)". Nothing in this pane deletes. It reloads when shown and when the MPD database changes.
The Download column says whether the downloaders skip the song (`blocked`: Hits, `hits fetch`, `yt-mp3-mb`,
Live playlists never download it again; the details list what it matches: video, recording, chart song) or
`allowed`; the menu switches it (`musicdb deletions allow|block ID`). Restoring a song lifts the block.
Music shows the pane as a panel (`gd`, or the `Deleted ! N` badge while steps failed or are unresolved); the
default tabs no longer have a Deleted tab, but an explicit config can still put the pane in one:

```ron
(name: "Deleted", pane: Split(size: "100%", direction: Vertical, panes: [
    (pane: Pane(Deleted()), size: "100%", borders: "ALL", border_symbols: Rounded),
])),
```

## Live playlists pane

Public YouTube playlists and Omarchy Radio (https://radio.omarchy.org/, a community playlist of MP3s; rormpc-tools
newer than 0.2.42) followed by `liveplaylist` (rormpc-tools newer than 0.2.33): left the subscriptions (`!`
when the last check failed, the number of new items), right the selected one's items in playlist order with their
decision (`?` to review, `✓` accepted, `✗` rejected) and state (to review, queued, downloading, needs match, ready,
in library, failed, blocked: deleted from the library before, accept it again after allowing it in the Deleted
panel (`gd`), gone upstream). Left/Right moves between the two lists. The footer shows the download's progress
(from the CLI's stderr and its `status.json`), the last download's errors, and for the selected item its error,
the uncertain MusicBrainz proposal, its file or its source link (the video, the radio's MP3). A radio item shows
its artist where a video shows its channel; radio tracks are tagged with the station's own names, never matched.

The URL modal lives here: Enter (or the context menu) → "Add a playlist URL…"; with no subscription yet, Enter
opens it directly. Adding lists the playlist (no download) and every item waits for review, the first import too.
The menu also has "Accept" / "Reject (never download it)" for the selected item, "Accept all pending (N)", "Check
for new tracks", "Check all playlists", "Download queued (N)" and "Cancel the download". The add keys work too:
add accepts the selected item, add all accepts every pending item; delete rejects the selected item.

Accepting queues the items (`liveplaylist accept ID ... --no-download`), then starts `liveplaylist download` unless
one runs (it downloads everything queued, also what is accepted while it runs). Every command runs with argv in a
background thread; "Cancel the download" sends SIGTERM, which stops yt-dlp and queues the item again. Songs already
in the library are referenced, uncertain downloads wait as "needs match" outside the library until accepted as
they are, and the MPD playlist (named after the YouTube playlist) holds only accepted, ready, still listed items.
Nothing runs on a timer: checking is "Check for new tracks". The pane reloads when shown.

```ron
(name: "Live", pane: Split(size: "100%", direction: Vertical, panes: [
    (pane: Pane(LivePlaylists()), size: "100%", borders: "ALL", border_symbols: Rounded),
])),
```

## Versions pane

Song names that several different library files share, from `musicdb versions --json --all` (rormpc-tools 0.2.2
or newer; resolved groups are listed too, dimmed with ✓, after the open ones). A play matched only by artist + title to such a name counts for no file until it is decided here, once
per source track (a Spotify URI, a recording MBID, or the name for plays without an id); later plays of the same
track follow the decision. Left: the groups, open ones first, most plays first, then YouTube ids / MBIDs found
on several files. Right: the group's files (length, version label, plays) and the played tracks (source, plays,
Spotify album, first and last play, longest play, decision, suggestion). The footer counts the open items, or
says "clean".

A hint line at the bottom shows the keys for where the cursor is, from the actual bindings. `/` filters the
group list as you type (every typed word, diacritics folded, close matches for typos as in the Queue; files and
tracks inside a group are never hidden),
↑/↓ move while typing, Enter keeps the filter, Esc clears it and restores the selection; Space (Select) toggles
"unresolved only". A suggestion reads "Suggested: <file>" and "Why: … longest listen 3:27, file 3:38": Spotify
records how long you listened each time, so the longest listen is about the song's length if you ever played it
to the end. Undecided tracks say "Unresolved".

h / l (Left / Right) move between the list, the files and the tracks; Enter opens a menu (actions):

- on a track: "Accept suggestion" (shown with its reason under the table, never applied by itself), "N. It is
  <file>", "A version I don't own" (it joins musicdb's missing list), "Clear the decision";
- on a file: "Preview from 0:00 / 0:30 / 1:00 / 2:00" and "Stop the preview" (mpv, else ffplay, outside MPD, so
  listening to compare never scrobbles or counts as a skip; it stops when you leave the tab), "Label: original /
  live / remix / edit / cover / other", "Same recording: keep this file, merge the others…" (asks first; the
  others go to the quarantine with aliases, like `musicdb dedupe`), "Delete this file…", and for a shared id
  "These files are fine (shared-ok)…".

Files whose audio matches (musicdb's chromaprint comparison within the group: 0.88 or more) show "Same recording?
audio match 93% …: files 1, 2; keep 1 (official channel …)" under the group name, with the file to keep and why
(an MBID or the official channel, then the longer, the higher bitrate, the more played). Add (`a`) or the file
menu's "Same recording? 93%: merge…" asks first, and "Keep another file…" there picks a different file to keep;
nothing is merged by itself. Pairs at 0.72-0.88 show as "similar audio (another master or edit?)", with no merge
offered. Files without a fingerprint are computed in the background (`musicdb versions fingerprint`, needs
fpcalc; the footer says "comparing audio…"), then the list reloads.

"Delete this file…" first asks what the file is. "A copy of <other file> (same recording): merge it…" runs the
merge for that pair (`musicdb versions same OTHER FILE`, after a confirmation): the copy goes to the quarantine
with an alias and its plays and decisions move to the file that stays. "A different recording I don't want: delete
it…" opens the delete menu (Ctrl-x; Trash by default, Ctrl-y undoes, the Deleted pane lists it): its plays and
decisions stay with the deleted file, never moved to another version; "+ delete the history" deletes them too.
The deletion checks `musicdb delete --preview` again before it runs (nothing is deleted when the file is gone) and
the status bar shows musicdb's last line only when it has finished, or why it failed. Deleting the last other file
of a group is allowed: the group leaves the list and the cursor takes the next group.

From the Queue, "Find versions…" (its context menu, or `V`: QueueActions `FindVersions`; add `"V": FindVersions`
to an explicit Queue keymap) opens this pane on the song's group with its file selected, without a filter and
without playing anything; Esc (or h on the group list) goes back to the Queue, to the same row and scroll. The
Queue column `Versions()` shows `≋` on songs whose group has several owned files and is blank otherwise; the
membership is read once in the background (`musicdb versions --json --all`, or this pane's own load) and again
after a deletion, a merge or a library update, never per row. Sorting by the column puts those songs first.

```ron
(prop: (kind: Property(Versions()), default: (kind: Text(""))), label_prop: (kind: Text("≋")), width: "1"),
```

A decision whose file is gone, or whose group changed since (a download, a deletion), shows as "review" and is
open again. At startup, when the hourly `musicdb update` found open items (`~/.cache/rormpc-tools/doctor.json`),
the status bar says "doctor: N open (Versions pane)".

```ron
(name: "Versions", pane: Split(size: "100%", direction: Vertical, panes: [
    (pane: Pane(Versions()), size: "100%", borders: "ALL", border_symbols: Rounded),
])),
```

## Not finished

`musicdb sync` (rormpc-tools) writes the playlist "Not finished": songs I rarely play to the end lately (deletion
candidates to review, never deleted by themselves), with the reason in the `notFinished` sticker (usable as a
column). The Queue menu offers "Keep (drop from Not finished)" on them (`musicdb keep`).

## Playing from + Up next

The whole queue stays in MPD (phones, media keys and mpc keep working); rormpc remembers where it came from in
`$XDG_STATE_HOME/rormpc/source.json` and shows it on the border above the queue: "Playing from: Tag God · Up next 2",
"(modified)" when another client changed the queue since. Up next itself belongs to mpd-player (rormpc-tools), so
it keeps working with rormpc closed: rormpc sends `upnext add|playnow|play|first|move|remove|clear` over MPD
messages and shows its `upnext.json`. Without mpd-player, Play next says so and Enter plays the song without
removing it afterwards.

- Queue menu "Sources… (play the library or a playlist)" and Hits "Play these N songs": the source replaces the
  queue after a confirmation; the song playing goes on (it is not part of the new source), Up next is kept and
  plays first, and the status line says how long preparing took ("11 songs, prepared in 0.4 s").
- "Play next" in the Queue menu (marked rows, else the cursor row) and on owned Hits
  rows: with random on the songs get MPD priorities 255, 254, … (first asked plays first; MPD drops a song's
  priority when it starts); with random off they move right after the current song, in order. Waiting entries
  carry `↑1`, `↑2`. A song that was not in the queue is added and removed again after it played, so the source
  stays as it was. Consume must be off. MPD song ids don't survive an MPD restart, so Up next doesn't either.
- The menus of Directories, Artists, Album Artists, Albums, Playlists and Search have "Play next (Up next)" too (a
  directory, artist or album: all its songs). Asking again for a song that is already waiting moves it to the top.
- Enter on a song in those panes plays it now and leaves the queue alone (upstream replaced the queue with the
  whole list): a queued song plays from its entry, another one is put right after the current song, played, and
  removed again after it played, so with random off the source goes on from there.
- **Up next in Music** (decided by the user 2026-10-10; Sol and MiMo: the tab duplicated Music): while a request
  waits, Music's table has a header row `── Up next · N ──` above the requests, right after the playing song in
  queue order (weighted off: mpd-player keeps them there) and between `0 ▶` and the forecast in the plan
  (weighted). On a request row Enter plays it now, K/J (MoveUp/MoveDown) move it only within the block, `d`
  (Delete) takes it out of Up next (a song added only for Up next leaves the queue, a source song keeps its place),
  and its menu has Make next and Remove from Up next. No other song moves into the block (it goes there with `t`).
  On the header row Enter, the context menu or a double/right click opens "Clear Up next (N)…" (confirmed), and
  "New round" when a Hits source's round is done; `D` there clears Up next after the same confirmation, not the
  queue. `2` and `gu` (`ShowPlay(UpNext)`) put the cursor on that row from anywhere, or say "Up next is empty".
  A rejected play keeps the request waiting and the previous playback unchanged; its error is shown in red at
  the bottom of Music, also when no request is left. An event-driven watcher of the atomically published state
  redraws it even while paused or stopped, without polling delays or playback retries. A later explicit
  successful action clears the error.
- The Up next pane (`Pane(UpNext())`) still loads from an explicit config: the waiting songs in play order with
  the same request actions, the error and the shuffle's next pick in its footer. It left the default tabs and
  `assets/example_config.ron` on 2026-10-10.
- Artists, Album Artists and Albums select the playing song's group every time the tab is shown (its tag value,
  else the root item contained in it, e.g. an artist inside "A feat. B"); nothing playing or no match keeps the
  cursor.

## Add to playlist

"Add to playlist…" in the Queue menu (marked rows, else the cursor row) and on owned Hits rows: a new playlist by
name, or a stored one; each shows `✓` when it already has all the songs (then Enter does nothing) or "3/5 there,
adds 2". It only ever adds; removing stays in the Playlists pane. The genres all the songs share (`hits genres of`,
up to 5) are offered as "+ genre" to create a playlist of that name, unless a playlist has it already; without a
`hits` that knows `genres of` (rormpc-tools 0.2.29) the menu just has no suggestions.

## Tags

"Tags…" in the Queue menu and on owned Hits rows lists my hand-made song lists (`musicdb tag`, rormpc-tools: God,
melancholic, tearjerkers, …) with a ✓ on those the song is on; Enter toggles, "+ New list…" starts one. Each list
is also an MPD playlist "Tag NAME" in the Playlists pane; nothing is queued by itself. The same menu corrects the
song's genres ("Add a genre…", "Remove a MusicBrainz genre…", "Undo genre …"), which the Hits genre filter uses.

## Lyrics

`musicdb lyrics` (rormpc-tools) fetches lyrics from LRCLIB into `lyrics_dir`, mirroring the library paths: synced
lyrics as `.lrc` (upstream's Lyrics pane shows them), plain ones as `.txt`. rormpc shows the plain ones too,
scrolled along with the song, and says why there are none (instrumental, nothing on LRCLIB within 2 s of the
file's length, not checked yet). LRCLIB matches by length, so YouTube rips with intros often miss: the Queue
menu's "Choose lyrics…" lists LRCLIB's entries for the song with their length difference and takes the one picked
(`musicdb lyrics use`).

A Polish translation shows beside the original (the original left, Polish right, both left-aligned) when
`musicdb lyrics translate` has found one on tekstowo.pl. Enter in the Lyrics pane looks it up for the playing song,
in the background and only on request; the status row under the lyrics says what there is (a human or machine
translation, none on tekstowo.pl, a Polish original, lyrics changed since) and what Enter does. The current line is
highlighted on both sides: `.lrc` lines carry their timestamps over to the translation, plain `.txt` lyrics estimate
the line from the song's progress (the status says so). A translation that does not pair line by line (merged or
split verses) is aligned by stanza and gets no highlight on its side. Below 100 columns the pane shows one column,
and h/l switches between the original and the translation. The translation lives in `<song stem>.pl.json` next to
the lyrics; `musicdb lyrics lang FILE CODE` overrides the detected language (`pl`: no translation).

```ron
lyrics_dir: "~/.local/share/rormpc-tools/lyrics",
enable_lyrics_hot_reload: true,
```

## Last tab

rormpc reopens the tab that was active when it last ran (saved in `$XDG_STATE_HOME/rormpc/last_tab`, default
`~/.local/state`, not in the hand-edited config; a tab missing from the config falls back to the first one).

## Tab bar

When the tabs do not fit the width, the bar scrolls instead of cutting them at the right edge: `‹` and `›` at
its ends mean more tabs on that side. Switching tabs (by key or click) or resizing scrolls the active tab into
view. The mouse wheel over the bar and a click on `‹`/`›` move the bar one tab without switching tabs, so the
wheel can never open a tab by accident; a click on a tab opens the tab drawn there.

## Delete menu (Ctrl-x)

A global key bound to `ExternalCommand(["…/musicdb", "delete"])` (Ctrl-x in my config) doesn't run it: rormpc
opens a delete menu for the songs the command would get (the selection, else the playing song). The Queue and
Hits context menus open the same menu ("Delete library file…"). Four items, two independent choices:
Move to Trash · Move to Trash + delete the history · Delete permanently · Delete permanently + delete the history.
"History" = the song's ListenBrainz listens (irreversible; kept when another library file has the same
recording), the video in my chosen YouTube playlists and the local plays. `musicdb delete --preview [--youtube]`
fills in the listen count and the playlist titles in a background thread; the history items wait for both, so
their confirmation names exactly what goes. Everything except plain Trash asks first with "Cancel" as the default
button. `musicdb delete [--permanent] [--listenbrainz]` does the work in the background; failed remote steps stay
in its journal and `musicdb update` retries them hourly. Ctrl-y (`musicdb undo`) restores the last trashed file,
not deleted history.

## Consume off only

Consume (MPD deletes every played song from the queue) breaks "Playing from" and Up next, so rormpc never turns
it on. The global action `ConsumeOff` only turns it off (my config binds it to `c` instead of `ToggleConsume`);
the ones already removed don't come back. When another client turns consume on, the status line says so. My
theme shows the modes as labelled badges with their keys (`z rep x rnd v single consume off`; single armed by
mpd-player shows as `gap`), consume on as a red ` CONSUME c=off `.

## Weighted shuffle

A mode of its own next to plain random: mpd-player draws the next song by weight and gives it MPD priority 1,
below Up next (2-255), so a request always plays first. Priorities steer MPD only with random on, so the mode owns
MPD's random (other clients see random on): `w` turns both on, or both off (the queue plays in order); `x` while
it runs switches to plain random (random stays on, the mode turns off, `shuffle release`); random turned off by
a phone or mpc turns the mode off. Nothing in the queue is moved. While the mode runs, the `x rnd` badge shows
off and `w shuf` on. How a song is chosen (mpd-player's shuffle module has the details): every 10 picks are
a shuffled cycle of 7 familiar (heard in the last year: by plays, a like, skips, and how overdue the song is for
its own usual gap between plays), 2 rediscovery (the heard songs that played longest ago) and 1 new (never heard,
at most 5 a day). Skips lower a song's weight (early skip x0.6, late x0.85, fading over months) and rest it (48 h
after an early skip, 12 h after a late one or a full play); a new song skipped early on two days rests 30 days.
The shuffle's own picks don't raise a song's play count used for weights, so it does not feed on itself. Data:
`musicdb sync` (hourly) writes `$XDG_STATE_HOME/rormpc/weights.json` from the play history and the scrobbler's
skips; mpd-player adds what it saw since and logs its own picks to `auto.jsonl`. `w` (ToggleWeightedShuffle) turns it on
or off; `e` (HeardEnough) rests the playing song for 1, 3, 7, then 14 days and skips to the next (the Queue menu
has it for the selected song, and "Back in the weighted shuffle"). Enter and Play next still play a resting song.
Previous (the key and `rmpc prev`) sends `shuffle prev` to mpd-player while it is subscribed to channel `rormpc`
and its shuffle.json has the `trail` (only an mpd-player that handles `shuffle prev` writes it): in the weighted
shuffle it walks back through the songs that really played and counts no skip, with the shuffle off it does MPD's
own `previous`. Without that mpd-player it is MPD's `previous`, which with random on follows MPD's random order.
`rewind_to_start_sec` still restarts the playing song first.
The Up next pane's second footer line (an explicit config only) shows the pick ("Then likely: … · may change") or
why there is none; in Music the plan shows it. The theme
property `Status(WeightedShuffle(on_label, off_label, on_style, waiting_style, off_style))` shows its state, e.g.
a `w shuf` badge next to the mode badges: on_style while it picks, waiting_style while it is on but idle (consume
on, single on, stopped), off_style when off.

A Hits result played as the source ("Play these N songs (as the source)…" in the Hits menu: its owned rows, a
snapshot that moving a filter never changes) is shuffled in rounds: each song once, then the shuffle stops
nominating and says "round done"; a queue song's menu in Music (or the "Up next · N" row's) starts a new round. The Hits footer says what plays
("Playing: Hits · 1980s top 10% · 84 playable · heard 12/84") and "browsing other results" when the filters on
screen differ. Hits' State column shows only exceptions: `⏳5d` resting, `heard` in this round, `·` not in the
snapshot being played.

The weighted shuffle draws the next 10 songs ahead (a plan) and publishes it as MPD priorities 10..1, below the
Up next requests (255, 254, ...), so MPD itself plays it in order (also "next" on a phone). The song property
`ShuffleNext()` shows each song's turn in a column, one sequence: `-1`..`-10` the last plays (they stay in the
queue), `0` the song playing, `↑1`, `↑2` the requests, then the plan numbered on (`3`..`12`): my Queue's first column "Next", and the Hits table has it too. Sorting the Queue by it (a header click) puts the requests, then the
plan 1..10, then the rest; like every Queue sort it only reorders the list, playback follows the priorities. The Shuffle
pane (`Pane(Shuffle())`, my tab "Shuffle", `gs`) is a timeline: the last 5 plays (`-5`..`-1`, ✓ played to the end,
⏭ skipped; mpd-player keeps 20 in shuffle.json), `0 ▶` the song playing, `↑` the Up next requests, then the plan
with why each was drawn. The plan changes only when a planned song leaves the queue, is
requested, gets "heard enough" or is played by hand (its lane goes to the replacement), and is topped up after
every song. Enter on a planned or past song asks for it with Play next, on a request plays it now; `C` (the Queue's
JumpToCurrent) goes to the playing song; the menu has "heard enough". It never reorders the queue (a sort by weight
would move 770 songs in MPD and be stale one song later; consulted Sol and MiMo).

## Queue plan view

`o` (`TogglePlanView`, add it to an existing explicit Queue keymap) toggles a **view**, not an MPD sort.
It shows the last two plays still identified in Queue as dimmed `-2`, `-1`, current `0 ▶`, requests `↑n`,
forecast `1`..`10`, and nothing after it (decided by the user 2026-10-10: an "unplanned · queue order" tail
misled, as the shuffle never plays that order, and a big source buried the forecast). The rest of the pool is
reached through `/` and Browse: a `/` match outside the forecast shows after it with the turn `·`, and the filter
line counts them ("N · in the pool, not in the forecast"), so Play now, Play next, pin/exclude and delete stay
on its row.
An ID appears only once: current/request/forecast takes precedence over history, and an old ID belonging to
another file is never reused. Selection, marks, scrolling and mouse actions follow song IDs across refreshes;
filtering retains the original turn numbers. Next-header sorting (and other physical sorts) is disabled here.
`o` returns to ordinary Queue order with the same selected song and marks.

J/K reorder requests only within their section, or ask mpd-player to swap adjacent forecast slots using the
published plan version. Only the daemon writes priorities and verifies MPD state again before acknowledging
(a song can start during the writes); rormpc never moves queue positions or draws an optimistic order.
A correlated acknowledgement confirms the change; stale versions are rejected, never retried.
The temporary patch survives heartbeats and more confirmed swaps, but expires at the first new draw (including
per-song top-up), a planned song playing/leaving/becoming a request or heard-enough, reroll/new round/source
change, or daemon restart. Surviving entries regain their pre-patch order **before** replacements are drawn;
the played head is classified against the effective patched order first.

The title calls this a forecast. Missing daemon channel, old/missing/future heartbeat, incompatible version or
failed priority publication means dimmed forecast numbers and `stale · Xm` (unknown age without a heartbeat),
not a fresh prediction. mpd-player publishes on its existing 30-second idle wake; 60 seconds without a heartbeat
is stale. A Subscription event (and a reconnect) re-reads whether the daemon's channel exists, so a daemon start
or a clean unsubscribe shows at once. MPD (0.24.15) sends no Subscription event when a subscribed client just
disconnects, so a stopped or crashed daemon is seen by the operating system instead: shuffle.json carries the
daemon's `pid`, and rormpc waits on that process's exit (kqueue `EVFILT_PROC` on macOS, `pidfd` on Linux) once
per daemon session (the session part of `plan_version`); a pid it cannot observe counts as stale. Native
atomic-file notifications refresh the view even while paused, and one deadline render expires freshness without
polling. A missing patch reply uses
the existing 2-second external acknowledgement deadline and re-reads state before reporting failure; no retry.

## Scrobble status

`Status(Scrobble)` shows what the scrobbler (ro-listenbrainz-mpd) will do with the playing song:
`62% counted · need 90% · in 1:24` ("counted" is the share of the song the listen has so far: under the
uninterrupted rule only the playback since the last seek; a pause freezes it and the countdown), `62% counted ·
need 80%/4:00 · in 0:54` when the rule's maximum playtime is less than its fraction (the effective share and the
maximum), `no scrobble: seek to 20%; need 90%` when too little of the song is left after a seek, `no scrobble:
unknown length`, `no scrobble: song too short`, `scrobbling off` (the last two only once the scrobbler reports
them), `scrobbled ✓`, `sending to ListenBrainz…` while a manual send waits for its answer, and `scrobbler not
running` when its process ended. The counted share rounds down, the needed share and the countdown up, so the line
never claims the listen early. Nothing without a scrobbler status file, while stopped, or before the scrobbler has
seen the playing song.
For example next to the progress bar:

    (kind: Property(Status(Scrobble)), style: (fg: "#7aa0cd")),

The rule is never computed here: the scrobbler writes `status.json` next to its `listens.jsonl` (macOS
`~/Library/Application Support/listenbrainz-mpd/`, Linux `$XDG_DATA_HOME/listenbrainz-mpd/`) on each change, atomic
replacement, never on a timer; rormpc watches the directory and adds MPD's elapsed time since the file's
`position_s`. Liveness is the scrobbler's process (its `pid`), as for mpd-player.

`oL` (`ScrobbleNow`) sends the playing song's listen to ListenBrainz now, through the scrobbler: once, with the
listen's start as its time, logged in `listens.jsonl` with `"manual": true`, and the automatic listen of that play
is then not sent. When the rule is not met yet it asks first. The Queue menu has the same action, "Send to
ListenBrainz now", on the playing song's row. It is `submit <instance> <play>` on the MPD channel
`listenbrainz_listen` (a request for an earlier play or scrobbler run is refused, never sent for another song);
the status line says "Sent to ListenBrainz" or why not once `status.json` answers. A scrobbler older than this
is not subscribed to the channel, and rormpc says so instead of sending.

## Likes in Hits and Queue

Hits has a ♥ column (rmpc's like sticker: ♥ like, ✗ dislike; `·` for missing songs, which have no file to rate).
In Hits and Queue (Music in both modes, the weighted plan view included), the like cell under the mouse is underlined
so it reads as clickable: ♥ and ✗ also turn bold, an unrated song shows a dimmed ♥. A click on the cell toggles like and nothing
else (no selection, no playback). `r` in Hits toggles like for the selected row; dislike is in the menu. `/` in
Hits searches artist and title (words in any order, diacritics folded) within the result, "23 shown / 410
results"; Esc clears it. Mouse moves reach the active tab's panes (upstream drops them), without a render unless
a pane's hover changed.

## Build revision

`Status(BuildRevision)` renders "rormpc <short sha>[+] <commit subject>" (`+` if the binary was built with
uncommitted changes), usable in any theme property, e.g. a border title:
`(kind: Property(Status(BuildRevision)), style: (fg: "#7aa0cd"))`.

### Filters

The left column edits what `hits` computes: Period (decades, several pooled into one ranking, or a year range
from/to), Top % ranges (1-10, 11-20, 21-50; percent of the whole chart cohort after the genre filter, songs you
don't have included), genres with three states each (off, `+` include, `−` exclude), owned only, and Apply.
`h` / `l` move between the filters and the table (on a year row they change the year), Space / Enter toggle,
Enter on Apply runs `hits ... --json PATH` in a background thread. One run at a time; an Apply during a run is
queued; the table keeps the previous result and the status line says "running hits…" or shows the error.
The column starts from the arguments stored in the current JSON file.

"Source" switches between the Billboard US year-end charts and your liked songs (rmpc's like sticker); with
likes, "Sort" is "by plays" or "rediscover" (liked, often played, not lately), no decade ticked means all years,
and Top % is within your likes. The details panel explains what the rank means for the current source.

Option `command` (default `["hits"]`) names the program; use an absolute path if rormpc is not started from a
shell with `hits` on PATH: `Pane(Hits(command: ["/Users/me/.local/bin/hits"]))`.
