"""Offline recording-only repair: service evidence, transaction boundaries and compensation."""
import json, os, shutil, subprocess, types, urllib.error

import pytest
from mutagen.id3 import APIC, ID3, TDRC, TIT2, TPE1, TXXX

from rormpc_tools import banned, identity, mbid, mbtag, musicdb, tags, versions, years

A = "00000000-0000-0000-0000-000000000001"
B = "00000000-0000-0000-0000-000000000002"
C = "00000000-0000-0000-0000-000000000003"
META = {"artist": "Example Artist", "title": "Example Song", "duration": 120}


def rec(rid=A, **kw):
    return {"id": rid, "title": META["title"], "length": 120000,
            "artist-credit": [{"name": META["artist"], "artist": {"id": "artist"}}], **kw}


def fp(rid=A, score=1.0, **kw):
    return {"mbid": rid, "score": score, "title": META["title"], "artists": [META["artist"]],
            "duration": 120, **kw}


@pytest.fixture
def service(monkeypatch):
    data = types.SimpleNamespace(fp=[fp()], search=[], recs={A: rec()}, calls=[])
    monkeypatch.setattr(mbid, "acoustid", lambda p: {"results": data.fp, "fp_sha1": "fingerprint"})
    monkeypatch.setattr(mbid, "search", lambda a, t: data.search)
    monkeypatch.setattr(mbid, "recording", lambda i: data.recs[i])
    def forbidden(*a, **kw):
        pytest.fail("Shazam/downloader resolver must never run")
    monkeypatch.setattr(mbtag, "shazam", forbidden)
    monkeypatch.setattr(mbtag, "collect", forbidden)
    monkeypatch.setattr(mbtag, "resolve", forbidden)
    return data


def test_sure_and_search_only(service):
    row = mbid.identify("unused", META)
    assert row["status"] == "SURE"
    assert row["candidates"][0]["adjusted_score"] == 1.15
    service.fp = []
    service.search = [rec()]
    assert mbid.identify("unused", META)["reason"] == "search-only evidence"
    service.search = []
    assert mbid.identify("unused", META)["reason"] == "no match"


@pytest.mark.parametrize("score,sure", [(0.5, True), (0.499, False)])
def test_auto_score_boundary(service, score, sure):
    service.fp = [fp(score=score)]
    assert (mbid.identify("unused", META)["status"] == "SURE") is sure


def test_auto_check_boundary_and_raw_unrounded_values(service, monkeypatch):
    def scores(a, t, ct, ca):
        return (1, 1) if ct == META["title"] else (0.8, 0.8)
    monkeypatch.setattr(mbid, "name_score", scores)
    service.recs[A] = rec(title="Canonical")
    assert mbid.identify("unused", META)["status"] == "SURE"
    monkeypatch.setattr(mbid, "name_score", lambda a, t, ct, ca: (1, 1) if ct == META["title"] else (0.7999, 0.7999))
    assert mbid.identify("unused", META)["reason"] == "below provisional auto thresholds"


@pytest.mark.parametrize("score", [1, 0.6])
def test_ties_and_conflicting_lower_candidates(service, score):
    service.fp.append(fp(B, score=score))
    service.recs[B] = rec(B)
    row = mbid.identify("unused", META)
    assert row["reason"] == "tie or conflicting canonical evidence"
    assert row["runner_up"] is not None


def test_redirects_merge_methods_not_repeated_queries(service):
    service.fp.append(fp(B))
    service.recs[B] = rec(A)
    service.search = [rec(B, score=100), rec(B, score=100)]
    row = mbid.identify("unused", META)
    assert row["status"] == "SURE"
    assert len(row["candidates"]) == 1
    assert row["candidates"][0]["adjusted_score"] == 1.25
    assert row["candidates"][0]["redirects"] == [{"from": B, "to": A}]


@pytest.mark.parametrize("change,reason", [
    ({"video": True}, "video/DJ recording; swaps withheld"),
    ({"disambiguation": "part of a DJ-mix"}, "video/DJ recording; swaps withheld"),
    ({"disambiguation": "live"}, "version ambiguity"),
    ({"length": None}, "missing or conflicting duration"),
    ({"length": 160000}, "missing or conflicting duration"),
])
def test_exclusions(service, change, reason):
    service.recs[A].update(change)
    row = mbid.identify("unused", META)
    assert row["reason"] == reason
    if "swaps" in reason:
        assert row["swap"]["from"] == A and row["swap"]["to"] is None


