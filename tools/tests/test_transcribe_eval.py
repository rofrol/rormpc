"""musicdb lyrics transcribe-eval with a fake backend: WER, sync, the separating ranges, --from reuse and no writes
to the lyrics dir. No audio is decoded and no model runs."""
import hashlib, json, math, types

import pytest

from rormpc_tools import lyrics, settings, transcribe, transcribe_eval as ev, translation
from test_transcribe import VERSE, result, words_of


def test_wer_counts_substitutions_deletions_and_insertions():
    assert ev.wer("a b c d".split(), "a x c".split()) == 0.5  # b->x, d deleted
    assert ev.wer(["a"], ["a", "b", "c"]) == 2.0
    assert ev.wer(VERSE[0].split(), VERSE[0].split()) == 0
    assert ev.wer([], ["a"]) is None


def transcript(ref, offset=0.0, scale=1.0):
    return [{"start": at * scale + offset, "text": text} for at, text in ref]


def test_sync_signed_median_and_share_within_matched_pairs():
    ref = [(10.0, "one"), (20.0, "two"), (31.5, "three")]
    lines = [{"start": at, "text": text} for at, text in
             [(10.2, "one"), (19.0, "two"), (30.0, "three"), (60.0, "extra")]]
    s = ev.sync(ref, lines)
    assert s["p50"] == pytest.approx(-1.0)
    assert s["p90"] == pytest.approx(1.5)
    assert s["within"] == pytest.approx(2 / 3) and s["lines"] == 3 and s["matched"] == 1
    assert ev.sync(ref, []) is None
    assert ev.sync(None, lines) is None


@pytest.mark.parametrize("offset", [0.0, 2.0])
def test_sync_over_segmentation_is_not_rewarded(offset):
    # Samples fall at the reference starts; equal-size fragments tie to the earliest owner.
    ref = [(10.0, "one two three four five six"), (12.0, "seven eight nine ten eleven twelve"),
           (14.0, "red green blue black white gold")]
    whole = ev.sync(ref, transcript(ref, offset=offset))
    split = [{"start": at + offset + k * 0.5, "text": " ".join(text.split()[2*k:2*k+2])}
             for at, text in ref for k in range(3)]
    fragmented = ev.sync(ref, split)
    assert fragmented["p50"] == pytest.approx(whole["p50"], abs=0.01)
    assert fragmented["hit"] == whole["hit"]
    assert fragmented["matched"] == 1


def test_sync_fragment_changes_highlight_between_reference_starts():
    ref = [(10.0, "one two three four five six"), (16.0, "seven eight nine ten eleven twelve")]
    split = [{"start": at + k, "text": " ".join(text.split()[2*k:2*k+2])}
             for at, text in ref for k in range(3)]
    # Later fragments are different displayed lines, not extra chances to match the reference start.
    assert ev.sync(ref, transcript(ref))["hit"] == 1
    assert ev.sync(ref, split)["hit"] == 0.5


def test_sync_constant_offset_and_clock_drift():
    ref = [(10.0, "one two"), (12.0, "three four"), (14.0, "five six")]
    late = ev.sync(ref, transcript(ref, offset=2.0))
    assert late["p50"] == pytest.approx(2.0) and late["drift"] == pytest.approx(0)
    assert late["hit"] == 0
    scaled = ev.sync(ref, transcript(ref, scale=1.1))
    assert scaled["drift"] == pytest.approx(6.0) and scaled["hit"] == 0


def test_sync_absent_words_lower_coverage_without_timing_error():
    ref = [(10.0, "one two"), (12.0, "absent words"), (14.0, "three four")]
    s = ev.sync(ref, transcript([ref[0], ref[2]]))
    assert s["matched"] == pytest.approx(2 / 3) and s["lines"] == 2
    assert s["p50"] == 0 and s["p90"] == 0 and s["within"] == 1
    assert s["hit"] == pytest.approx(2 / 3)
    none = ev.sync(ref, [{"start": 1000, "text": "unrelated"}])
    assert none == {"matched": 0, "lines": 0, "p50": None, "p90": None, "drift": None,
                    "within": None, "hit": 0}


