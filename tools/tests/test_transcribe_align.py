"""Synthetic acoustic evidence only: no real models, downloads or real state."""
import contextlib
import copy
import json
import math
from pathlib import Path
import runpy
import sys
import struct
import wave
from types import SimpleNamespace

import pytest

from rormpc_tools.transcribe_align import align_words, refine_lines


VOCAB = ["-", "n", "a", "b"]


def emissions(labels, confidence=0.97):
    return [[math.log(confidence if i == VOCAB.index(label) else (1 - confidence) / (len(VOCAB) - 1))
             for i in range(len(VOCAB))] for label in labels]


def words(*texts):
    return [{"word": text, "start": 0.3 * i, "end": 0.3 * i + 0.2, "probability": 0.8}
            for i, text in enumerate(texts)]


def test_two_words_and_summary():
    original = words(" na", " ba!")
    before = copy.deepcopy(original)
    matrix = emissions("-nnaa-bba-")
    aligned = align_words(original, matrix, 10, VOCAB)
    assert [(w["start_frame"], w["end_frame"]) for w in aligned] == [(1, 5), (6, 9)]
    assert [(w["start"], w["end"]) for w in aligned] == [(0.1, 0.5), (0.6, 0.9)]
    assert all(w["aligned"] and w["evidence"] > 0.9 for w in aligned)
    assert [w["word"] for w in aligned] == [w["word"] for w in original]
    assert [w["probability"] for w in aligned] == [0.8, 0.8]
    lines, summary = refine_lines(original, matrix, 10, VOCAB, [[0], [1]])
    assert lines == [{"start": 0.1, "end": 0.5, "text": "na", "words": [0, 0]},
                     {"start": 0.6, "end": 0.9, "text": "ba!", "words": [1, 1]}]
    assert summary["words_aligned"] == 2
    assert summary["words_kept"] == 0
    assert summary["mean_shift_s"] == pytest.approx((0.1 + 0.3 + 0.3 + 0.4) / 4)
    assert summary["evidence"] > 0.9
    assert original == before


@pytest.mark.parametrize("matrix", [
    [[math.log(0.25)] * 4 for _ in range(12)],
    emissions("------------"),
    [],
    emissions("na"),  # too few frames to consume both words
])
def test_no_usable_evidence_keeps_whisper(matrix):
    original = words(" na", " ba")
    result = align_words(original, matrix, 10, VOCAB)
    assert [(w["start"], w["end"]) for w in result] == [(w["start"], w["end"]) for w in original]
    assert all(not w["aligned"] and w["reason"] != "aligned" for w in result)
    _, summary = refine_lines(original, matrix, 10, VOCAB)
    assert summary["words_aligned"] == 0
    assert summary["words_kept"] == 2
    assert summary["mean_shift_s"] == 0
    assert summary["evidence"] == 0


def test_repeated_words_are_monotonic():
    original = words(" na", " na", " na")
    result = align_words(original, emissions("-na-na-na-"), 10, VOCAB)
    assert len(result) == 3
    assert all(w["aligned"] for w in result)
    assert [(w["start_frame"], w["end_frame"]) for w in result] == [(1, 3), (4, 6), (7, 9)]
    assert all(a["end"] <= b["start"] for a, b in zip(result, result[1:]))


def test_identical_labels_require_a_blank_even_across_words():
    result = align_words(words(" a", " a"), emissions("aa-aa"), 10, VOCAB)
    assert [(w["start_frame"], w["end_frame"]) for w in result] == [(0, 2), (3, 5)]
    assert all(w["aligned"] for w in result)
    assert not any(w["aligned"] for w in align_words(words(" a", " a"), emissions("aa"), 10, VOCAB))


def test_empty_transcript():
    assert align_words([], [], 50, VOCAB) == []
    assert refine_lines([], [], 50, VOCAB) == ([], {
        "words_aligned": 0, "words_kept": 0, "mean_shift_s": 0.0, "evidence": 0.0})


def test_unsupported_letter_keeps_all_words_without_changing_text():
    original = words(" na", " été")
    result = align_words(original, emissions("-na-ba-"), 10, VOCAB)
    assert [w["word"] for w in result] == [w["word"] for w in original]
    assert all(not w["aligned"] and w["reason"] == "unsupported_word" for w in result)