@pytest.mark.parametrize("label", ["live", "remix", "edit", "cover", "other"])
def test_versions_labels_never_promote(service, label):
    assert mbid.identify("unused", META, label)["reason"] == "version ambiguity"


def test_preserve_version_in_raw_and_cleaned_query(service):
    meta = dict(META, title="Example Song (Live) (Official Video)")
    row = mbid.identify("unused", meta)
    assert row["raw_tags"]["title"] == meta["title"]
    assert "Live" in row["query"]["title"]
    assert row["status"] != "SURE"


def test_fingerprint_version_conflict(service):
    service.fp = [fp(title="Example Song (Live)", score=1)]
    assert mbid.identify("unused", META)["reason"] == "version ambiguity"


def test_errors_are_not_empty_results_and_shazam_is_never_called(service, monkeypatch):
    def fail(*a):
        raise RuntimeError("service unavailable")
    monkeypatch.setattr(mbid, "acoustid", fail)
    assert mbid.identify("unused", META)["reason"] == "lookup error"
    service.search = [rec()]
    assert mbid.identify("unused", META)["reason"] == "lookup error"


@pytest.mark.parametrize("meta", [dict(META, artist=""), dict(META, title=""),
                                  dict(META, duration=0), dict(META, duration=900)])
def test_missing_tags_and_long_audio_skip(service, meta):
    assert mbid.identify("unused", meta)["status"] == "withheld"


def test_real_lookup_protocol_strict_errors_and_fpcalc(monkeypatch):
    calls = []
    def run(argv, **kw):
        assert argv[:4] == ["fpcalc", "-json", "-length", "120"]
        return types.SimpleNamespace(returncode=0, stdout=json.dumps({"duration": 120, "fingerprint": "abc"}))
    def http(url, **kw):
        calls.append((url, kw))
        assert kw["strict"] is True
        if "acoustid" in url:
            assert kw["host_interval"] == 0.4
            return {"status": "ok", "results": [{"score": 1, "recordings": [
                {"id": A, "title": META["title"], "artists": [{"name": META["artist"]}]}]}]}
        if "query=" in url:
            return {"recordings": [rec()]}
        return rec()
    monkeypatch.setattr(subprocess, "run", run)
    monkeypatch.setattr(mbtag, "http", http)
    assert mbid.acoustid("file")["results"][0]["mbid"] == A
    assert mbid.search(META["artist"], META["title"])[0]["id"] == A
    assert mbid.recording(A)["id"] == A
    assert len(calls) == 3
    monkeypatch.setattr(mbtag, "http", lambda *a, **kw: {"status": "error"})
    with pytest.raises(ValueError):
        mbid.acoustid("file")


def test_shared_http_external_pacing_and_backoff_with_fake_clock(monkeypatch):
    clock, sleeps, attempts = [100.0], [], [0]
    def sleep(n):
        sleeps.append(n)
        clock[0] += n
    class Response:
        def __enter__(self):
            return self
        def __exit__(self, *args):
            pass
        def read(self):
            return b'{}'
    def open_url(*a, **kw):
        attempts[0] += 1
        if attempts[0] == 1:
            raise urllib.error.HTTPError("https://musicbrainz.org/test", 503, "busy", {}, None)
        return Response()
    monkeypatch.setattr(mbtag, "_last", {"musicbrainz.org": 100.0})
    monkeypatch.setattr(mbtag.time, "time", lambda: clock[0])
    monkeypatch.setattr(mbtag.time, "sleep", sleep)
    monkeypatch.setattr(mbtag.urllib.request, "urlopen", open_url)
    assert mbtag.http("https://musicbrainz.org/test", strict=True) == {}
    assert sleeps == pytest.approx([1.1, 3])
    mbtag.http("https://musicbrainz.org/test", strict=True)
    assert sleeps[-1] == pytest.approx(1.1)


