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

use std::collections::HashSet;

use crate::{
    ctx::Ctx,
    shared::macros::modal,
    ui::{
        modals::confirm_modal::{Action, ConfirmModal},
        panes::hits::PreviewInfo,
        rormpc_upnext::{self, HitsSource},
    },
};

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
    let rest: Vec<&Entry<'_>> =
        queue.iter().filter(|e| Some(e.id) != playing && !up_next.contains(&e.id)).collect();
    if rest.is_empty() {
        return None;
    }
    if current_kind != Some("hits") {
        let what = match current_kind {
            Some("library") => "the whole library".to_owned(),
            Some("playlist") => "a playlist".to_owned(),
            Some(kind) => format!("a {kind} source"),
            None => "songs from no known source".to_owned(),
        };
        return Some(format!("The queue holds {what} ({} songs); Apply makes it a Hits result.", rest.len()));
    }
    let gone = rest.iter().filter(|e| !files.contains(e.file)).count();
    (gone * 4 > rest.len()).then(|| format!("{gone} of the {} songs in the queue would go.", rest.len()))
}

/// The banner over a preview and whether its Apply would play anything.
pub fn banner(info: &PreviewInfo, playing_file: Option<&str>) -> (String, bool) {
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
        let (text, on) = banner(&info(&["a", "b"], 3), Some("x"));
        assert_eq!(text, "Preview · 1980s · 3 matched · 2 owned · 1 missing · playing song: outside (plays on, not in the round)");
        assert!(on);
        let (text, on) = banner(&info(&["a"], 1), Some("a"));
        assert!(text.ends_with("playing song: in it"), "{text}");
        assert!(on);
        let (text, on) = banner(&info(&[], 4), None);
        assert!(text.contains("0 owned · 4 missing · nothing to play"), "{text}");
        assert!(!on);
        let running = PreviewInfo { running: true, ..info(&["a"], 1) };
        assert!(!banner(&running, None).1);
        let stale = PreviewInfo { ready: false, ..info(&["a"], 1) };
        assert!(!banner(&stale, None).1);
    }
}