def test_sync_half_words_and_strictly_increasing_line_owners():
    ref = [(10.0, "one two three four"), (12.0, "five six"), (14.0, "seven eight nine")]
    s = ev.sync(ref, [{"start": 10.0, "text": "ONE, two five six"}, {"start": 14.0, "text": "seven"}])
    # First line has half its words; second cannot reuse its owner; third has less than half.
    assert s["matched"] == pytest.approx(1 / 3) and s["lines"] == 1
    assert s["p50"] == 0 and s["drift"] == 0


def test_summary_uses_absolute_song_medians_and_includes_unmatched_hits():
    def record(p50, drift, hit):
        return {"label": "lyrics", "ok": True, "wer": 0,
                "sync": {"p50": p50, "drift": drift, "hit": hit}}
    s = ev.summary([record(-2, -6, 0.25), record(4, 2, 0.75), record(None, None, 0)])
    assert s["median_sync"] == 3 and s["median_drift"] == 4 and s["median_hit"] == 0.25
    assert ev.summary([])["median_sync"] is None
    assert ev.summary([])["median_drift"] is None
    assert ev.summary([])["median_hit"] is None


def test_reference_in_time_order_normalized_with_offset(tmp_path, monkeypatch):
    monkeypatch.setattr(lyrics, "LYRICS", tmp_path)
    (tmp_path / "A").mkdir()
    (tmp_path / "A/s.lrc").write_text("[ar:X]\n[offset:+500]\n[00:20.50]Hello, World!\n[00:10.50][00:30.50]Oh, yeah\n"
                                      "[00:40.00]\n")
    assert ev.reference("A/s.mp3") == [(10.0, "oh yeah"), (20.0, "hello world"), (30.0, "oh yeah")]
    assert ev.reference("A/none.mp3") is None


def test_pad_needed_is_where_words_outside_speech_stops(monkeypatch):
    words = [{"start": t, "end": t} for t in (10.0, 11.0, 12.0, 21.0, 23.0)]  # two words 1 and 3 s after the region
    regions = [(9.0, 20.0)]
    pad = ev.pad_needed(words, regions)  # 0.25 of 5 words: one may lie outside, so the nearer one decides
    assert pad == pytest.approx(1.0)
    for p, fires in ((pad, False), (pad - 0.01, True)):
        monkeypatch.setattr(transcribe, "SPEECH_PAD", p)
        assert ("words_outside_speech" in transcribe.review_flags([], words, regions)) is fires


def song(name, label, catches=(), **inputs):
    return {"file": name, "label": label, "inputs": dict.fromkeys(c[0] for c in ev.CHECKS) | inputs,
            "catches": list(catches)}