def test_word_gate_and_summary_counts():
    matrix = emissions("-na-") + [[math.log(0.25)] * 4 for _ in range(3)]
    original = words(" na", " ba")
    result = align_words(original, matrix, 10, VOCAB, song_threshold=1)
    assert [w["aligned"] for w in result] == [True, False]
    assert result[1]["reason"] == "weak_word"
    _, summary = refine_lines(original, matrix, 10, VOCAB, song_threshold=1)
    assert (summary["words_aligned"], summary["words_kept"]) == (1, 1)
    result = align_words(original, matrix, 10, VOCAB)
    assert all(w["reason"] == "weak_song" for w in result)


def test_fallback_conflict_does_not_move_weak_word():
    original = words(" na", " ba")
    original[1].update(start=0.1, end=0.9)
    matrix = emissions("-na-") + [[math.log(0.25)] * 4 for _ in range(3)]
    result = align_words(original, matrix, 10, VOCAB, song_threshold=1)
    assert not any(w["aligned"] for w in result)
    assert result[0]["reason"] == "fallback_conflict"
    assert [(w["start"], w["end"]) for w in result] == [(w["start"], w["end"]) for w in original]


@pytest.mark.parametrize("matrix", [[[0.0]], [[float("nan")] * 4], [[-math.inf] * 4]])
def test_invalid_matrix(matrix):
    with pytest.raises(ValueError, match="emission"):
        align_words(words(" na"), matrix, 10, VOCAB)


def test_invalid_frame_rate():
    with pytest.raises(ValueError, match="frame_rate"):
        align_words(words(" na"), [], 0, VOCAB)


def test_romanized_polish_word_aligns_without_changing_text():
    original = words(" nań")
    before = copy.deepcopy(original)
    matrix = emissions("-nan-")
    assert not align_words(original, matrix, 10, VOCAB)[0]["aligned"]
    result = align_words(original, matrix, 10, VOCAB, romanized=[" nan"])
    assert result[0]["aligned"]
    assert result[0]["word"] == " nań"
    assert (result[0]["start"], result[0]["end"]) == (0.1, 0.4)
    lines, summary = refine_lines(original, matrix, 10, VOCAB, romanized=[" nan"])
    assert lines[0]["text"] == "nań"
    assert summary["words_aligned"] == 1
    assert original == before


@pytest.mark.parametrize("unusable", ["", "é", " !!! "])
def test_unusable_romanization_keeps_only_its_word(unusable):
    original = words(" na", " ba", " na", " ???")
    original[3].update(start=1.2, end=1.4)
    result = align_words(original, emissions("-na-ba-na-"), 10, VOCAB,
                         romanized=["na", "ba", "na", unusable])
    assert [w["aligned"] for w in result] == [True, True, True, False]
    assert result[3]["reason"] == "unsupported_word"
    assert (result[3]["start"], result[3]["end"]) == (1.2, 1.4)
    assert [w["word"] for w in result] == [w["word"] for w in original]


def test_empty_romanization_in_middle_preserves_word_indexes():
    original = words(" na", " ???", " ba", " na")
    original[1].update(start=0.3, end=0.4)
    result = align_words(original, emissions("-na--ba-na-"), 10, VOCAB,
                         romanized=["na", "", "ba", "na"])
    assert [w["aligned"] for w in result] == [True, False, True, True]
    assert [(w["start_frame"], w["end_frame"]) for w in result] == [
        (1, 3), (None, None), (5, 7), (8, 10)]
    assert (result[1]["start"], result[1]["end"]) == (0.3, 0.4)


@pytest.mark.parametrize("weak_count", [3, 4])
def test_song_gate_is_share_of_weak_words_and_strictly_over_30_percent(weak_count):
    original = words(*([" a"] * 10))
    # All words have valid paths; only the final words have weak evidence.
    matrix = []
    for i, w in enumerate(original):
        w.update(start=0.2 * i, end=0.2 * i + 0.1)
        matrix += emissions("a-", confidence=0.5 if i >= 10 - weak_count else 0.97)
    result = align_words(original, matrix, 10, VOCAB)
    if weak_count == 3:
        assert sum(w["aligned"] for w in result) == 7
        assert all(w["reason"] == "weak_word" for w in result[-3:])
    else:
        assert all(not w["aligned"] and w["reason"] == "weak_song" for w in result)
        assert [(w["start"], w["end"]) for w in result] == [
            (w["start"], w["end"]) for w in original]


