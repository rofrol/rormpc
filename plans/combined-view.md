# Combined Queue, Hits and Shuffle view (plan)

Design only, nothing here is built yet. Written 2026-10-09 from two consult rounds (GPT-6.1 Sol and Xiaomi MiMo,
see "Consult rounds"). The choices still open are at the end, each with options.

The tab this plan calls "Play" is labelled "Music" since 2026-10-10 (the user: "Play" looked like a button); the
pane type `Pane(Play())`, `ShowPlay(...)` and the action names keep "Play". The text below keeps its original wording.

## Goal (the user's words, translated)

> There should be one combined view now: Queue, Hits, Shuffle. So I can choose to prepare weighted. If I change the
> year in a filter as in Hits, a new list is prepared. If I turn weighted off, the normal view comes back. The things
> chosen in Hits, like year and genres, can be saved as a live list, like collections on Steam or in Calibre.

> Of course I can also remove or add songs by hand. Then something has to be done with that. I don't know what. A
> dynamic list but from a limited set?

> It can be exceptions. Pins and exclusions on the whole library or on a chosen subset such as Billboard 100. And
> those subsets like Billboard should be made the way genres are. They can be - or +.

## Today

Four tabs and a daemon do the work this view combines:

| Piece | Where | What it does |
|---|---|---|
| Queue | `rmpc/src/ui/panes/queue.rs` | MPD's queue in physical order; `/` filter (`rormpc_filter.rs`); context menu with Play next, Sources…, Tags…, Add to playlist… |
| Queue plan view | `rmpc/src/ui/rormpc_queue_plan.rs` (`o`, `TogglePlanView`) | A projection: `-2`,`-1` past plays, `0 ▶`, `↑n` Up next, forecast `1`..`10`, nothing after it (pool songs only as `/` matches); J/K patch the forecast through mpd-player with a plan version |
| Hits | `rmpc/src/ui/panes/hits.rs` | Filter column (`Filters`, `Source` enum: Billboard / my charts / whole library / my playlists / my likes / recommended; Period; Top %; genres and artists tri-state; owned; show hidden; Downloads) and a ranked table. Apply runs `hits ... --json ~/.cache/rormpc/hits/current.json` in a thread (`HitsPane::apply`) |
| Hits → queue | `rmpc/src/ui/rormpc_upnext.rs` `play_hits_source` → `confirm_replace_with` | "Play these N songs (as the source)…": after a confirmation deletes everything but the playing song, adds the owned rows, drops the playing song's duplicate, writes `$XDG_STATE_HOME/rormpc/source.json` `{kind: "hits", name, len, files}`. Moving a filter later never changes it ("browsing other results") |
| Sources… | same file, `open_sources` | The whole library or a stored MPD playlist replaces the queue the same way |
| Weighted shuffle | rormpc-tools `src/rormpc_tools/player/shuffle.py`; rormpc `rormpc_player.rs` (`toggle_shuffle`, `release_shuffle`, `shuffle_state`) | mpd-player draws 10 songs ahead (lanes 7 familiar / 2 rediscovery / 1 new, skips, rests) from the songs in the queue and publishes MPD priorities 10..1 with random on. `fill_plan` reads `source.json`: a `hits` source plays in rounds (each song of the snapshot once, `ensure_round` starts a new round on a new source) |
| Shuffle tab | `rmpc/src/ui/panes/shuffle.rs` | Timeline: last plays, playing, requests, the plan with why each was drawn |
| Up next | `rormpc_upnext.rs`, `panes/up_next.rs`; mpd-player `player/upnext.py` | Requests, MPD priorities 255.. above the plan |
| Live playlists | `panes/live_playlists.rs`; rormpc-tools `liveplaylist.py` | Followed YouTube playlists, state in `<data_dir>/liveplaylists/`, each written as an MPD `.m3u` |
| Tags | rormpc-tools `tags.py` | Hand-made lists as an event log `<data_dir>/collections.jsonl`, exported as MPD playlists "Tag NAME" |
| Hidden chart songs | rormpc-tools `hits.py` `hide_key`, `hidden_set` | `<data_dir>/hits-hidden.jsonl`, keyed `main artist|title` (a chart song, owned or not); applied after the Top % cut so ranks never move |
| Song identity | music-data `songs.jsonl`, `aliases.jsonl` | Every library song has an internal UUID `id` with its `paths`, md5, ytid; merges and moves are recorded as aliases |

`data_dir` is the private data repository music-data (`~/.config/rormpc-tools/config.toml`). The flow today: Hits
filters → `hits` CLI → JSON → (menu, confirmation) → rormpc rewrites the MPD queue + `source.json` → mpd-player
reads the queue and `source.json`, draws the plan, writes `shuffle.json` → Queue plan view, Shuffle tab and the
`ShuffleNext` column read `shuffle.json`.

## Design

### One tab, two modes

A new pane `Pane(Play())` in a tab "Music" (first called "Play") replaces Hits, Queue and Shuffle as the default tabs (`1`). It is the
filter column of Hits on the left and one table on the right. The old panes stay in the code and in explicit
configs; Up next and Live playlists stay separate tabs (both models: the request list must never be buried in a
filter-driven table).

- **Normal mode** (weighted off): the table is MPD's queue in its physical order, like today's Queue. The forecast
  column is empty, never stale. The filter column is collapsed to one line naming the source
  (`Source: 1980s · +rock · Top 10%`) and opens with `h` or a click.
- **Weighted mode** (`w`): the table is today's plan view: past plays, `0 ▶`, `↑n` requests, forecast `1`..`10`
  with the lane, and nothing after it (decided by the user 2026-10-10: the rest of the pool is reached through `/`
  and Browse). The filter column is open. A header line under the tab title
  always says what is on screen and why (MiMo):
  `▶ Playing: 1980s +rock Top 10% · weighted · round 12/84 · rest 2 · skip 1`.