def test_separating_ranges():
    songs = [song("l1", "lyrics", outside_speech_max=0.1), song("l2", "lyrics", outside_speech_max=0.2),
             song("i1", "instrumental", ["words_outside_speech"], outside_speech_max=0.6),
             song("i2", "instrumental", ["words_outside_speech"], outside_speech_max=0.5),
             # caught by no_vocals anyway: no constraint
             song("i3", "instrumental", ["no_vocals_seconds"], outside_speech_max=0.05)]
    r = ev.separating(songs, "outside_speech_max", "gt", "words_outside_speech")
    assert r["range"] == [0.2, True, math.inf, False] and r["relied_on"] == 0 and r["review_only"]
    assert ev.inside(r["range"], 0.2) and ev.inside(r["range"], 0.5)
    assert ev.interval(r["range"]) == "[0.2, inf)"
    blocked = ev.separating(songs + [song("l3", "lyrics", outside_speech_max=0.55)], "outside_speech_max", "gt",
                            "words_outside_speech")
    assert blocked["range"] == [0.55, True, math.inf, False] and blocked["instrumentals"] == []
    # fires below its input: density_min, lyrics bound it from above
    d = ev.separating([song("l1", "lyrics", density_min=1.0), song("l2", "lyrics", density_min=2.0),
                       song("i1", "instrumental", density_min=0.1)], "density_min", "lt", "low_word_density")
    assert d["range"] == [0.1, False, 1.0, True] and ev.interval(d["range"]) == "(0.1, 1]"
    # an integer threshold that fires at or above: (3, 6] is 4..6
    rp = ev.separating([song("l1", "lyrics", repeat_min=3), song("i1", "instrumental", repeat_min=6)], "repeat_min",
                       "ge", "repeated_line")
    assert rp["range"] == [3, False, 6, True] and not ev.inside(rp["range"], 3) and ev.inside(rp["range"], 6)
    # an instrumental this check cannot catch at all
    none = ev.separating([song("l1", "lyrics", repeat_min=3), song("i1", "instrumental")], "repeat_min", "ge",
                         "repeated_line")
    assert none["range"] is None and none["instrumentals"] == ["i1"] and none["relied_on"] == 1
    # nothing relies on it: only the lyrics bound it
    free = ev.separating([song("l1", "lyrics", density_max=3.0)], "density_max", "gt", "high_word_density")
    assert free["range"] == [3.0, True, math.inf, False] and free["relied_on"] == 0


def lrc_of(segments):
    return "".join(f"[{int(s['start'] // 60):02d}:{s['start'] % 60:05.2f}]{s['text'].strip()}\n" for s in segments)


@pytest.fixture
def ev_env(tmp_path, monkeypatch):
    lyr, music = tmp_path / "lyrics", tmp_path / "music"
    lyr.mkdir(); music.mkdir()
    monkeypatch.setattr(lyrics, "LYRICS", lyr)
    monkeypatch.setattr(lyrics, "INDEX", lyr / "index.json")
    monkeypatch.setattr(translation, "LYRICS", lyr)
    monkeypatch.setattr(settings, "MUSIC_DIR", music)
    monkeypatch.setattr(transcribe, "check_backend", lambda: None)
    monkeypatch.setattr(transcribe, "audio_identity",
                        lambda p: {"pcm_sha256": hashlib.sha256(p.read_bytes()).hexdigest(), "samples": 1, "rate": 16000})
    ns = types.SimpleNamespace(lyrics=lyr, music=music, calls=[], outs={})

    def fake_backend(path, identity, language):
        rel = path.relative_to(music).as_posix()
        ns.calls.append(rel)
        return ns.outs[rel]

    monkeypatch.setattr(transcribe, "run_backend", fake_backend)
    segs = words_of(*VERSE)
    for rel in ("A/sung.mp3", "A/loop.mp3", "B/instrumental.mp3"):
        (music / rel).parent.mkdir(parents=True, exist_ok=True)
        (music / rel).write_bytes(rel.encode())
    (lyr / "A").mkdir()
    (lyr / "A/sung.lrc").write_text(lrc_of(segs))
    (lyr / "A/loop.lrc").write_text(lrc_of(segs))
    lyrics.save({"A/sung.mp3": {"state": "synced"}, "A/loop.mp3": {"state": "synced"}})
    ns.outs = {"A/sung.mp3": result(segments=segs),
               # sung words, but half of them after the VAD's last region
               "A/loop.mp3": result(segments=segs, regions=[(5.0, 20.0)]),
               "B/instrumental.mp3": result(segments=[], regions=[], skipped="no_vocals")}
    ns.list = tmp_path / "eval.tsv"
    ns.list.write_text("# a comment\nA/sung.mp3\tlyrics\n\nA/loop.mp3\tlyrics\nB/instrumental.mp3\tinstrumental\n")
    return ns


def listing(d):
    return sorted((p.relative_to(d).as_posix(), p.read_bytes()) for p in d.rglob("*") if p.is_file())