@pytest.fixture
def files(env, tmp_path, monkeypatch):
    root = tmp_path / "music"
    root.mkdir()
    monkeypatch.setattr(musicdb, "MUSIC", root)
    monkeypatch.setattr(tags, "LISTS", musicdb.DATA / "collections.jsonl")
    monkeypatch.setattr(tags, "GENRES", musicdb.DATA / "genres.jsonl")
    monkeypatch.setattr(banned, "DATA", musicdb.DATA)
    monkeypatch.setattr(mbid, "update", lambda dirs: None)
    # This is a fake decoded-audio digest, independent of unrelated tag changes.
    monkeypatch.setattr(mbid, "audio_hash", lambda rel: "audio")
    def make(ext="mp3"):
        if not shutil.which("ffmpeg"):
            pytest.skip("ffmpeg needed for temporary audio fixtures")
        p = root / ("example." + ext)
        subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "anullsrc=r=44100:cl=mono",
                        "-t", "0.2", "-y", str(p)], check=True, capture_output=True)
        if ext == "mp3":
            t = ID3(p)
            t.add(TIT2(encoding=3, text=[META["title"]]))
            t.add(TPE1(encoding=3, text=[META["artist"]]))
            t.add(TDRC(encoding=3, text=["2001"]))
            t.add(TXXX(encoding=3, desc="rormpc Song ID", text=["file-uuid"]))
            t.add(APIC(mime="image/png", type=3, desc="cover", data=b"cover-data"))
            t.save(p)
        else:
            import mutagen
            f = mutagen.File(p)
            f["artist"], f["title"], f["date"] = [META["artist"]], [META["title"]], ["2001"]
            f["rormpc_song_id"] = ["file-uuid"]
            f["custom"] = ["keep"]
            f.save()
        env([{"file": p.name}])
        musicdb.write_jsonl(identity.registry_path(), [{"id": "file-uuid", "path": p.name, "paths": [p.name],
                                                       "state": "live", "mbid": None}])
        identity._cache.clear()
        row = {"id": 1, "years_id": 1, "song_id": "file-uuid", "file": p.name, "status": "SURE", "mbid": A,
               "snapshot": mbid.snapshot(p), "audio": "audio"}
        mbid.save(mbid.report_path(), {"version": 1, "id": "report", "rows": [row]})
        return p
    return make


def other_tags(path):
    import mutagen
    t = mutagen.File(path).tags
    return {k: repr(v) for k, v in t.items() if k.lower() not in
            ("ufid:http://musicbrainz.org", "musicbrainz_trackid")}


@pytest.mark.parametrize("ext", ["mp3", "flac", "ogg", "opus"])
def test_recording_only_apply_rollback_keeps_later_unrelated_tags(files, ext):
    p = files(ext)
    from rormpc_tools import dedupe
    before = other_tags(p)
    audio = dedupe.audio_hash(p)
    assert audio
    assert mbid.apply()["skipped"] == []
    assert dedupe.audio_hash(p) == audio
    assert years.read_recording(p) == A
    assert other_tags(p) == before
    assert identity.load(fresh=True)["rows"]["file-uuid"]["mbid"] == A
    if ext == "mp3":
        t = ID3(p)
        t.add(TXXX(encoding=3, desc="later", text=["human edit"]))
        t.save(p)
    else:
        import mutagen
        f = mutagen.File(p)
        f["later"] = ["human edit"]
        f.save()
    later = other_tags(p)
    assert mbid.rollback()["skipped"] == []
    assert years.read_recording(p) is None
    assert other_tags(p) == later
    assert dedupe.audio_hash(p) == audio
    assert identity.load(fresh=True)["rows"]["file-uuid"]["mbid"] is None
    assert mbid.rollback()["rolled_back"] == []


def test_setter_can_restore_old_mbid_or_absence(files):
    p = files()
    mbid.set_recording(p, B)
    before = other_tags(p)
    mbid.set_recording(p, A)
    mbid.set_recording(p, B)
    assert years.read_recording(p) == B and other_tags(p) == before