@pytest.mark.parametrize("shift, kept", [(2.5, False), (2.51, True)])
def test_displacement_guard_keeps_whisper_and_counts_as_weak(shift, kept):
    original = words(" na", " ba", " na", " ba")
    original[3].update(start=1.0 + shift, end=1.3 + shift)
    matrix = emissions("-na-ba-na-ba-")
    result = align_words(original, matrix, 10, VOCAB)
    assert [w["aligned"] for w in result] == [True, True, True, not kept]
    if kept:
        assert result[3]["reason"] == "max_shift"
        assert (result[3]["start"], result[3]["end"]) == (original[3]["start"], original[3]["end"])
        result = align_words(original, matrix, 10, VOCAB, song_threshold=0.2)
        assert all(w["reason"] == "weak_song" for w in result)


@pytest.mark.parametrize("failure", [None, "load", "labels", "inference", "tolist"])
@pytest.mark.parametrize("with_words", [False, True])
def test_backend_words_are_optional_parallel_and_failed_words_are_empty(
        monkeypatch, capsys, tmp_path, with_words, failure):
    # Fake every model/tensor API used by main: no torch or model download.
    class Tensor:
        shape = (1, 2)

        def __getitem__(self, index):
            return self

        def log_softmax(self, axis):
            return self

        def to(self, dtype):
            return self

        def div_(self, value):
            assert value == 32768
            return self

        def unsqueeze(self, axis):
            assert axis == 0
            return self

        def tolist(self):
            if failure == "tolist":
                raise AttributeError("conversion\nfailed")
            return [[-0.1, -2.352168]]

    class Model:
        # Like MMS_FA's wrapper, this exposes no feature_extractor or numpy API.

        def cpu(self):
            return self

        def eval(self):
            return self

        def __call__(self, waveform):
            if failure == "inference":
                raise AttributeError("inference\nfailed")
            return Tensor(), None

    def get_model(*, with_star):
        assert with_star is False
        if failure == "load":
            raise AttributeError("load\nfailed")
        return Model()

    def get_labels(*, star):
        assert star is None
        if failure == "labels":
            raise AttributeError("labels\nfailed")
        return ("-", "a")

    class Romanizer:
        def romanize_string(self, word):
            print("romanizer progress")
            if word == "fail":
                raise ValueError("cannot romanize")
            return {" nań": " nan", "été": "ete"}[word]

    monkeypatch.setitem(sys.modules, "torch", SimpleNamespace(
        inference_mode=contextlib.nullcontext, float32="float32", int16="int16",
        frombuffer=lambda samples, dtype: Tensor()))
    monkeypatch.setitem(sys.modules, "torchaudio", SimpleNamespace(
        pipelines=SimpleNamespace(
            MMS_FA=SimpleNamespace(get_model=get_model, get_labels=get_labels))))
    # The no-words run must not even construct a romanizer.
    monkeypatch.setitem(sys.modules, "uroman", SimpleNamespace(Uroman=Romanizer if with_words else None))
    audio = tmp_path / "vocals.wav"
    with wave.open(str(audio), "wb") as out:
        out.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
        out.writeframes(struct.pack("<h", 0) * 320)
    argv = ["align_backend.py", "--audio", str(audio)]
    if with_words:
        source = tmp_path / "words.json"
        source.write_text(json.dumps([" nań", "fail", "été"]))
        argv.extend(["--words", str(source)])
    monkeypatch.setattr(sys, "argv", argv)
    backend = Path(__file__).parents[1] / "src/rormpc_tools/data/align_backend.py"
    if failure:
        with pytest.raises(SystemExit) as exc:
            runpy.run_path(str(backend), run_name="__main__")
        assert exc.value.code == 1
        captured = capsys.readouterr()
        assert captured.out == ""
        errors = [line for line in captured.err.splitlines() if line.startswith("align backend:")]
        assert len(errors) == 1
        assert errors[0].endswith("failed")
        assert "\nfailed" not in captured.err
        assert "Traceback" not in captured.err
        return
    runpy.run_path(str(backend), run_name="__main__")
    output = json.loads(capsys.readouterr().out)
    expected = {"vocabulary": ["-", "a"], "blank_id": 0, "emissions": [[-0.1, -2.352168]],
                "frame_rate": 50.0, "model_id": "torchaudio/2.5.1/MMS_FA",
                "emission_format": "log_probabilities_float32_rounded_6"}
    if with_words:
        expected["words_uroman"] = [" nan", "", "ete"]
    assert output == expected


