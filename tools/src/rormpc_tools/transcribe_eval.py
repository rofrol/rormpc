"""An evaluation run for the transcript checks: how today's thresholds in transcribe.py judge labelled songs, and
which values would judge them right (docs/transcribe-lyrics-plan.md, "Build order" step 1).

  musicdb lyrics transcribe-eval LIST [--out DIR] [--json]   # run the backend once per song, keep its JSON in DIR
  musicdb lyrics transcribe-eval LIST --from DIR [--json]    # judge again from DIR, without the backend

LIST is a text file kept outside the repository, one song per line: `<path relative to the music dir><TAB><label>`;
blank lines and lines starting with # are skipped. Labels:
  lyrics        the song has LRCLIB synced lyrics in the lyrics dir (<song>.lrc): the reference text and times
  instrumental  labelled by hand: no singing
Nothing is written to the lyrics dir: no sidecar, .lrc, .txt or index.json. The backend's raw JSON for each song goes
to DIR (default a new temporary directory, printed on stderr), one file per song keyed by its path; a rerun with the
same DIR reuses a song's result while the backend's inputs (model, VAD and no_vocals values) are the same.

Per song: the measures each check compares (share of words outside speech and the SPEECH_PAD that would pass it,
the top line's repeat count and factor over the next one, words per second of speech, VAD speech seconds and their
share of the song), the state, reasons and review flags transcribe.judge gives with today's constants; for `lyrics`
songs the WER against the reference (both lowercased, punctuation removed: transcribe.norm) and sync from monotonic
word alignment: matched line share, signed median start error (p50), absolute p90, drift in seconds per minute,
and correct highlighted line share sampled every 2 s. The share within SYNC_WITHIN uses matched pairs only;
for `instrumental` songs whether they ended no_vocals.

Separating ranges: for each threshold, the values that keep its check from firing on every `lyrics` song and make it
catch every `instrumental` song no other check (at today's values) catches; or which songs make that impossible.
The speech_pad and outside_speech_max candidates only flag review, never withhold or catch instrumentals.
The VAD values and LANG_WINDOWS change the backend's output itself: they are calibrated by rerunning with other
constants, not from saved results.
"""
import hashlib, json, math, os, pathlib, statistics, sys, tempfile

from . import lyrics, lyrics_check, settings, transcribe

LABELS = ("lyrics", "instrumental")
SYNC_WITHIN = 1.0  # seconds: the plan's "lines within 1 s", among matched line pairs only
# (threshold, transcribe's constant, how its check fires on the song's input x, the catch it makes); "gt": x > T,
# "ge": x >= T, "lt": x < T
CHECKS = [("no_vocals_min_seconds", "NO_VOCALS_MIN_SECONDS", "lt", "no_vocals_seconds"),
          ("no_vocals_min_ratio", "NO_VOCALS_MIN_RATIO", "lt", "no_vocals_ratio"),
          ("speech_pad", "SPEECH_PAD", "gt", "words_outside_speech"),
          ("outside_speech_max", "OUTSIDE_SPEECH_MAX", "gt", "words_outside_speech"),
          ("repeat_min", "REPEAT_MIN", "ge", "repeated_line"),
          ("repeat_factor", "REPEAT_FACTOR", "ge", "repeated_line"),
          ("density_min", "DENSITY_MIN", "lt", "low_word_density"),
          ("density_max", "DENSITY_MAX", "gt", "high_word_density")]
BACKEND_ONLY = ("vad_threshold", "vad_min_speech_ms", "vad_min_silence_ms", "lang_windows")


# ---------------------------------------------------------------- the list and the raw results

