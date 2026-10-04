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
- `mpd-gap` (rormpc-tools): seconds of silence between songs, default 3, `--gap 0` removes it.

On Linux, building the scrobbler needs a C toolchain, OpenSSL and SQLite headers (Debian/Ubuntu:
`sudo apt install build-essential pkg-config libssl-dev libsqlite3-dev`), and the services need a systemd user
session: log in normally, or on a machine nobody logs into run `sudo loginctl enable-linger $USER` once.

They run as launchd agents `io.github.rofrol.rormpc.*` (logs in `~/Library/Logs/`) on macOS, systemd user units
`rormpc-*.service` (musicdb with a `.timer`) on Linux; the installer writes them, so edits there are overwritten.
`companions --local` installs both from checkouts instead (`$RORMPC_TOOLS_DIR`, `$RO_LB_DIR`), rormpc-tools editable.
Without rormpc-tools those features report that the command cannot be run; the rest works.

## Hits pane

Ranked chart hits produced by the `hits` CLI ([rormpc-tools](https://github.com/rofrol/rormpc-tools)), e.g.

    hits --years 1985-1992 --top 11-20 -g "+rock +pop -country" --json ~/.cache/rormpc/hits/current.json

A table (rank, percentile, ✓/✗ owned, artist, title, year, plays) with details for the selected row and a
status line (label, counts, when `hits` ran). Missing songs are dimmed rows with nothing to play. Enter or a
double click plays the selected owned song, `a` appends it; the queue is never replaced. A file already in
the queue is not appended again: Enter plays its existing entry (the playing one is not restarted, a paused
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

## Not finished

`musicdb sync` (rormpc-tools) writes the playlist "Not finished": songs I rarely play to the end lately (deletion
candidates to review, never deleted by themselves), with the reason in the `notFinished` sticker (usable as a
column). The Queue menu offers "Keep (drop from Not finished)" on them (`musicdb keep`).

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
