"""Synthetic acoustic evidence only: no model imports, downloads or real state."""
import copy
import math

import pytest

from rormpc_tools.transcribe_align import align_words, refine_lines


VOCAB = ["-", "n", "a", "b"]


def emissions(labels, confidence=0.97):
    return [[math.log(confidence if i == VOCAB.index(label) else (1 - confidence) / (len(VOCAB) - 1))
             for i in range(len(VOCAB))] for label in labels]


def words(*texts):
    return [{"word": text, "start": 10.0 + i, "end": 10.5 + i, "probability": 0.8}
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
    assert summary["mean_shift_s"] == pytest.approx((9.9 + 10 + 10.4 + 10.6) / 4)
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
    result = align_words(original, matrix, 10, VOCAB, song_threshold=0)
    assert [w["aligned"] for w in result] == [True, False]
    assert result[1]["reason"] == "weak_word"
    _, summary = refine_lines(original, matrix, 10, VOCAB, song_threshold=0)
    assert (summary["words_aligned"], summary["words_kept"]) == (1, 1)
    result = align_words(original, matrix, 10, VOCAB, song_threshold=0.9)
    assert all(w["reason"] == "weak_song" for w in result)


def test_fallback_conflict_does_not_move_weak_word():
    original = words(" na", " ba")
    original[1].update(start=0.1, end=0.9)
    matrix = emissions("-na-") + [[math.log(0.25)] * 4 for _ in range(3)]
    result = align_words(original, matrix, 10, VOCAB, song_threshold=0)
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
