# rormpc

A personal fork of [rmpc](https://github.com/mierak/rmpc). Fork code lives in its own files; upstream files
only get the few lines that register new panes, so rebasing on upstream stays mechanical.

The binary is `rormpc` and its config lives in `~/.config/rormpc/`, so upstream `rmpc` can stay installed
next to it. Install with `scripts/rormpc_install.sh install` (keeps the previous binary;
`rollback` and `list` too). Commits carry "(AI-assisted)" (see AGENTS.md).

## Companion tools

rormpc alone needs only MPD. Some fork features run my other tools, which are not installed with it:

- `musicdb` and `hits`, Python CLIs in my dotfiles (`scripts/` in github.com/rofrol/dotfiles), on PATH: the delete
  menu (Ctrl-x, Ctrl-y), the play counts in the `plays` sticker, likes sent to ListenBrainz, and the Hits pane.
  Without them those features report that the command cannot be run; the rest works.
- A ListenBrainz scrobbler, for play history: my fork [ro-listenbrainz-mpd](https://github.com/rofrol/ro-listenbrainz-mpd)
  counts a listen only after 90% of the song played in one run without a seek. It is a separate daemon, so it
  scrobbles with rormpc closed. Upstream listenbrainz-mpd works too, with its own rule.

`music-companions install` (dotfiles) installs the scrobbler at a pinned tag and starts the launchd agents;
`music-companions status` checks all of the above.

## Hits pane

Ranked chart hits produced by the `hits` CLI (dotfiles `~/scripts/hits`), e.g.

    hits --years 1985-1992 --top 11-20 -g "+rock +pop -country" --json ~/.cache/rormpc/hits/current.json

A table (rank, percentile, ✓/✗ owned, artist, title, year, plays) with details for the selected row and a
status line (label, counts, when `hits` ran). Missing songs are dimmed rows with nothing to play. Enter or a
double click plays the selected owned song, `a` appends it; the queue is never replaced. A file already in
the queue is not appended again: Enter plays its existing entry (the playing one is not restarted, a paused
one resumes), `a` only says so; the row's menu has "Add another copy" for a deliberate duplicate. The pane re-reads the file when it changes and when the MPD database changes.

```ron
(name: "Hits", pane: Split(size: "100%", direction: Vertical, panes: [
    (pane: Pane(Hits()), size: "100%", borders: "ALL", border_symbols: Rounded), // path: "~/.cache/rormpc/hits/current.json"
])),
```

## Context menus

Queue (ContextMenu, e.g. Ctrl-z): besides upstream's items, Like ♥ / Dislike ✗ / Clear like (rmpc's like
sticker) and, in its own section, "Move library file to Trash…" with a confirmation (the same as Ctrl-x,
`musicdb delete`); "Remove" is renamed "Remove from queue (keep file)", and "Remove duplicate entries (N)…"
collapses a file queued several times to one entry (the playing one, else the first), after a confirmation;
library files stay. Hits rows: Play now, Add to queue, like
items (owned songs), "Hide song across charts" / "Unhide" for every chart song (`hits hide|unhide`, an
append-only log in the private data repo; hidden songs keep their place in the ranking and are dropped after
the Top % cut), and "Move library file to Trash…" for owned songs. "show hidden" in the filter column lists
hidden songs (marked `h`) to review and unhide them. Menu items show the key that does the same thing directly.

Mouse in menus and modals: the menu item under the pointer is selected on hover and one click runs it (upstream
needs a double click). A click outside a modal closes it like Esc and does not reach the pane underneath, even
as the second click of a double click; confirmation dialogs stay open. With `modal_backdrop: true` in the theme
the screen behind a modal is dimmed with the terminal's faint attribute (as herdr does), keeping its colours.

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
shell with `hits` on PATH: `Pane(Hits(command: ["/Users/me/scripts/hits"]))`.