def read_list(path):
    songs = []
    for n, line in enumerate(pathlib.Path(path).read_text().splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        rel, sep, label = line.partition("\t")
        if not sep or not rel.strip() or label.strip() not in LABELS:
            sys.exit(f"{path}:{n}: want <path relative to the music dir><TAB>lyrics|instrumental")
        songs.append((rel, label.strip()))
    return songs


def backend_params():
    """The values the backend's output depends on."""
    t = transcribe
    return {"model": t.MODEL, "vad_threshold": t.VAD_THRESHOLD, "vad_min_speech_ms": t.VAD_MIN_SPEECH_MS,
            "vad_min_silence_ms": t.VAD_MIN_SILENCE_MS, "no_vocals_min_seconds": t.NO_VOCALS_MIN_SECONDS,
            "no_vocals_min_ratio": t.NO_VOCALS_MIN_RATIO, "lang_windows": t.LANG_WINDOWS}


def raw_path(d, rel):
    return pathlib.Path(d) / (hashlib.sha256(rel.encode()).hexdigest()[:16] + ".json")


def load_raw(d, rel):
    try:
        rec = json.loads(raw_path(d, rel).read_text())
    except (OSError, ValueError):
        return None
    return rec if rec.get("file") == rel else None


def save_raw(d, rec):
    p = raw_path(d, rec["file"])
    fd, tmp = tempfile.mkstemp(dir=p.parent, prefix=p.name + ".", suffix=".tmp")
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(rec, f, ensure_ascii=False)
        os.replace(tmp, p)
    except BaseException:
        os.unlink(tmp)
        raise


def obtain(rel, out_dir, from_dir):
    """(raw record, None), or (None, why not): from DIR only with --from, else a saved result for the same backend
    inputs or a new backend run."""
    if from_dir:
        rec = load_raw(from_dir, rel)
        return (rec, None) if rec else (None, f"no saved result in {from_dir}")
    rec = load_raw(out_dir, rel)
    if rec and rec.get("params") == backend_params():
        return rec, None
    path = settings.MUSIC_DIR / rel
    if not path.exists():
        return None, "no such file"
    identity = transcribe.audio_identity(path)
    override = transcribe.language_override(rel)
    try:
        out = transcribe.run_backend(path, identity, override)
    except RuntimeError as e:
        return None, str(e)
    rec = {"file": rel, "identity": identity, "language_override": override, "params": backend_params(), "out": out}
    save_raw(out_dir, rec)
    return rec, None


# ---------------------------------------------------------------- the measures

def reference(rel):
    """[(start, normalized words)] of the song's synced lyrics in time order, or None without an .lrc."""
    try:
        text = lyrics.paths(rel)[0].read_text()
    except OSError:
        return None
    lines = sorted((at, transcribe.norm(words)) for at, words, _ in lyrics_check.parse(text))
    return [(at, w) for at, w in lines if w] or None


def wer(ref, hyp):
    """Word error rate of the word list hyp against ref: substitutions, deletions and insertions over len(ref)."""
    if not ref:
        return None
    prev = list(range(len(hyp) + 1))
    for i, r in enumerate(ref, 1):
        cur = [i]
        for j, h in enumerate(hyp, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (r != h)))
        prev = cur
    return prev[-1] / len(ref)


