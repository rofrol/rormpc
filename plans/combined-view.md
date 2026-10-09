# Combined Queue, Hits and Shuffle view (plan)

Design only, nothing here is built yet. Written 2026-10-09 from two consult rounds (GPT-6.1 Sol and Xiaomi MiMo,
see "Consult rounds"). The choices still open are at the end, each with options.

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
| Queue plan view | `rmpc/src/ui/rormpc_queue_plan.rs` (`o`, `TogglePlanView`) | A projection: `-2`,`-1` past plays, `0 ▶`, `↑n` Up next, forecast `1`..`10`, "unplanned · queue order"; J/K patch the forecast through mpd-player with a plan version |
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

A new pane `Pane(Play())` in a tab "Play" replaces Hits, Queue and Shuffle as the default tabs (`1`). It is the
filter column of Hits on the left and one table on the right. The old panes stay in the code and in explicit
configs; Up next and Live playlists stay separate tabs (both models: the request list must never be buried in a
filter-driven table).

- **Normal mode** (weighted off): the table is MPD's queue in its physical order, like today's Queue. The forecast
  column is empty, never stale. The filter column is collapsed to one line naming the source
  (`Source: 1980s · +rock · Top 10%`) and opens with `h` or a click.
- **Weighted mode** (`w`): the table is today's plan view: past plays, `0 ▶`, `↑n` requests, forecast `1`..`10`
  with the lane, then "unplanned · source order". The filter column is open. A header line under the tab title
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
 Top %  [x]1-10 [ ]11-20 │ ┄┄ unplanned · source order ┄┄
 Genres                  │       ✚      Kombi              Black and White          pinned · library
  + rock   - country     │       ⊘      Phil Collins       Another Day in Paradise  excluded · Billboard
 (Billboard) − Christmas │
   ∩ rock · 287 of 8,312 │
 [ Apply ] • changed     │
```

The ghost row `⊘` shows only with "show excluded"; the rule summary under the genres is the printed formula.

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
- default config: tab "Play" first, Queue/Hits/Shuffle tabs removed from the default (explicit configs untouched).

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

Each phase ships on its own; the old tabs stay usable until phase 3 is in the default config.

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
