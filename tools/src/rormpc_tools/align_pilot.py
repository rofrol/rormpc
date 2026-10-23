"""Compare cached Whisper words with forced alignment; never transcribe or write lyrics."""
import ast
import json
import math
import pathlib
import statistics
import struct
import subprocess
import sys
import tempfile
import wave

from . import transcribe, transcribe_align, transcribe_eval

BACKEND = pathlib.Path(__file__).with_name("data") / "align_backend.py"
# Consult round 20261018-113455-9cab (Sol, MiMo, Argon); seconds and seconds/minute.
P50_MAX = 0.35
P90_MAX = 1.20
DRIFT_MAX = 0.50
HIT_GAIN = 0.10
HIT_TARGET = 0.75
P90_REGRESSION_MAX = 0.25
ALIGNED_MIN = 0.70
ABANDON_IMPROVEMENT = 0.15
COHORT_SIZE = 10


def read_list(path):
    """Evaluation TSV (path, label), also accepting label first; both cohorts need LRCs."""
    songs = []
    for n, line in enumerate(pathlib.Path(path).read_text().splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        fields = [v.strip() for v in line.split("\t")]
        labels = {"lyrics", "held-out"}
        if len(fields) == 2 and fields[0] in labels:
            fields.reverse()
        if len(fields) != 2 or fields[1] not in labels:
            raise ValueError(f"{path}:{n}: want path<TAB>lyrics|held-out (both with synced lyrics)")
        rel, label = fields
        if not rel or pathlib.Path(rel).is_absolute() or ".." in pathlib.Path(rel).parts:
            raise ValueError(f"{path}:{n}: expected a path relative to the music directory")
        if any(old == rel for old, _ in songs):
            raise ValueError(f"{path}:{n}: duplicate song {rel}")
        songs.append((rel, label))
    return songs


def reference_partition(ref, words):
    """Lock word membership once, by lexical edit alignment, never by candidate times.

    Insertions remain in the preceding line; deletions do not fabricate words. A
    reference line with no matching word or a non-increasing first match is omitted
    from both the returned reference and partition, rather than a free timing hit.
    The traceback tie order matches transcribe_eval.sync().
    """
    rw = [(w, i) for i, (_, text) in enumerate(ref) for w in text.split()]
    hw = [(token, i) for i, w in enumerate(words) for token in transcribe.norm(w["word"]).split()]
    trace = [bytearray(len(hw) + 1)]
    prev = list(range(len(hw) + 1))
    for i, (r, _) in enumerate(rw, 1):
        cur, steps = [i], bytearray(len(hw) + 1)
        steps[0] = 1
        for j, (h, _) in enumerate(hw, 1):
            costs = (prev[j - 1] + (r != h), prev[j] + 1, cur[j - 1] + 1)
            step = min(range(3), key=costs.__getitem__)
            steps[j] = step
            cur.append(costs[step])
        trace.append(steps)
        prev = cur
    matches = [[] for _ in ref]
    i, j = len(rw), len(hw)
    while i and j:
        step = trace[i][j]
        if step == 0:
            if rw[i - 1][0] == hw[j - 1][0]:
                matches[rw[i - 1][1]].append(hw[j - 1][1])
            i, j = i - 1, j - 1
        elif step == 1:
            i -= 1
        else:
            j -= 1
    kept, starts = [], []
    for line, matched in zip(ref, matches):
        if not matched:
            continue
        first = min(matched)
        if starts and first <= starts[-1]:
            continue
        kept.append(line)
        starts.append(first)
    if starts:
        starts[0] = 0  # Leading insertions belong to the first scorable line.
    groups = [list(range(a, b)) for a, b in zip(starts, starts[1:] + [len(words)])]
    return kept, groups


def reference_groups(ref, words):
    return reference_partition(ref, words)[1]


def check_words(raw, refined):
    if [w["word"] for w in raw] != [w["word"] for w in refined]:
        raise ValueError("word identity changed: refined text/order differs from raw Whisper (WER changed)")
    return True


def score(ref, raw, refined, groups=None):
    check_words(raw, refined)
    if groups is None:
        ref, groups = reference_partition(ref, raw)
    def lines(words):
        return [{"start": words[g[0]]["start"], "text": " ".join(words[i]["word"] for i in g)}
                for g in groups]
    return {"raw": transcribe_eval.sync(ref, lines(raw)),
            "refined": transcribe_eval.sync(ref, lines(refined)), "words_identical": True}


def gates(songs):
    """Equal-song medians; overall p90 improvement is the relative drop in median p90.

    Hit is the equal-song mean; aligned share is word-weighted. Undefined measures
    fail closed. ABANDON takes precedence over PASS, including a zero-error baseline
    (no improvement possible). Partial runs are marked separately by cmd().
    """
    valid = [s for s in songs if s.get("status") == "ok"]
    valid = [s for s in valid if all(s.get(side) and all(
        isinstance(s[side].get(k), (int, float)) and math.isfinite(s[side][k])
        for k in ("p50", "p90", "drift", "hit")) for side in ("raw", "refined"))]
    if not valid:
        return {"verdict": "INCOMPLETE", "gates": [], "reason": "no scorable songs"}
    med = lambda side, k, absolute=False: statistics.median(
        abs(s[side][k]) if absolute else s[side][k] for s in valid)
    p50, p90, drift = med("refined", "p50", True), med("refined", "p90"), med("refined", "drift", True)
    raw_p90 = med("raw", "p90")
    improvement = (raw_p90 - p90) / raw_p90 if raw_p90 else 0.0
    raw_hit = statistics.mean(s["raw"]["hit"] for s in valid)
    hit = statistics.mean(s["refined"]["hit"] for s in valid)
    regression = max(s["refined"]["p90"] - s["raw"]["p90"] for s in valid)
    aligned = sum(s["words_aligned"] for s in valid)
    total = sum(s["words_aligned"] + s["words_kept"] for s in valid)
    share = aligned / total if total else 0.0
    checks = [(p50 <= P50_MAX, f"median |p50| {p50:.3f} <= {P50_MAX:.2f} s"),
              (p90 <= P90_MAX, f"median p90 {p90:.3f} <= {P90_MAX:.2f} s"),
              (drift <= DRIFT_MAX, f"median |drift| {drift:.3f} <= {DRIFT_MAX:.2f} s/min"),
              (hit - raw_hit >= HIT_GAIN - 1e-12 or hit >= HIT_TARGET,
               f"mean hit {hit:.3f}, gain {hit - raw_hit:.3f}: gain >= {HIT_GAIN} or hit >= {HIT_TARGET}"),
              (regression <= P90_REGRESSION_MAX, f"worst p90 regression {regression:.3f} <= {P90_REGRESSION_MAX} s"),
              (share >= ALIGNED_MIN, f"aligned words {share:.1%} >= {ALIGNED_MIN:.0%}")]
    verdict = "ABANDON" if improvement < ABANDON_IMPROVEMENT else (
        "PASS" if all(ok for ok, _ in checks) and len(valid) == len(songs) else "FAIL")
    return {"verdict": verdict, "p90_improvement": improvement,
            "abandon_below": ABANDON_IMPROVEMENT,
            "gates": [{"number": i, "passed": ok, "detail": detail}
                      for i, (ok, detail) in enumerate(checks, 1)]}


def stem_wav(source, destination):
    """Read the transcribe cache's 1-D float32 NPY without importing model dependencies.

    Only np.save's non-object, contiguous float32 format is accepted. The disposable
    PCM16 WAV is a backend input, not a new cache stem or a library artifact.
    """
    with source.open("rb") as f:
        if f.read(6) != b"\x93NUMPY":
            raise ValueError("invalid cached stem: not NPY")
        version = f.read(2)
        if version not in (b"\x01\x00", b"\x02\x00"):
            raise ValueError("unsupported cached stem NPY version")
        fmt = "<H" if version[0] == 1 else "<I"
        size = struct.unpack(fmt, f.read(struct.calcsize(fmt)))[0]
        header = ast.literal_eval(f.read(size).decode("latin1"))
        shape = header["shape"]
        if header["descr"] not in ("<f4", ">f4") or len(shape) != 1:
            raise ValueError("cached stem must be mono float32")
        data = f.read()
    if len(data) != shape[0] * 4 or not data:
        raise ValueError("invalid cached stem length")
    pcm = bytearray()
    for (sample,) in struct.iter_unpack(header["descr"][0] + "f", data):
        if not math.isfinite(sample):
            raise ValueError("nonfinite cached stem sample")
        pcm.extend(struct.pack("<h", round(max(-1.0, min(1.0, sample)) * 32767)))
    with wave.open(str(destination), "wb") as out:
        out.setparams((1, 2, transcribe.RATE, 0, "NONE", "not compressed"))
        out.writeframes(pcm)


def run_backend(stem, words, backend):
    with tempfile.TemporaryDirectory(prefix="align-pilot-") as tmp:
        tmp = pathlib.Path(tmp)
        audio, text = tmp / "vocals.wav", tmp / "words.json"
        stem_wav(stem, audio)
        text.write_text(json.dumps([w["word"] for w in words], ensure_ascii=False))
        p = subprocess.run(["uv", "run", "--quiet", "--script", str(backend),
                            "--audio", str(audio), "--words", str(text)], stdout=subprocess.PIPE, text=True)
        if p.returncode:
            raise RuntimeError(f"alignment backend exited with {p.returncode}")
        return json.loads(p.stdout)


def evaluate(rel, label, raw_dir, backend):
    rec = {"file": rel, "label": label}
    raw = transcribe_eval.load_raw(raw_dir, rel)
    if not raw:
        return rec | {"status": "skipped", "reason": f"no saved Whisper result in {raw_dir}"}
    key = (raw.get("identity") or {}).get("pcm_sha256", "")
    if not key or any(c not in "0123456789abcdef" for c in key):
        return rec | {"status": "skipped", "reason": "no cached vocal stem identity"}
    stem = transcribe.CACHE / f"{key}.npy"
    if not stem.is_file():
        return rec | {"status": "skipped", "reason": f"missing cached vocal stem: {stem}"}
    ref = transcribe_eval.reference(rel)
    if not ref:
        return rec | {"status": "skipped", "reason": "no synced lyrics reference"}
    words = transcribe.flat_words(raw["out"].get("segments", []))
    reduced, groups = reference_partition(ref, words)
    rec.update(ref_lines=len(ref), ref_lines_scored=len(reduced))
    if len(reduced) * 2 < len(ref):
        return rec | {"status": "skipped",
                      "reason": f"unscorable: {len(reduced)} of {len(ref)} reference lines"}
    ref = reduced
    out = run_backend(stem, words, backend)
    args = (words, out["emissions"], out["frame_rate"], out["vocabulary"])
    options = {"blank_id": out.get("blank_id", 0), "romanized": out.get("words_uroman")}
    lines, summary = transcribe_align.refine_lines(*args, groups=groups, **options)
    # refine_lines exposes only display lines and aggregate counts. Inspect the
    # same deterministic alignment separately for exact word identity and reasons.
    refined = transcribe_align.align_words(*args, **options)
    check_words(words, refined)
    if lines != [{"start": refined[g[0]]["start"], "end": refined[g[-1]]["end"],
                  "text": "".join(refined[i]["word"] for i in g).strip(), "words": [g[0], g[-1]]}
                 for g in groups]:
        raise ValueError("refine_lines output differs from the checked word alignment")
    reasons = sorted({w["reason"] for w in refined if not w["aligned"]})
    return rec | {"status": "ok", **score(ref, words, refined, groups), **summary,
                  "fallback_reason": reasons or None}


def cmd(a):
    songs = read_list(a.list)
    if a.limit is not None:
        if a.limit <= 0:
            sys.exit("align-pilot: --limit must be positive")
        songs = songs[:a.limit]
    out_dir = pathlib.Path(a.out)
    # transcribe-eval has no persistent default: its default is a fresh temp dir.
    # Reuse --out as the raw directory unless --raw names that earlier eval run.
    raw_dir = pathlib.Path(a.raw) if a.raw else out_dir
    results = []
    for rel, label in songs:
        try:
            results.append(evaluate(rel, label, raw_dir, pathlib.Path(a.backend) if a.backend else BACKEND))
        except Exception as exc:
            results.append({"file": rel, "label": label, "status": "error", "reason": str(exc)})
    verdict = gates(results)
    complete = all(sum(s["label"] == label and s["status"] == "ok" for s in results) == COHORT_SIZE
                   for label in ("lyrics", "held-out")) and len(results) == 2 * COHORT_SIZE
    verdict["protocol_complete"] = complete
    if not complete and verdict["verdict"] != "ABANDON":
        verdict["verdict"] = "INCOMPLETE"
    report = {"raw_dir": str(raw_dir), "songs": results, **verdict}
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "pilot.json").write_text(json.dumps(report, ensure_ascii=False, indent=2, allow_nan=False) + "\n")
    stream = sys.stderr if a.json else sys.stdout
    print("song\tcohort\tref_lines\tref_lines_scored\traw p50/p90/drift/hit -> refined\taligned/kept\tmean shift", file=stream)
    for s in results:
        counts = f"{s.get('ref_lines', 'n/a')}\t{s.get('ref_lines_scored', 'n/a')}"
        if s["status"] != "ok":
            print(f"{s['file']}\t{s['status']}: {s['reason']}\t{counts}", file=stream)
            continue
        def metrics(side):
            return "/".join(f"{s[side][k]:.3f}" if s[side][k] is not None else "n/a"
                            for k in ("p50", "p90", "drift", "hit"))
        print(f"{s['file']}\t{s['label']}\t{counts}\t{metrics('raw')} -> {metrics('refined')}\t"
              f"{s['words_aligned']}/{s['words_kept']}\t{s['mean_shift_s']:.3f}"
              f"\t{s['fallback_reason'] or '-'}", file=stream)
    for gate in verdict["gates"]:
        print(f"gate {gate['number']}: {'PASS' if gate['passed'] else 'FAIL'}: {gate['detail']}", file=stream)
    if "p90_improvement" in verdict:
        print(f"p90 improvement {verdict['p90_improvement']:.1%}; ABANDON below {ABANDON_IMPROVEMENT:.0%}", file=stream)
    if not complete:
        print("protocol incomplete: need 10 lyrics + 10 held-out songs, all scored with synced references", file=stream)
    print(verdict["verdict"], file=stream)
    if a.json:
        print(json.dumps(report, ensure_ascii=False, allow_nan=False))
    if any(s["status"] == "error" for s in results):
        sys.exit(1)