@pytest.mark.parametrize("event", ["prepared", "after-replace", "written", "rekeyed"])
def test_interrupted_apply_resumes_without_second_write(files, monkeypatch, event):
    p = files()
    writes = []
    setter = mbid.set_recording
    monkeypatch.setattr(mbid, "set_recording", lambda *a: (writes.append(1), setter(*a))[1])
    def crash(ev, row):
        if ev == event:
            raise RuntimeError("power loss")
    monkeypatch.setattr(mbid, "hook", crash)
    with pytest.raises(RuntimeError, match="power loss"):
        mbid.apply()
    monkeypatch.setattr(mbid, "hook", lambda *a: None)
    assert mbid.apply()["skipped"] == []
    assert years.read_recording(p) == A
    assert len(writes) == 1
    assert mbid.apply()["applied"] == []


@pytest.mark.parametrize("event", ["undo-prepared", "after-undo-replace", "undo-written", "undo-rekeyed"])
def test_interrupted_rollback_resumes(files, monkeypatch, event):
    p = files()
    mbid.apply()
    def crash(ev, row):
        if ev == event:
            raise RuntimeError("power loss")
    monkeypatch.setattr(mbid, "hook", crash)
    with pytest.raises(RuntimeError):
        mbid.rollback()
    monkeypatch.setattr(mbid, "hook", lambda *a: None)
    assert mbid.rollback()["skipped"] == []
    assert years.read_recording(p) is None


@pytest.mark.parametrize("when", ["before-apply", "before-replace", "gone"])
def test_stale_file_skips_with_positive_hook(files, monkeypatch, when):
    p = files()
    def edit():
        t = ID3(p)
        t.add(TXXX(encoding=3, desc="external", text=["changed"]))
        t.save(p)
    if when == "before-apply":
        edit()
    elif when == "gone":
        p.unlink()
    else:
        monkeypatch.setattr(mbid, "hook", lambda ev, r: edit() if ev == "before-replace" else None)
    assert mbid.apply()["skipped"]
    if p.exists():
        assert years.read_recording(p) is None
        assert "TXXX:external" in ID3(p)


def test_rollback_refuses_changed_audio_or_recording(files, monkeypatch):
    p = files()
    mbid.apply()
    mbid.set_recording(p, B)
    assert mbid.rollback()["skipped"]
    mbid.set_recording(p, A)
    monkeypatch.setattr(mbid, "audio_hash", lambda rel: "different")
    assert mbid.rollback()["skipped"]
    assert years.read_recording(p) == A


def seed_associations(p):
    ref = {"key": "file:" + p.name, "file": p.name, "mbid": None, "artist": "Example", "title": "Song"}
    tags.append(tags.LISTS, [{"ts": "2000", "list": "Examples", "action": "add", "song": ref}])
    tags.append(tags.GENRES, [{"ts": "2000", "genre": "rock", "action": "add", "song": ref}])
    banned.append([{"ts": "2000", "action": "ban", "op": "original", **ref}])
    versions.log([{"ts": "2000", "action": "label", "version": "original", **ref},
                  {"ts": "2000", "action": "set", "source": "spotify", "track": "example", **ref}])


def test_associations_journalled_compensated_and_later_edits_win(files, monkeypatch):
    p = files()
    seed_associations(p)
    def crash(ev, row):
        if ev == "association-written":
            raise RuntimeError("interrupted association batch")
    monkeypatch.setattr(mbid, "hook", crash)
    with pytest.raises(RuntimeError):
        mbid.apply()
    monkeypatch.setattr(mbid, "hook", lambda *a: None)
    assert not mbid.apply()["skipped"]
    key = "mb:" + A
    assert key in tags.lists()["Examples"] and "file:" + p.name not in tags.lists()["Examples"]
    assert key in banned.fold() and key in versions.fold()["labels"]
    assert versions.fold()["tracks"][("spotify", "example")]["key"] == key
    tags.append(tags.GENRES, [{"ts": "2002", "genre": "rock", "action": "exclude", "song": {"key": key}}])
    assert not mbid.rollback()["skipped"]
    assert "file:" + p.name in tags.lists()["Examples"] and key not in tags.lists()["Examples"]
    assert tags.manual_genres()[key]["exclude"] == {"rock"}
    assert "file:" + p.name in banned.fold()
    events = musicdb.jsonl(tags.LISTS)
    markers = [x["mbid_operation"] for x in events if "mbid_operation" in x]
    assert len(markers) == len(set(markers))