- Turning weighted off (`w`) only sends `shuffle off` and switches the table to normal mode. **The queue stays the
  source that was applied**; nothing restores an older queue (both models: a shadow queue would be a second source
  of truth and lose phone edits, requests and hours of play). Going back to an earlier source is an explicit
  "Previous sources…" list (see Smart lists), confirmed like Sources… today.

### A filter change prepares, Apply plays

Moving a filter (a year, a genre, a set chip) recomputes a **preview**: `hits` runs as on today's Apply (~0.3 s),
into a preview file, and the table shows the preview with a banner. MPD is not touched. The banner counts before
anything is committed:

    Preview · 1985-1992 +rock Top 10% · 312 matched · 287 owned · 25 missing · playing song: outside  [Apply a]

`a` (Apply) or the banner's button replaces the queue with the preview's owned songs exactly as
"Play these N songs" does now: the playing song goes on, Up next is kept, `source.json` is rewritten, a new round
starts. Both models rejected an automatic replace on every change (destructive browsing, a queue-replace race on
each keystroke); a confirmation is shown only when the source kind changes or more than 25% of the queue would go.
Esc drops the preview and shows the playing source again.

- 0 owned songs: Apply is disabled ("widen the filter"); the queue is never emptied.
- Missing songs stay visible as dimmed rows in the preview and in the counts, never in the queue.
- The playing song outside the new source keeps playing, marked `▶ off-source`, and is not counted in the new
  round (both models).
- Up next requests outside the source stay and are marked as requests.
- Re-applying the same rules (same canonical rules hash) keeps the round; a different source starts a new one.
- Race rule (MiMo): Apply carries the queue version it previewed against (MPD `playlist` version); the replace
  re-reads it first and refuses with "the queue changed, preview again" when it differs. No retry, no delay.

In normal mode the same preview works: Apply replaces the queue, the queue plays in order (random off), nothing is
drawn.

### Sets as chips, like genres

The `Source` cycle becomes **set chips**, each tri-state like a genre: `+` include, `-` exclude, off. The fixed
kinds always have a row; user-made sets are added through a picker (both models: hundreds of chips do not fit a
30-column column):

- fixed: Billboard US (year-end Hot 100), my likes, my playlists (all stored, non-generated), recommended;
- through "+ set…": each tag list ("Tag God"), each stored MPD playlist, each followed YouTube playlist, each saved
  smart list (a smart list can be a set of another one; a cycle is refused).

Selection, printed under the chips so it is never guessed (MiMo):

    candidates = (union of + sets, or the whole library when no set is +)
               - (union of - sets)
               ∩ genres (+ any of, - none of) ∩ artists (same) ∩ Period ∩ Top % ∩ owned
    then exceptions (below)

    (Billboard ∪ Likes) − Tag Christmas ∩ rock · 1,204 of 8,312

Ranking and years were part of `Source` and are not memberships (both models), so they become their own rows:

- **Rank by**: Billboard position · my plays · rediscover (often played, not lately) · none. Top % needs a rank and
  is computed in the rank's own population (the Billboard cohort of the chosen years, my played songs), before the
  genre and artist filters as today, so a song's rank never depends on which chips are on. With Rank by none
  (recommended, a playlist) Top % is disabled. Several + sets do not merge rankings: a song outside the rank's
  population has no rank and is kept only when Top % is off.
- **Years of**: release year · chart year · listened year. Default follows Rank by (Billboard → chart year, my
  plays → listened year, else release year), shown and changeable. "my charts" today = Rank by my plays +
  Years of listened.

The `hits` CLI gets the same model (`--set +billboard --set -tag:Christmas --rank plays --years-of release`); the
old `--source` stays as a shorthand that maps onto it, so old `current.json` files and saved args keep loading.

### Exceptions: pins and exclusions with a scope

A song added or removed by hand becomes an **exception** only when asked for; a plain queue remove ("Remove from
queue") stays a one-off and is undone by the next Apply (both models: never infer a permanent exclusion from an MPD
delete; a phone's delete is not a decision).

- **Pin** `✚`: the song is in, whatever the filters say ("include unless excluded"). It needs an owned file
  (Sol). Pins have no rank (`—`), sit after the ranked rows, play with their normal shuffle weight, and are not
  part of `hits`' ranking or Top % cut, so they never change other songs' ranks or "my charts" (MiMo).
- **Exclusion** `⊘`: the song is out, whatever else says.
- **Scope**: `library` (every list) or one set (`Billboard`, `Tag God`, a smart list "80s party"). A set-scoped
  exception applies only while that set is `+` in the current selection (Sol); with the chip off or `-` it does
  nothing. A smart list's own fixes are exceptions scoped to that list (Sol; MiMo's reproducibility worry is met by
  showing them as part of the list: "80s party = rules + 3 pins + 2 exclusions").
- **Precedence**, one rule: **any applicable exclusion wins; otherwise any applicable pin includes; otherwise the
  chips and filters decide.** Among exclusions or among pins the scope does not matter (all exclude, all include).
  Both models put every exclusion above every pin. They differ on whether a `-` chip or `-` genre beats a pin (Sol
  yes, MiMo puts pins above filters); this plan takes the user's word "exceptions": an exception is an exception to
  the rules, so a pin beats a `-` genre or `-` set. Open choice 6 asks it.