def run_json(capsys, *argv):
    lyrics.main(["transcribe-eval", *argv, "--json"])
    return json.loads(capsys.readouterr().out)


def test_report_wer_sync_states_and_no_lyrics_writes(ev_env, tmp_path, capsys):
    before = listing(ev_env.lyrics)
    out = tmp_path / "raw"
    rep = run_json(capsys, str(ev_env.list), "--out", str(out))
    assert listing(ev_env.lyrics) == before  # no sidecar, .lrc, .txt or index.json change
    by = {s["file"]: s for s in rep["songs"]}
    sung, loop, inst = by["A/sung.mp3"], by["A/loop.mp3"], by["B/instrumental.mp3"]
    assert sung["state"] == "shown" and sung["ok"] and sung["wer"] == 0
    assert sung["sync"]["p50"] == pytest.approx(0, abs=0.006) and sung["sync"]["within"] == 1
    assert sung["sync"]["matched"] == 1
    assert sung["measures"]["outside_share"] == 0 and sung["measures"]["speech_seconds"] == 115.0
    assert loop["state"] == "shown" and loop["reasons"] == [] and loop["ok"]
    assert loop["review"] == ["words_outside_speech"] and "words_outside_speech" not in loop["catches"]
    assert loop["measures"]["outside_share"] > transcribe.OUTSIDE_SPEECH_MAX and loop["wer"] == 0
    assert inst["state"] == "no_vocals" and inst["no_vocals"] and inst["ok"]
    assert rep["summary"]["lyrics_shown"] == 2 and rep["summary"]["instrumentals_caught"] == 1
    r = {x["threshold"]: x for x in rep["ranges"]}
    pad = r["speech_pad"]
    assert pad["range"][0] == pytest.approx(loop["measures"]["pad_needed"]) and not pad["today_inside"]
    assert r["outside_speech_max"]["range"][0] == pytest.approx(loop["measures"]["outside_share"])
    # the review-flagged song's 15 s of speech bound it; the instrumental is caught by the ratio as well, so not from below
    assert r["no_vocals_min_seconds"]["range"] == ["-inf", False, 15.0, True]
    assert r["no_vocals_min_seconds"]["today_inside"] and r["no_vocals_min_seconds"]["relied_on"] == 0
    assert sorted(p.name for p in out.iterdir()) == sorted(ev.raw_path(out, f).name for f in by)


def test_rerun_reuses_raw_results_and_from_needs_no_backend(ev_env, tmp_path, capsys, monkeypatch):
    out = tmp_path / "raw"
    first = run_json(capsys, str(ev_env.list), "--out", str(out))
    assert len(ev_env.calls) == 3
    assert run_json(capsys, str(ev_env.list), "--out", str(out))["songs"] == first["songs"]
    assert len(ev_env.calls) == 3  # same backend inputs: reused

    def no_backend(*a):
        raise AssertionError("the backend ran")

    monkeypatch.setattr(transcribe, "run_backend", no_backend)
    monkeypatch.setattr(transcribe, "check_backend", no_backend)
    monkeypatch.setattr(transcribe, "audio_identity", no_backend)
    again = run_json(capsys, str(ev_env.list), "--from", str(out))
    assert again["songs"] == first["songs"]
    # judging again with another threshold, from the same raw results
    monkeypatch.setattr(transcribe, "OUTSIDE_SPEECH_MAX", 0.9)
    changed = {s["file"]: s for s in run_json(capsys, str(ev_env.list), "--from", str(out))["songs"]}["A/loop.mp3"]
    assert changed["state"] == "shown" and changed["review"] == []


def test_changed_backend_inputs_run_again(ev_env, tmp_path, capsys, monkeypatch):
    out = tmp_path / "raw"
    run_json(capsys, str(ev_env.list), "--out", str(out))
    monkeypatch.setattr(transcribe, "VAD_THRESHOLD", 0.4)
    run_json(capsys, str(ev_env.list), "--out", str(out))
    assert len(ev_env.calls) == 6


