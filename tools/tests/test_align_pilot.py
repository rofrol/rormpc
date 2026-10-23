"""Offline pilot protocol and arithmetic; no Whisper, model imports or downloads."""
import argparse
import copy
import json
import struct
import wave

import pytest

from rormpc_tools import align_pilot as p, lyrics, transcribe, transcribe_align, transcribe_eval


def words():
    return [{"word": " a", "start": 1.0, "end": 1.5},
            {"word": " b", "start": 5.0, "end": 5.5},
            {"word": " c", "start": 9.0, "end": 9.5}]


def test_reference_breaks_not_candidate_segmentation():
    ref = [(0.0, "a"), (4.0, "b"), (8.0, "c")]
    raw = words()
    scores = p.score(ref, raw, copy.deepcopy(raw))
    assert scores["raw"] == scores["refined"]
    assert scores["raw"] == transcribe_eval.sync(ref, [
        {"start": w["start"], "text": w["word"]} for w in raw])
    # One candidate line would hide two reference boundaries; it is not an
    # input to the scorer. Re-segmentation cannot change the locked projection.
    regrouped = transcribe.make_lines([[0, 1, 2]], raw)
    flattened = [dict(raw[i]) for line in regrouped for i in range(line["words"][0], line["words"][1] + 1)]
    assert p.score(ref, raw, flattened) == scores
    assert p.reference_groups(ref, raw) == [[0], [1], [2]]


def test_breaks_are_lexical_not_reference_time_windows():
    raw = words()
    ref = [(100.0, "a b"), (200.0, "c")]
    groups = p.reference_groups(ref, raw)
    assert groups == [[0, 1], [2]]
    moved = [dict(w, start=w["start"] + 100) for w in raw]
    assert p.reference_groups(ref, moved) == groups
    assert p.score(ref, raw, raw)["raw"]["lines"] == 2


def test_insertion_deletion_and_unscorable_line():
    raw = [{"word": s, "start": i, "end": i + .5} for i, s in enumerate(["a", "extra", "b"])]
    assert p.reference_groups([(0, "a missing"), (4, "b")], raw) == [[0, 1], [2]]
    assert p.reference_groups([(0, "a"), (4, "absent")], raw) == [[0, 1, 2]]


@pytest.mark.parametrize("ref,scored,groups", [
    ([(1, "a"), (3, "absent"), (5, "b"), (9, "c")],
     [(1, "a"), (5, "b"), (9, "c")], [[0], [1], [2]]),
    # Both normalized tokens belong to the same Whisper word: the second
    # reference line's first match cannot advance the partition.
    ([(1, "a"), (3, "b"), (9, "c")], [(1, "a"), (9, "c")], [[0], [1]]),
])
def test_reduced_reference_is_shared(ref, scored, groups):
    raw = words() if len(ref) == 4 else [dict(words()[0], word=" a b"), words()[2]]
    assert p.reference_partition(ref, raw) == (scored, groups)
    result = p.score(ref, raw, copy.deepcopy(raw))
    assert result["raw"] == result["refined"]
    assert result["raw"] == transcribe_eval.sync(scored, [
        {"start": raw[g[0]]["start"], "text": " ".join(raw[i]["word"] for i in g)} for g in groups])
    assert result["raw"]["lines"] == len(scored)


@pytest.mark.parametrize("changed", ["replacement", "order", "whitespace", "dropped"])
def test_word_identity(changed):
    raw, refined = words(), words()
    if changed == "replacement":
        refined[1]["word"] = " wrong"
    elif changed == "order":
        refined.reverse()
    elif changed == "whitespace":
        refined[0]["word"] = "a"
    else:
        refined.pop()
    with pytest.raises(ValueError, match="word identity"):
        p.score([(0, "a"), (4, "b"), (8, "c")], raw, refined)


def song(raw_p90=2.0, p90=1.0):
    return {"status": "ok", "raw": {"p50": 1.0, "p90": raw_p90, "drift": 1.0, "hit": .5},
            "refined": {"p50": -.2, "p90": p90, "drift": -.3, "hit": .6},
            "words_aligned": 70, "words_kept": 30}


def test_gates_pass_and_abandon():
    assert p.gates([song()])["verdict"] == "PASS"
    assert p.gates([song(1.1, 1.0)])["verdict"] == "ABANDON"
    assert p.gates([song(0, 0)])["verdict"] == "ABANDON"
    assert p.gates([])["verdict"] == "INCOMPLETE"
    s = song()
    s["refined"]["hit"] = .75
    s["raw"]["hit"] = .74
    assert p.gates([s])["gates"][3]["passed"]