def sync(ref, lines):
    """Align normalized words with unit gap/substitution costs, then compare strictly increasing line pairs.

    At least half a reference line's words must match. The transcript owner with most matches wins (earliest
    on ties); an owner already used is not reused. p90 uses the nearest-rank absolute error percentile.
    No matched pairs leaves error measures undefined, not zero; within is over matched pairs only.
    """
    if not ref or not lines:
        return None
    ref = [(at, text.split()) for at, text in ref if text.split()]
    if not ref:
        return None
    rw = [(w, i) for i, (_, words) in enumerate(ref) for w in words]
    hw = [(w, j) for j, line in enumerate(lines) for w in transcribe.norm(line["text"]).split()]
    # Keep traceback directions in bytes; only the current/previous cost rows need Python integers.
    trace = [bytearray(len(hw) + 1)]
    prev = list(range(len(hw) + 1))
    for i, (r, _) in enumerate(rw, 1):
        cur, steps = [i], bytearray(len(hw) + 1)
        steps[0] = 1
        for j, (h, _) in enumerate(hw, 1):
            costs = (prev[j - 1] + (r != h), prev[j] + 1, cur[j - 1] + 1)
            step = min(range(3), key=costs.__getitem__)  # prefer the diagonal on equal costs
            steps[j] = step
            cur.append(costs[step])
        trace.append(steps)
        prev = cur
    counts = [{} for _ in ref]
    i, j = len(rw), len(hw)
    while i and j:
        step = trace[i][j]
        if step == 0:
            if rw[i - 1][0] == hw[j - 1][0]:
                owners, owner = counts[rw[i - 1][1]], hw[j - 1][1]
                owners[owner] = owners.get(owner, 0) + 1
            i, j = i - 1, j - 1
        elif step == 1:
            i -= 1
        else:
            j -= 1
    pairs, last = {}, -1
    for i, (_, words) in enumerate(ref):
        owners = counts[i]
        if sum(owners.values()) * 2 < len(words):
            continue
        owner = min(owners, key=lambda j: (-owners[j], j))
        if owner > last:
            pairs[i], last = owner, owner
    times = [ref[i][0] for i in pairs]
    errors = [lines[j]["start"] - ref[i][0] for i, j in pairs.items()]
    absolute = sorted(abs(e) for e in errors)
    drift = None
    if errors:
        mean_t, mean_e = statistics.mean(times), statistics.mean(errors)
        variance = sum((t - mean_t) ** 2 for t in times)
        drift = 60 * sum((t - mean_t) * (e - mean_e) for t, e in zip(times, errors)) / variance \
            if variance else 0.0
    # Simulate the player's latest-start-not-after-time rule, including unmatched/extra transcript lines.
    ri, hi, hits = 0, -1, 0
    samples = math.floor((ref[-1][0] - ref[0][0]) / 2) + 1
    for k in range(samples):
        at = ref[0][0] + 2 * k
        while ri + 1 < len(ref) and ref[ri + 1][0] <= at:
            ri += 1
        while hi + 1 < len(lines) and lines[hi + 1]["start"] <= at:
            hi += 1
        hits += ri in pairs and pairs[ri] == hi
    return {"matched": len(pairs) / len(ref), "p50": statistics.median(errors) if errors else None,
            "p90": absolute[math.ceil(0.9 * len(absolute)) - 1] if absolute else None, "drift": drift,
            "hit": hits / samples, "within": sum(e <= SYNC_WITHIN for e in absolute) / len(errors)
            if errors else None, "lines": len(errors)}


def distance(word, regions):
    """How far a word's middle lies outside the VAD regions (0 inside): it is in speech while this <= SPEECH_PAD."""
    mid = (word["start"] + word["end"]) / 2
    return min((max(s - mid, mid - e, 0.0) for s, e in regions), default=math.inf)


def pad_needed(words, regions):
    """The smallest SPEECH_PAD at which words_outside_speech would not fire, with today's OUTSIDE_SPEECH_MAX."""
    n = len(words)
    allowed = max(c for c in range(n + 1) if c / n <= transcribe.OUTSIDE_SPEECH_MAX)
    if allowed >= n:
        return 0.0
    return sorted((distance(w, regions) for w in words), reverse=True)[allowed]


