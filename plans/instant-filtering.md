# Instant filtering in the Music view, without Apply (plan)

Design only, nothing here is built yet. Written 2026-10-10 from the code, measurements on the real library
(read-only) and consult round `20261010-230739-29d8` (GPT-6.1 Sol `7f68ca37`, Xiaomi MiMo `829929fd`). The choices
still open are at the end, each with options. It changes the decision "a filter change prepares, Apply plays" of
plans/combined-view.md (open choice 2 there, decided 2026-10-09); the rest of that plan stays.

## Goal (the user's words, translated)

> Filtering must be instant, without apply. Work out with the models how to do it.

Every filter change in Music (periods, genres, ± sets, Rank by, Years of, Top %, artists, owned, a picked smart list,
a pin or exclusion) takes effect at once: the table, the forecast and the MPD queue follow it, and the playing song
is never interrupted.

## Today

| Step | Where | What it does |
|---|---|---|
| Filter change | `rmpc/src/ui/panes/hits.rs` `HitsPane::preview_if_changed` → `apply` → `start_run` | Builds `hits` argv (`Filters::args`, `rule_args`) and runs `hits ... --json ~/.cache/rormpc/hits/preview.json` in a thread. The newest run wins: a run asked for while another runs SIGTERMs it (`Job::kill`), queues itself, and a superseded run's temp file is never renamed over the result (`Job::superseded`). Esc stops a run (`stop_run`) |
| Preview | `rmpc/src/ui/panes/play.rs` `preview_active`, `render_queue_body`; `rormpc_play::banner` | Filters whose `rules_hash` differs from source.json's are a preview: the table shows the `hits` result under a banner with counts; MPD untouched |
| Apply | `play.rs` `PlayPane::apply` → `rormpc_play::apply` | `a`, the banner button, the column's `[ Apply ]`. A preview older than MPD's `db_update` or the newest deletion is recomputed first (`preview_outdated`). Confirmation only on another source kind or more than 25% of the queue going (`confirm_reason`) |
| Replace | `rmpc/src/ui/rormpc_upnext.rs` `apply_hits_source` → `Replace::run` (on the MPD command thread, `ctx.command`) | Refuses when MPD's `playlist` version moved since the decision; one `add` per file after the old queue (`add_known` skips files MPD lacks); two range deletes around the playing song; deletes the playing song's duplicate; then writes `source.json` (`kind: "hits"`, `files`, `rules`, `rules_hash`, `added`) atomically and remembers the rules in Previous sources |
| Plan | rormpc-tools `src/rormpc_tools/player/shuffle.py` `on_status`, `check_plan`, `fill_plan`, `publish`, `ensure_round` | On each MPD idle event: drops planned entries whose song id left the queue, tops the plan up to 10 from `queue ∩ (files ∪ added)`, writes only changed priorities. `ensure_round`: the round key is `hits:<rules_hash>`, so other rules start a new round with nothing heard; `heard` is appended when a song starts (`song_started`) |
| Esc | `play.rs` `escape` | Stops a running `hits`, else drops the preview (filters back to the playing rules) |

Measured 2026-10-10 on this Mac, library 834 files, read-only (`hits ... --json <scratch>`, three runs each):

| Run | Wall time |
|---|---|
| Billboard, chart years 1980-1989, all rows / Top 1-10% / + `rock` | 0.24-0.31 s |
| Rank by my plays, whole library (817 rows) | 0.27-0.30 s |
| `+likes`, rediscover | 0.20-0.21 s |
| Rank none, release years 1980-1989 | 0.19-0.20 s |
| `hits --version` (interpreter + imports only) | 0.07 s |
| MPD: 800 `ping` round trips / `listallinfo` / `playlistinfo` (18) | 0.054 s / 0.010 s / 0.0005 s |

A cProfile of the library run puts about half of `hits`' own time in rebuilding musicdb's library index
(`musicdb`/`mbtag` matching) and about 0.1 s (profiled) in MPD reads. So a filter change reaches MPD in about
0.2-0.3 s, almost all of it the `hits` process; replacing even the whole library is about 0.06 s of MPD round trips.
The Replace on a scratch MPD was not timed (it would change a queue); the round-trip figure bounds it.

Found while reading (a bug today, independent of this plan): `Replace::run` writes `source.json` after the queue
writes, and mpd-player reads `source.json` only on an MPD idle event or its 30 s heartbeat. The idle events of the
add/delete can be handled before `source.json` names the new files, so the plan is drawn from the old members for
up to 30 s. The design below orders it the other way.

