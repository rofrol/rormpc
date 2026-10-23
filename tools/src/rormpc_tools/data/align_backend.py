# /// script
# requires-python = ">=3.11,<3.13"
# dependencies = ["torch==2.5.1", "torchaudio==2.5.1", "uroman==1.3.1.1"]
# ///
"""CTC emissions only; run with uv run --script, never import in the package.

Usage: uv run --script align_backend.py --audio vocals.wav [--model MMS_FA] [--words words.json]
Default model: torchaudio.pipelines.MMS_FA (multilingual, romanized labels).
Input must already be 16 kHz mono WAV; this script does not separate/resample.
Output is one JSON document: vocabulary (index order), blank_id, log-probability
emissions (frames x labels, float32 rounded to 6 decimals), frame_rate (Hz),
and model_id. No wildcard/star label: an unsupported word must fall back, not
silently be absorbed by a wildcard. With --words (a JSON list of word strings),
uroman provides a parallel words_uroman list for matching only; a failed word is
an empty string. Singing quality must be evaluated before this is connected to
the transcript path.
"""
import argparse
import array
import contextlib
import json
import math
import sys
import wave


def read_pcm16(path):
    """Read the pilot's mono 16 kHz PCM16 WAV without audio/model dependencies."""
    with wave.open(str(path), "rb") as source:
        if (source.getnchannels(), source.getframerate(), source.getsampwidth()) != (1, 16000, 2):
            raise ValueError("expected a 16 kHz mono PCM16 WAV vocal stem")
        frames = source.getnframes()
        data = source.readframes(frames)
    if not data:
        raise ValueError("audio is empty")
    if len(data) != frames * 2:
        raise ValueError("truncated PCM16 audio")
    samples = array.array("h")
    samples.frombytes(data)
    if sys.byteorder != "little":
        samples.byteswap()
    return samples, 16000


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--audio", required=True, help="16 kHz mono vocal-stem WAV")
    parser.add_argument("--model", choices=["MMS_FA"], default="MMS_FA",
                        help="torchaudio pipeline (default: MMS_FA)")
    parser.add_argument("--words", help="JSON list of transcript word strings to romanize")
    args = parser.parse_args()
    words_uroman = None
    if args.words:
        try:
            with open(args.words, encoding="utf-8") as source:
                words = json.load(source)
            if not isinstance(words, list) or not all(isinstance(word, str) for word in words):
                raise ValueError("expected a JSON list of word strings")
        except (OSError, ValueError) as exc:
            parser.exit(1, f"align backend: cannot read words: {exc}\n")
        with contextlib.redirect_stdout(sys.stderr):
            import uroman

            romanizer = uroman.Uroman()
            words_uroman = []
            for word in words:
                try:
                    text = romanizer.romanize_string(word)
                    words_uroman.append(text if isinstance(text, str) else "")
                except Exception:
                    words_uroman.append("")
    # Keep dependency/model progress away from the JSON stream.
    with contextlib.redirect_stdout(sys.stderr):
        import torch
        import torchaudio

        try:
            samples, rate = read_pcm16(args.audio)
            waveform = torch.frombuffer(samples, dtype=torch.int16).to(torch.float32).div_(32768).unsqueeze(0)
        except (OSError, RuntimeError, ValueError, wave.Error, EOFError) as exc:
            parser.exit(1, f"align backend: cannot read vocal stem: {exc}\n")
        bundle = torchaudio.pipelines.MMS_FA
        try:
            model = bundle.get_model(with_star=False).cpu().eval()
        except Exception as exc:
            parser.exit(1, f"align backend: cannot load/download {args.model}; "
                        f"check network access and the torch model cache: {exc}\n")
        vocabulary = list(bundle.get_labels(star=None))
        try:
            with torch.inference_mode():
                logits, _ = model(waveform)
                emissions = logits[0].log_softmax(-1).to(torch.float32)
            stride = math.prod(layer.conv.stride[0] for layer in model.feature_extractor.conv_layers)
        except (RuntimeError, ValueError) as exc:
            parser.exit(1, f"align backend: CTC inference failed: {exc}\n")
    extra = {"words_uroman": words_uroman} if words_uroman is not None else {}
    json.dump({"vocabulary": vocabulary, "blank_id": 0,
               "emissions": [[round(p, 6) for p in row] for row in emissions.tolist()],
               "frame_rate": rate / stride, "model_id": "torchaudio/2.5.1/MMS_FA",
               "emission_format": "log_probabilities_float32_rounded_6", **extra}, sys.stdout, allow_nan=False)
    print()


if __name__ == "__main__":
    main()