- `hits hide` already is a set-scoped exclusion: it is keyed by chart song (`main artist|title`), covers missing
  rows and applies to every Billboard year. It becomes "exclusion · Billboard" in this model and keeps its log
  (Sol; MiMo's "merge it into library exclusion" would broaden it to songs that are not chart rows).
- "This file is the 1987 chart song" is a match, not a pin: it stays with the Hits fetch / match correction ("Accept
  as the chart song"), so the file gets the chart rank and year (answers MiMo's per-chart-instance point).
- Excluded songs are hidden but reachable: "show excluded (N)" puts them back as dim ghost rows with `⊘` and the
  scope, like "show hidden" today (MiMo).

Identity: an exception is keyed by music-data's internal song `id` (UUID in `songs.jsonl`, which follows moves and
merges through `aliases.jsonl`); an exclusion of a missing chart song uses the chart key like `hits hide`. Each
event carries its own UUID; folding takes the file order, and a git merge of two machines' appends keeps both lines
(the later line wins, as `collections.jsonl` and `hits-hidden.jsonl` work today). `musicdb dedupe`/merge rewrites
nothing: the alias leads the old id to the kept song.

### Smart lists

A saved set of rules is a **smart list** (both models: "live list" collides with Live playlists, "collection" with
the tags' `collections.jsonl`; open choice 1 keeps the user's word if wanted).

- Stored in music-data as an event log `smartlists.jsonl`: `{id (UUID), event: create|update|rename|delete, name,
  schema: 1, rules: {sets: {"billboard": 1, "tag:Christmas": -1}, rank, years_of, period, top, genres, artists,
  owned}}`. Rules are semantic fields, not `hits` argv (Sol); a rule this version cannot read blocks the list with
  "made by a newer rormpc-tools, update it" and never runs with a field dropped (both models).
- Exceptions scoped to a list are in the same exceptions log with `scope: "list:<id>"`.
- Saving (`S`) asks for a name with the rules shown; a name that exists offers "Update 80s party" or a new name.
- The picker (`L`) lists smart lists, then stored MPD playlists and Live playlists in their own sections (different
  kinds, different marks). Enter loads a list's rules into the filter column as a preview (it never plays by
  itself); Apply plays it. `r` rename, `d` delete (confirmed; its exceptions go with it), `u` update with the
  filters on screen, `c` duplicate.
- Export: `musicdb update` (hourly) and Apply write each smart list as an MPD playlist "Smart NAME" for phones,
  with the time in the picker ("exported 3 h ago"). It is a snapshot, never read back as rules. `hits`' "my
  playlists" set leaves "Smart " playlists out (add it to `GENERATED_PLAYLISTS`), or a list would feed on itself
  (Sol).
- "Previous sources…" in the picker: the last 10 applied rule sets (from `source.json` revisions, local state, not
  the data repo), so "weighted off, now give me the old queue back" is one explicit Apply.

### Keys

| Key | Where | Action |
|---|---|---|
| `w` | everywhere | weighted on/off (as today); `x` plain random (as today) |
| `h` / `l` | Play | filter column / table (as in Hits) |
| `a` | Play | Apply the preview (today `a` appends a Hits row; in Play the row menu keeps "Add to queue") |
| `Esc` | Play | drop the preview |
| `S` | Play | save the filters as a smart list |
| `L` | Play | smart list picker |
| `+` / `-` | Play row | pin… / exclude… (scope menu, default: the open smart list, else library); the same keys cycle a chip on a set row |

`o` stays TogglePlanView for the old Queue pane; in Play the table follows `w`. Checked against
`~/.config/rormpc/config.ron` on 2026-10-09: `S`, `L`, `+`, `-` are free there, while `p` (TogglePause), `P` (sort
by plays) and `a` (Add) are bound, so `a` is rebound only inside the Play pane. `assets/example_config.ron` binds `L`
to SelectAlbum in the Queue; Play is a different pane, so it does not clash, but the implementation checks both.

## Mockups

### 1. Normal mode (weighted off)

```
 Play ─ Source: 1980s · +Billboard · rock · Top 10%  [h: filters] ──────────────────────────
  #    ♥  Artist               Title                         Album            Time
  1       Toto                 Africa                        Toto IV          4:55
  2    ♥  a-ha                 Take On Me                    Hunting High…    3:46
▶ 3       Tears for Fears      Everybody Wants to Rule…      Songs from…      4:11
  4  ✚    Kombi                Słodkiego miłego życia        Kombi 4          4:02
  5       Madonna              Like a Prayer                 Like a Prayer    5:39
  …
 ── 84 songs · in queue order · w: weighted · L: lists · S: save ───────────────────────────
```

### 2. Weighted mode with the filter column and a preview

```
 Play ─ ▶ Playing: 1980s +Billboard rock Top 10% · weighted · round 12/84 · rest 2 ─────────
 Downloads (2 to review) │ Preview · 1985-1992 · 312 matched · 287 owned · 25 missing   [a Apply]
 Sets                    │ Next  Lane   Artist             Title                    Why
  + Billboard US         │  -2   ✓      Toto               Africa                   played to the end
    my likes             │  -1   ⏭      Madonna            Like a Prayer            skipped early
  - Tag Christmas        │   0 ▶        Tears for Fears    Everybody Wants to Rule… (off-source)
    my playlists         │  ↑1          Kombi              Słodkiego miłego życia   your request
    recommended          │   2   fam    a-ha               Take On Me               plays 41 · ♥
  + set…                 │   3   fam    Queen              Radio Ga Ga              overdue 1.6x
 Rank by  Billboard      │   4   redis  Cutting Crew       (I Just) Died in Your…   played 2 y ago
 Years of chart year     │   5   new    Level 42           Lessons in Love          never heard
 Period  1985 – 1992  ‹› │   …
 Top %  [x]1-10 [ ]11-20 │  10   fam    Kombi              Black and White          plays 12 · ✚ pinned
 Genres                  │
  + rock   - country     │
 (Billboard) − Christmas │
   ∩ rock · 287 of 8,312 │
 [ Apply ] • changed     │
```

Nothing follows the forecast: a song outside it shows only as a `/` match (turn `·`), with its row actions. The
rule summary under the genres is the printed formula.

### 3. Save as smart list (`S`)

```
╭─ Save as smart list ─────────────────────────────────────╮
│ Name: 80s party_                                         │
│                                                          │
│ Sets      + Billboard US   - Tag Christmas               │
│ Rank by   Billboard · Top 1-10%                          │
│ Years of  chart year 1985-1992                           │
│ Genres    + rock  - country                              │
│ Owned     287 of 312 (25 missing stay in the rules)      │
│ Exceptions on screen: 1 pin (library), 1 excl. (Billboard)│
│   they stay where they are; new fixes go to this list    │
│                                                          │
│ [x] Export as MPD playlist "Smart 80s party"             │
│                                  [ Cancel ]  [ Save ]    │
╰──────────────────────────────────────────────────────────╯
```