def evaluate(rel, label, raw):
    """One song's report from its raw backend result."""
    t, out = transcribe, raw["out"]
    rec = t.judge(out, raw.get("identity") or {}, raw.get("language_override"))
    regions = out.get("vad", {}).get("regions", [])
    speech, duration = sum(e - s for s, e in regions), out.get("duration") or 0
    segments = out.get("segments", [])
    words = t.flat_words(segments)
    lines = t.make_lines(t.regroup(segments, words), words)
    ratio = speech / duration if duration else None
    m = {"speech_seconds": speech, "speech_ratio": ratio, "duration": duration, "words": len(words),
         "lines": len(lines), "outside_share": None, "pad_needed": None, "repeat_top": None, "repeat_next": None,
         "repeat_factor": None, "density": None}
    x = dict.fromkeys(c[0] for c in CHECKS)
    x["no_vocals_min_seconds"], x["no_vocals_min_ratio"] = speech, ratio
    catches = set()
    if speech < t.NO_VOCALS_MIN_SECONDS:
        catches.add("no_vocals_seconds")
    if ratio is not None and ratio < t.NO_VOCALS_MIN_RATIO:
        catches.add("no_vocals_ratio")
    if out.get("skipped") == "no_vocals" and not catches:
        catches.add("no_vocals_backend")  # skipped by the backend with other values than today's
    if words:
        w = t.measures(lines, words, regions)
        factor = w["repeat_top"] / max(w["repeat_next"], 1)
        m |= {"outside_share": w["outside_share"], "pad_needed": pad_needed(words, regions),
              "repeat_top": w["repeat_top"], "repeat_next": w["repeat_next"], "repeat_factor": factor,
              "density": w["density"]}
        x |= {"speech_pad": m["pad_needed"], "outside_speech_max": w["outside_share"], "density_min": w["density"],
              "repeat_min": w["repeat_top"] if w["repeat_top"] >= t.REPEAT_FACTOR * max(w["repeat_next"], 1)
              else None,
              "repeat_factor": factor if w["repeat_top"] >= t.REPEAT_MIN else None,
              "density_max": w["density"] if w["density"] >= t.DENSITY_MIN else None}
        catches |= set(t.withheld_reasons(lines, words, regions))
    if not lines:
        catches.add("no_words")
    song = {"file": rel, "label": label, "state": rec["state"], "reasons": rec["reasons"], "review": rec["review"],
            "measures": m, "inputs": x, "catches": sorted(catches)}
    if label == "lyrics":
        ref = reference(rel)
        song["reference_lines"] = len(ref) if ref else 0
        song["wer"] = wer(" ".join(w for _, w in ref).split(), t.norm(" ".join(w["word"] for w in words)).split()) \
            if ref else None
        song["sync"] = sync(ref, lines)
        song["ok"] = rec["state"] == "shown"
    else:
        song["no_vocals"] = rec["state"] == "no_vocals"
        song["ok"] = rec["state"] in ("no_vocals", "withheld")
    return song


# ---------------------------------------------------------------- separating ranges

def separating(songs, threshold, kind, catch):
    """The values of one threshold that keep its check off every lyrics song and make it catch every instrumental
    that no other check catches: {"range": [lo, lo_inclusive, hi, hi_inclusive] or None, "lyrics": [files that block
    it], "instrumentals": [files that block it], "relied_on": how many instrumentals need this check}.
    Review-only candidates bound lyrics flags only; they cannot catch instrumentals."""
    lyr = [s for s in songs if s["label"] == "lyrics" and s["inputs"][threshold] is not None]
    # A review flag cannot catch an instrumental, whatever its threshold.
    need = [s for s in songs if s["label"] == "instrumental" and not set(s["catches"]) - {catch}
            and catch != "words_outside_speech"]
    uncatchable = [s["file"] for s in need if s["inputs"][threshold] is None]
    need = [s for s in need if s["inputs"][threshold] is not None]
    x = lambda s: s["inputs"][threshold]
    xl, xi = [x(s) for s in lyr], [x(s) for s in need]
    if kind in ("gt", "ge"):  # fires at T below the input: lyrics bound it from below, instrumentals from above
        lo, hi, lo_in, hi_in = max(xl, default=-math.inf), min(xi, default=math.inf), kind == "gt", kind == "ge"
        bad_l, bad_i = [s for s in lyr if x(s) >= hi], [s for s in need if x(s) <= lo]
    else:  # fires at T above the input
        lo, hi, lo_in, hi_in = max(xi, default=-math.inf), min(xl, default=math.inf), False, True
        bad_l, bad_i = [s for s in lyr if x(s) <= lo], [s for s in need if x(s) >= hi]
    empty = lo > hi or (lo == hi and not (lo_in and hi_in))
    return {"threshold": threshold, "range": None if empty or uncatchable else [lo, lo_in, hi, hi_in],
            "lyrics": [s["file"] for s in bad_l] if empty else [],
            "instrumentals": ([s["file"] for s in bad_i] if empty else []) + uncatchable,
            "relied_on": len(need) + len(uncatchable), "review_only": catch == "words_outside_speech"}


