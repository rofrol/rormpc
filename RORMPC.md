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

On Linux, building the scrobbler needs a C toolchain, OpenSSL and SQLite headers (Debian/Ubuntu:
`sudo apt install build-essential pkg-config libssl-dev libsqlite3-dev`), and the services need a systemd user
session: log in normally, or on a machine nobody logs into run `sudo loginctl enable-linger $USER` once.

They run as launchd agents `io.github.rofrol.rormpc.*` (logs in `~/Library/Logs/`) on macOS, systemd user units
`rormpc-*.service` (musicdb with a `.timer`) on Linux; the installer writes them, so edits there are overwritten.
The hourly `musicdb update` runs at background priority (launchd `ProcessType Background` + `LowPriorityIO`;
systemd `Nice=10`, `IOSchedulingClass=idle`): about a second of CPU per run. The always-on services stay at
standard priority, since background priority lets macOS stretch timers (mpd-player's silence).
`companions --local` installs both from checkouts instead (`$RORMPC_TOOLS_DIR`, `$RO_LB_DIR`), rormpc-tools editable.
Without rormpc-tools those features report that the command cannot be run; the rest works.

## Hits pane

Ranked chart hits produced by the `hits` CLI ([rormpc-tools](https://github.com/rofrol/rormpc-tools)), e.g.

    hits --years 1985-1992 --top 11-20 -g "+rock +pop -country" --json ~/.cache/rormpc/hits/current.json

A table (rank, percentile, ✓/✗ owned, artist, title, year, plays) with details for the selected row and a
status line (label, counts, when `hits` ran). Missing songs are dimmed rows with nothing to play. Enter or a
double click plays the selected owned song, `a` appends it; the queue is never replaced. A file already in
the queue is not appended again (from any pane: Directories, Search, Find too): Enter plays its existing entry (the playing one is not restarted, a paused
one resumes), `a` only says so; the row's menu has "Add another copy" for a deliberate duplicate. The pane re-reads the file when it changes and when the MPD database changes.

The filter column on the left (h/l moves between it and the table) builds the `hits` arguments. Source cycles
Billboard US (year-end charts) / my likes / recommended (songs of artists similar to the ones I play most,
from ListenBrainz Radio; no years, the details say which artists led to each song). "+ other genre…"
under the genre checkboxes asks for genres that have no checkbox (`italo-disco, -schlager`: no sign includes,
`-` excludes) and adds them as rows. The genre checkboxes are the ones pinned with `hits genres pin` (rormpc-tools,
`~/.config/rormpc-tools/hits-genres.json`). "⋯ explore genres…" lists every genre of the library with its song
count (how many from the recording's own tags) and plays; picking one filters by it alone, on the chart or
among my liked songs, or pins/unpins its checkbox. `[ Apply ]` stays at the bottom of the column however far it is scrolled, and
says "• changed" when the filters differ from the result on screen, "running…" while `hits` runs.

Missing songs can be fetched from a missing row's menu: "Fetch this song", "Fetch the first 10 missing" or
"Fetch all N missing…" (confirmed). This queues them in `hits fetch` (rormpc-tools), a verified import queue that
keeps running when rormpc closes: it downloads one song at a time with pauses, and only an exact match of the
chart's recording goes into `Hits/<decade>s`; anything else waits for review. The `✗` column then shows the state:
`…` queued, `↓` fetching, `?` review (menu: Accept / Reject the download), `!` failed (menu: Retry, the details say
why), `+` arrived (the result reruns to show it as owned); the status line counts them.

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
`musicdb delete`); "Remove" is renamed "Remove from queue (keep file)", and "Remove duplicate entries (N)…"
collapses a file queued several times to one entry (the playing one, else the first), after a confirmation (such
entries carry a dim `⧉` in the first column);
library files stay. Hits rows: Play now, Add to queue, like
items (owned songs), "Hide song across charts" / "Unhide" for every chart song (`hits hide|unhide`, an
append-only log in the private data repo; hidden songs keep their place in the ranking and are dropped after
the Top % cut), and "Move library file to Trash…" for owned songs. "show hidden" in the filter column lists
hidden songs (marked `h`) to review and unhide them. Menu items show the key that does the same thing directly.

Mouse in menus and modals: the menu item under the pointer is selected on hover and one click runs it (upstream
needs a double click). A click outside a modal closes it like Esc and does not reach the pane underneath, even
as the second click of a double click; confirmation dialogs stay open. With `modal_backdrop: true` in the theme
the screen behind a modal is dimmed with the terminal's faint attribute (as herdr does), keeping its colours.

## Deleted pane

Songs deleted with Ctrl-x, newest first, from `musicdb deletions --json --all` (rormpc-tools): when, whether the
file went to the Trash or was deleted permanently, whether its history was kept or deleted, and failed steps
(`!`). The details show the ListenBrainz listens deleted, the YouTube playlists the video was removed from and
each step's outcome. Enter (or the context menu) restores a song still in the Trash (`musicdb undo --id`) or
retries failed steps; nothing in this pane deletes. It reloads when shown and when the MPD database changes.

```ron
(name: "Deleted", pane: Split(size: "100%", direction: Vertical, panes: [
    (pane: Pane(Deleted()), size: "100%", borders: "ALL", border_symbols: Rounded),
])),
```

## Versions pane

Song names that several different library files share, from `musicdb versions --json` (rormpc-tools 0.2.2 or
newer). A play matched only by artist + title to such a name counts for no file until it is decided here, once
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
  others go to the quarantine with aliases, like `musicdb dedupe`), and for a shared id "These files are fine
  (shared-ok)…".

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

- Queue menu "Sources… (play the library or a playlist)": the whole library or a saved playlist replaces the queue
  and plays now, after a confirmation. Up next is kept and plays first.
- "Play next (Up next)" in the Queue menu (marked rows, else the cursor row) and "Add to Up next" on owned Hits
  rows: with random on the songs get MPD priorities 255, 254, … (first asked plays first; MPD drops a song's
  priority when it starts); with random off they move right after the current song, in order. Waiting entries
  carry `↑1`, `↑2`. A song that was not in the queue is added and removed again after it played, so the source
  stays as it was. Consume must be off. MPD song ids don't survive an MPD restart, so Up next doesn't either.
- The menus of Directories, Artists, Album Artists, Albums, Playlists and Search have "Play next (Up next)" too (a
  directory, artist or album: all its songs). Asking again for a song that is already waiting moves it to the top.
- Enter on a song in those panes plays it now and leaves the queue alone (upstream replaced the queue with the
  whole list): a queued song plays from its entry, another one is put right after the current song, played, and
  removed again after it played, so with random off the source goes on from there.
- The Up next pane (`Pane(UpNext())`, my tab "Up next", `gu`) lists the waiting songs in play order: Enter plays
  now, K/J (MoveUp/MoveDown) reorder, D (Delete) removes the request (a song added only for Up next leaves the
  queue, a source song keeps its place), the context menu has Make next and "Clear Up next…" (confirmed). There is
  no Play next inside it.
- Artists, Album Artists and Albums select the playing song's group every time the tab is shown (its tag value,
  else the root item contained in it, e.g. an artist inside "A feat. B"); nothing playing or no match keeps the
  cursor.

## Add to playlist

"Add to playlist…" in the Queue menu (marked rows, else the cursor row) and on owned Hits rows: a new playlist by
name, or a stored one; each shows `✓` when it already has all the songs (then Enter does nothing) or "3/5 there,
adds 2". It only ever adds; removing stays in the Playlists pane.

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

```ron
lyrics_dir: "~/.local/share/rormpc-tools/lyrics",
enable_lyrics_hot_reload: true,
```

## Last tab

rormpc reopens the tab that was active when it last ran (saved in `$XDG_STATE_HOME/rormpc/last_tab`, default
`~/.local/state`, not in the hand-edited config; a tab missing from the config falls back to the first one).

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

With random on, mpd-player draws the next song instead of MPD's uniform random and nominates it with priority 1,
below Up next (2-255), so a request always plays first. Weights come from `musicdb sync` (hourly,
`$XDG_STATE_HOME/rormpc/weights.json`): 1 for a song never played, up to 3 for songs played a lot lately (a play
counts half after 60 days, log-compressed) or liked; a dislike makes it rare. One pick in five ignores the weights.
Recently played songs and songs in a "heard enough" cooldown are left out. `w` (ToggleWeightedShuffle) turns it on
or off; `e` (HeardEnough) rests the playing song for 1, 3, 7, then 14 days and skips to the next (the Queue menu
has it for the selected song, and "Back in the weighted shuffle"). Enter and Play next still play a resting song.
The Up next pane's second footer line shows the pick ("Then likely: … · may change") or why there is none.

A Hits result played as the source ("Play these N songs (as the source)…" in the Hits menu: its owned rows, a
snapshot that moving a filter never changes) is shuffled in rounds: each song once, then the shuffle stops
nominating and says "round done"; the Up next menu starts a new round. The Hits footer says what plays
("Playing: Hits · 1980s top 10% · 84 playable · heard 12/84") and "browsing other results" when the filters on
screen differ. Hits' State column shows only exceptions: `⏳5d` resting, `heard` in this round, `·` not in the
snapshot being played.

## Likes in Hits and Queue

Hits has a ♥ column (rmpc's like sticker: ♥ like, ✗ dislike; `·` for missing songs, which have no file to rate).
In Hits and Queue, hovering the like cell of an unrated song shows ♡; a click on the cell toggles like and nothing
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