### 4. List picker (`L`)

```
╭─ Lists ──────────────────────────────── / search ───────╮
│ Smart lists                                            │
│ ▶ 80s party        287 · +2 pins −1   exported 3 h ago │
│   Rediscover rock  412 · rank: rediscover              │
│   Polish 90s       96  · ! made by a newer tools       │
│ Previous sources                                       │
│   1980s Billboard Top 10%           today 21:14        │
│   Whole library                     yesterday          │
│ MPD playlists                                          │
│   Tag God (24)   melancholic (41)   …                  │
│ Live playlists (YouTube)                               │
│   Discover Weekly copy (55)                            │
│ Enter load · a apply · r rename · u update · d delete  │
╰────────────────────────────────────────────────────────╯
```

### 5. Exceptions per scope (`+` / `-` on a row, and the list)

```
╭─ Pin "Black and White" (Kombi) ─────────────╮   ╭─ Exceptions (8) ───────── scope: all ▾ ─╮
│ ▸ in every list (library)                   │   │ Library                                │
│   in Billboard US only                      │   │  ✚ Kombi · Black and White             │
│   in the smart list "80s party"             │   │  ⊘ Crazy Frog · Axel F                 │
│   Cancel                                    │   │ Billboard US                           │
╰─────────────────────────────────────────────╯   │  ⊘ Phil Collins · Another Day in Par…  │
                                                  │  ⊘ Steve Winwood · Roll with It (hide) │
 Row marks: ✚ pinned · ⊘ excluded, scope letter   │ 80s party                              │
 in the details: "pinned · library",              │  ✚ Lady Pank · Mniej niż zero          │
 "excluded · Billboard (hits hide)"               │  ! ✚ Maanam · Kocham cię… (file gone)  │
                                                  │ Enter: remove the exception · / search │
                                                  ╰────────────────────────────────────────╯
```

### 6. Set chips, the three states

```
 Sets                      click / Space cycles  off → + → - → off
  + Billboard US           (bold, include)
  - my likes               (red, exclude)
    my playlists           (dim, off)
  + Tag God                (added through "+ set…")
  + set…                   picker of tags, playlists, Live playlists, smart lists
  × clear sets             (dim while none is + or -)
```

## Behaviour rules and edge cases

- The playing song is never stopped, restarted or replaced by Apply, a filter change, an exception or `w`.
- Only Apply changes the queue; a preview never does. A filter change never starts `hits` twice at once (today's
  queued-run rule in `HitsPane::apply`).
- Weighted off keeps the queue; the forecast column empties at once (no stale plan); the plan's priorities are
  cleared by mpd-player as today.
- Up next is kept across Apply and mode changes and always plays first.
- A pinned song whose file is gone shows `!` in the exceptions list and is skipped with a note; it is never
  silently dropped from the log.
- An exclusion added while the song plays takes effect after it ("excluded · plays to the end").
- Pins in a Hits round play once per round like every member; the round total counts them.
- `hits` writes the preview to its own file; `current.json` keeps meaning "the result on screen" so the Hits pane
  keeps working in explicit configs.
- A smart list using another smart list as a set is evaluated recursively with a cycle check; a cycle is an error
  shown in the picker, not a crash.
- Phones and mpc keep working: the queue stays the truth; a phone's delete is a one-off until the next Apply.

## What changes where

rormpc:
- new `rmpc/src/ui/panes/play.rs` (and a `rormpc_play/` module if it grows) composed from the Hits filter column,
  the Queue table and the plan projection; registered in `panes/mod.rs` and `config/tabs.rs` with a few lines
  only, as the fork's rule says;
- `hits.rs` `Filters`: sets as tri-state rows, Rank by, Years of, rule summary; `Source` kept as an alias;
- preview file + Apply through `confirm_replace_with` extended with the queue version check and the rules hash in
  `source.json`;
- exception actions, marks, "show excluded", exceptions list; smart list save modal and picker;
- default config: tab "Music" (first called "Play") first, Queue/Hits/Shuffle tabs removed from the default (explicit configs untouched).

