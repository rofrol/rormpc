"""Offline CTC alignment pilot; not used by the shipped transcription path.

Emissions are *log probabilities*, frames x vocabulary (the backend's JSON list),
not logits. Frame intervals are half-open. The vocabulary contains single-character
labels and a blank; no implicit transliteration is performed. NFC/case folding and
punctuation removal are matching-only: original word strings are always retained.
Unsupported letters (including accents absent from a model) keep the whole song.

Evidence is the mean positive posterior margin over the best competing label on
nonblank path frames, averaged first by token, then by word. Uniform or blank-only
emissions score zero. The 0.2 word/song gates are conservative pilot defaults, not
calibrated singing confidence: melisma/backing vocals still need evaluation against
stable-ts. This measures acoustic support, not a calibrated probability of timing.
"""
import math
import unicodedata


def _tokens(word, vocabulary, blank_id):
    labels = {label: i for i, label in enumerate(vocabulary) if i != blank_id and len(label) == 1}
    text = unicodedata.normalize("NFC", word.strip()).casefold()
    text = "".join(c for c in text if not c.isspace() and not unicodedata.category(c).startswith("P"))
    if not text or any(c not in labels for c in text):
        return None
    return [labels[c] for c in text]


def align_words(words, emissions, frame_rate, vocabulary, *, blank_id=0,
                word_threshold=0.2, song_threshold=0.2):
    """Return copies with times and alignment metadata; never edit the input.

    Full-utterance CTC Viterbi uses stay/advance/skip transitions, with a mandatory
    blank between identical labels. All transcript tokens must be consumed. Memory
    is O(frames * tokens) bytes for traceback and O(tokens) floats for scores.
    Invalid matrices raise ValueError; absent support/impossible paths fall back.
    Conflicting mixed Whisper/aligned intervals reject the aligned candidate(s),
    rather than moving a weak word or silently returning nonmonotonic times.
    """
    if not math.isfinite(frame_rate) or frame_rate <= 0:
        raise ValueError("frame_rate must be finite and positive")
    if not vocabulary or not 0 <= blank_id < len(vocabulary) or len(set(vocabulary)) != len(vocabulary):
        raise ValueError("vocabulary must be unique and contain blank_id")
    if not all(0 <= t <= 1 for t in (word_threshold, song_threshold)):
        raise ValueError("evidence thresholds must be between zero and one")
    result = [dict(w, start_frame=None, end_frame=None, evidence=0.0,
                   aligned=False, reason="no_path") for w in words]
    if not words:
        return result
    # Normalize even rounded log probabilities so the evidence remains comparable.
    rows = []
    for row in emissions:
        if len(row) != len(vocabulary) or any(math.isnan(x) or x == math.inf for x in row):
            raise ValueError("invalid emission row")
        peak = max(row)
        if peak == -math.inf:
            raise ValueError("emission row has no finite probability")
        z = peak + math.log(sum(math.exp(x - peak) for x in row))
        rows.append([x - z for x in row])
    token_words = [_tokens(w["word"], vocabulary, blank_id) for w in words]
    if any(t is None for t in token_words):
        for w in result:
            w["reason"] = "unsupported_word"
        return result
    tokens = [t for ts in token_words for t in ts]
    states = [blank_id]
    for token in tokens:
        states.extend((token, blank_id))
    previous = [-math.inf] * len(states)
    previous[0] = 0.0
    trace = []
    for row in rows:
        current = [-math.inf] * len(states)
        steps = bytearray(len(states))
        for s, label in enumerate(states):
            candidates = [(previous[s], 0)]
            if s:
                candidates.append((previous[s - 1], 1))
            if s > 1 and label != blank_id and label != states[s - 2]:
                candidates.append((previous[s - 2], 2))
            score, step = max(candidates)
            current[s] = score + row[label]
            steps[s] = step
        previous = current
        trace.append(steps)
    state = max((len(states) - 2, len(states) - 1), key=lambda s: previous[s])
    if previous[state] == -math.inf:
        return result
    frames = [[] for _ in tokens]
    for f in range(len(rows) - 1, -1, -1):
        if state % 2:
            frames[state // 2].append(f)
        state -= trace[f][state]
    token_evidence = []
    for label, matched in zip(tokens, frames):
        margins = [max(0.0, math.exp(rows[f][label]) -
                       max(math.exp(p) for i, p in enumerate(rows[f]) if i != label)) for f in matched]
        token_evidence.append(sum(margins) / len(margins))
    offset = 0
    for w, ts in zip(result, token_words):
        count = len(ts)
        matched = [f for fs in frames[offset:offset + count] for f in fs]
        w.update(start_frame=min(matched), end_frame=max(matched) + 1,
                 evidence=sum(token_evidence[offset:offset + count]) / count)
        offset += count
    evidence = sum(w["evidence"] for w in result) / len(result)
    for w in result:
        if evidence < song_threshold:
            w["reason"] = "weak_song"
        elif w["evidence"] <= 0 or w["evidence"] < word_threshold:
            w["reason"] = "weak_word"
        else:
            w.update(start=w["start_frame"] / frame_rate, end=w["end_frame"] / frame_rate,
                     aligned=True, reason="aligned")
    # Each pass rejects at least one candidate, so this is bounded by word count.
    for _ in range(len(result)):
        reject = set()
        for i in range(1, len(result)):
            if result[i - 1]["end"] > result[i]["start"]:
                reject.update(j for j in (i - 1, i) if result[j]["aligned"])
        if not reject:
            break
        for i in reject:
            result[i].update(start=words[i]["start"], end=words[i]["end"], aligned=False,
                             reason="fallback_conflict")
    return result


def refine_lines(words, emissions, frame_rate, vocabulary, groups=None, **options):
    """Return (make_lines-shaped lines, summary); groups are original word indexes.

    With no groups, the utterance is one line. Supply the existing regroup() result
    to preserve line membership. mean_shift_s is the mean absolute endpoint shift
    across *all* words, including zero shifts for words kept at Whisper's times.
    align_words() exposes per-word frames, evidence and fallback reasons.
    """
    refined = align_words(words, emissions, frame_rate, vocabulary, **options)
    if groups is None:
        groups = [list(range(len(words)))] if words else []
    aligned = sum(w["aligned"] for w in refined)
    shift = sum(abs(w["start"] - old["start"]) + abs(w["end"] - old["end"])
                for w, old in zip(refined, words))
    # Match transcribe.make_lines without importing its settings/library dependencies.
    lines = [{"start": refined[g[0]]["start"], "end": refined[g[-1]]["end"],
              "text": "".join(refined[i]["word"] for i in g).strip(), "words": [g[0], g[-1]]}
             for g in groups]
    return lines, {
        "words_aligned": aligned, "words_kept": len(words) - aligned,
        "mean_shift_s": shift / (2 * len(words)) if words else 0.0,
        "evidence": sum(w["evidence"] for w in refined) / len(words) if words else 0.0,
    }
