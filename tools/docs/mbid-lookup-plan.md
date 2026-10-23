# Missing recording MBIDs: lookup plan

Status: implemented offline (AI-assisted), with the user's 2026-10-23 decisions below; no live lookup or library
changes performed during implementation. The approved direction is lookup,
then years with only sure rows applied, without manual row review; the existing 66 review rows stay untouched
(`TODO.md:1387-1409`). Keep inventories, reports and account data outside this public repository
(`tools/README.md:339-340`). Source references below abbreviate `tools/src/rormpc_tools/`.

## What exists, and what not to run

`acoustid(path)` fingerprints an on-disk file with fpcalc (first 120 seconds) and returns recording candidates,
scores and a fingerprint hash (`mbtag.py:157-177`). `shazam(path)` also accepts a local file; cached unofficial
recognition supplies names, not a recording MBID (`mbtag.py:180-206`). `collect()` builds YouTube-oriented
evidence, including URL relations, parsed names, fingerprints and optional ListenBrainz lookup; it calls Shazam
only without URL/fingerprint results and below 900 seconds (`mbtag.py:228-245`). Reuse these components, not
invented YouTube metadata for files without it. `mb_search()` searches artist/title, `mb_search_fallback()` checks
names and duration, and `from_recording()` supplies the returned canonical MBID, video flag, disambiguation and
length (`mbtag.py:327-368`). None of these identification steps needs to rewrite the audio file.

**Do not use `yt-mp3-mb --retag` for this repair.** It reads existing MP3 tags but calls the normal `process()`
without passing `batch`, even if `--batch` was supplied (`yt_mp3_mb.py:263-279`). That pipeline can ask questions;
normal batch downloads instead retain uncertain proposals without an MBID (`yt_mp3_mb.py:130-139`). Both use a
writer that replaces artist/title, optionally album, recording/artist IDs and date/provenance tags
(`mbtag.py:451-490`). Processing fetches/replaces embedded cover art, runs ReplayGain and renames the file to
its generated name (`yt_mp3_mb.py:140-154`; `mbtag.py:564-577`). It is not an MBID-only operation.

## Proposed interface and SURE policy

Add **`musicdb mbid --dry-run | --apply | --rollback [ID...]`**, with its own versioned report and backup in
configured data_dir (`tools/README.md:11`, `214-215`). Separating identification from dating prevents a successful match from authorizing year
changes; existing years acceptance can write both recording and date tags (`years.py:708-758`). This is a new
command, not a change to the existing years report/CLI contract (`years.py:814-846`).

Input: a fresh years report's `needs_mbid` **and** `no-recording` rows, rechecked against actual file tags before
lookup. These are missing-recording rows (`years.py:540-549`); `needs_mbid` alone also includes existing recordings
with no work link (`years.py:296-300`), which must not be overwritten. Library enumeration currently takes MBIDs
from MPD, so do not trust that snapshot over the file (`years.py:587-595`, `378-388`). Preserve song UUID/path and
years row ID in the new report; years already keys rows by UUID, falling back to path (`years.py:569-575`).

Evidence order: AcoustID first; MusicBrainz search on cleaned **file** artist/title second. Preserve raw tags and
version markers alongside cleaned query strings (existing normalization and version vocabulary:
`mbtag.py:251-281`). Shazam is an optional hint only in the existing no-URL/no-fingerprint-result, under-900-second
case; it may seed MB search but never authorize a write (`mbtag.py:237-245`, `434-443`). Respect the existing
under-900-second fingerprint collection restriction too (`mbtag.py:236`). No downloads or MPD playback changes.

Provisional named constants for implementation, copied from the current rules, not calibrated guarantees:

- `NAME_TITLE_WEIGHT = 0.6`, `NAME_ARTIST_WEIGHT = 0.4`, `VERSION_PENALTY = 0.15` per differing marker
  (`score_candidate`, `mbtag.py:284-300`).
- `ACOUSTID_WEIGHT = 0.3`, `ACOUSTID_BASE = 0.5`, `DURATION_PENALTY_AFTER_S = 40`,
  `DURATION_PENALTY = 0.1`, `MULTISOURCE_BONUS = 0.1` (distinct methods, not repeated queries;
  `decide`, `mbtag.py:303-324`).