@pytest.mark.parametrize("number,field,value", [(1, "p50", -.36), (2, "p90", 1.21),
                                               (3, "drift", -.51), (4, "hit", .59)])
def test_numbered_timing_gates(number, field, value):
    s = song()
    s["refined"][field] = value
    result = p.gates([s])
    assert result["verdict"] == "FAIL"
    assert not result["gates"][number - 1]["passed"]


def test_no_regression_and_word_weighted_share():
    result = p.gates([song(), song(), song(.5, .76)])
    assert result["verdict"] == "FAIL"
    assert not result["gates"][4]["passed"]
    small, big = song(), song()
    small.update(words_aligned=1, words_kept=0)
    big.update(words_aligned=69, words_kept=31)
    assert not p.gates([small, big])["gates"][5]["passed"]


def test_undefined_and_errors_cannot_pass():
    bad = song()
    bad["refined"]["p90"] = None
    assert p.gates([bad])["verdict"] == "INCOMPLETE"
    assert p.gates([song(), {"status": "error"}])["verdict"] != "PASS"


def save_raw(tmp_path):
    raw = {"file": "song.flac", "identity": {"pcm_sha256": "a" * 64},
           "out": {"segments": [{"words": words()}]}}
    transcribe_eval.save_raw(tmp_path, raw)
    return raw


def test_missing_stem_skipped(tmp_path, monkeypatch):
    save_raw(tmp_path)
    monkeypatch.setattr(transcribe, "CACHE", tmp_path)
    monkeypatch.setattr(p, "run_backend", lambda *a: pytest.fail("missing stem must not run a backend"))
    rec = p.evaluate("song.flac", "lyrics", tmp_path, p.BACKEND)
    assert rec["status"] == "skipped"
    assert "missing cached vocal stem" in rec["reason"]


@pytest.mark.parametrize("ref", [
    [(1, "a"), (5, "b"), (9, "c")],
    [(1, "a"), (3, "absent"), (5, "b"), (9, "c")],
    [(0, "missing"), (1, "a"), (3, "absent"), (5, "b"), (7, "gone"), (9, "c")],
])
def test_fake_emissions_end_to_end(tmp_path, monkeypatch, ref):
    save_raw(tmp_path)
    monkeypatch.setattr(transcribe, "CACHE", tmp_path)
    (tmp_path / ("a" * 64 + ".npy")).touch()
    monkeypatch.setattr(transcribe_eval, "reference", lambda rel: ref)
    # Strong, separated a/b/c frames at exactly the raw starts.
    emissions = [[0.0, -20., -20., -20.] for _ in range(20)]
    for frame, label in [(2, 1), (10, 2), (18, 3)]:
        emissions[frame] = [-20.] * 4
        emissions[frame][label] = 0.0
    monkeypatch.setattr(p, "run_backend", lambda *a: {
        "emissions": emissions, "frame_rate": 2, "vocabulary": ["-", "a", "b", "c"]})
    rec = p.evaluate("song.flac", "held-out", tmp_path, p.BACKEND)
    assert rec["status"] == "ok"
    assert rec["raw"] == rec["refined"]
    assert rec["words_identical"]
    assert rec["ref_lines"] == len(ref)
    assert rec["ref_lines_scored"] == 3
    assert rec["words_aligned"] == 3
    assert rec["fallback_reason"] is None
    assert rec["mean_shift_s"] == 0
    # Unsupported labels must be explicitly recorded, not silently dropped.
    monkeypatch.setattr(p, "run_backend", lambda *a: {
        "emissions": [[0., -20.]], "frame_rate": 2, "vocabulary": ["-", "x"]})
    rec = p.evaluate("song.flac", "lyrics", tmp_path, p.BACKEND)
    assert rec["words_kept"] == 3
    assert rec["fallback_reason"] == ["unsupported_word"]


@pytest.mark.parametrize("ref,scored", [
    ([(1, "a"), (3, "absent"), (5, "missing")], 1),
    ([(1, "absent"), (5, "missing")], 0),
])
def test_unscorable_song_is_skipped(tmp_path, monkeypatch, capsys, ref, scored):
    save_raw(tmp_path)
    monkeypatch.setattr(transcribe, "CACHE", tmp_path)
    (tmp_path / ("a" * 64 + ".npy")).touch()
    monkeypatch.setattr(transcribe_eval, "reference", lambda rel: ref)
    monkeypatch.setattr(p, "run_backend", lambda *a: pytest.fail("unscorable song must not run backend"))
    path = tmp_path / "list.tsv"
    path.write_text("song.flac\tlyrics\n")
    p.cmd(argparse.Namespace(list=path, out=tmp_path, raw=None, backend=None, limit=None, json=False))
    report = json.loads((tmp_path / "pilot.json").read_text())
    rec = report["songs"][0]
    assert rec["status"] == "skipped"
    assert rec["reason"] == f"unscorable: {scored} of {len(ref)} reference lines"
    assert (rec["ref_lines"], rec["ref_lines_scored"]) == (len(ref), scored)
    assert report["verdict"] == "INCOMPLETE"
    assert p.gates([song(), rec])["verdict"] != "PASS"
    output = capsys.readouterr().out
    assert "ref_lines\tref_lines_scored" in output
    assert f"\t{len(ref)}\t{scored}" in output


