"""Manage your own YouTube playlists through the official YouTube Data API (OAuth, no cookies).

  yt-playlist auth                     # one-time browser login; token in secrets_dir (settings)/youtube-token.json
  yt-playlist list                     # your playlists with ids and sizes
  yt-playlist use ID [ID ...]          # the music playlists musicdb deletions should clean up
  yt-playlist find VIDEO_ID [--json]   # which of those playlists contain the video (--json: with titles)
  yt-playlist remove VIDEO_ID [--dry-run]   # remove the video from those playlists
  yt-playlist add VIDEO_ID --playlist ID [...]  # put it back (musicdb restore): appended, skipped where it is
  yt-playlist index [--json]           # cache which videos the chosen playlists hold (musicdb update: daily)

When the login has expired, `find --json` answers from the cached index and says when it was checked
("cached_at"), so rormpc's delete menu still names the playlists; removing always needs a live login.

Setup (once): Google Cloud project with "YouTube Data API v3" enabled, OAuth consent
screen in "Testing" with yourself as a test user, an OAuth client of type "Desktop app", its JSON saved as
youtube-client.json in secrets_dir (settings). Testing apps get refresh tokens that expire after 7 days, so a
run from a terminal opens the browser to log in again when needed (publishing would need a privacy policy). Playlist choice is kept in
$MUSICDB_DATA/youtube-playlists.json. Quota: a list page costs 1 unit, a removal or an insertion 50 (10,000 a
day).
"""
import argparse, datetime as dt, json, sys

from . import settings

SECRETS = settings.SECRETS_DIR
CLIENT, TOKEN = SECRETS / "youtube-client.json", SECRETS / "youtube-token.json"
DATA = settings.DATA_DIR
CHOSEN = DATA / "youtube-playlists.json"
INDEX = settings.XDG_CACHE / "rormpc-tools" / "youtube-index.json"
SCOPES = ["https://www.googleapis.com/auth/youtube"]  # read + edit your playlists


def creds(interactive=False):
    from google.auth.transport.requests import Request
    from google.oauth2.credentials import Credentials
    from google.auth.exceptions import RefreshError
    c = Credentials.from_authorized_user_file(str(TOKEN), SCOPES) if TOKEN.exists() else None
    if c and c.expired and c.refresh_token:
        try:
            c.refresh(Request())
        except RefreshError:  # the OAuth app stays in "Testing": Google revokes its refresh tokens after 7 days
            c = None
    if not (c and c.valid):
        if not (interactive or sys.stdin.isatty()):
            sys.exit("YouTube login expired: run `yt-playlist auth`")
        if not CLIENT.exists():
            sys.exit(f"missing OAuth client JSON: {CLIENT} (see `yt-playlist --help`)")
        from google_auth_oauthlib.flow import InstalledAppFlow
        c = InstalledAppFlow.from_client_secrets_file(str(CLIENT), SCOPES).run_local_server(port=0)
    SECRETS.mkdir(parents=True, exist_ok=True)
    TOKEN.write_text(c.to_json())
    TOKEN.chmod(0o600)
    return c


def api(interactive=False):
    from googleapiclient.discovery import build
    return build("youtube", "v3", credentials=creds(interactive), cache_discovery=False)


def pages(request_fn, **kw):
    token = None
    while True:
        r = request_fn(pageToken=token, maxResults=50, **kw).execute()
        yield from r.get("items", [])
        token = r.get("nextPageToken")
        if not token:
            return


def chosen():
    return json.loads(CHOSEN.read_text()) if CHOSEN.exists() else []


def titles(yt):
    """{playlist id: title} of the chosen playlists (one request)."""
    ids = chosen()
    if not ids:
        return {}
    r = yt.playlists().list(part="snippet", id=",".join(ids), maxResults=50).execute()
    return {pl["id"]: pl["snippet"]["title"] for pl in r.get("items", [])}


def api_error(err):
    """Keep the structured API reason, not just HttpError's human-readable message."""
    from googleapiclient.errors import HttpError
    if isinstance(err, HttpError):
        try:
            reason = json.loads(err.content)["error"]["errors"][0]["reason"]
        except (ValueError, KeyError, IndexError, TypeError):
            reason = "unknown"
        return f"{err.resp.status} {reason}", reason
    return str(err), type(err).__name__