def test_backend_with_torch_and_wrapped_model(monkeypatch, capsys, tmp_path):
    torch = pytest.importorskip("torch")

    class Model:
        def cpu(self):
            return self

        def eval(self):
            return self

        def __call__(self, waveform):
            assert waveform.shape == (1, 16000)
            return torch.zeros((1, 50, 2)), None

    bundle = SimpleNamespace(get_model=lambda **kwargs: Model(), get_labels=lambda **kwargs: ("-", "a"))
    monkeypatch.setitem(sys.modules, "torchaudio", SimpleNamespace(pipelines=SimpleNamespace(MMS_FA=bundle)))
    audio = tmp_path / "vocals.wav"
    with wave.open(str(audio), "wb") as out:
        out.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
        out.writeframes(struct.pack("<h", 0) * 16000)
    monkeypatch.setattr(sys, "argv", ["align_backend.py", "--audio", str(audio)])
    backend = Path(__file__).parents[1] / "src/rormpc_tools/data/align_backend.py"
    runpy.run_path(str(backend), run_name="__main__")
    output = json.loads(capsys.readouterr().out)
    assert output["frame_rate"] == 50.0
    assert output["emissions"] == [[-0.693147, -0.693147]] * 50


@pytest.mark.parametrize("frames,samples,rate,expected", [
    (50, 16000, 16000, 50.0),
    (75, 32000, 16000, 37.5),
    (50, 8000, 8000, 50.0),
    (0, 16000, 16000, None),
    (50, 0, 16000, None),
    (50, 16000, 0, None),
])
def test_backend_frame_rate(frames, samples, rate, expected):
    backend = Path(__file__).parents[1] / "src/rormpc_tools/data/align_backend.py"
    compute = runpy.run_path(str(backend))["frame_rate"]
    if expected is None:
        with pytest.raises(ValueError, match="positive"):
            compute(frames, samples, rate)
    else:
        assert compute(frames, samples, rate) == expected


@pytest.mark.parametrize("channels,rate,width,frames,error", [
    (1, 16000, 2, 3, None),
    (2, 16000, 2, 3, "16 kHz mono PCM16"),
    (1, 44100, 2, 3, "16 kHz mono PCM16"),
    (1, 16000, 1, 3, "16 kHz mono PCM16"),
    (1, 16000, 2, 0, "empty"),
])
def test_backend_pcm16_reader(tmp_path, channels, rate, width, frames, error):
    backend = Path(__file__).parents[1] / "src/rormpc_tools/data/align_backend.py"
    reader = runpy.run_path(str(backend))["read_pcm16"]  # No torch import needed.
    audio = tmp_path / "vocals.wav"
    with wave.open(str(audio), "wb") as out:
        out.setparams((channels, width, rate, 0, "NONE", "not compressed"))
        out.writeframes(struct.pack("<3h", -32768, 0, 32767)[:frames * width] * channels)
    if error:
        with pytest.raises(ValueError, match=error):
            reader(audio)
    else:
        samples, sample_rate = reader(audio)
        assert list(samples) == [-32768, 0, 32767]
        assert sample_rate == 16000


def test_invalid_romanized_length():
    with pytest.raises(ValueError, match="parallel"):
        align_words(words(" na"), [], 10, VOCAB, romanized=[])


@pytest.mark.parametrize("shift", [-1, math.inf, math.nan])
def test_invalid_max_shift(shift):
    with pytest.raises(ValueError, match="max_shift_s"):
        align_words(words(" na"), [], 10, VOCAB, max_shift_s=shift)