def inside(rng, value):
    lo, lo_in, hi, hi_in = rng
    return (lo < value or (lo_in and lo == value)) and (value < hi or (hi_in and value == hi))


def ranges(songs):
    out = []
    for threshold, const, kind, catch in CHECKS:
        r = separating(songs, threshold, kind, catch)
        today = getattr(transcribe, const)
        out.append(r | {"today": today, "today_inside": bool(r["range"]) and inside(r["range"], today)})
    return out


# ---------------------------------------------------------------- the report

def fmt(x, spec=".3g"):
    if x is None:
        return "-"
    if isinstance(x, float) and math.isinf(x):
        return "inf" if x > 0 else "-inf"
    return format(x, spec)


def interval(rng):
    lo, lo_in, hi, hi_in = rng
    left, right = "[" if lo_in and math.isfinite(lo) else "(", "]" if hi_in and math.isfinite(hi) else ")"
    return f"{left}{fmt(lo)}, {fmt(hi)}{right}"


def plain(x):
    """JSON without infinities."""
    if isinstance(x, float) and math.isinf(x):
        return "inf" if x > 0 else "-inf"
    if isinstance(x, dict):
        return {k: plain(v) for k, v in x.items()}
    if isinstance(x, list):
        return [plain(v) for v in x]
    return x


def summary(songs):
    lyr = [s for s in songs if s["label"] == "lyrics" and "error" not in s]
    ins = [s for s in songs if s["label"] == "instrumental" and "error" not in s]
    wers = [s["wer"] for s in lyr if s["wer"] is not None]
    # median_sync remains a magnitude across songs, not the median signed p50 printed below.
    syncs = [abs(s["sync"]["p50"]) for s in lyr if s["sync"] and s["sync"]["p50"] is not None]
    drifts = [abs(s["sync"]["drift"]) for s in lyr if s["sync"] and s["sync"]["drift"] is not None]
    hits = [s["sync"]["hit"] for s in lyr if s["sync"]]
    return {"lyrics": len(lyr), "lyrics_shown": sum(s["ok"] for s in lyr),
            "instrumentals": len(ins), "instrumentals_caught": sum(s["ok"] for s in ins),
            "instrumentals_no_vocals": sum(s["no_vocals"] for s in ins),
            "mean_wer": statistics.mean(wers) if wers else None,
            "median_sync": statistics.median(syncs) if syncs else None,
            "median_drift": statistics.median(drifts) if drifts else None,
            "median_hit": statistics.median(hits) if hits else None,
            "failed": sum("error" in s for s in songs)}


