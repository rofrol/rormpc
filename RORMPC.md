# rormpc

A personal fork of [rmpc](https://github.com/mierak/rmpc). Fork code lives in its own files; upstream files
only get the few lines that register new panes, so rebasing on upstream stays mechanical.

The binary is `rormpc` and its config lives in `~/.config/rormpc/`, so upstream `rmpc` can stay installed
next to it. Install with `scripts/rormpc_install.sh install` (keeps the previous binary;
`rollback` and `list` too). Commits carry "(AI-assisted)" (see AGENTS.md).

## Hits pane

Ranked chart hits produced by the `hits` CLI (dotfiles `~/scripts/hits`), e.g.

    hits --years 1985-1992 --top 11-20 -g "+rock +pop -country" --json ~/.cache/rormpc/hits/current.json

A table (rank, percentile, ✓/✗ owned, artist, title, year, plays) with details for the selected row and a
status line (label, counts, when `hits` ran). Missing songs are dimmed rows with nothing to play. Enter or a
double click appends the selected owned song to the queue and plays it, `a` appends; the queue is never
replaced. The pane re-reads the file when it changes and when the MPD database changes.

```ron
(name: "Hits", pane: Split(size: "100%", direction: Vertical, panes: [
    (pane: Pane(Hits()), size: "100%", borders: "ALL", border_symbols: Rounded), // path: "~/.cache/rormpc/hits/current.json"
])),
```

## Context menus

Queue (ContextMenu, e.g. Ctrl-z): besides upstream's items, Like ♥ / Dislike ✗ / Clear like (rmpc's like
sticker) and, in its own section, "Move library file to Trash…" with a confirmation (the same as Ctrl-x,
`musicdb delete`); "Remove" is renamed "Remove from queue (keep file)". Hits rows: Play now, Add to queue, like
items (owned songs), "Hide song across charts" / "Unhide" for every chart song (`hits hide|unhide`, an
append-only log in the private data repo; hidden songs keep their place in the ranking and are dropped after
the Top % cut), and "Move library file to Trash…" for owned songs. "show hidden" in the filter column lists
hidden songs (marked `h`) to review and unhide them. Menu items show the key that does the same thing directly.

## Last tab

rormpc reopens the tab that was active when it last ran (saved in `$XDG_STATE_HOME/rormpc/last_tab`, default
`~/.local/state`, not in the hand-edited config; a tab missing from the config falls back to the first one).

## Deletion queue (`ox`, action `ShowDeletions`)

Songs trashed with Ctrl-x (`musicdb delete`, Ctrl-y undoes) wait in a queue. `ox` opens it: per song choose
"remove from the YouTube playlists" (`[x]` by default) and "delete the ListenBrainz listens" (off by default,
irreversible); h/l pick the column, Space toggles, Enter shows exactly what will happen with "Cancel" as the
default button. `musicdb deletions --confirm ID --[no-]youtube --[no-]listenbrainz` runs in a background thread
per song; each step's outcome is shown and a failed step stays queued for a retry.

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