def test_existing_target_associations_are_never_removed(files):
    p = files()
    seed_associations(p)
    key = "mb:" + A
    tags.append(tags.LISTS, [{"ts": "1999", "list": "Examples", "action": "add", "song": {"key": key}}])
    mbid.apply()
    mbid.rollback()
    assert key in tags.lists()["Examples"]
    assert "file:" + p.name in tags.lists()["Examples"]


def test_dry_run_filters_fresh_years_report_rechecks_file_tags(files, service, monkeypatch):
    p = files()
    calls = []
    def fresh(a):
        calls.append(a)
        return {"generated": "fresh", "rows": [
            {"id": 1, "file": p.name, "needs_mbid": True, "classes": ["no-recording"]},
            {"id": 2, "file": "ignore.mp3", "needs_mbid": True, "classes": ["no-work"]}]}
    monkeypatch.setattr(years, "dry_run", fresh)
    monkeypatch.setattr(mbid, "file_tags", lambda p: META)
    rep = mbid.dry_run()
    assert len(calls) == 1 and len(rep["rows"]) == 1
    assert rep["rows"][0]["status"] == "SURE"
    assert rep["rows"][0]["years_id"] == 1
    mbid.set_recording(p, B)
    assert mbid.dry_run()["rows"][0]["reason"] == "file already has a recording MBID"


def test_unsupported_files_are_reported_without_lookup(files, service, monkeypatch):
    p = files()
    monkeypatch.setattr(years, "dry_run", lambda a: {"generated": "fresh", "rows": [
        {"id": 1, "file": p.name, "needs_mbid": True, "classes": ["no-recording"]}]})
    monkeypatch.setattr(years, "recording_untaggable", lambda p: "unsupported format")
    assert mbid.dry_run()["rows"][0]["reason"] == "unsupported format"


def test_waited_mpd_update_failure_is_resumable(files, monkeypatch):
    p = files()
    calls = []
    def update(dirs):
        calls.append(dirs)
        if len(calls) == 1:
            raise subprocess.CalledProcessError(1, "mpc")
    monkeypatch.setattr(mbid, "update", update)
    assert mbid.apply()["skipped"]
    before = mbid.snapshot(p)
    assert not mbid.apply()["skipped"]
    assert mbid.snapshot(p) == before and len(calls) == 2


def test_shared_target_key_survives_rollback(files, env):
    p = files()
    seed_associations(p)
    mbid.apply()
    env([{"file": p.name, "musicbrainz_trackid": A}, {"file": "another.mp3", "musicbrainz_trackid": A}])
    assert not mbid.rollback()["skipped"]
    assert "mb:" + A in tags.lists()["Examples"]
    assert "file:" + p.name in tags.lists()["Examples"]
    assert "mb:" + A in banned.fold()


def test_torn_journal_checkpoint_recovers_using_staged_byte_digest(files, monkeypatch):
    p = files()
    def crash(ev, e):
        if ev == "after-replace":
            with mbid.backup_path().open("ab") as f:
                f.write(b'{"version":1,"phase":')
            raise RuntimeError("interrupted checkpoint")
    monkeypatch.setattr(mbid, "hook", crash)
    with pytest.raises(RuntimeError):
        mbid.apply()
    before = mbid.snapshot(p)
    monkeypatch.setattr(mbid, "hook", lambda *a: None)
    assert not mbid.apply()["skipped"]
    assert mbid.snapshot(p) == before
    assert list(mbid.transactions().values())[0]["phase"] == "done"


def test_lost_checkpoint_does_not_mistake_external_tags_for_own_staged_copy(files, monkeypatch):
    p = files()
    def crash(ev, e):
        if ev == "after-replace":
            raise RuntimeError("power loss")
    monkeypatch.setattr(mbid, "hook", crash)
    with pytest.raises(RuntimeError):
        mbid.apply()
    t = ID3(p)
    t.add(TXXX(encoding=3, desc="external", text=["after the crash"]))
    t.save(p)
    monkeypatch.setattr(mbid, "hook", lambda *a: None)
    assert mbid.apply()["skipped"]
    assert years.read_recording(p) == A
    assert not mbid.rollback()["skipped"]
    assert "TXXX:external" in ID3(p)


