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

Option `command` (default `["hits"]`) names the program; use an absolute path if rormpc is not started from a
shell with `hits` on PATH: `Pane(Hits(command: ["/Users/me/scripts/hits"]))`.