def video_of(item):
    return (item.get("contentDetails", {}).get("videoId")
            or item.get("snippet", {}).get("resourceId", {}).get("videoId"))


def playlist_hits(yt, video_id, pid, unfiltered=False):
    """Never interpret a lookup's videoNotFound as evidence of playlist absence."""
    reason = None
    if not unfiltered:
        try:
            items = list(pages(yt.playlistItems().list, part="id,snippet,contentDetails",
                               playlistId=pid, videoId=video_id))
            return [it["id"] for it in items if video_of(it) == video_id], reason
        except Exception as err:
            _, reason = api_error(err)
            if reason != "videoNotFound":
                raise
    items = list(pages(yt.playlistItems().list, part="id,snippet,contentDetails", playlistId=pid))
    return [it["id"] for it in items if video_of(it) == video_id], reason


def find(yt, video_id):
    out = {"video": video_id, "playlists": [], "results": []}
    for pid in chosen():
        result = {"id": pid}
        try:
            hits, reason = playlist_hits(yt, video_id, pid)
            result.update(outcome="present" if hits else "absent", reason=reason)
            out["playlists"].extend({"id": pid, "item": item} for item in hits)
        except Exception as err:
            message, reason = api_error(err)
            result.update(outcome=f"failed: {message}", reason=reason, operation="list")
        out["results"].append(result)
    failures = [r["outcome"] for r in out["results"] if r["outcome"].startswith("failed:")]
    if failures:
        out["error"] = "; ".join(failures)
    return out


def items_with(yt, video_id):
    out = find(yt, video_id)
    if out.get("error"):
        raise RuntimeError(out["error"])
    return [(p["id"], p["item"]) for p in out["playlists"]]


def remove(yt, video_id, dry_run=False, progress=None, intents=None, playlist_ids=None):
    """Complete scans reconcile prior intents; journal callbacks run before and after each DELETE."""
    out = {"video": video_id, "removed": [], "results": []}
    intents = intents if intents is not None else []
    for pid in chosen() if playlist_ids is None else playlist_ids:
        result = {"id": pid, "removed_items": [], "outcome": "absent"}
        out["results"].append(result)
        try:
            # Unfiltered enumeration also reconciles a lost DELETE response without sending it twice.
            hits, _ = playlist_hits(yt, video_id, pid, unfiltered=True)
            for intent in intents:
                if intent["id"] == pid and intent["item"] not in hits:
                    intent["state"] = "removed"
            for item in hits:
                intent = next((x for x in intents if x["id"] == pid and x["item"] == item), None)
                if intent and intent["state"] == "removed":
                    continue
                if dry_run:
                    result["outcome"] = "would remove"
                    continue
                if intent is None:
                    intent = {"id": pid, "item": item, "state": "intent"}
                    intents.append(intent)
                if progress:
                    progress(out, intents)  # durable intent before the remote mutation
                try:
                    yt.playlistItems().delete(id=item).execute()
                    intent["state"] = "removed"
                except Exception as err:
                    message, reason = api_error(err)
                    result.update(outcome=f"failed: {message}", reason=reason, operation="delete")
                if progress:
                    progress(out, intents)
            result["removed_items"] = [x["item"] for x in intents if x["id"] == pid and x["state"] == "removed"]
            if result["removed_items"]:
                out["removed"].append(pid)
                if not result["outcome"].startswith("failed:"):
                    result["outcome"] = "removed"
        except Exception as err:
            message, reason = api_error(err)
            result.update(outcome=f"failed: {message}", reason=reason, operation="list")
        if progress:
            progress(out, intents)
    return out


def add(yt, video_id, playlist_ids):
    """Append the video to each playlist that does not hold it yet (looked up first, so a rerun never adds it
    twice): {"video", "items": {playlist id: [playlist item ids]}, "added": [...], "present": [...]}."""
    out = {"video": video_id, "items": {}, "added": [], "present": []}
    for pid in playlist_ids:
        have = [it["id"] for it in pages(yt.playlistItems().list, part="id", playlistId=pid, videoId=video_id)]
        if have:
            out["present"].append(pid)
        else:
            it = yt.playlistItems().insert(part="snippet", body={"snippet": {
                "playlistId": pid, "resourceId": {"kind": "youtube#video", "videoId": video_id}}}).execute()
            have = [it["id"]]
            out["added"].append(pid)
        out["items"][pid] = have
    return out


