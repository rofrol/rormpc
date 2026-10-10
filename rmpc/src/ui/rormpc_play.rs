//! rormpc: the preview and Apply of the Play pane (plans/combined-view.md "A filter change prepares, Apply plays").
//!
//! - A filter change runs `hits` into its own preview file (`PREVIEW_FILE`); `current.json` keeps meaning the Hits
//!   pane's result. Play shows the preview's counts in a banner; nothing in MPD changes.
//! - Apply (`a`, the banner's button, the column's Apply) replaces the queue with the preview's owned songs as
//!   "Play these N songs" does: the playing song goes on, Up next is kept, source.json names the rules and their
//!   canonical hash (mpd-player's round key: the same rules keep the round, other rules start a new one).
//! - It asks first only when the source kind changes or more than a quarter of the queue would go.
//! - The counts and the confirmation are judged on MPD's queue at that moment; the replace carries that queue's
//!   `playlist` version and refuses ("the queue changed, preview again") when MPD's differs. No retry, no delay.
//! - Browse's `P` (phase 3b) replaces the queue with an album, folder, artist or list in its order under the same
//!   confirmation rule, and starts it at once.
//! - `ShowPlay` (5-9, 0, gl) leaves the view it asks for here; Play takes it on its next render.
//! - A preview older than the library's last change (MPD's database update, the deletion journal's newest
//!   entry) is recomputed before Apply plays it; the banner says "Preview outdated, recomputing" meanwhile. The
//!   replace itself leaves out files MPD no longer has and never deletes the old queue before the new songs are in.

use std::{
    collections::HashSet,
    sync::Mutex,
    time::{Duration, SystemTime},
};

use rmpc_mpd::{
    errors::MpdError,
    from_mpd::{FromMpd, LineHandled},
    proto_client::ProtoClient,
};

use crate::{
    config::tabs::PlayView,
    ctx::Ctx,
    shared::macros::modal,
    ui::{
        modals::confirm_modal::{Action, ConfirmModal},
        panes::hits::PreviewInfo,
        rormpc_upnext::{self, Collection, HitsSource},
    },
};

static VIEW_REQUEST: Mutex<Option<PlayView>> = Mutex::new(None);

/// `ShowPlay`: what the Play pane shows next (taken by `take_view` on its render).
pub fn request_view(view: PlayView) {
    if let Ok(mut v) = VIEW_REQUEST.lock() {
        *v = Some(view);
    }
}

/// The view `ShowPlay` asked for, once.
pub fn take_view() -> Option<PlayView> {
    VIEW_REQUEST.lock().ok().and_then(|mut v| v.take())
}

/// Where Play's `hits` runs write their result.
pub const PREVIEW_FILE: &str = "~/.cache/rormpc/hits/preview.json";

/// The canonical hash of a rules key (FNV-1a, 64 bits, hex): stable across runs and machines.
pub fn rules_hash(key: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// One queue entry as the confirmation rule sees it.
#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    pub id: u32,
    pub file: &'a str,
}

