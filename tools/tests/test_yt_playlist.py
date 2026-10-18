"""Unavailable videos, partial deletion and ambiguous responses, without network access."""
import argparse, io, json
from types import SimpleNamespace

import pytest
from googleapiclient.errors import HttpError

from rormpc_tools import musicdb, yt_playlist
from test_skips_delete import lib, delete, RICK, YT


def http_error(reason="videoNotFound", status=404):
    return HttpError(SimpleNamespace(status=status, reason="Not Found"),
                     json.dumps({"error": {"errors": [{"reason": reason}]}}).encode())


class Request:
    def __init__(self, fn):
        self.fn = fn

    def execute(self):
        return self.fn()


class YouTube:
    def __init__(self):
        self.items = {"a": ["a1", "a2"], "b": ["b1"], "c": ["c1"]}
        self.deletes = []
        self.lists = []
        self.fail = set()
        self.ambiguous = set()
        self.lookup_error = True

    def playlistItems(self):
        return self

    def list(self, **kw):
        self.lists.append(kw)

        def execute():
            if "videoId" in kw and self.lookup_error:
                raise http_error()
            items = self.items[kw["playlistId"]]
            offset = int(kw.get("pageToken") or 0)
            # Each item is on its own page; both API videoId fields are exercised.
            result = {"items": [{"id": item, **({"contentDetails": {"videoId": YT}} if offset else
                       {"snippet": {"resourceId": {"videoId": YT}}})}]
                      if (item := next(iter(items[offset:]), None)) else []}
            if offset + 1 < len(items):
                result["nextPageToken"] = str(offset + 1)
            return result
        return Request(execute)

    def delete(self, id):
        def execute():
            self.deletes.append(id)
            if id in self.fail:
                raise http_error("forbidden", 403)
            for items in self.items.values():
                if id in items:
                    items.remove(id)
            if id in self.ambiguous:
                raise http_error("backendError", 500)
            return {}
        return Request(execute)


@pytest.fixture
def yt(monkeypatch):
    api = YouTube()
    monkeypatch.setattr(yt_playlist, "chosen", lambda: ["a", "b", "c"])
    monkeypatch.setattr(yt_playlist, "api", lambda: api)
    return api


def test_unavailable_lookup_falls_back_to_complete_local_matching(yt):
    found = yt_playlist.find(yt, YT)
    assert [p["item"] for p in found["playlists"]] == ["a1", "a2", "b1", "c1"]
    assert all(p["reason"] == "videoNotFound" for p in found["results"])
    assert any(p.get("pageToken") == "1" and "videoId" not in p for p in yt.lists)
    out = yt_playlist.remove(yt, YT)
    assert out["removed"] == ["a", "b", "c"]
    assert yt.deletes == ["a1", "a2", "b1", "c1"]
    assert all(p["part"] == "id,snippet,contentDetails" for p in yt.lists)


def test_filtered_lookup_does_not_trust_unrelated_items(yt):
    yt.lookup_error = False
    assert yt_playlist.find(yt, "different-video")["playlists"] == []


def test_remove_cli_reports_partial_failure_and_nonzero_exit(yt, monkeypatch, capsys):
    yt.fail = {"b1"}
    monkeypatch.setattr("sys.argv", ["yt-playlist", "remove", YT])
    with pytest.raises(SystemExit) as exc:
        yt_playlist.main()
    assert exc.value.code == 1
    out = json.loads(capsys.readouterr().out)
    assert out["removed"] == ["a", "c"]
    assert [p["outcome"] for p in out["results"]] == ["removed", "failed: 403 forbidden", "removed"]