def test_from_missing_song_fails_after_reporting_the_rest(ev_env, tmp_path, capsys):
    out = tmp_path / "raw"
    run_json(capsys, str(ev_env.list), "--out", str(out))
    ev.raw_path(out, "A/loop.mp3").unlink()
    with pytest.raises(SystemExit) as e:
        lyrics.main(["transcribe-eval", str(ev_env.list), "--from", str(out)])
    assert e.value.code == 1
    text = capsys.readouterr().out
    assert "A/loop.mp3: failed: no saved result" in text and "lyrics: 1/1 shown" in text
    assert "separating ranges" in text and "speech_pad" in text


def test_text_table(ev_env, tmp_path, capsys):
    lyrics.main(["transcribe-eval", str(ev_env.list), "--out", str(tmp_path / "raw")])
    text = capsys.readouterr().out
    assert "A/loop.mp3: lyrics, shown; review: words_outside_speech" in text
    assert "lyrics: 2/2 shown" in text and "instrumentals: 1/1 no_vocals or withheld (1 no_vocals)" in text
    assert "outside_speech_max" in text and "review flag only:" in text and "(today outside)" in text
    assert all(column in text.splitlines()[0].split() for column in ("p50", "p90", "drift(s/min)", "hit"))
    assert "median p50 +0.00 s, median drift 0.0 s/min, median hit" in text


def test_text_summary_p50_keeps_sign_and_failed_row_file_alignment(ev_env, capsys):
    rec = ev.evaluate("A/sung.mp3", "lyrics", {"out": ev_env.outs["A/sung.mp3"]})
    rec["sync"] = ev.sync([(10.0, "one"), (12.0, "two")],
                          transcript([(10.0, "one"), (12.0, "two")], offset=-2))
    failed = {"file": "missing.mp3", "label": "lyrics", "error": "missing", "ok": False}
    ev.print_text([rec, failed], [], "unused")
    text = capsys.readouterr().out
    header, shown, failure = text.splitlines()[:3]
    assert shown.index(rec["file"]) == failure.index(failed["file"]) == header.index("file")
    assert "median p50 -2.00 s, median drift 0.0 s/min, median hit 50%" in text
    assert ev.summary([rec])["median_sync"] == 2


def test_review_flag_does_not_catch_an_instrumental(ev_env):
    rec = ev.evaluate("A/loop.mp3", "instrumental", {"out": ev_env.outs["A/loop.mp3"]})
    assert rec["state"] == "shown" and rec["reasons"] == []
    assert rec["review"] == ["words_outside_speech"] and not rec["ok"]
    assert rec["catches"] == []
    assert ev.summary([rec])["instrumentals_caught"] == 0


def test_withheld_record_also_reports_review(ev_env, tmp_path, capsys):
    ev_env.outs["A/loop.mp3"] = result(segments=words_of("Subtitles by the Amara.org community", start=130.0))
    rep = run_json(capsys, str(ev_env.list), "--out", str(tmp_path / "raw"))
    rec = next(s for s in rep["songs"] if s["file"] == "A/loop.mp3")
    assert rec["state"] == "withheld" and not rec["ok"]
    assert "credit_line_outside_speech" in rec["reasons"] and "words_outside_speech" not in rec["reasons"]
    assert rec["review"] == ["words_outside_speech"]
    assert "words_outside_speech" not in rec["catches"]
    assert rep["summary"]["lyrics_shown"] == 1
    ev.print_text(rep["songs"], [], "unused")
    text = capsys.readouterr().out
    assert "A/loop.mp3: lyrics, withheld; review: words_outside_speech" in text


def test_bad_list_line(tmp_path):
    p = tmp_path / "eval.tsv"
    p.write_text("A/song.mp3 lyrics\n")
    with pytest.raises(SystemExit) as e:
        ev.read_list(p)
    assert ":1:" in str(e.value)
    p.write_text("A/song.mp3\tsung\n")
    with pytest.raises(SystemExit):
        ev.read_list(p)