rormpc-tools:
- `hits`: `--set ±KIND[:NAME]`, `--rank`, `--years-of`, exceptions applied after the Top % cut, `--rules FILE`
  (a smart list's rules JSON), `--preview` output; `--source` mapped onto the new options; contract test update
  (`tests/test_rormpc_contract.py` and rormpc's side) because the JSON gains fields;
- `hits except pin|exclude|remove --scope library|set:KIND[:NAME]|list:ID` and `hits exceptions --json`, log
  `<data_dir>/exceptions.jsonl`; `hits hide` keeps its log, read as Billboard-scope exclusions;
- `hits lists` (or `musicdb smartlist`): create/update/rename/delete/export, log `<data_dir>/smartlists.jsonl`;
  `GENERATED_PLAYLISTS` gains "Smart ";
- mpd-player: round key = the rules hash from `source.json` instead of `hits:<name>`; the off-source playing song
  is outside the round; nothing else changes (it still draws from the queue).

## Build order

1. `hits` set chips, Rank by, Years of (CLI + Hits pane), `--source` kept as shorthand. Useful alone in today's
   Hits tab.
2. Exceptions: CLI + log, marks and actions in Hits and Queue, "show excluded", exceptions list.
3. Play pane: Queue table + plan projection + collapsed filter column; preview and Apply with the queue version
   check; header line. Default config switches to it.
4. Smart lists: save, picker, load as preview, Previous sources, MPD export.
5. Sets from tags, playlists, Live playlists and smart lists in "+ set…"; cycle check.

Each phase ships on its own; the old tabs stay usable until phase 3 is in the default config. Phase 3b (below,
"Play absorbs the browsing tabs") comes right after phase 3 and before phase 4.

## Play absorbs the browsing tabs

Added 2026-10-10. The user decided on 2026-10-09: "only Play". Artists, Album Artists, Albums, Directories,
Playlists and Live playlists leave the tab bar and their tasks move into Play. Search, Up next, Lyrics, Deleted and
Versions are not affected. Inputs: the coordinator's round (Sol `a91fcbfc`, MiMo `0fd6ccf8`, recorded in TODO.md)
and round `20261010-013604-3fd0` (Sol `c6f273c4`, MiMo `626603d0`) on placement and on how `w` and Apply interact
with browsing.

### What the code already has

| Piece | Where | What Play reuses |
|---|---|---|
| `BrowserPane<T>` trait | `rmpc/src/ui/browser.rs` | Navigation, `/` search in a column, Space marks, `a` Add, `A` AddAll, `D` Delete, Ctrl-r Rename, J/K MoveUp/MoveDown, the context menu (Add to queue, Play next (Up next), Replace queue, Create playlist, Add to playlist, Rename). Enter on a song plays it now without replacing the queue (`rormpc_upnext::play_now`); on a directory it drills down |
| Browser widget | `rmpc/src/ui/widgets/browser.rs` | Three Miller columns (`BrowserArea::Previous`, `Current`, `Preview`) plus a scrollbar. It needs width: at 30 columns each column gets about 10 |
| Artists, Album Artists, Albums | `panes/tag_browser.rs`: one struct `TagBrowserPane::new(levels, target_pane)` | Built three times in `panes/mod.rs` (`PaneContainer::new`) with different tag levels: Artist > Album (date split, `artists.album_sort_by`), albumartist > Album, Album |
| Directories | `panes/directories.rs` (`DirectoriesPane`) | The same trait over MPD's `lsinfo`: folders, then files in filename order |
| Playlists | `panes/playlists.rs` (`PlaylistsPane`) | Already an editor: J/K `move_in_playlist`, `D` deletes a song from the playlist or a whole playlist, Ctrl-r renames. Each edit is an immediate MPD command |
| Live playlists | `panes/live_playlists.rs` (`LivePlaylistsPane`) | Self-contained: subscriptions on the left, items on the right with decision (pending / accepted / rejected) and job status, a footer with the worker's progress, the menu (add URL, check, accept, reject, accept all pending, download, cancel). Every action is `liveplaylist ... --json` in a background thread. `Sub::pending()` already counts pending items |

MPD query results reach a pane by its `PaneType` target (`ctx.query().target(...)`, dispatched in
`ui/mod.rs` `on_command_finished` through `PaneContainer::get_mut`), and every browser uses the same string ids
(`INIT`, `FETCH_DATA`). Both models of the second round flagged that forwarding results to "the active child" is
wrong: switch from Artists to Folders before a reply arrives and it lands in the wrong list. So each child of Play
gets its own target, `PaneType::PlayBrowse(Grouping)` (and `PaneType::PlayLive`), which `PaneContainer::get_mut`
maps to the Play pane, and Play hands the result to that child, active or not. These variants are internal: they
are not parsed from the config.

### Layout: full-width bodies

Play gets three **bodies**, each using the whole area under the header line: **Queue** (phase 3: the queue or the
plan with the filter column), **Browse** and **Live**. One body is shown at a time; the header line always names the
body, the playing song, the weighted state and the Live pending count.

Neither earlier placement survives the code (both models of round `20261010-013604-3fd0`, independently):
- Sol's left column (Build | Browse) would squeeze the three-column browser into about 30 columns (about 10 per
  column, so `Loveless (1991)` is cut), which means a new narrow browser instead of reuse; the right table would mean
  three things (selection, preview, queue).
- MiMo's right-pane modes keep the filter column next to Browse and Live, where it means nothing, and leave it 30
  columns narrower.

Full-width bodies reuse `TagBrowserPane`, `DirectoriesPane`, `PlaylistsPane` and `LivePlaylistsPane` unchanged
inside Play: Play owns one instance of each (created on first use, kept afterwards so each grouping keeps its path,
cursor and marks), renders the active one into the body area and passes it the actions. The filter column belongs
to the Queue body only; Browse and Live never show it, so `h` in Browse is always "parent column" (MiMo).

**Groupings** of the Browse body, in a chip row above the columns (clickable): **Artists · Album artists · Albums ·
Folders · Lists**. Artists and Album artists stay separate (MiMo, both rounds; they are different tag levels and
the code has both). "Lists" is the stored MPD playlists, the `PlaylistsPane`. Smart lists stay in the `L` picker
(phase 4); a later phase may add them as a section of Lists.

### Browse actions: play, play next, append, never Apply

Browse never touches the preview or Apply. Its actions act on the queue at once:

| Action | Key | On a song | On a container (artist, album, folder, playlist) |
|---|---|---|---|
| Play now | Enter | plays it now, the queue stays (today's `play_now`) | drills down (as today) |
| Play, replacing the queue | `P` | the song's album from this song | replaces the queue with it in source order and starts it (below) |
| Play next | `t` | Up next request (`rormpc_upnext::play_next`) | every song of it, in source order, as Up next requests |
| Append | `a` / `A` | adds to the end of the queue (today's Add / AddAll) | the same, every song in source order |
| Add to playlist… | menu | as today | as today |

`P` (Play) on a container is the "play one album in track order" task:
- the queue is replaced the way Sources… does it (`confirm_replace_with`), but the chosen collection **starts at
  once** (Sol: say so in the confirmation, unlike Apply, which lets the playing song finish). Up next requests stay
  and play first;
- weighted shuffle goes off (`shuffle off`: mpd-player clears its priorities) and random goes off, so MPD plays the
  queue in order; both models: priorities outrank position order, so "play this album" with weighted on would lie;
- source order: an album in disc/track order, a folder in filename order, a playlist in list order, an artist in
  album (date) then track order;
- `source.json` gets the kind and a stable name: `{kind: "album", name: "<albumartist> / <album> (<date>)"}`,
  `"directory"` (the folder path), `"playlist"` (the playlist name), `"artist"`, with `files` in source order. These
  are new kinds; the header's "modified" check (`source.json` `len`) keeps working;
- the confirmation follows Apply's rule (only when the source kind changes or more than 25% of the queue would go;
  Sol: Browse must not be a destructive bypass of that rule). MiMo saw a contradiction with "no Apply"; there is
  none: "no Apply" means no preview step, not no confirmation. With the queue holding a Hits source this always
  confirms; "Previous sources…" (phase 4) brings the old source back. Open choice 14 asks whether to confirm.
- `w` afterwards weights the album's songs (it draws from the queue as always); it never rebuilds a Hits result.

Append on a Hits source: mpd-player draws a Hits source only from `source.json` `files`
(`shuffle.py` `fill_plan`: `base` is cut to `members`), so a song appended with weighted on is **never drawn**
today. Both models expected it to join the pool. This plan adds the appended files to `source.json` `files` and
marks the source "+N added" (Sol: a Hits source with additions is no longer exactly its filter result); the next
Apply drops them like any hand edit. Open choice 15.

Play next (`t`) needs nothing new: Up next requests already sit at MPD priorities 255.. above the plan's 10..1 and
mpd-player re-finds them after a queue change (MiMo's priority race is already handled there).

An open preview (Queue body) survives a Browse action: the filters stay, the banner's counts are recomputed against
the changed queue (its MPD `playlist` version moved), and Apply's confirmation is judged at Apply time against the
queue as it is then (Sol). MiMo would drop the preview; Apply's version check already refuses a stale preview, so
keeping it loses nothing. Open choice 19.

**Order** is not a new selector (Sol: Source / Ranked / Weighted in one control conflicts with "weighted off keeps
the queue", which cannot restore an order it never kept). The order is what the queue holds: Apply adds a Hits
result in rank order, `P` adds a collection in source order, `w` draws on top of either. The header line says it:
`in album order`, `in rank order`, `weighted · round 3/84`.

### Stored playlist editor

Browse › Lists › a playlist is the editor; `PlaylistsPane` already does the work:
- J/K move the song (marked songs move together), `D` removes it from the playlist, Space marks, `a`/`P`/`t` as
  above;
- at the Lists level: Ctrl-r renames, `D` deletes the playlist, the menu's "Create playlist" makes one;
- edits stay **immediate MPD commands**, as today (both models of the second round, reversing the first round's
  "explicit save"): phones see them at once, a phone's concurrent edit is not overwritten by a later save, nothing
  is lost on a crash, and "Add to playlist…" already writes at once. The editor's title says
  `Editing "melancholic" · changes are saved at once`. Open choice 16;
- new: deleting a whole playlist asks for a confirmation (today `D` at the Lists level deletes at once); a failed
  MPD command is reported and the cursor stays.

Not editable here: "Tag NAME", "Smart NAME" and Live playlist `.m3u` files are generated (tags, smart lists,
`liveplaylist`) and rewritten by their owner; the editor shows them read-only with a note where to change them.

### Live inbox

The Live body is `LivePlaylistsPane` as it is: subscriptions | items with decision and job status, the worker's
progress in the footer, the same menu (add URL, check for new tracks, accept, reject, accept all pending, download,
cancel). It never touches the queue. Two additions:
- **multi-select**: Space marks items; accept and reject act on the marked items (today one item, or all pending);
- **the pending badge**: the header line of every body shows `Live 3` when items wait for a decision (the sum of
  `Sub::pending()`), and `↓ 2` while downloads run; clicking it opens the Live body. Play reads the same
  `liveplaylist list --json` and `status.json` as the pane, on show and on the worker's notifications, never on a
  timer of its own.

The pane's job surface stays whole: per-item errors, cancel, the worker's last stderr line (MiMo: do not shrink it
to a badge).

### Keys

| Key | Where | Action |
|---|---|---|
| `B` | Play | Queue body ↔ Browse body (the last grouping) |
| `[` / `]` | Browse | previous / next grouping (also a click on the chip row) |
| `5` `6` `7` `8` `9` | everywhere | Play › Browse › Folders, Artists, Album artists, Albums, Lists (the digits these tabs had in the user's config) |
| `0`, `gl` | everywhere | Play › Live (as before) |
| `Esc` | Browse/Live root | back to the Queue body (inside a column `Esc` keeps clearing marks and the search first) |
| `P` | Browse | Play, replacing the queue (above) |
| `t` | Browse | Play next (Up next) |
| `a` / `A` | Browse | Append / append all (as in every browser today) |
| `/`, `n` / `N`, Space, J/K, `D`, Ctrl-r | Browse | as in today's browsers |

The digits and `gl` become a new global action `ShowPlay(body)` (e.g. `ShowPlay(Browse(Artists))`,
`ShowPlay(Live)`), so the old muscle memory survives the removed tabs (both models of the coordinator's round:
keep keys and a click target).

Checked on 2026-10-10 against `~/.config/rormpc/config.ron` and `assets/example_config.ron`: `B`, `[`, `]` and `t`
are free in both (`b` is SeekBack, `zt` only exists as a `z` prefix in the example config). `p` (TogglePause) and
`n` (NextResult) are bound, so play and play next do not use them. `P` is SortByColumn(4) in the user's queue map:
`P` in Browse is a navigation-map action claimed only by the Browse body, and the Queue body keeps sorting by
plays; the implementation checks that both maps reach Play in one `ActionEvent` and that each body claims only its
own. `a` stays Append in Browse and is Apply only in the Queue body (both models: state the body in the header,
since `a` means two things). `L` (smart list picker) vs the example config's queue `SelectAlbum` is the existing
note in "Keys" above. Tab / Shift-Tab are NextTab / PreviousTab in both configs, so they do not switch bodies.

### Mockups

#### 7. Browse body, drilled into an album

```
 Play ─ Browse · ▶ Toto – Africa · weighted · round 12/84 · Live 3 ─────────────────────────────────────
  Artists   Album artists  [Albums]  Folders   Lists                        / search · [ ] grouping
 ┌ Albums ─────────────────────┬ Toto IV (1982) ──────────────────┬ Preview ────────────────────────┐
 │  The Seventh One (1988)     │   1  Rosanna               5:31  │ Africa                          │
 │  Isolation (1984)           │   2  Make Believe          3:43  │ Toto · Toto IV (1982)           │
 │▸ Toto IV (1982)             │   3  I Won't Hold You Back 4:56  │ Track 10 · 4:55 · FLAC          │
 │  Hydra (1979)               │   …                              │ plays 41 · ♥ · in queue         │
 │  Turn Back (1981)           │ ▸10  Africa                4:55  │                                 │
 └─────────────────────────────┴──────────────────────────────────┴─────────────────────────────────┘
 Enter play now · P play album (replaces the queue, album order) · t play next · a append · Esc queue
```

`P` here asks (the queue holds a Hits source):

```
╭─ Play "Toto IV" (10 songs)? ─────────────────────────────────╮
│ The queue (84 songs of 1980s +Billboard rock) is replaced by │
│ the album in track order, starting now with "Rosanna".       │
│ Weighted shuffle and random go off. Up next (1) plays first. │
│                                    [ Cancel ]  [ Play ]      │
╰──────────────────────────────────────────────────────────────╯
```

#### 8. Stored playlist editor (Browse › Lists)

```
 Play ─ Browse · ▶ Toto – Africa · in album order · Live 3 ──────────────────────────────────────────────
  Artists   Album artists   Albums   Folders  [Lists]                       / search · [ ] grouping
 ┌ Lists ──────────────────────┬ Editing "melancholic" · changes are saved at once ┬ Preview ──────┐
 │  Billboard … (generated)    │   1  Radiohead       No Surprises          3:49   │ Hurt          │
 │▸ melancholic (41)           │   2  Portishead      Roads                 5:05   │ Johnny Cash   │
 │  Not finished (3)           │ ● 3  Johnny Cash     Hurt                  3:38   │ American IV   │
 │  Tag God (generated)        │ ● 4  Massive Attack  Teardrop              5:29   │ 2002 · 3:38   │
 │  roadtrip (112)             │   5  Sade            By Your Side          4:34   │               │
 └─────────────────────────────┴───────────────────────────────────────────────────┴───────────────┘
 2 marked · J/K move · D remove from playlist · Ctrl-r rename · P play the list · t play next · a append
```

#### 9. Live body with the badge

```
 Play ─ Live · ▶ Toto – Africa · weighted · Live 3 · ↓ 2 ─────────────────────────────────────────────────
 ┌ Followed playlists ─────────┬ Discover Weekly copy · 55 items · checked 2 h ago ──────────────────────┐
 │▸ Discover Weekly copy   3 ● │ ● pending      Fontaines D.C.   Favourite            —                  │
 │  KEXP live              0   │ ● pending      Wet Leg          Chaise Longue        —                  │
 │  Polish indie           0   │   pending      Bad Bunny        Tití Me Preguntó     —                  │
 │                             │   accepted     Phoenix          Lisztomania          downloading 61%    │
 │                             │   accepted     MGMT             Kids                 needs match        │
 │                             │   rejected     Crazy Frog       Axel F               —                  │
 └─────────────────────────────┴─────────────────────────────────────────────────────────────────────────┘
 2 marked · Enter menu: accept · reject · accept all pending · check · download · cancel · Esc queue
 yt-dlp: [download] 61.0% of 4.12MiB at 1.2MiB/s
```

The `Live 3` in the header shows in every body (also Queue and Browse) and opens this body when clicked.

### Default config

- `assets/example_config.ron` and the built-in default (`config/tabs.rs`, which also has a debug-only Logs tab):
  tabs **Music (first called Play), Up next, Search**. The Directories, Artists, Album Artists, Albums and Playlists tabs leave (Live
  playlists is no default tab today); their digits become `ShowPlay(...)` and the remaining tabs renumber (`1`
  Music, `2` Up next, `3` Search), as the coordinator's round warned. The example config has the browsing tabs on
  `3`..`7`; the new map uses the user's `5`..`0` scheme from "Keys" above, so both configs agree.
- The panes stay: `Pane(Artists)`, `Pane(Directories)`, `Pane(LivePlaylists())` and the others still load from an
  explicit config, and `SwitchToTab("Artists")` keeps working where such a tab exists. A config naming a tab that
  no longer exists in the default is untouched (configs are explicit, nothing is merged).
- The user's own `~/.config/rormpc/config.ron` (dotfiles) has an explicit tab list and digit map, so nothing changes
  there by itself; open choice 18 asks whether the coordinator mirrors the new default into it.

### Build order: phase 3b

Phase 3 builds Play with the Queue body and switches the default config from Queue/Hits/Shuffle to Play. **Phase
3b** follows it, before phase 4:

1. `PaneType::PlayBrowse(Grouping)` / `PlayLive` targets and their mapping to the Play pane; Play owns the five
   browsers and the Live pane, created on first use; bodies and `B`, `[`, `]`, `Esc`.
2. Browse actions `P` (replace in source order, weighted and random off, new `source.json` kinds, Apply's
   confirmation rule), `t`, `a`; append to a Hits source joins its files.
3. Editor: confirmation before deleting a playlist, generated playlists read-only, the "saved at once" title.
4. Live: multi-select, the pending/download badge in the header.
5. `ShowPlay(body)` and the default config without the six tabs; example config and docs.

Each step ships on its own; the old tabs stay in the default config until step 5.

## Consult rounds

Round `20261009-230419-b57c` (layout, modes, filter change, weighted off, smart lists, manual edits): Sol
`151af75f`, MiMo `8b072df9`. Round `20261009-230647-3341` (scoped exceptions, ± set chips, precedence, TUI):
Sol `ec4b760c`, MiMo `50bbe9e4`.

Agreed (both models): explicit Apply over automatic replace; weighted off keeps the current queue; the name "smart
lists" in music-data with versioned semantic rules; exceptions only on an explicit action, keyed by a stable song
id; set algebra union of `+` sets minus `-` sets intersected with attribute filters; Top % needs one declared
ranking and is undefined across several; year axes must stay distinct; every exclusion beats every pin; fixed chip
kinds plus a picker; the playing song outside a new source is out of the round.

Diverged: freezing a list (Sol: offer "freeze as static playlist"; MiMo: freeze is a trap, prefer pins) — this plan
has no freeze; a static copy is the MPD export. Smart-list-scoped exceptions (Sol yes, MiMo no) — taken, shown as
part of the list. `hits hide` (Sol: a chart-scope exclusion; MiMo: merge into library exclusion) — Sol's, because
the hide key is a chart song. Pins vs negative filters (Sol: `-` filters beat pins; MiMo: pins beat filters) —
open choice 6.

Dismissed: MiMo's "the forecast is not committed with random on" (MPD plays by priority, which is why the plan is
published as priorities) and a hard cap of 20 pins (no evidence; the exception count is shown instead).

Round `20261010-013604-3fd0` (Play absorbs the browsing tabs: placement, Browse vs `w`/Apply, editor, keys): Sol
`c6f273c4`, MiMo `626603d0`. Agreed (both): full-width bodies, not the left column (the three-column browser does
not fit 30 columns) nor right-pane modes; forwarding query results to the active child is a race, give each child
its own target; playing a collection must turn weighted and random off or "album order" is a lie; keep immediate
playlist edits (a save buffer loses phone edits and adds a conflict model); appended songs must join the weighted
pool (the code does not do it for a Hits source: verified in `shuffle.py`); `a` means Append in Browse and Apply
only in the Queue body. Sol alone: Order as one selector conflicts with "weighted off keeps the queue" (taken: no
selector, the header names the order); Browse must not bypass Apply's confirmation rule (taken); a Hits source
with additions is marked modified (taken). MiMo alone: Browse never shows the filter column so `h` keeps one meaning
(taken); keep the Live job surface whole (taken); confirm deleting a whole playlist (taken). Diverged: an open
preview after a Browse action (Sol keep and recount, MiMo drop): open choice 19. Dismissed: MiMo's priority race
between Up next and the plan (requests already sit at 255.. above the plan's 10..1 and mpd-player re-finds them),
MiMo's Enter = play container (Enter drills down in every browser today and `P` covers it), Sol's Tab/Shift-Tab for
bodies (bound to NextTab/PreviousTab in both configs), MiMo's "digits vs `r` rate prompts" (a modal takes the keys
while open).

## Open choices

1. What is a saved filter set called in the UI?
   Options: Smart list (recommended, no clash with Live playlists or tags) | Live list (the user's word) | Preset
2. Does a filter change wait for Apply, or replace the queue at once?
   Options: Preview, Apply plays (recommended) | Replace at once when weighted is on | Replace at once always
3. When does Apply ask for a confirmation?
   Options: Only when the source kind changes or more than 25% of the queue goes (recommended) | Always | Never
4. What happens to the old Queue, Hits and Shuffle tabs?
   Options: Play replaces them in the default config, the panes stay for explicit configs (recommended) | Keep all four tabs | Remove the old panes
5. Where is the filter column in normal mode?
   Options: Collapsed to one source line, h opens it (recommended) | Always open | Hidden
6. Does a pin beat a `-` set or `-` genre?
   Options: Yes, an exception beats every rule (recommended) | No, a `-` rule beats pins (Sol) | Only a library-scope pin beats it
7. What do a hand removal and a hand addition in the queue do by default?
   Options: Nothing lasting, pin/exclude only on + / - (recommended) | Ask each time: just now or exclude/pin | Always record an exception in the open smart list
8. Default scope of a new pin or exclusion?
   Options: The open smart list, else library (recommended) | Always library | The first + set
9. Are smart lists exported as MPD playlists "Smart NAME"?
   Options: Yes, per list, on by default (recommended) | Only when asked | Never
10. Which sets get a fixed chip row?
    Options: Billboard, my likes, my playlists, recommended; the rest via "+ set…" (recommended) | Every tag and playlist as a row | Only Billboard
11. Which "years" does Period filter by default?
    Options: Follow Rank by (Billboard chart year, my plays listened year, else release year) (recommended) | Always release year | Always ask
12. Is there a "freeze as static list" action?
    Options: No, the MPD export is the static copy (recommended) | Yes, freezes into a tag list | Yes, into an MPD playlist
13. Where do Browse and Live go inside Play?
    Options: Full-width bodies Queue | Browse | Live, B and the digits switch (recommended, reuses the browsers unchanged) | Browse in the left column, editor and Live as overlays (Sol, first round) | Right-pane modes Sources | Queue | Live (MiMo, first round)
14. Does `P` (play an album, folder, artist or playlist, replacing the queue) ask for a confirmation?
    Options: Apply's rule: only on a source kind change or when more than 25% of the queue goes (recommended) | Never, Previous sources undoes it | Always
15. A song appended (`a`) to a queue holding a Hits source while weighted is on: what happens to it?
    Options: It joins the source's files and the round, the source shows "+N added" (recommended) | It becomes an Up next request instead | It stays outside the round (today: never drawn)
16. Stored playlist editor: immediate edits or a save buffer?
    Options: Immediate MPD edits as today, plus a confirmation before deleting a whole playlist (recommended) | Edit a copy, explicit Save and Discard
17. Keys for Browse?
    Options: B body, [ ] grouping, P play replacing the queue, t play next, digits 5..9 groupings and 0/gl Live (recommended) | The same without the digits (digits only for remaining tabs) | Other keys (name them)
18. Your own ~/.config/rormpc/config.ron has explicit tabs and digits: change it too when phase 3b ships?
    Options: Yes, the coordinator mirrors the new default (Play, Up next, Search plus Versions, Deleted, Lyrics as now; digits to ShowPlay) (recommended) | Keep your tabs as they are | Remove only the six tabs, keep your digits
19. An unapplied preview when a Browse action changes the queue?
    Options: The preview stays, its counts are recomputed, Apply judges the confirmation then (recommended, Sol) | The preview is dropped with a "preview dropped" note (MiMo)
20. Artists and Album artists in Browse?
    Options: Two separate groupings (recommended, MiMo, the code has both) | One grouping with an artist / album artist toggle (the coordinator's first round)