def test_partial_removal_is_durable_and_retry_only_deletes_remaining(lib, yt, monkeypatch, capsys):
    (musicdb.DATA / "youtube-playlists.json").write_text('["a", "b", "c"]')
    monkeypatch.setattr(musicdb.urllib.request, "urlopen", lambda *a, **k: io.BytesIO(b"{}"))
    monkeypatch.setattr(musicdb, "notify", lambda *a: None)
    yt.fail = {"b1"}
    original_delete = yt.delete

    def checked_delete(id):
        [rec] = musicdb.jsonl(musicdb.PENDING)
        assert {"id": id[0], "item": id, "state": "intent"} in rec["youtube_items"]
        if id == "c1":
            assert rec["youtube_removed"] == ["a"]
        return original_delete(id)
    monkeypatch.setattr(yt, "delete", checked_delete)
    delete(permanent=True, listenbrainz=True)
    output = capsys.readouterr()
    assert not output.err
    line = output.out.splitlines()[-1]
    assert line.startswith("Deleted permanently:")
    assert "YouTube removal pending (retried hourly): 403 forbidden" in line
    [rec] = musicdb.jsonl(musicdb.PENDING)
    assert rec["youtube_removed"] == ["a", "c"]
    assert rec["youtube_failures"][0]["id"] == "b"
    assert rec["ops"]["youtube"] != "done" and rec["error"]
    monkeypatch.setattr(yt, "delete", original_delete)
    yt.fail.clear()
    monkeypatch.setattr(yt_playlist, "chosen", lambda: [])  # retry keeps the journaled target scope
    musicdb.deletions(argparse.Namespace(retry=True, json=False))
    [rec] = musicdb.jsonl(musicdb.DONE)
    assert rec["youtube_removed"] == ["a", "c", "b"]
    assert rec["ops"]["youtube"] == "done" and rec["youtube_failures"] == []
    assert yt.deletes == ["a1", "a2", "b1", "c1", "b1"]
    musicdb.deletions(argparse.Namespace(retry=True, json=False))
    assert len(yt.deletes) == 5
    assert not (lib[0] / RICK).exists()


def test_crash_after_remote_delete_reconciles_durable_intent(lib, yt, monkeypatch):
    (musicdb.DATA / "youtube-playlists.json").write_text('["a", "b", "c"]')
    monkeypatch.setattr(musicdb.urllib.request, "urlopen", lambda *a, **k: io.BytesIO(b"{}"))
    original_delete = yt.delete

    def interrupted(id):
        def execute():
            original_delete(id).execute()
            raise KeyboardInterrupt("crash before acknowledgement")
        return Request(execute)
    monkeypatch.setattr(yt, "delete", interrupted)
    with pytest.raises(KeyboardInterrupt):
        delete(permanent=True, listenbrainz=True)
    [record] = musicdb.jsonl(musicdb.PENDING)
    assert record["error"] and record["youtube_items"] == [{"id": "a", "item": "a1", "state": "intent"}]
    monkeypatch.setattr(yt, "delete", original_delete)
    musicdb.deletions(argparse.Namespace(retry=True, json=False))
    [record] = musicdb.jsonl(musicdb.DONE)
    assert record["youtube_removed"] == ["a", "b", "c"]
    assert record["ops"]["youtube"] == "done"
    assert yt.deletes == ["a1", "a2", "b1", "c1"]


def test_successful_status_line_reports_remote_count(lib, yt, monkeypatch, capsys):
    (musicdb.DATA / "youtube-playlists.json").write_text('["a", "b", "c"]')
    monkeypatch.setattr(musicdb.urllib.request, "urlopen", lambda *a, **k: io.BytesIO(b"{}"))
    delete(permanent=True, listenbrainz=True)
    output = capsys.readouterr()
    assert not output.err
    assert " · YouTube: removed from 3 playlists · " in output.out.splitlines()[-1]


def test_ambiguous_delete_is_reconciled_without_repeating_it(yt):
    yt.ambiguous = {"a1"}
    intents = []
    first = yt_playlist.remove(yt, YT, intents=intents)
    assert first["results"][0]["outcome"] == "failed: 500 backendError"
    second = yt_playlist.remove(yt, YT, intents=intents)
    assert all(p["outcome"] == "removed" for p in second["results"])
    assert yt.deletes.count("a1") == 1
    assert all(i["state"] == "removed" for i in intents)
