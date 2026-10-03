# rormpc

A personal fork of [rmpc](https://github.com/mierak/rmpc). Fork code lives in its own files; upstream files
only get the few lines that register new panes, so rebasing on upstream stays mechanical.

## Hits pane

Ranked chart hits produced by the `hits` CLI (dotfiles `~/scripts/hits`), e.g.

    hits --years 1985-1992 --top 11-20 -g "+rock +pop -country" --json ~/.cache/rormpc/hits/current.json

Songs you have are normal MPD songs (play, add to queue, info); missing ones are `✗` placeholder rows.
The pane re-reads the file when it changes and when the MPD database changes. Each song gets extra metadata
`hits_rank` ("#12 (3%)"), `hits_year` and `hits_genres` for the song format:

```ron
(name: "Hits", pane: Split(size: "100%", direction: Vertical, panes: [(
    pane: Pane(Hits(
        path: "~/.cache/rormpc/hits/current.json", // default
        format: [
            (kind: Property(Other("hits_rank"))), (kind: Text("  ")),
            (kind: Property(Artist)), (kind: Text(" - ")), (kind: Property(Title)),
            (kind: Text("  (")), (kind: Property(Other("hits_year"))), (kind: Text(")")),
        ],
    )),
    size: "100%", borders: "ALL", border_symbols: Rounded,
)])),
```

Next: filter column in the pane (decades or year range, Top %, genres) that runs `hits --json` itself.