def print_text(songs, rngs, raw_dir):
    print(f"{'label':<12} {'state':<9} {'ok':<3} {'outside':>7} {'pad':>5} {'repeat':>9} {'density':>7} "
          f"{'speech':>7} {'ratio':>5} {'WER':>5} {'p50':>6} {'p90':>5} {'drift(s/min)':>12} {'hit':>5}  file")
    for s in songs:
        if "error" in s:
            print(f"{s['label']:<12} {'failed':<9} {'no':<3} {'':>7} {'':>5} {'':>9} {'':>7} {'':>7} {'':>5} "
                  f"{'':>5} {'':>6} {'':>5} {'':>12} {'':>5}  {s['file']}")
            continue
        m, sy = s["measures"], s.get("sync")
        rep = f"{m['repeat_top']}/{m['repeat_next']} {fmt(m['repeat_factor'], '.1f')}x" if m["repeat_top"] else "-"
        print(f"{s['label']:<12} {s['state']:<9} {'yes' if s['ok'] else 'no':<3} "
              f"{fmt(m['outside_share'], '.2f'):>7} {fmt(m['pad_needed'], '.2f'):>5} {rep:>9} "
              f"{fmt(m['density'], '.2f'):>7} {fmt(m['speech_seconds'], '.1f'):>7} {fmt(m['speech_ratio'], '.2f'):>5} "
              f"{fmt(s.get('wer'), '.2f'):>5} {fmt(sy and sy['p50'], '+.2f'):>6} "
              f"{fmt(sy and sy['p90'], '.2f'):>5} {fmt(sy and sy['drift'], '+.1f'):>12} "
              f"{fmt(sy and sy['hit'] * 100, '.0f') + ('%' if sy else ''):>5}  {s['file']}")
    for s in songs:
        if "error" in s:
            print(f"{s['file']}: failed: {s['error']}")
        elif not s["ok"]:
            print(f"{s['file']}: {s['label']}, {s['state']}" + (f": {', '.join(s['reasons'])}" if s["reasons"] else ""))
        if s.get("review"):
            print(f"{s['file']}: {s['label']}, {s['state']}; review: {', '.join(s['review'])}")
    t = summary(songs)
    signed = [s['sync']['p50'] for s in songs if s.get('sync') and s['sync']['p50'] is not None]
    median_p50 = statistics.median(signed) if signed else None
    print(f"lyrics: {t['lyrics_shown']}/{t['lyrics']} shown, mean WER {fmt(t['mean_wer'], '.2f')}, median p50 "
          f"{fmt(median_p50, '+.2f')} s, median drift {fmt(t['median_drift'], '.1f')} s/min, "
          f"median hit {fmt(t['median_hit'] * 100 if t['median_hit'] is not None else None, '.0f')}%; "
          f"instrumentals: {t['instrumentals_caught']}/{t['instrumentals']} "
          f"no_vocals or withheld ({t['instrumentals_no_vocals']} no_vocals)"
          + (f"; {t['failed']} failed" if t["failed"] else ""))
    print("separating ranges (each threshold alone, the others at today's values):")
    for r in rngs:
        head = f"  {r['threshold']:<22} today {fmt(r['today'], 'g'):<6} "
        if r["review_only"]:
            head += "review flag only: "
        if r["range"]:
            print(head + interval(r["range"]) + (" (today inside)" if r["today_inside"] else " (today outside)")
                  + ("" if r["relied_on"] else ", no instrumental relies on it"))
        else:
            blocks = [f"lyrics {', '.join(r['lyrics'])}"] * bool(r["lyrics"]) + \
                     [f"instrumentals {', '.join(r['instrumentals'])}"] * bool(r["instrumentals"])
            print(head + "no value separates them: " + "; ".join(blocks))
    print(f"  {', '.join(BACKEND_ONLY)}: change the backend's output; rerun with other constants")
    print(f"raw backend results: {raw_dir}")


def cmd(a):
    songs_in = read_list(a.list)
    if a.from_dir:
        raw_dir = pathlib.Path(a.from_dir)
        if not raw_dir.is_dir():
            sys.exit(f"musicdb lyrics transcribe-eval: no directory {raw_dir}")
    else:
        transcribe.check_backend()
        raw_dir = pathlib.Path(a.out) if a.out else pathlib.Path(tempfile.mkdtemp(prefix="transcribe-eval-"))
        raw_dir.mkdir(parents=True, exist_ok=True)
    print(f"raw backend results: {raw_dir}", file=sys.stderr)
    songs = []
    for rel, label in songs_in:
        try:
            raw, why = obtain(rel, None if a.from_dir else raw_dir, raw_dir if a.from_dir else None)
            songs.append(evaluate(rel, label, raw) if raw else {"file": rel, "label": label, "error": why,
                                                                  "ok": False})
        except Exception as e:  # one bad song must not lose the others' results
            songs.append({"file": rel, "label": label, "error": f"{type(e).__name__}: {e}", "ok": False})
    judged = [s for s in songs if "error" not in s]
    rngs = ranges(judged)
    if a.json:
        print(json.dumps(plain({"raw_dir": str(raw_dir), "thresholds": transcribe.thresholds(),
                                "summary": summary(songs), "songs": songs, "ranges": rngs}), ensure_ascii=False))
    else:
        print_text(songs, rngs, raw_dir)
    if len(judged) < len(songs):
        sys.exit(1)