- `AUTO_SCORE = 1.0`, `AUTO_CHECK = 0.8`; the existing resolver starts fetching candidates at score 0.75
  but that alone is not auto confidence (`mbtag.py:403-409`, `434-440`). Search's MB score >=90,
  length delta <=30 seconds and check >=0.85 still produce **review**, never auto (`mbtag.py:350-368`, `441-443`).

Recommend SURE only for an AcoustID-supported, MB-verified canonical recording meeting both auto thresholds,
with no tie/conflicting evidence or version ambiguity. Report raw and adjusted scores, runner-up, methods,
lengths, candidate IDs, canonical redirect and reasons; do not promote search-only, Shazam-only, errors or
nomatches. Existing code reports runner-up but has no margin gate (`mbtag.py:400-407`); tie/conflict exclusion is
a proposed additional safety gate, not an existing guarantee. Leave all non-SURE files alone, with reasons,
not a queue the user must review.

Version/live/remix markers and explicit Versions labels must agree; do not strip qualifiers to force a studio
match. Search fallback already refuses version-marked inputs (`mbtag.py:350-353`); Versions has labels and marker
hints, and audio similarity only suggests a merge (`versions.py:6-23`, `29-31`, `98-100`). Its first-120-second
comparison can miss an extended version (`versions.py:203-212`). Never merge files or decide play attribution.

`resolve()` swaps video/DJ-mix recordings to an audio alternative (title similarity >=0.9, artist >=0.8), else
through the work (`mbtag.py:410-427`). The work rule requires same first artist, no video/version/DJ segment,
and length within 10 seconds (`years.py:230-261`). Record that evidence, but recommend **withholding every swap
from automatic apply in this first repair**: the alternative path has no length check and the work rule picks
an earliest release, not proof of this exact audio. An unswapped video/DJ segment is likewise not SURE.

## Write, undo, then date

Write only ID3 `UFID:http://musicbrainz.org` or Vorbis `musicbrainz_trackid`, not artist IDs, names, covers,
dates, ReplayGain, filenames or UUIDs (supported formats/readers: `years.py:378-445`). Use a dedicated
recording-only setter: passing dates through `set_dates()` would unnecessarily rewrite them.

Copy the years discipline: lock, fsynced pre-write backup, same-directory dot-copy, fsync, atomic replace and
parent fsync (`years.py:447-464`, `579-585`, `691-698`, `708-758`). Strengthen its audio/date/recording freshness
guards with full-file digest and stat snapshot: **any file change since dry run skips the row**, even a cover or
name-tag edit that preserves audio md5 (`years.py:724-747`). Revalidate before replace; detect concurrent edits
with a deterministic test hook, not a sleep. Journal per-file progress for interrupted apply/rollback; a rerun
must recognize an already-written MBID rather than duplicate a write. Rollback restores absence or the old MBID
only while audio and written MBID still match, preserving unrelated later tags; otherwise skip and report
(the existing guarded rollback pattern is `years.py:762-811`).

Preserve non-YouTube song-key associations: keys can change from `file:` to `mb:`; lists/manual genres use those
keys (`identity.py:90-111`; `tags.py:30-35`). Reuse and journal `identity.rekey()` semantics for lists, genres,
bans, version labels/decisions, copying rather than removing shared-key data; compensate on rollback
(`identity.py:114-166`). Keep UUIDs and historical ListenBrainz events unchanged. After writes, request MPD's
waited directory update (`years.py:472-475`); refresh registry metadata without adding unrelated file tags.

Then run `musicdb years --dry-run` again. Accept only newly computed `confidence == high` proposals and apply
through its existing guards; never accept the previous 66 review rows. High means same-length, release-group
year equal to proposal (missing RG date falls back to proposal), and strictly earlier than the current shown
year; **undated files remain review**, even after identification (`years.py:330-337`, `531-563`). Acceptance is
still required before years apply (`years.py:653-666`, `716-718`); automate selection of eligible IDs, not human
row review, as approved in `TODO.md:1406-1409`.

## Rate limits and validation

