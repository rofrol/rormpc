"""mpd-player's rate watchdog: fake clock, fake status sequences."""
import asyncio, json

import pytest

from rormpc_tools.player import rate


@pytest.fixture(autouse=True)
def state(tmp_path, monkeypatch):
    monkeypatch.setenv("XDG_STATE_HOME", str(tmp_path))
    return tmp_path


class MPD:
    async def currentsong(self):
        return {"file": "a/b.mp3", "title": "Title"}


class D:
    mpd = MPD()
    status_clock = None


class Rig:
    def __init__(self):
        self.t = 1000.0
        self.wall = 5000.0
        self.notes = []
        self.w = rate.RateWatch(wall=lambda: self.wall, device_rate=lambda: 48000, notify=self.notes.append)
        self.d = D()
        self.e = 0.0
        self.song = "1"
        self.state = "play"

    def tick(self, dt, speed=1.0, took=0.0, wall_dt=None, **over):
        """Advance dt monotonic seconds; elapsed advances dt*speed while playing."""
        self.t += dt
        self.wall += dt if wall_dt is None else wall_dt
        if self.state == "play":
            self.e += dt * speed
        self.d.status_clock = (self.t, took)
        s = {"state": self.state, "songid": self.song, "elapsed": str(self.e), "audio": "48000:24:2"}
        s.update(over)
        asyncio.run(self.w.on_status(self.d, s, set()))

    def play(self, seconds, speed=1.0, step=10):
        for _ in range(int(seconds // step)):
            self.tick(step, speed)

    def alarm_lines(self):
        p = rate.state_dir() / "rate-alarms.jsonl"
        return [json.loads(l) for l in p.read_text().splitlines()] if p.exists() else []


def shown():
    return json.loads((rate.state_dir() / "player.json").read_text())["rate_alarm"]


def test_normal_play_never_alarms():
    r = Rig()
    r.tick(0)
    r.play(300)
    assert r.w.alarm is None and not r.notes and not r.alarm_lines()


def test_fast_clock_alarms_once_and_logs():
    r = Rig()
    r.tick(0)
    r.play(30, 1.09)
    assert r.w.alarm is None  # one window is not enough
    r.play(30, 1.09)
    assert r.w.alarm
    r.play(200, 1.09)
    assert r.notes == ["Playback clock runs about 9% fast: Title"]
    a = shown()
    assert a["song"] == "a/b.mp3" and a["audio"] == "48000:24:2" and 1.08 < a["ratio"] < 1.10
    lines = r.alarm_lines()
    assert len(lines) == 1 and lines[0]["device_rate"] == 48000 and lines[0]["samples"]


def test_slow_clock_alarms():
    r = Rig()
    r.tick(0)
    r.play(100, 0.92)
    assert r.notes == ["Playback clock runs about 8% slow: Title"]


def test_one_odd_window_is_not_enough():
    r = Rig()
    r.tick(0)
    r.play(30, 1.09)
    r.play(120, 1.0)
    assert r.w.alarm is None and not r.notes


def test_pause_resume_resets():
    r = Rig()
    r.tick(0)
    r.play(30, 1.09)
    r.state = "pause"
    r.tick(10)
    r.state = "play"
    r.play(40, 1.09)  # the first tick is the fresh baseline: only one window since the pause
    assert r.w.alarm is None
    r.play(30, 1.09)
    assert r.w.alarm


def test_seek_resets_instead_of_counting():
    r = Rig()
    r.tick(0)
    r.play(30, 1.09)
    r.e += 100  # seek
    r.play(30, 1.09)
    assert r.w.alarm is None


def test_song_change_resets():
    r = Rig()
    r.tick(0)
    r.play(30, 1.09)
    r.song, r.e = "2", 0.0
    r.play(30, 1.09)
    assert r.w.alarm is None


def test_gap_paused_zero_resets():
    r = Rig()
    r.tick(0)
    r.play(30, 1.09)
    r.state, r.e = "pause", 0.0
    r.tick(3)
    r.state = "play"
    r.song = "2"
    r.tick(0)
    r.play(30, 1.09)
    assert r.w.alarm is None


def test_sleep_gap_resets():
    r = Rig()
    r.tick(0)
    r.play(30, 1.09)
    r.tick(600, wall_dt=600)  # a gap over 45 s
    r.play(30, 1.09)
    assert r.w.alarm is None


def test_wall_monotonic_discontinuity_resets():
    r = Rig()
    r.tick(0)
    r.play(30, 1.09)
    r.tick(10, 1.09, wall_dt=3600)
    r.play(30, 1.09)
    assert r.w.alarm is None


def test_slow_status_call_is_dropped():
    r = Rig()
    r.tick(0)
    r.play(30, 1.09)
    r.play(20, 1.09)
    r.tick(10, 1.09, took=0.6)  # would close the window: dropped
    assert r.w.windows == [r.w.windows[0]]
    r.tick(10, 1.09)  # the next good sample closes it (span 40)
    assert len(r.w.windows) == 2


def test_window_over_40s_is_discarded():
    r = Rig()
    r.tick(0)
    r.tick(44, 1.09)
    r.tick(44, 1.09)
    r.tick(44, 1.09)
    assert r.w.alarm is None and not r.w.windows


def test_clears_after_two_healthy_windows():
    r = Rig()
    r.tick(0)
    r.play(60, 1.09)
    assert r.w.alarm
    r.play(30, 1.0)
    assert r.w.alarm  # one healthy window
    r.play(30, 1.0)
    assert r.w.alarm is None and shown() is None
    r.play(60, 1.09)
    assert len(r.notes) == 2


def test_a_reset_clears_the_alarm():
    r = Rig()
    r.tick(0)
    r.play(60, 1.09)
    assert shown()
    r.state = "pause"
    r.tick(1)
    assert shown() is None
