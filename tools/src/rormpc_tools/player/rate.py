"""Watchdog for the playback speed: MPD's `elapsed` against the monotonic clock.

MPD's elapsed once ran 1.0896x the wall clock (48000/44100) on a 48 kHz mp3 through the osx output, intermittently:
the output device's rate, not the file. This module only watches and tells; it fixes nothing.

Every status sample (a player idle event or the MAX_WAIT heartbeat; no timer of its own) is paired with the
monotonic clock at the midpoint of the status call; a sample whose call took over 250 ms is dropped. The rate is
Δelapsed / Δmonotonic over a window of 25-40 s of uninterrupted playback. The baseline starts afresh on a song
change, a state other than play (the gap's paused 0:00 too), a seek (elapsed off the prediction by over 3 s), a
sample gap over 45 s (sleep), a monotonic/wall discontinuity, a window that ran over 40 s, and at start (an MPD
reconnect restarts the daemon). Two consecutive windows both above 1.03 or both below 0.97 raise the alarm: one
notification, `rate_alarm` in player.json ({ratio, since, song, audio}) and one line in rate-alarms.jsonl next to
the other state (song file, MPD's `audio`, the output device's nominal rate on macOS, the samples, the ratio). It
clears after two healthy windows or a reset; windows of the same incident do not notify again.
"""
import asyncio, json, subprocess, sys, time

from . import Module, log, state_dir, write_state

MIN_WINDOW = 25.0
MAX_WINDOW = 40.0
MAX_GAP = 45.0  # between samples: a sleep, not playback
MAX_CALL = 0.25  # a slower status call makes the midpoint unreliable
SEEK_TOLERANCE = 3.0  # elapsed off the prediction by more than this: a seek
CLOCK_TOLERANCE = 5.0  # wall and monotonic time may differ by this much between samples
FAST, SLOW = 1.03, 0.97


def device_rate():
    """The default output device's nominal rate (macOS, `system_profiler`), else None. Read once per alarm."""
    if sys.platform != "darwin":
        return None
    try:
        out = subprocess.run(["system_profiler", "SPAudioDataType", "-json"], capture_output=True, text=True,
                             timeout=20).stdout
        for group in json.loads(out).get("SPAudioDataType", []):
            for dev in group.get("_items", []):
                if dev.get("coreaudio_default_audio_output_device") == "spaudio_yes":
                    return dev.get("coreaudio_device_srate")
    except Exception:
        pass
    return None


def notify(text):
    from ..musicdb import notify as n
    n(text, title="mpd-player")


def describe(ratio):
    pct = round(abs(ratio - 1) * 100)
    return f"{pct}% {'fast' if ratio > 1 else 'slow'}"


class RateWatch(Module):
    name = "rate"

    def __init__(self, mono=time.monotonic, wall=time.time, device_rate=device_rate, notify=notify):
        self.mono, self.wall, self.device_rate, self.notify = mono, wall, device_rate, notify
        self.reset()
        self.alarm = None  # the rate_alarm dict while raised
        self.windows = []  # consecutive window ratios since the last reset
        self.published = "unset"

    def reset(self):
        self.base = None  # (mono, elapsed) at the window's start
        self.samples = []  # (mono, elapsed) since the baseline
        self.last = None  # (mono, wall, elapsed, songid)
        self.windows = []
        if getattr(self, "alarm", None):
            self.alarm = None

    async def start(self, d):
        self.publish()

    def publish(self):
        if self.published != self.alarm:
            write_state("player", {"rate_alarm": self.alarm})
            self.published = dict(self.alarm) if self.alarm else None

    def restart(self, mono, wall, elapsed, song):
        self.reset()
        self.base = (mono, elapsed)
        self.samples = [self.base]
        self.last = (mono, wall, elapsed, song)

    async def on_status(self, d, s, changed):
        try:
            await self.sample(d, s)
        finally:
            self.publish()

    async def sample(self, d, s):
        clock = getattr(d, "status_clock", None)
        if clock is None:
            mono, took = self.mono(), 0.0
        else:
            mono, took = clock
        wall = self.wall()
        song = s.get("songid")
        if s.get("state") != "play" or "elapsed" not in s:
            self.reset()
            return
        elapsed = float(s["elapsed"])
        if took > MAX_CALL:
            if self.last and self.last[3] != song:
                self.reset()
            return
        last = self.last
        if (last is None or last[3] != song or mono - last[0] > MAX_GAP or mono < last[0]
                or abs((wall - last[1]) - (mono - last[0])) > CLOCK_TOLERANCE
                or abs(elapsed - (last[2] + mono - last[0])) > SEEK_TOLERANCE):
            self.restart(mono, wall, elapsed, song)
            return
        self.last = (mono, wall, elapsed, song)
        self.samples.append((mono, elapsed))
        span = mono - self.base[0]
        if span < MIN_WINDOW:
            return
        if span > MAX_WINDOW:
            self.restart(mono, wall, elapsed, song)
            return
        ratio = (elapsed - self.base[1]) / span
        window = self.samples
        self.base, self.samples = (mono, elapsed), [(mono, elapsed)]
        await self.window(d, s, ratio, window)

    async def window(self, d, s, ratio, window):
        self.windows = (self.windows + [ratio])[-2:]
        healthy = [FAST >= r >= SLOW for r in self.windows]
        if self.alarm:
            if len(healthy) == 2 and all(healthy):
                log(f"rate: playback clock back to normal ({ratio:.3f})")
                self.alarm = None
            return
        if len(self.windows) == 2 and (all(r > FAST for r in self.windows) or all(r < SLOW for r in self.windows)):
            avg = sum(self.windows) / 2
            await self.raise_alarm(d, s, avg, window)

    async def raise_alarm(self, d, s, ratio, window):
        try:
            cur = await d.mpd.currentsong()
        except Exception:
            cur = {}
        song = cur.get("file")
        title = cur.get("title") or song or "?"
        self.alarm = {"ratio": round(ratio, 4), "since": self.wall(), "song": song, "audio": s.get("audio")}
        record = dict(self.alarm, windows=[round(r, 4) for r in self.windows],
                      device_rate=await asyncio.to_thread(self.device_rate),
                      samples=[[round(m, 3), e] for m, e in window])
        log(f"rate: playback clock runs about {describe(ratio)}: {song}")
        try:
            with open(state_dir() / "rate-alarms.jsonl", "a") as f:
                f.write(json.dumps(record) + "\n")
        except OSError as e:
            log(f"rate: log: {e}")
        await asyncio.to_thread(self.notify, f"Playback clock runs about {describe(ratio)}: {title}")