Use serial shared `http()` calls: MusicBrainz's 1 request/s policy is conservatively paced by the existing
1.1-second per-host interval; AcoustID uses 0.4 seconds (at most 2.5 requests/s configured pacing)
(`mbtag.py:65-95`, `169-170`; `tools/docs/release-years-plan.md:181-184`). Existing transient-error backoff is
3, 6, 9, 12 seconds across five attempts; Shazam already has a two-second external-rate-limit interval
(`mbtag.py:24`, `180-206`). No new timer, sleep or retry; classify these solely as external rate limiting and
designed backoff. Report exhausted lookups as errors, not proof of no match (`mbtag.py:65-95`).

For approximately 352 files (`TODO.md:1387-1389`), budget provisionally 15-40 minutes: 352 AcoustID requests
have about 141 seconds of pacing; assuming 2-5 uncached MB requests/file adds about 13-32 minutes, plus local
fingerprinting/network latency. This is a planning estimate, not a benchmark or timeout; work expansion and
backoff can exceed it, caches can shorten it. The years rerun is additional work.

Offline implementation tests: fake fpcalc/AcoustID/MB/Shazam responses, threshold boundaries and ties, MB
redirects, missing durations/tags, version/label conflicts, video/DJ swaps, lookup failures versus empty results,
search-only withholding, and filtering no-work versus no-recording. Temporary ID3/Vorbis fixtures must prove
only recording tags change, names/art/UUID/audio stay intact, unsupported formats skip, stale edits/deletions
skip, interrupted writes recover, shared keys survive rekey, and rollback restores absence without clobbering
later changes. Fake clocks test existing pacing/backoff without real waits. Integration fixtures must show
only high years IDs accepted; undated/review rows untouched. This follows the report/apply/rollback shape of
`tools/docs/release-years-plan.md:120-175`, not its historical measurements as current evidence.

Only an authorized live run can establish service coverage, real match precision, actual runtime, fpcalc/API
availability and MPD/scrobbler visibility. No network, library inspection or live writes belong to this plan.

## Implementation and decided policy (2026-10-23)

Built `musicdb mbid --dry-run | --apply | --rollback [ID...]` in `mbid.py`, with offline evidence and temporary
ID3/Vorbis transaction tests in `test_mbid.py`. The years report/CLI and downloader behavior are unchanged.
Dry-run refreshes years first and rechecks actual tags. The recording-only writer journals a full-file digest,
stat snapshot, old/new recording, staged-byte digest and association operations before replacing files. Apply,
rollback, association compensation and waited MPD updates resume at durable checkpoints. Rollback preserves
later unrelated tags and association edits, and copied associations still used by another file.

- d-mux3adcq: ship the AcoustID-supported canonical MB auto gate, with both provisional auto thresholds and
  conservative exclusions; non-SURE rows are reported with reasons, never a manual review queue. Versioned
  recordings are conservatively withheld even when their broad markers agree: that does not establish which
  live/remix/edit performance the file contains. Search contributes the distinct-method bonus only with the
  existing search name/score/duration checks. Repeated queries and redirected duplicate IDs do not add votes.
- d-nec7jsge: Shazam is disabled entirely for this repair, superseding the optional hint described above.
- d-awp3qij7: video/DJ-to-audio swaps cannot authorize writes. Canonical candidate identities, flags, lengths,
  redirects and any audio alternatives found by the lookups stay in the report. The work-based swap lookup is
  deliberately not invoked: its earliest-release heuristic cannot make an exact-audio repair safe.

The separate follow-up remains a fresh years run and acceptance of newly computed high rows only, not an
automatic side effect of MBID application. Precision, service coverage and live integration still require an
authorized run outside the sandbox; offline fixtures do not establish them.

## Original questions (answered above)

- Which automatic identification gate should ship?
  Options: AcoustID-supported auto thresholds plus the conservative exclusions above (recommended) | report-only
  pilot first. The current thresholds were built for downloader evidence, not validated for these files.
- May Shazam receive audio when the existing fallback condition holds?
  Options: disable it for this repair (recommended; no additional audio disclosure) | enable existing cached
  unofficial fallback as hints only. Existing behavior: `mbtag.py:180-206`, `237-245`.
- May a video/DJ-to-audio swap be automatically written?
  Options: withhold swaps, retain evidence (recommended) | permit only after a separately validated exact-audio
  gate. Current title/work heuristics alone do not establish exact identity (`mbtag.py:410-427`).