## Design

### What "takes effect" means

Three things, each reflecting one generation (MiMo; Sol the same in other words):

- **The filter column** shows the newest intent at once (the chip flips on the key press).
- **The table** shows the newest `hits` result once it is in; while a newer run is under way it keeps the last
  result dimmed under the existing "updating for …" banner (built 2026-10-10 for stale results). No preview, no
  `[ Apply ]`, no `Preview ·` banner.
- **The MPD queue, `source.json` and the forecast** reflect the newest *applied* generation: the last `hits` result
  that reached the MPD command thread while it was still the newest. The forecast is mpd-player's plan of that
  queue, never of a result that has not reached MPD (MiMo: a forecast row must exist in the queue).

In weighted mode the table is the plan projection as today, so the user sees the result as the forecast changing;
the counts (`312 matched · 287 owned · 25 missing`) move into the header line. A toggle shows the `hits` result
rows (with missing songs dimmed) instead of the plan; open choice 3.

In normal mode the table is the queue; after the playing song it is in rank order (below).

### Reconcile by difference, not replace

Each applied generation reconciles the queue with its target instead of replacing it (both models):

- **target** = the result's owned files ∪ `added` (hand-appended to this source) − `removed` (hand-removed, below);
- **delete** only queue entries that the previous generation put there (its `files`) and that are not in the target,
  never the playing song, never Up next requests, never an entry nobody can attribute (a phone's or mpc's add):
  unknown entries stay (MiMo, Sol: "never delete what you cannot attribute");
- **add** target files not in the queue (missing ones skipped as today, `add_known`);
- songs in both keep their MPD ids, so mpd-player's planned entries survive (`check_plan` keeps an entry whose id
  is still queued) and the forecast changes only where the filter changed it;
- **normal mode** also moves the kept entries after the playing song into rank order (Sol: a diff alone does not
  order; `moveid`), so "in rank order" stays true; weighted mode needs no moves (priorities decide);
- adds, deletes and moves go in one MPD command list per generation (fewer round trips and fewer idle events); MPD
  has no transaction, so a failure is recovered by reading the queue back, not by rollback (Sol).

The base is the previous generation's `files` in `source.json`; a `source.json` that is not a filter source (an
album played with `P`, Sources…, none) is "another source": see "Leaving another source".

### One generation counter, two stages

- Every filter change (a key, a click, an exception, an undo, a smart list pick) bumps one generation `g` in the
  Play pane and starts `hits` through today's `start_run`, which already kills a stale run and never publishes a
  superseded result. The counter replaces nothing there; it travels with the run.
- When a run's result is in and `g` is still the newest, Play sends one reconcile to the MPD command thread tagged
  with `g`. The closure checks `g == latest` when it is dequeued and is a no-op otherwise (both models), so a
  backlog of reconciles collapses to the newest. A reconcile that has started finishes its command list; the next
  generation reconciles from the queue as it then is (Sol: a write in flight cannot be taken back, only
  converged).
- Holding `›` on a period or clicking three decades: each press kills the running `hits`; while presses come
  faster than a run (0.2-0.3 s), no intermediate result reaches MPD at all; when they come slower, each finished
  generation is a valid state and the next converges from it. No delay, debounce or timer anywhere (Rule 10): the
  only coalescing is "the newest generation wins" at both stages.
- `source.json` is written **before** the generation's queue writes, atomically (temp + rename, as now), with the
  new `files`, `rules`, `rules_hash` and a `generation`. mpd-player draws from `queue ∩ members`, so new members
  that are not queued yet are harmless, and the queue writes that follow raise the idle event on which it
  re-plans with the new members (fixes the 30 s window above). Neither side needs atomicity: mpd-player converges
  on the next event (MiMo).
- The MPD `playlist` version check of Apply goes: there is no decision to protect any more. A phone's concurrent
  edit is handled by attribution (above), not by refusing.

### Rounds belong to the source, not to the rules hash

With instant filtering the rules hash changes on every click, so today's key would start a new round each time
(both models). Instead:

- the round key is the source's identity: a new `source_id` (UUID) written by Play when the queue starts holding a
  filter source (the first filter change after another kind of source); filter changes, smart list picks and
  undo keep it;
- `heard` stays keyed by file path (as now); a filter change keeps it, songs that newly match join the round
  unheard, songs that leave are no longer members but stay in `heard`, so removing and re-adding a song does not
  replay it at once (MiMo);