def build_index(yt):
    """{checked_at, playlists: {id: {title, items: {video id: [playlist item ids]}}}} of the chosen playlists,
    written atomically. A list page costs 1 quota unit per 50 videos."""
    names = titles(yt)
    out = {"checked_at": dt.datetime.now().isoformat(timespec="seconds"), "playlists": {}}
    for pid in chosen():
        items = {}
        for it in pages(yt.playlistItems().list, part="id,snippet", playlistId=pid):
            vid = it["snippet"].get("resourceId", {}).get("videoId")
            if vid:
                items.setdefault(vid, []).append(it["id"])
        out["playlists"][pid] = {"title": names.get(pid, pid), "items": items}
    INDEX.parent.mkdir(parents=True, exist_ok=True)
    tmp = INDEX.with_suffix(".tmp")
    tmp.write_text(json.dumps(out, ensure_ascii=False))
    tmp.replace(INDEX)
    return out


def cached_find(video, why):
    """find --json from the cached index, or the error when there is none (never "in no playlist")."""
    if not INDEX.exists():
        return {"video": video, "error": why}
    idx = json.loads(INDEX.read_text())
    hits = [{"id": pid, "title": pl["title"], "item": item}
            for pid, pl in idx["playlists"].items() if pid in chosen() for item in pl["items"].get(video, [])]
    return {"video": video, "playlists": hits, "cached_at": idx["checked_at"], "note": why}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = ap.add_subparsers(dest="cmd", required=True)
    sp.add_parser("auth")
    sp.add_parser("list")
    p = sp.add_parser("use"); p.add_argument("ids", nargs="+")
    p = sp.add_parser("find"); p.add_argument("video"); p.add_argument("--json", action="store_true")
    p = sp.add_parser("remove"); p.add_argument("video"); p.add_argument("--dry-run", action="store_true")
    p = sp.add_parser("add"); p.add_argument("video"); p.add_argument("--playlist", action="append", required=True)
    p = sp.add_parser("index"); p.add_argument("--json", action="store_true")
    a = ap.parse_args()
    if a.cmd == "find" and a.json:
        try:
            yt = api()
        except SystemExit as e:  # login expired: answer from the cache, saying how old it is
            print(json.dumps(cached_find(a.video, str(e)), ensure_ascii=False))
            return
        out = find(yt, a.video)
        names = titles(yt)
        for hit in out["playlists"]:
            hit["title"] = names.get(hit["id"], hit["id"])
        print(json.dumps(out, ensure_ascii=False))
        if out.get("error"):
            raise SystemExit(1)
        return
    if a.cmd == "index":
        out = build_index(api())
        n = sum(len(pl["items"]) for pl in out["playlists"].values())
        print(json.dumps(out, ensure_ascii=False) if a.json else f"indexed {len(out['playlists'])} playlists, {n} videos -> {INDEX}")
        return
    if a.cmd == "add":
        out = add(api(), a.video, a.playlist)
        for pid in out["added"]:
            print(f"added {a.video} to {pid}")
        print(json.dumps(out))
        return
    if a.cmd == "auth":
        api(interactive=True)
        print(f"logged in; token in {TOKEN}")
    elif a.cmd == "list":
        mine = set(chosen())
        for pl in pages(api().playlists().list, part="snippet,contentDetails", mine=True):
            print(f"{'*' if pl['id'] in mine else ' '} {pl['id']}  {pl['contentDetails']['itemCount']:4d}  {pl['snippet']['title']}")
    elif a.cmd == "use":
        CHOSEN.parent.mkdir(parents=True, exist_ok=True)
        CHOSEN.write_text(json.dumps(a.ids, indent=1) + "\n")
        print(f"{len(a.ids)} playlists saved to {CHOSEN}")
    elif a.cmd == "find":
        yt = api()
        hits = items_with(yt, a.video)
        if a.json:
            names = titles(yt)
            print(json.dumps({"video": a.video, "playlists": [{"id": pid, "title": names.get(pid, pid), "item": item}
                                                              for pid, item in hits]}, ensure_ascii=False))
        else:
            for pid, item in hits:
                print(pid, item)
    elif a.cmd == "remove":
        out = remove(api(), a.video, dry_run=a.dry_run)
        print(json.dumps(out))
        if any(r["outcome"].startswith("failed:") for r in out["results"]):
            raise SystemExit(1)


if __name__ == "__main__":
    main()