def test_changed_word_from_alignment_is_an_error(tmp_path, monkeypatch):
    save_raw(tmp_path)
    monkeypatch.setattr(transcribe, "CACHE", tmp_path)
    (tmp_path / ("a" * 64 + ".npy")).touch()
    monkeypatch.setattr(transcribe_eval, "reference", lambda rel: [(1, "a b c")])
    monkeypatch.setattr(p, "run_backend", lambda *a: {
        "emissions": [], "frame_rate": 2, "vocabulary": ["-", "a", "b", "c"]})
    original = transcribe_align.align_words
    def changed(*args, **kwargs):
        result = original(*args, **kwargs)
        result[0]["word"] = " changed"
        return result
    monkeypatch.setattr(transcribe_align, "align_words", changed)
    with pytest.raises(ValueError, match="word identity"):
        p.evaluate("song.flac", "lyrics", tmp_path, p.BACKEND)


def test_list_and_cli_wiring(tmp_path, monkeypatch):
    path = tmp_path / "list.tsv"
    path.write_text("# comment\nlyrics\tsong.flac\nother.flac\theld-out\n")
    assert p.read_list(path) == [("song.flac", "lyrics"), ("other.flac", "held-out")]
    seen = []
    monkeypatch.setattr(p, "cmd", seen.append)
    lyrics.main(["align-pilot", str(path), "--out", str(tmp_path), "--limit", "1", "--json"])
    assert seen[0].limit == 1 and seen[0].json
    path.write_text("../escape\tlyrics\n")
    with pytest.raises(ValueError, match="relative"):
        p.read_list(path)


def test_partial_command_writes_report_and_prints_skip(tmp_path, monkeypatch, capsys):
    path = tmp_path / "list.tsv"
    path.write_text("song.flac\tlyrics\n")
    save_raw(tmp_path)
    monkeypatch.setattr(transcribe, "CACHE", tmp_path)
    p.cmd(argparse.Namespace(list=path, out=tmp_path, raw=None, backend=None, limit=None, json=True))
    captured = capsys.readouterr()
    report = json.loads(captured.out)
    assert report == json.loads((tmp_path / "pilot.json").read_text())
    assert report["verdict"] == "INCOMPLETE"
    assert report["songs"][0]["status"] == "skipped"
    assert "missing cached vocal stem" in captured.err


def test_complete_protocol_and_numbered_verdict(tmp_path, monkeypatch, capsys):
    path = tmp_path / "list.tsv"
    path.write_text("".join(f"song{i}.flac\t{'lyrics' if i < 10 else 'held-out'}\n" for i in range(20)))
    def evaluate(rel, label, *args):
        return song() | {"file": rel, "label": label, "words_identical": True,
                         "mean_shift_s": .1, "fallback_reason": None}
    monkeypatch.setattr(p, "evaluate", evaluate)
    args = argparse.Namespace(list=path, out=tmp_path, raw=None, backend=None, limit=None, json=False)
    p.cmd(args)
    report = json.loads((tmp_path / "pilot.json").read_text())
    assert report["verdict"] == "PASS" and report["protocol_complete"]
    output = capsys.readouterr().out
    for i in range(1, 7):
        assert f"gate {i}: PASS" in output
    args.limit = 10
    p.cmd(args)
    assert json.loads((tmp_path / "pilot.json").read_text())["verdict"] == "INCOMPLETE"


def test_cached_npy_conversion(tmp_path):
    # np.save-compatible float32 fixture, without importing numpy.
    header = repr({"descr": "<f4", "fortran_order": False, "shape": (3,)}).encode() + b"\n"
    source, destination = tmp_path / "stem.npy", tmp_path / "stem.wav"
    source.write_bytes(b"\x93NUMPY\x01\x00" + struct.pack("<H", len(header)) + header +
                       struct.pack("<3f", -1, 0, 1))
    p.stem_wav(source, destination)
    with wave.open(str(destination)) as wav:
        assert wav.getframerate() == 16000
        assert wav.getnchannels() == 1
        assert struct.unpack("<3h", wav.readframes(3)) == (-32767, 0, 32767)