- the round ends when every current member is heard (today's rule), and `shuffle newround` starts one explicitly;
- `rules_hash` stays in `source.json` for Previous sources and smart lists, no longer for rounds. mpd-player keeps
  reading an older `source.json` (no `source_id`) the old way.

Sol would mark "heard" only on a scrobble-qualifying play instead of on start; that changes rounds beyond this
plan and goes to "Proposed" (see Open choice 7).

### Undo instead of the confirmation

Apply's confirmation (source kind change, more than 25% of the queue) is gone with Apply. In its place:

- an **undo history of filter states**: each applied generation pushes the rules it replaced (generations that never
  reached MPD are not entries, so a held `›` is one entry without any timing; this is our answer to Sol's "one
  gesture, one entry"); depth 20 (MiMo 10-20, Sol 50); redo after undo;
- undo is itself a filter change: it bumps `g` and reconciles like any other (both models). It restores the
  membership, never playback time, never a hand edit of the queue (MiMo: no queue undo);
- the first entry when a filter change leaves another source is that source (album, playlist, library, with its
  files in order), so undo brings the album back in its order (Sol);
- the header line says what the last change did: `Filter: +rock · 287 songs (was 312) · Backspace undoes`;
- keys: `u` (Update), `<C-r>` (Rename) and `<C-z>` (ContextMenu) are bound in the user's config; `<BS>` and `Z` are
  free in both configs (checked 2026-10-10). Open choice 4.

Previous sources (the last 10 applied rule sets) keeps working; it now records a rule set when the source is left or
after it played a song, not on every click (open choice 6).

### Esc and `a`

- Esc stops a running `hits` (as now). It no longer "drops the preview": there is none; undo goes back.
- `a` in the Queue body: no Apply. It says once "Filters apply at once; Backspace undoes" (Sol: keep the hint for
  muscle memory, MiMo: announce it), then nothing. Browse's `a` keeps appending.
- The column's `[ Apply ]` row and the banner button are removed; the rule formula line under the filters stays.

### Leaving another source

When the queue holds another kind of source (an album or artist played with `P`, a stored playlist, the whole
library, Sources…, an unknown queue), the first filter touch replaces it by the filter's result: the playing song
goes on, Up next stays, the old source is the undo entry. Sol: do it at once, show it, keep undo; the old
confirmation rule would bring back a modal on the first click. Open choice 2.

### Edge cases

- **Zero owned songs**: the queue is not touched; the header says `0 owned · the queue keeps <previous>` (MiMo).
  Sol would empty the upcoming songs ("stops after this song"); open choice 5.
- **Hand-appended songs** (`a` / `A` in Browse) stay in `added` and survive every reconcile (as today).
- **Hand-removed songs**: today a removal is a one-off undone by the next Apply; with instant filtering the next
  click would bring the song back (MiMo: "the most visible it's-broken bug"). Removals of source songs from the
  queue in rormpc are recorded in `source.json` `removed` (session scope: cleared when the source is left), not as
  exceptions; `-` (exclude…) stays the lasting way. A phone's delete is seen as a missing source song and is
  re-added by the next reconcile, as the next Apply did. Open choice 1.
- **Exceptions** (`+` / `-` on a row, the Exceptions list): `hits` applies them, so recording one is a filter change:
  `hits` runs and the queue reconciles at once (was: "the next Apply takes it into account").
- **Smart lists**: `L` → Enter loads the list's rules and they apply at once (was: a preview); the list's own
  `a` (apply) goes. Saving (`S`) is unchanged.
- **Outdated results**: each change runs `hits`, so a result is never older than the filters; `add_known` still skips
  files deleted since. The "Preview outdated, recomputing" path goes with Apply.
- **The playing song outside the new result** plays on, marked `▶ off-source`, out of the round (as today).
- **Up next** is never touched by a reconcile; requests stay above the plan.
- **Weighted off** (`w`): nothing changes in the queue; filters keep applying (in rank order).
- **A `hits` failure**: the queue keeps the last applied generation; the header shows the error; the filter column
  keeps the intent, marked `not applied`, and undo reverts it.
- **Stable order**: identical rules must give identical `files` order, or normal mode moves songs on every rerun
  (MiMo). `hits_rules.py` sorts ranked rows by rank and unranked rows by `order`; the build adds a test that two
  runs give the same order and a path tie-break where keys can tie.
- **Idle storms**: one command list per generation gives a few idle events, not one per song; mpd-player already
  writes only changed priorities and keeps surviving plan entries. Scrobbling follows the playing song id, which a
  reconcile never removes.
- **Linux and macOS**: nothing platform-specific; MPD commands and `source.json` renames only.

### Keeping `hits` warm

0.2-0.3 s is acceptable as a first step: the chip flips at once, the table follows within a run, and newest-wins
keeps a held key from piling up work. Both models call it "not perceptually instant". In order of cost:

1. cache musicdb's library index across runs (keyed by the library's mtime/MPD `db_update` and the deletion
   journal), as the play-history split was cached on 2026-10-10: about half of a run (Sol: do this first);
2. a long-lived `hits serve` worker (one request per generation over stdin/stdout or a Unix socket, the index
   invalidated by MPD `db_update` and the musicdb files' mtime, killed and respawned on a newer generation since a
   Python computation cannot be interrupted cooperatively, the plain process per run kept as fallback). Saves the
   0.07 s start-up and the index; risks: a stale index, process lifecycle, a protocol to version (both models);
3. MiMo's "an in-process Rust index" is dismissed: it would duplicate `hits`' rules in two languages.

Step 1 is part of the build; step 2 only if the end-to-end time measured after step 1 is still over 0.15 s.

## Mockup

```
 Music ─ ▶ Playing from: 1985-1992 +Billboard rock Top 10% · weighted · round 12/84 ─────── Live 3
 Filters │ Browse        │ 287 owned · 25 missing · was 312 · Backspace undoes      [rows ⇄ plan]
 Sets                    │ Next  Lane   Artist             Title                    Why
  + Billboard US         │  -1   ⏭      Madonna            Like a Prayer            skipped early
  - Tag Christmas        │   0 ▶        Tears for Fears    Everybody Wants to Rule… (off-source)
 Rank by  Billboard      │  ↑1          Kombi              Słodkiego miłego życia   your request
 Years of chart year     │   2   fam    a-ha               Take On Me               plays 41 · ♥
 Period  1985 – 1992  ‹› │   3   fam    Queen              Radio Ga Ga              overdue 1.6x
 Top %  [x]1-10 [ ]11-20 │   4   redis  Cutting Crew       (I Just) Died in Your…   played 2 y ago
 Genres                  │   …
  + rock   - country     │
 (Billboard) − Christmas │
   ∩ rock · 287 of 8,312 │
```

While a newer run is under way the second line reads
`287 owned · updating for Period: 1985 – 1993 · 0:00 · Esc stops`, and the rows keep showing the applied
generation. The `[ Apply ]` row and the `Preview ·` banner are gone.

## What changes where

rormpc:
- `panes/play.rs`: no preview state (`preview_active`, `ApplyWait`, `apply`, `escape`'s drop); a generation
  counter; on a finished newest run, send the reconcile; undo/redo history; the header's counts and last-change
  line; `a` hint; the rows ⇄ plan toggle (if chosen);
- `panes/hits.rs`: the run carries its generation; `[ Apply ]` hidden in Play mode (the Hits pane in explicit
  configs keeps its Apply);
- `rormpc_upnext.rs`: a `Reconcile` next to `Replace` (attribution base = the previous `files`, `added`,
  `removed`; command list; `moveid` in normal mode; `source.json` written first with `source_id` and
  `generation`); hand removals in Music record `removed`; `Replace` itself keeps serving Sources… and Browse `P`;
- `rormpc_play.rs`: `confirm_reason` no longer used by Music's filters (Browse `P` keeps it); `preview_outdated`
  and the banner go;
- `rormpc_smartlists.rs`: Enter on a list applies its rules; Previous sources records on leaving or after a play;
- RORMPC.md "Music tab" rewritten; plans/combined-view.md gets a note that choice 2 was changed.

rormpc-tools:
- `player/shuffle.py`: round key = `source_id` when present (else today's rules hash or name); `heard` kept across
  member changes; members = `files ∪ added − removed`; tests for a filter change keeping the round and the
  forecast entries still queued;
- `hits` / musicdb: the cached library index (warm step 1); a stable-order test; later `hits serve` if needed;
- `tests/test_rormpc_contract.py` and rormpc's side: `source.json` gains `source_id`, `generation`, `removed`
  (mpd-player must read old files too).

## Build order

1. mpd-player: `source_id` round key, `heard` kept across member changes, `removed`; reads old `source.json`.
   Released and installed first (rormpc's new fields are ignored by an older mpd-player, but the round would reset
   on every click).
2. rormpc: write `source.json` before the queue writes (fixes today's 30 s window; useful alone with Apply).
3. rormpc: `Reconcile` with attribution, command list, normal-mode order; generation tagging and the dequeue
   check; Music applies on every finished newest run; Apply, preview banner and `[ Apply ]` removed; zero-result
   rule; `a` hint; RORMPC.md. Tested on a scratch MPD (AGENTS.md: never the user's queue), including a held key, a
   phone `mpc add` between generations, Up next, normal and weighted mode.
4. Undo/redo history with its keys; leaving another source as the first undo entry; header's last-change line.
5. Hand removals → `removed`; exceptions and smart list picks apply at once; Previous sources' new recording rule.
6. Warm step 1: cached library index in `hits`; measure end to end; step 2 only if still over 0.15 s.

Each step ships on its own; steps 1-2 change no behaviour the user sees.

## Consult round

Round `20261010-230739-29d8`: Sol `7f68ca37`, MiMo `829929fd`. The brief said the user decided against Apply, so
neither argued for it.

Agreed (both): reconcile by difference, keeping the ids of songs in both; kill stale `hits`, drop stale MPD writes at
dequeue, never tear a write in flight; queue and `source.json` cannot be atomic, so converge; never delete queue
entries nobody can attribute (phones); rounds must stop being keyed by the rules hash; undo restores filter state
and membership, not playback or queue edits, and is itself a filter change; exceptions are a filter change applied
at once; 0.2-0.3 s is usable but not instant, warming has lifecycle and staleness risks.

Sol alone (taken): normal mode needs moves to keep rank order; the boundary "cancellation cannot retract commands
already sent"; the departed source as the first undo entry; cache the library index before building a server; keep
`a` as a hint; Esc needs a new meaning. MiMo alone (taken): the forecast reflects only the applied generation;
hand removals must persist or every click resurrects them; `heard` survives a song leaving and re-joining;
deterministic order or normal mode churns.

Diverged: zero results (Sol: upcoming songs go, "stops after this song"; MiMo: the queue untouched) → open choice
5; undo depth (Sol 50, MiMo 10-20) → 20; an undo entry per "gesture" (Sol, by input events) vs per applied
generation (ours, needs no gesture detection); suspend mpd-player's planning during a reconcile (Sol) vs converge
(MiMo) → converge, since `source.json` written first already gives mpd-player the members before the queue moves.

Dismissed: MiMo's "full replace churns the playing song's id" (today's Replace deletes around the playing song and
keeps its id; the diff is chosen for the forecast and the phone edits instead); MiMo's in-process Rust index
(duplicates `hits`' rules); MiMo's latest-generation drop inside mpd-player (it already converges per idle event
and writes only changed priorities); Sol's `u` / `Ctrl-r` and MiMo's `u` / `U` / `Ctrl-z` (all bound in the user's config).

## Open choices

1. What does removing a source song from the queue in Music do, now that the next click would bring it back?
   Options: Remembered for this source (`removed`), cleared when the source is left; `-` stays the lasting exclusion (recommended) | It becomes an exclusion scoped to the source (Sol) | Nothing: the next filter change brings it back
2. Does the first filter touch replace another kind of source (an album played with P, a playlist, the whole library)?
   Options: At once; the old source is the first undo entry (recommended, Sol) | Ask once per source, then instant | Never: filters apply only while a filter source plays
3. In weighted mode, what does the table show while filtering?
   Options: The forecast, with the counts in the header and a rows ⇄ plan toggle for the full result (recommended) | The result rows while the filter column has the keys, the forecast otherwise | Always the result rows
4. Which keys undo and redo a filter change?
   Options: Backspace undo, Z redo (recommended, both free) | Z undo, no redo | Other keys (name them)
5. A filter with 0 owned songs?
   Options: The queue stays as it was, the header says so (recommended, MiMo) | The upcoming songs go, it stops after the playing song (Sol)
6. When does Previous sources record a rule set, now that every click applies?
   Options: When the source is left, or once a song of it has played (recommended) | On every applied change | Only when saved as a smart list
7. Should a round count a song as heard only after a scrobble-qualifying play instead of on start (Sol)?
   Options: Not in this plan, add it to Proposed (recommended) | Yes, in step 1
8. Keep `hits` warm beyond the cached library index?
   Options: Only if the end-to-end time after the cache is over 0.15 s (recommended) | Build `hits serve` in step 6 anyway | Never
