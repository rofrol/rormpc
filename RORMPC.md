# rormpc

A personal fork of [rmpc](https://github.com/mierak/rmpc). Fork code lives in its own files; upstream files
only get the few lines that register new panes, so rebasing on upstream stays mechanical.

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

`Status(BuildRevision)` renders "rormpc <short sha>" (with `+` if the binary was built with uncommitted
changes), usable in any theme property, e.g. a border title:
`(kind: Property(Status(BuildRevision)), style: (fg: "#7aa0cd"))`.

Next: filter column in the pane (decades or year range, Top %, genres) that runs `hits --json` itself.
