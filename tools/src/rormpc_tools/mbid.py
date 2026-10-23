"""Missing recording identification, independent of release-year acceptance.

Only AcoustID-supported, canonical MusicBrainz matches pass the provisional SURE
policy. Shazam, downloader resolution and video/audio swaps are never invoked.
"""
import argparse, contextlib, fcntl, hashlib, json, os, pathlib, re, shutil, subprocess, urllib.parse, uuid

from . import identity, mbtag, musicdb, years

REPORT_VERSION = 1
NAME_TITLE_WEIGHT, NAME_ARTIST_WEIGHT, VERSION_PENALTY = 0.6, 0.4, 0.15
ACOUSTID_WEIGHT, ACOUSTID_BASE = 0.3, 0.5
DURATION_PENALTY_AFTER_S, DURATION_PENALTY, MULTISOURCE_BONUS = 40, 0.1, 0.1
AUTO_SCORE, AUTO_CHECK = 1.0, 0.8  # provisional downloader thresholds, not calibrated precision
MB = "https://musicbrainz.org/ws/2/recording"
MBID = re.compile(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\Z")


def report_path():
    return musicdb.DATA / "mbid-report.json"


def backup_path():
    return musicdb.DATA / "mbid-backup.jsonl"


def fsync_parent(path):
    fd = os.open(pathlib.Path(path).parent, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def save(path, value):
    years.write_atomic(path, json.dumps(value, ensure_ascii=False, indent=1) + "\n")
    fsync_parent(path)


def load():
    if not report_path().exists():
        raise ValueError("run musicdb mbid --dry-run first")
    rep = json.loads(report_path().read_text())
    if rep.get("version") != REPORT_VERSION:
        raise ValueError("unsupported MBID report version")
    return rep


@contextlib.contextmanager
def locked():
    path = musicdb.DB.parent / "mbid.lock"
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a") as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        yield


def hook(event, row):
    """Deterministic test seam at transaction boundaries; production is a no-op."""


def snapshot(path):
    before = path.stat()
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    after = path.stat()
    stat = lambda s: [s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns, s.st_mode]
    if stat(before) != stat(after):
        raise ValueError("file changed while reading")
    return {"stat": stat(after), "sha256": h.hexdigest()}


def file_path(rel):
    p = musicdb.MUSIC / rel
    if pathlib.Path(rel).is_absolute() or p.is_symlink() or not p.resolve().is_relative_to(musicdb.MUSIC.resolve()):
        raise ValueError("not a regular library path")
    return p


def file_tags(path):
    import mutagen
    f = mutagen.File(path)
    if f is None:
        raise ValueError("unsupported audio")
    t = f.tags or {}
    def get(id3, vorbis):
        v = t.get(id3 if hasattr(t, "getall") else vorbis)
        return str(musicdb.one(getattr(v, "text", v))) if v else ""
    return {"artist": get("TPE1", "artist"), "title": get("TIT2", "title"),
            "duration": float(getattr(f.info, "length", 0) or 0)}


def audio_hash(rel):
    # Rollback must not trust the size/mtime cache: unrelated later tags are allowed,
    # and an external editor can preserve both metadata values while changing audio.
    from . import dedupe
    return dedupe.audio_hash(file_path(rel))


def set_recording(path, recording):
    """Change exactly one logical tag, leaving every unrelated frame/comment alone."""
    import mutagen
    from mutagen.id3 import ID3, UFID
    from mutagen._vorbis import VComment
    f = mutagen.File(path)
    if f is None:
        raise ValueError("unsupported audio")
    if f.tags is None:
        f.add_tags()
    t = f.tags
    if isinstance(t, ID3):
        t.delall(f"UFID:{years.UFID_OWNER}")
        if recording:
            t.add(UFID(owner=years.UFID_OWNER, data=recording.encode("ascii")))
    elif isinstance(t, VComment):
        if recording:
            t["musicbrainz_trackid"] = [recording]
        elif "musicbrainz_trackid" in t:
            del t["musicbrainz_trackid"]
    else:
        raise ValueError("unsupported recording tag format")
    f.save()


def name_score(artist, title, candidate_title, candidate_artists):
    artists = candidate_artists
    a = max([mbtag.sim(artist, " ".join(artists))] + [mbtag.sim(x, artist) for x in artists] +
            [1.0 if mbtag.norm(x) and mbtag.norm(x) in mbtag.norm(artist) else 0 for x in artists])
    raw = NAME_TITLE_WEIGHT * mbtag.sim(title, candidate_title) + NAME_ARTIST_WEIGHT * a
    return raw, raw - VERSION_PENALTY * len(mbtag.versions(title) ^ mbtag.versions(candidate_title))


def acoustid(path):
    """Same fpcalc/API protocol as mbtag, but strict failures cannot become no-match."""
    p = subprocess.run(["fpcalc", "-json", "-length", "120", str(path)], capture_output=True, text=True, timeout=120)
    if p.returncode:
        raise ValueError("fpcalc failed: " + p.stderr.strip())
    fp = json.loads(p.stdout)
    data = urllib.parse.urlencode({"client": mbtag.ACOUSTID_KEY, "meta": "recordings",
                                  "duration": int(fp["duration"]), "fingerprint": fp["fingerprint"]}).encode()
    r = mbtag.http("https://api.acoustid.org/v2/lookup", data=data, host_interval=0.4, strict=True,
                   headers={"Content-Type": "application/x-www-form-urlencoded"})
    if not r or r.get("status") != "ok" or not isinstance(r.get("results"), list):
        raise ValueError("invalid AcoustID response")
    results = []
    for x in r["results"]:
        for rec in x.get("recordings") or []:
            score = float(x["score"])
            if not 0 <= score <= 1:
                raise ValueError("invalid AcoustID score")
            results.append({"mbid": rec["id"], "title": rec.get("title", ""),
                            "artists": [a["name"] for a in rec.get("artists", [])],
                            "duration": rec.get("duration"), "score": score})
    return {"results": results, "fp_sha1": hashlib.sha1(fp["fingerprint"].encode()).hexdigest()}


def search(artist, title):
    q = f'artist:"{artist}" AND recording:"{title}"'
    r = mbtag.http(MB + "?" + urllib.parse.urlencode({"query": q, "limit": 10, "fmt": "json"}), strict=True)
    if not r or not isinstance(r.get("recordings"), list):
        raise ValueError("invalid MusicBrainz search response")
    return r["recordings"]


def recording(mbid):
    if not MBID.fullmatch(mbid):
        raise ValueError("invalid candidate MBID")
    r = mbtag.http(f"{MB}/{mbid}?inc=artist-credits&fmt=json", strict=True)
    if not r or not MBID.fullmatch(r.get("id", "")) or not r.get("title") or not r.get("artist-credit"):
        raise ValueError("MusicBrainz recording missing or malformed")
    return r


def identify(path, tags, label=None):
    artist = mbtag.fix(tags["artist"]).strip()
    title = mbtag.NOISE.sub("", mbtag.fix(tags["title"])).strip()
    out = {"raw_tags": tags, "query": {"artist": artist, "title": title}, "label": label,
           "markers": sorted(mbtag.versions(tags["title"])), "candidates": [], "errors": [],
           "status": "withheld", "reason": "no match", "mbid": None}
    if not artist or not title or not 0 < tags["duration"] < 900:
        out["reason"] = "missing names/duration or fingerprint duration restriction"
        return out
    try:
        fp = acoustid(path)
        out["fingerprint"] = fp.get("fp_sha1")
    except Exception as e:
        fp = {"results": []}
        out["errors"].append(f"AcoustID: {type(e).__name__}: {e}")
    try:
        found = search(artist, title)
    except Exception as e:
        found = []
        out["errors"].append(f"MusicBrainz search: {type(e).__name__}: {e}")
    by = {}
    for x in fp["results"]:
        raw, score = name_score(artist, title, x["title"], x["artists"])
        if x.get("duration") and abs(x["duration"] - tags["duration"]) > DURATION_PENALTY_AFTER_S:
            score -= DURATION_PENALTY
        score += ACOUSTID_WEIGHT * (x["score"] - ACOUSTID_BASE)
        c = by.setdefault(x["mbid"], {"requested": x["mbid"], "methods": [], "score": -1, "evidence": []})
        c["score"] = max(c["score"], score)
        c["methods"] = ["acoustid"]
        c["evidence"].append(dict(x, raw_score=raw, adjusted_score=score))
    for rec in found:
        c = by.setdefault(rec["id"], {"requested": rec["id"], "methods": [], "score": 0, "evidence": []})
        c["search_score"] = rec.get("score")
        c.setdefault("search_evidence", []).append(rec)
        _, check = name_score(artist, title, rec.get("title", ""), [a["name"] for a in rec.get("artist-credit", [])])
        credible = (int(rec.get("score") or 0) >= 90 and rec.get("length") and
                    abs(rec["length"] / 1000 - tags["duration"]) <= 30 and check >= 0.85)
        if credible and "mb-search" not in c["methods"]:
            c["methods"].append("mb-search")
    canonical = {}
    for c in by.values():
        try:
            rec = recording(c["requested"])
            names = [a["name"] for a in rec["artist-credit"]]
            raw, check = name_score(artist, title, rec["title"], names)
            c.update(mbid=rec["id"], title=rec["title"], artists=names, video=bool(rec.get("video")),
                     disambiguation=rec.get("disambiguation", ""), length=rec.get("length"), check=check,
                     raw_check=raw, version=years.version_word(rec), dj_mix=years.dj_mix(rec))
            c["redirect"] = rec["id"] if rec["id"] != c["requested"] else None
            c["redirects"] = [{"from": c["requested"], "to": rec["id"]}] if c["redirect"] else []
            prev = canonical.get(c["mbid"])
            if prev:
                prev["methods"] = sorted(set(prev["methods"] + c["methods"]))
                prev["score"] = max(prev["score"], c["score"])
                prev["evidence"] += c["evidence"]
                prev["redirects"] += c["redirects"]
                prev.setdefault("search_evidence", []).extend(c.get("search_evidence", []))
            else:
                canonical[c["mbid"]] = c
        except Exception as e:
            out["errors"].append(f"recording {c['requested']}: {type(e).__name__}: {e}")
    candidates = list(canonical.values())
    for c in candidates:
        c["adjusted_score"] = c["score"] + MULTISOURCE_BONUS * max(0, len(c["methods"]) - 1)
    candidates.sort(key=lambda c: (-c["adjusted_score"], c["mbid"]))
    out["candidates"] = candidates
    if not candidates:
        if out["errors"]:
            out["reason"] = "lookup error"
        return out
    best = candidates[0]
    out["runner_up"] = candidates[1]["adjusted_score"] if len(candidates) > 1 else None
    out["mbid"] = best["mbid"]
    # All credible, different canonical identities conflict, even below the automatic threshold.
    conflict = any("acoustid" in c["methods"] or (c["check"] >= AUTO_CHECK and c["length"] and
                   abs(c["length"] / 1000 - tags["duration"]) <= 30) for c in candidates[1:])
    ambiguous = (out["markers"] != sorted(mbtag.versions(best["title"] + " " + best["disambiguation"])) or
                 bool(best["version"]) or bool(out["markers"]) or label not in (None, "original") or
                 any(mbtag.versions(x["title"]) != mbtag.versions(tags["title"]) for x in best["evidence"]))
    if out["errors"]:
        reason = "lookup error"
    elif "acoustid" not in best["methods"]:
        reason = "search-only evidence"
    elif best["video"] or best["dj_mix"] or re.search(r"\b(video|videoclip|clip|MV)\b",
                                                                   best["title"] + " " + best["disambiguation"], re.I):
        reason = "video/DJ recording; swaps withheld"
        out["swap"] = {"from": best["mbid"], "to": None, "why": reason,
                       "alternatives": [c["mbid"] for c in candidates[1:] if not c["video"] and not c["dj_mix"]],
                       "work_lookup": "not attempted: no swap can authorize this repair"}
    elif ambiguous:
        reason = "version ambiguity"
    elif not best["length"] or abs(best["length"] / 1000 - tags["duration"]) > 30:
        reason = "missing or conflicting duration"
    elif conflict or (out["runner_up"] is not None and out["runner_up"] >= best["adjusted_score"]):
        reason = "tie or conflicting canonical evidence"
    elif best["adjusted_score"] < AUTO_SCORE or best["check"] < AUTO_CHECK:
        reason = "below provisional auto thresholds"
    else:
        out["status"], reason = "SURE", "AcoustID-supported canonical recording"
    out["reason"] = reason
    return out


def dry_run():
    from . import versions
    with locked():
        if backup_path().exists():
            pending = [e for e in transactions().values() if e["phase"] not in ("done", "rolled-back")]
            if pending:
                raise ValueError("resume apply or use rollback to close interrupted work before another report")
        # Generate rather than trusting a stale inventory; actual file tags still win below.
        source = years.dry_run(argparse.Namespace(file=None, limit=None))
        labels = versions.fold()["labels"]
        rows = []
        for r in source["rows"]:
            if not r.get("needs_mbid") or "no-recording" not in r.get("classes", []):
                continue
            row = {"id": r["id"], "years_id": r["id"], "song_id": r.get("song_id"), "file": r["file"],
                   "status": "withheld", "reason": "", "mbid": None}
            try:
                p = file_path(r["file"])
                before = snapshot(p)
                if years.read_recording(p):
                    row["reason"] = "file already has a recording MBID"
                elif why := years.recording_untaggable(p):
                    row["reason"] = why
                else:
                    tags = file_tags(p)
                    label = labels.get(identity.key(r["file"], None), {}).get("version")
                    row.update(identify(p, tags, label))
                    row["snapshot"] = before
                    row["audio"] = audio_hash(r["file"])
                    if not row["audio"] or snapshot(p) != before:
                        row.update(status="withheld", reason="file changed during lookup or missing audio hash")
            except Exception as e:
                row.update(status="withheld", reason=f"{type(e).__name__}: {e}")
            rows.append(row)
        rep = {"version": REPORT_VERSION, "id": uuid.uuid4().hex, "generated": years.now(),
               "years_generated": source["generated"], "rows": rows}
        save(report_path(), rep)
        return rep


def complete_events(raw):
    """Only newline-terminated journal records committed; a torn tail is not an event."""
    end = raw.rfind(b"\n") + 1
    return [json.loads(line) for line in raw[:end].splitlines() if line.strip()], end


def append(entry):
    path = backup_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a+b") as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        f.seek(0)
        _, end = complete_events(f.read())
        f.truncate(end)
        f.write((json.dumps(entry, ensure_ascii=False) + "\n").encode())
        f.flush()
        os.fsync(f.fileno())
    fsync_parent(path)


def transactions():
    entries = {}
    events, _ = complete_events(backup_path().read_bytes()) if backup_path().exists() else ([], 0)
    for e in events:
        if e.get("version") != REPORT_VERSION:
            raise ValueError("unsupported MBID backup version")
        entries[e["tx"]] = e
    return entries


def checkpoint(e, phase):
    e["phase"] = phase
    append(e)
    hook(phase, e)


def replace(path, want, expected, row, staged=None):
    tmp = path.with_name(f".{path.name}.mbid-tmp{path.suffix}")
    try:
        shutil.copy2(path, tmp)
        set_recording(tmp, want)
        with tmp.open("rb+") as f:
            os.fsync(f.fileno())
        if staged:
            staged(snapshot(tmp)["sha256"])
        hook("before-replace", row)
        if snapshot(path) != expected:
            raise ValueError("file changed before atomic replace")
        os.replace(tmp, path)
        fsync_parent(path)
    finally:
        tmp.unlink(missing_ok=True)


def registry(rel, mbid):
    """Refresh existing registry metadata, never add UUID or unrelated file tags."""
    with identity.locked():
        reg = identity.load(fresh=True)
        rid = reg["by_path"].get(rel)
        if rid and reg["rows"][rid].get("path") == rel and reg["rows"][rid].get("mbid") != mbid:
            reg["rows"][rid]["mbid"] = mbid
            identity.save(reg["rows"])
            with identity.registry_path().open("rb") as f:
                os.fsync(f.fileno())
            fsync_parent(identity.registry_path())


def association_plan(e):
    """Materialize identity.rekey semantics before any association changes.

    Recording absence has a file key (or an unchanged YouTube key), so there is
    no old recording's listening history to reattribute. Existing target data
    wins. Each compensating event restores only the operation we actually made.
    """
    from . import banned, tags, versions
    old, new = identity.key(e["file"], e["old"]), identity.key(e["file"], e["new"])
    if old == new:
        return []
    keep = any(identity.key(f, e["old"]) == old for f in years.sharing(e["file"], e["old"]))
    ts, ops = years.now(), []
    def op(path, forward, backward, selector):
        before = next((x for x in reversed(musicdb.jsonl(path)) if selects(x, selector)), {})
        ops.append({"path": str(path), "forward": forward, "backward": backward,
                    "selector": selector, "before": before})
    song = lambda s: dict(s, key=new, file=e["file"], mbid=e["new"])
    for name, songs in tags.lists().items():
        if src := songs.get(old):
            if new not in songs:
                add = dict(src, song=song(src["song"]))
                op(tags.LISTS, add, dict(add, ts=ts, action="remove"), ["list", name, new])
            if not keep:
                op(tags.LISTS, dict(src, ts=ts, action="remove"), src, ["list", name, old])
    state = tags.manual_genres()
    src = next((x["song"] for x in reversed(tags.events(tags.GENRES)) if x["song"]["key"] == old), None)
    if src and old in state:
        mine = state.get(new, {"add": set(), "exclude": set()})
        for genre in sorted(state[old]["add"] | state[old]["exclude"]):
            action = "add" if genre in state[old]["add"] else "exclude"
            ev = {"ts": ts, "genre": genre, "action": action, "song": src}
            if genre not in mine["add"] | mine["exclude"]:
                add = dict(ev, song=song(src))
                op(tags.GENRES, add, dict(add, action="reset"), ["genre", genre, new])
            if not keep:
                op(tags.GENRES, dict(ev, action="reset"), ev, ["genre", genre, old])
    state = banned.fold()
    if src := state.get(old):
        if new not in state:
            add = dict(src, key=new, file=e["file"])
            op(banned.log_path(), add, dict(add, ts=ts, action="unban"), ["ban", new])
        if not keep:
            op(banned.log_path(), dict(src, ts=ts, action="unban"), src, ["ban", old])
    state = versions.fold()
    if src := state["labels"].get(old):
        if new not in state["labels"]:
            add = dict(src, ts=ts, key=new, file=e["file"])
            op(versions.log_path(), add, dict(add, version="clear"), ["label", new])
        if not keep:
            op(versions.log_path(), dict(src, ts=ts, version="clear"), src, ["label", old])
    for src in state["tracks"].values():
        if src["action"] == "set" and src.get("key") == old and src.get("file") == e["file"]:
            op(versions.log_path(), dict(src, ts=ts, key=new), src, ["track", src["source"], src["track"]])
    return ops


def selects(event, selector):
    kind, *key = selector
    song = event.get("song", {})
    if kind == "list":
        return [event.get("list"), song.get("key")] == key
    if kind == "genre":
        return [event.get("genre"), song.get("key")] == key
    if kind == "ban":
        return [event.get("key")] == key
    if kind == "label":
        return event.get("action") == "label" and [event.get("key")] == key
    return [event.get("source"), event.get("track")] == key


def rekey(e, undo=False):
    shared = undo and bool(years.sharing(e["file"], e["new"]))
    for i, op in enumerate(e["associations"]):
        if shared and op["selector"][-1] == identity.key(e["file"], e["new"]):
            continue  # another file now needs the copied target associations
        path = pathlib.Path(op["path"])
        path.parent.mkdir(parents=True, exist_ok=True)
        marker = f"{e['tx']}:{i}:" + ("undo" if undo else "apply")
        with path.open("a+b") as f:
            fcntl.flock(f, fcntl.LOCK_EX)
            f.seek(0)
            events, end = complete_events(f.read())
            f.truncate(end)
            if any(x.get("mbid_operation") == marker for x in events):
                continue
            last = next((x for x in reversed(events) if selects(x, op["selector"])), {})
            if undo and last.get("mbid_operation") != f"{e['tx']}:{i}:apply":
                continue  # a later human edit is never compensated away
            if not undo and last != op["before"]:
                raise ValueError("association changed since preparation; rollback preserves that edit")
            event = dict(op["backward" if undo else "forward"], mbid_operation=marker)
            f.write((json.dumps(event, ensure_ascii=False) + "\n").encode())
            f.flush()
            os.fsync(f.fileno())
        fsync_parent(path)
        hook("association-written", e)
    registry(e["file"], e["old"] if undo else e["new"])


def update(dirs):
    # Unlike years' legacy helper, failures must remain resumable journal work.
    for d in sorted(dirs):
        subprocess.run(["mpc", "-q", "update", "--wait", d or "/"], check=True)


def apply(ids=None):
    done, skipped = [], []
    with locked():
        rep = load()
        entries = transactions()
        for r in rep["rows"]:
            if r["status"] != "SURE" or (ids and r["id"] not in ids):
                continue
            tx = rep["id"] + ":" + str(r["id"])
            e = entries.get(tx)
            if e and e["phase"] in ("done", "rolled-back"):
                continue
            try:
                p = file_path(r["file"])
                if e is None:
                    if snapshot(p) != r["snapshot"] or years.read_recording(p):
                        raise ValueError("file changed since dry run")
                    e = {"version": REPORT_VERSION, "tx": tx, "row": r["id"], "file": r["file"],
                         "old": None, "new": r["mbid"], "audio": r["audio"], "before": r["snapshot"]}
                    e["associations"] = association_plan(e)
                    checkpoint(e, "prepared")  # durable backup before touching audio or associations
                if e["phase"] == "prepared":
                    cur = years.read_recording(p)
                    if cur == e["new"] and snapshot(p)["sha256"] == e.get("written_sha256"):
                        checkpoint(e, "written")  # replace committed, process died before checkpoint
                    elif cur == e["old"] and snapshot(p) == e["before"]:
                        def staged(digest):
                            e["written_sha256"] = digest
                            append(e)  # recognize our exact staged bytes after a lost replace checkpoint
                        replace(p, e["new"], e["before"], r, staged)
                        hook("after-replace", e)
                        checkpoint(e, "written")
                    else:
                        raise ValueError("file changed since preparation")
                if e["phase"].startswith("undo-"):
                    raise ValueError("resume rollback first")
                if years.read_recording(p) != e["new"] or audio_hash(r["file"]) != e["audio"]:
                    raise ValueError("audio or recording changed after write")
                if e["phase"] == "written":
                    rekey(e)
                    checkpoint(e, "rekeyed")
                if e["phase"] == "rekeyed":
                    update({os.path.dirname(e["file"])})
                    checkpoint(e, "done")
                done.append({"id": r["id"], "file": r["file"], "mbid": e["new"]})
            except (OSError, ValueError, subprocess.CalledProcessError) as err:
                skipped.append({"id": r["id"], "file": r["file"], "why": str(err)})
    return {"applied": done, "skipped": skipped}


def rollback(ids=None):
    done, skipped = [], []
    with locked():
        for e in transactions().values():
            if e["phase"] == "rolled-back" or (ids and e["row"] not in ids):
                continue
            try:
                p = file_path(e["file"])
                before = snapshot(p)
                cur = years.read_recording(p)
                if e["phase"] == "prepared" and cur != e["new"]:
                    checkpoint(e, "rolled-back")  # cancel preparation; no owned tag or associations to undo
                    done.append({"id": e["row"], "file": e["file"], "cancelled": True})
                    continue
                if audio_hash(e["file"]) != e["audio"]:
                    raise ValueError("audio changed since apply")
                if snapshot(p) != before:
                    raise ValueError("file changed during rollback validation")
                if e["phase"] in ("undo-written", "undo-rekeyed") and cur != e["old"]:
                    raise ValueError("recording changed after rollback write")
                if e["phase"] not in ("undo-written", "undo-rekeyed"):
                    if e["phase"] != "undo-prepared":
                        if cur != e["new"]:
                            raise ValueError("recording changed since apply")
                        e["undo_before"] = before
                        checkpoint(e, "undo-prepared")
                    if cur == e["new"]:
                        replace(p, e["old"], e["undo_before"], e)
                        hook("after-undo-replace", e)
                    elif cur != e["old"]:
                        raise ValueError("recording changed during rollback")
                    checkpoint(e, "undo-written")
                if e["phase"] == "undo-written":
                    rekey(e, undo=True)
                    checkpoint(e, "undo-rekeyed")
                if e["phase"] == "undo-rekeyed":
                    update({os.path.dirname(e["file"])})
                    checkpoint(e, "rolled-back")
                done.append({"id": e["row"], "file": e["file"]})
            except (OSError, ValueError, subprocess.CalledProcessError) as err:
                skipped.append({"id": e["row"], "file": e["file"], "why": str(err)})
    return {"rolled_back": done, "skipped": skipped}


def main(argv):
    ap = argparse.ArgumentParser(prog="musicdb mbid", description=__doc__)
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--dry-run", action="store_true")
    g.add_argument("--apply", action="store_true")
    g.add_argument("--rollback", action="store_true")
    ap.add_argument("ids", nargs="*", type=int, metavar="ID")
    a = ap.parse_args(argv)
    if a.dry_run and a.ids:
        ap.error("IDs apply only to --apply or --rollback")
    out = dry_run() if a.dry_run else apply(set(a.ids)) if a.apply else rollback(set(a.ids))
    print(json.dumps(out, ensure_ascii=False))