/// Why Apply asks before replacing the queue, or None to replace it at once. The playing song (it goes on) and
/// Up next requests (they are kept) never count; an empty rest never asks.
pub fn confirm_reason(
    current_kind: Option<&str>,
    queue: &[Entry<'_>],
    playing: Option<u32>,
    up_next: &[u32],
    files: &HashSet<&str>,
) -> Option<String> {
    confirm_reason_for("hits", current_kind, queue, playing, up_next, files)
}

/// Apply's rule for any new source kind (Browse's `P`: "album", "directory", ...): ask only when the kind
/// changes or more than a quarter of the queue would go.
pub fn confirm_reason_for(
    new_kind: &str,
    current_kind: Option<&str>,
    queue: &[Entry<'_>],
    playing: Option<u32>,
    up_next: &[u32],
    files: &HashSet<&str>,
) -> Option<String> {
    let rest: Vec<&Entry<'_>> =
        queue.iter().filter(|e| Some(e.id) != playing && !up_next.contains(&e.id)).collect();
    if rest.is_empty() {
        return None;
    }
    if current_kind != Some(new_kind) {
        let what = match current_kind {
            Some("library") => "the whole library".to_owned(),
            Some("playlist") => "a playlist".to_owned(),
            Some("hits") => "a Hits result".to_owned(),
            Some(kind) => format!("{} source", with_article(rormpc_upnext::kind_label(kind))),
            None => "songs from no known source".to_owned(),
        };
        let makes = match new_kind {
            "hits" => "Apply makes it a Hits result".to_owned(),
            kind => format!("P makes it {}", with_article(rormpc_upnext::kind_label(kind))),
        };
        return Some(format!("The queue holds {what} ({} songs); {makes}.", rest.len()));
    }
    let gone = rest.iter().filter(|e| !files.contains(e.file)).count();
    (gone * 4 > rest.len()).then(|| format!("{gone} of the {} songs in the queue would go.", rest.len()))
}

/// MPD's `stats`, of which only the database's last update counts here.
#[derive(Debug, Default)]
struct DbStats {
    db_update: Option<u64>,
}

impl FromMpd for DbStats {
    fn next_internal(&mut self, key: &str, value: String) -> Result<LineHandled, MpdError> {
        if key == "db_update" {
            self.db_update = Some(value.parse().map_err(|_| MpdError::Parse(format!("db_update: {value}")))?);
        }
        Ok(LineHandled::Yes)
    }
}

/// When MPD's database last changed (`stats` `db_update`), None when it never did or MPD cannot say.
pub fn db_updated_at(ctx: &Ctx) -> Option<SystemTime> {
    let stats = ctx.query_sync(|client| {
        client.execute("stats")?;
        Ok(client.read_response::<DbStats>()?)
    });
    stats.ok()?.db_update.map(|secs| SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
}

/// Whether a preview written at `preview` is older than the library's last change (MPD's database update, the
/// deletion journal's newest entry). MPD counts whole seconds: a change in the preview's own second counts as
/// newer, so a preview is never trusted over a deletion it may not have seen.
pub fn preview_outdated(preview: Option<SystemTime>, changes: &[Option<SystemTime>]) -> bool {
    let secs = |t: SystemTime| t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let Some(preview) = preview.map(secs) else { return false };
    changes.iter().flatten().any(|&t| secs(t) >= preview)
}

/// The banner over a preview and whether its Apply would play anything. `recomputing`: Apply found the preview
/// older than the library and runs `hits` again before it plays.
pub fn banner(info: &PreviewInfo, playing_file: Option<&str>, recomputing: bool) -> (String, bool) {
    if info.running && recomputing {
        return ("Preview outdated, recomputing… (Apply plays it when the counts are in)".to_owned(), false);
    }
    if info.running {
        return ("Preview · running hits… (Apply waits for it)".to_owned(), false);
    }
    if let Some(err) = &info.error {
        return (format!("Preview · {err}"), false);
    }
    if !info.ready {
        return ("Preview · not made yet".to_owned(), false);
    }
    let owned = info.files.len();
    let missing = info.matched.saturating_sub(owned);
    let head = format!("Preview · {} · {} matched · {owned} owned · {missing} missing", info.label, info.matched);
    if owned == 0 {
        return (format!("{head} · nothing to play: widen the filter"), false);
    }
    let playing = match playing_file {
        Some(f) if info.files.iter().any(|x| x == f) => " · playing song: in it",
        Some(_) => " · playing song: outside (plays on, not in the round)",
        None => "",
    };
    (format!("{head}{playing}"), true)
}

/// Apply: replace the queue with `src`, asking first when `confirm_reason` says so. The queue version is the
/// one the decision was made on: the replace refuses if MPD's queue moved since.
pub fn apply(ctx: &Ctx, src: HitsSource) {
    let version = ctx.status.playlist;
    let kind = rormpc_upnext::source_info().map(|(kind, _, _)| kind);
    let entries: Vec<Entry<'_>> = ctx.queue.iter().map(|s| Entry { id: s.id, file: s.file.as_str() }).collect();
    let files: HashSet<&str> = src.files.iter().map(String::as_str).collect();
    let reason = confirm_reason(kind.as_deref(), &entries, ctx.status.songid, &rormpc_upnext::up_next_ids(), &files);
    let Some(reason) = reason else {
        rormpc_upnext::apply_hits_source(ctx, src, version);
        return;
    };
    let waiting = rormpc_upnext::up_next_ids().len();
    let message = vec![format!(
        "Play {} songs of {}?\n\n{reason}\nThe song playing now goes on, then the new source plays.{}",
        src.files.len(),
        src.name,
        if waiting > 0 { format!("\nUp next ({waiting}) is kept and plays first.") } else { String::new() }
    )];
    let go = move |ctx: &Ctx| -> anyhow::Result<()> {
        rormpc_upnext::apply_hits_source(ctx, src, version);
        Ok(())
    };
    modal!(
        ctx,
        ConfirmModal::builder()
            .ctx(ctx)
            .message(message)
            .action(Action::CustomButtons { buttons: vec![("Cancel", Box::new(|_: &Ctx| Ok(()))), ("Apply", Box::new(go))] })
            .build()
    );
}

/// "an album", "a folder".
fn with_article(word: &str) -> String {
    let an = word.starts_with(['a', 'e', 'i', 'o', 'u']);
    format!("{} {word}", if an { "an" } else { "a" })
}

/// Browse's `P`: replace the queue with `col` and start it now, asking first under Apply's rule. The queue
/// version is the one the decision was made on: the replace refuses if MPD's queue moved since.
pub fn play_collection(ctx: &Ctx, col: Collection) {
    if col.files.is_empty() {
        return crate::shared::macros::status_info!("Nothing to play here");
    }
    let version = ctx.status.playlist;
    let kind = rormpc_upnext::source_info().map(|(kind, _, _)| kind);
    let entries: Vec<Entry<'_>> = ctx.queue.iter().map(|s| Entry { id: s.id, file: s.file.as_str() }).collect();
    let files: HashSet<&str> = col.files.iter().map(String::as_str).collect();
    let reason =
        confirm_reason_for(&col.kind, kind.as_deref(), &entries, ctx.status.songid, &rormpc_upnext::up_next_ids(), &files);
    let Some(reason) = reason else {
        rormpc_upnext::play_collection(ctx, col, version);
        return;
    };
    let first = col.files.get(col.start).map_or("", |f| f.rsplit('/').next().unwrap_or(f));
    let message = vec![format!(
        "Play {} \"{}\" ({} song{})?\n\n{reason}\nIt starts now with {first}, in its order: weighted shuffle and random go off.",
        rormpc_upnext::kind_label(&col.kind),
        col.name,
        col.files.len(),
        if col.files.len() == 1 { "" } else { "s" },
    )];
    let go = move |ctx: &Ctx| -> anyhow::Result<()> {
        rormpc_upnext::play_collection(ctx, col, version);
        Ok(())
    };
    modal!(
        ctx,
        ConfirmModal::builder()
            .ctx(ctx)
            .message(message)
            .action(Action::CustomButtons { buttons: vec![("Cancel", Box::new(|_: &Ctx| Ok(()))), ("Play", Box::new(go))] })
            .build()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue<'a>(files: &[&'a str]) -> Vec<Entry<'a>> {
        files.iter().enumerate().map(|(i, f)| Entry { id: i as u32 + 1, file: f }).collect()
    }

    #[test]
    fn the_rules_hash_is_stable_and_tells_rules_apart() {
        assert_eq!(rules_hash(""), "cbf29ce484222325");
        assert_eq!(rules_hash("a"), "af63dc4c8601ec8c");
        assert_ne!(rules_hash(r#"{"top":"1-10"}"#), rules_hash(r#"{"top":"11-20"}"#));
    }

    #[test]
    fn a_hits_queue_asks_only_when_more_than_a_quarter_goes() {
        let q = queue(&["a", "b", "c", "d", "e"]);
        let keep: HashSet<&str> = ["a", "b", "c", "d"].into();
        // 1 of 5 goes: no question
        assert_eq!(confirm_reason(Some("hits"), &q, None, &[], &keep), None);
        let half: HashSet<&str> = ["a", "b", "c"].into();
        assert_eq!(
            confirm_reason(Some("hits"), &q, None, &[], &half).as_deref(),
            Some("2 of the 5 songs in the queue would go.")
        );
        // exactly a quarter: no question
        let q4 = queue(&["a", "b", "c", "d"]);
        assert_eq!(confirm_reason(Some("hits"), &q4, None, &[], &keep.iter().copied().take(3).collect()), None);
    }

    #[test]
    fn the_playing_song_and_up_next_never_count() {
        let q = queue(&["playing", "request", "a", "b"]);
        let files: HashSet<&str> = ["a", "b"].into();
        assert_eq!(confirm_reason(Some("hits"), &q, Some(1), &[2], &files), None);
        // only the playing song and a request left: nothing to lose, whatever the kind
        let q = queue(&["playing", "request"]);
        assert_eq!(confirm_reason(Some("library"), &q, Some(1), &[2], &HashSet::new()), None);
    }

    #[test]
    fn another_source_kind_asks() {
        let q = queue(&["a", "b"]);
        let files: HashSet<&str> = ["a", "b"].into();
        let reason = confirm_reason(Some("library"), &q, None, &[], &files).unwrap();
        assert!(reason.contains("the whole library"), "{reason}");
        assert!(confirm_reason(None, &q, None, &[], &files).is_some());
        assert_eq!(confirm_reason(None, &[], None, &[], &files), None);
    }

    #[test]
    fn browse_play_uses_apply_rule_for_its_own_kind() {
        let q = queue(&["a", "b", "c", "d"]);
        let album: HashSet<&str> = ["x", "y"].into();
        // a Hits queue replaced by an album: the kind changes, it asks
        let reason = confirm_reason_for("album", Some("hits"), &q, None, &[], &album).expect("a kind change asks");
        assert_eq!(reason, "The queue holds a Hits result (4 songs); P makes it an album.");
        // album after album: only the share that goes counts
        let same: HashSet<&str> = ["a", "b", "c", "x"].into();
        assert_eq!(confirm_reason_for("album", Some("album"), &q, None, &[], &same), None);
        assert!(confirm_reason_for("album", Some("album"), &q, None, &[], &album).is_some());
        // a folder over a playlist names the old kind
        let reason = confirm_reason_for("directory", Some("playlist"), &q, None, &[], &album).expect("a kind change asks");
        assert!(reason.starts_with("The queue holds a playlist (4 songs); P makes it a folder"), "{reason}");
    }

    fn info(files: &[&str], matched: usize) -> PreviewInfo {
        PreviewInfo {
            running: false,
            error: None,
            ready: true,
            label: "1980s".to_owned(),
            matched,
            files: files.iter().map(|f| (*f).to_owned()).collect(),
        }
    }

    #[test]
    fn the_banner_counts_and_disables_apply_without_owned_songs() {
        let (text, on) = banner(&info(&["a", "b"], 3), Some("x"), false);
        assert_eq!(text, "Preview · 1980s · 3 matched · 2 owned · 1 missing · playing song: outside (plays on, not in the round)");
        assert!(on);
        let (text, on) = banner(&info(&["a"], 1), Some("a"), false);
        assert!(text.ends_with("playing song: in it"), "{text}");
        assert!(on);
        let (text, on) = banner(&info(&[], 4), None, false);
        assert!(text.contains("0 owned · 4 missing · nothing to play"), "{text}");
        assert!(!on);
        let running = PreviewInfo { running: true, ..info(&["a"], 1) };
        assert!(!banner(&running, None, false).1);
        let stale = PreviewInfo { ready: false, ..info(&["a"], 1) };
        assert!(!banner(&stale, None, false).1);
        let (text, on) = banner(&running, None, true);
        assert!(text.starts_with("Preview outdated, recomputing"), "{text}");
        assert!(!on);
    }

    #[test]
    fn a_preview_older_than_the_library_is_outdated() {
        let at = |secs: u64, nanos: u32| Some(SystemTime::UNIX_EPOCH + Duration::new(secs, nanos));
        // made at 15:30, a song deleted at 16:31 (the reported case): recomputed
        assert!(preview_outdated(at(1_000, 0), &[at(4_660, 0), None]));
        // the journal alone is enough
        assert!(preview_outdated(at(1_000, 0), &[None, at(1_001, 0)]));
        // MPD counts whole seconds: a change in the preview's own second is not trusted
        assert!(preview_outdated(at(1_000, 900_000_000), &[at(1_000, 0)]));
        // made after every change, or nothing known: played as it is
        assert!(!preview_outdated(at(1_001, 0), &[at(1_000, 0), at(999, 0)]));
        assert!(!preview_outdated(at(1_000, 0), &[None, None]));
        assert!(!preview_outdated(None, &[at(1_000, 0)]));
    }

    #[test]
    fn db_update_is_read_from_mpd_stats() {
        let mut stats = DbStats::default();
        for line in ["artists: 3", "uptime: 10", "db_update: 1760104260", "playtime: 0"] {
            stats.next(line.to_owned()).expect("a stats line");
        }
        assert_eq!(stats.db_update, Some(1_760_104_260));
    }
}