def test_cancel_stale_preparation_without_touching_file(files, monkeypatch):
    p = files()
    def stale(ev, e):
        if ev == "before-replace":
            t = ID3(p)
            t.add(TXXX(encoding=3, desc="external", text=["changed"]))
            t.save(p)
    monkeypatch.setattr(mbid, "hook", stale)
    assert mbid.apply()["skipped"]
    before = mbid.snapshot(p)
    monkeypatch.setattr(mbid, "hook", lambda *a: None)
    assert mbid.rollback()["rolled_back"][0]["cancelled"]
    assert mbid.snapshot(p) == before


def test_row_selection_and_cli_shape(files, monkeypatch, capsys):
    p = files()
    mbid.main(["--apply", "99"])
    assert json.loads(capsys.readouterr().out)["applied"] == []
    assert years.read_recording(p) is None
    mbid.main(["--apply", "1"])
    assert json.loads(capsys.readouterr().out)["applied"][0]["id"] == 1
    mbid.main(["--rollback", "99"])
    assert json.loads(capsys.readouterr().out)["rolled_back"] == []
    assert years.read_recording(p) == A
    mbid.main(["--rollback", "1"])
    assert json.loads(capsys.readouterr().out)["rolled_back"][0]["id"] == 1
    assert years.read_recording(p) is None


def test_weak_search_does_not_supply_auto_bonus(service):
    service.fp = [fp(score=0.4)]
    service.search = [rec(score=89)]
    assert mbid.identify("unused", META)["reason"] == "below provisional auto thresholds"
    service.search = [rec(score=90)]
    assert mbid.identify("unused", META)["status"] == "SURE"


def test_rollback_hash_is_uncached_even_when_audio_edit_preserves_size_and_mtime(files, monkeypatch):
    from rormpc_tools import dedupe
    p = files("flac")
    monkeypatch.setattr(mbid, "audio_hash", lambda rel: dedupe.audio_hash(musicdb.MUSIC / rel))
    rep = mbid.load()
    rep["rows"][0]["audio"] = mbid.audio_hash(p.name)
    mbid.save(mbid.report_path(), rep)
    assert not mbid.apply()["skipped"]
    before = p.stat()
    old_audio = mbid.audio_hash(p.name)
    content = p.read_bytes()
    p.write_bytes(content[:-1] + bytes([content[-1] ^ 1]))
    os.utime(p, ns=(before.st_atime_ns, before.st_mtime_ns))
    assert p.stat().st_size == before.st_size and p.stat().st_mtime_ns == before.st_mtime_ns
    assert mbid.audio_hash(p.name) != old_audio
    assert mbid.rollback()["skipped"]
    assert years.read_recording(p) == A


def test_association_edit_between_preparation_and_rekey_is_not_overwritten(files, monkeypatch):
    p = files()
    seed_associations(p)
    def edit(ev, e):
        if ev == "written":
            tags.append(tags.LISTS, [{"ts": "2002", "list": "Examples", "action": "add",
                                     "note": "later human edit", "song": {"key": "file:" + p.name}}])
    monkeypatch.setattr(mbid, "hook", edit)
    assert mbid.apply()["skipped"]
    monkeypatch.setattr(mbid, "hook", lambda *a: None)
    assert not mbid.rollback()["skipped"]
    assert tags.lists()["Examples"]["file:" + p.name]["note"] == "later human edit"
    assert "mb:" + A not in tags.lists()["Examples"]


def test_years_followup_high_selection_does_not_admit_undated_or_review():
    proposal = {"tdor": "1980", "rule": "same-length", "rg_date": "1980"}
    rows = [(1, proposal, 2000), (2, proposal, None),
            (3, dict(proposal, rule="any-clean"), 2000), (4, dict(proposal, rg_date="1979"), 2000)]
    assert [i for i, evidence, shown in rows if years.confidence(evidence, shown) == "high"] == [1]
