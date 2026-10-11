//! rormpc: the Play pane's helpers outside the pane itself.
//!
//! - Music's filters apply at once (plans/instant-filtering.md): every finished newest `hits` run into
//!   `PREVIEW_FILE` reconciles the queue (`rormpc_upnext::reconcile_hits_source`); there is no preview and no Apply.
//! - Browse's `P` (phase 3b) replaces the queue with an album, folder, artist or list in its order, and starts it
//!   at once. It asks first only when the source kind changes or more than a quarter of the queue would go; the
//!   replace carries the queue's `playlist` version and refuses when MPD's moved since. No retry, no delay.
//! - `ShowPlay` (5-9, 0, gl) leaves the view it asks for here; Play takes it on its next render. `z` / `Z`
//!   (`FilterUndo` / `FilterRedo`) leave their steps here too, taken by Play whether its tab is shown or not.

use std::{collections::HashSet, sync::Mutex};

use crate::{
    config::tabs::PlayView,
    ctx::Ctx,
    shared::macros::modal,
    ui::{
        modals::confirm_modal::{Action, ConfirmModal},
        panes::hits::PreviewInfo,
        rormpc_upnext::{self, Collection},
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

/// What the last filter change did, for the line under Music's header ("Filter: +rock", "Undo: rock cleared"),
/// and whether `z` / `Z` have something to do.
#[derive(Debug, Clone, Copy, Default)]
pub struct LastChange<'a> {
    pub what: Option<&'a str>,
    pub undo: bool,
    pub redo: bool,
}

/// The line under Music's header: what the filters made of the queue. `was`: the owned songs of the generation
/// before the newest one sent to MPD; `asked`: a filter change ran `hits` in this session (until then the file
/// on disk, or its absence, says nothing about the queue); `last`: the last change and the undo keys.
pub fn counts_line(info: &PreviewInfo, was: Option<usize>, asked: bool, last: LastChange<'_>) -> String {
    const IDLE: &str = "Filters apply at once · o: the result rows";
    if !asked {
        return IDLE.to_owned();
    }
    if info.running {
        return "updating for the new filters · Esc stops".to_owned();
    }
    if let Some(err) = &info.error {
        return format!("not applied: {err} (the queue keeps the last result)");
    }
    let mut keys = String::new();
    if last.undo {
        keys.push_str(" · z undoes");
    }
    if last.redo {
        keys.push_str(" · Z redoes");
    }
    if !info.ready {
        // another source came back (undo past the first touch): the filters have no result of their own
        return last.what.map_or_else(|| IDLE.to_owned(), |w| format!("{w}{keys}"));
    }
    let what = last.what.map_or(String::new(), |w| format!("{w} · "));
    let owned = info.files.len();
    let missing = info.matched.saturating_sub(owned);
    let was = was.filter(|w| *w != owned).map_or(String::new(), |w| format!(" · was {w}"));
    if owned == 0 {
        return format!("{what}0 owned · {missing} missing{was} · the upcoming songs went: it stops after the playing song{keys}");
    }
    format!("{what}{owned} owned · {missing} missing{was}{keys}")
}

/// `z` / `Z` (`FilterUndo` / `FilterRedo`) from any tab: a step through Music's filter history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Undo,
    Redo,
}

static STEPS: Mutex<Vec<Step>> = Mutex::new(Vec::new());

/// Leave an undo or redo for the Play pane, which takes them in order on its next frame (shown or not).
pub fn request_step(step: Step) {
    if let Ok(mut s) = STEPS.lock() {
        s.push(step);
    }
}

/// The steps asked for since the last call, in order.
pub fn take_steps() -> Vec<Step> {
    STEPS.lock().map(|mut s| std::mem::take(&mut *s)).unwrap_or_default()
}

/// One queue entry as the confirmation rule sees it.
#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    pub id: u32,
    pub file: &'a str,
}

/// Why Browse's `P` asks before replacing the queue with a source of `new_kind` ("album", "directory", ...), or
/// None to replace it at once: only when the kind changes or more than a quarter of the queue would go. The
/// playing song (it goes on) and Up next requests (they are kept) never count; an empty rest never asks.
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
        let makes = format!("P makes it {}", with_article(rormpc_upnext::kind_label(new_kind)));
        return Some(format!("The queue holds {what} ({} songs); {makes}.", rest.len()));
    }
    let gone = rest.iter().filter(|e| !files.contains(e.file)).count();
    (gone * 4 > rest.len()).then(|| format!("{gone} of the {} songs in the queue would go.", rest.len()))
}

/// "an album", "a folder".
fn with_article(word: &str) -> String {
    let an = word.starts_with(['a', 'e', 'i', 'o', 'u']);
    format!("{} {word}", if an { "an" } else { "a" })
}

/// Browse's `P`: replace the queue with `col` and start it now, asking first under `confirm_reason_for`. The queue
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

    fn confirm_reason(
        current_kind: Option<&str>,
        queue: &[Entry<'_>],
        playing: Option<u32>,
        up_next: &[u32],
        files: &HashSet<&str>,
    ) -> Option<String> {
        confirm_reason_for("album", current_kind, queue, playing, up_next, files)
    }

    #[test]
    fn an_album_queue_asks_only_when_more_than_a_quarter_goes() {
        let q = queue(&["a", "b", "c", "d", "e"]);
        let keep: HashSet<&str> = ["a", "b", "c", "d"].into();
        // 1 of 5 goes: no question
        assert_eq!(confirm_reason(Some("album"), &q, None, &[], &keep), None);
        let half: HashSet<&str> = ["a", "b", "c"].into();
        assert_eq!(
            confirm_reason(Some("album"), &q, None, &[], &half).as_deref(),
            Some("2 of the 5 songs in the queue would go.")
        );
        // exactly a quarter: no question
        let q4 = queue(&["a", "b", "c", "d"]);
        assert_eq!(confirm_reason(Some("album"), &q4, None, &[], &keep.iter().copied().take(3).collect()), None);
    }

    #[test]
    fn the_playing_song_and_up_next_never_count() {
        let q = queue(&["playing", "request", "a", "b"]);
        let files: HashSet<&str> = ["a", "b"].into();
        assert_eq!(confirm_reason(Some("album"), &q, Some(1), &[2], &files), None);
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
    fn the_counts_line_says_what_the_filters_made_of_the_queue() {
        let none = LastChange::default();
        assert_eq!(counts_line(&info(&["a", "b"], 3), Some(5), true, none), "2 owned · 1 missing · was 5");
        assert_eq!(counts_line(&info(&["a", "b"], 3), Some(2), true, none), "2 owned · 1 missing");
        assert_eq!(counts_line(&info(&["a"], 1), None, true, none), "1 owned · 0 missing");
        let zero = counts_line(&info(&[], 4), Some(3), true, none);
        assert_eq!(zero, "0 owned · 4 missing · was 3 · the upcoming songs went: it stops after the playing song");
        let running = PreviewInfo { running: true, ..info(&["a"], 1) };
        assert!(counts_line(&running, None, true, none).starts_with("updating"));
        let failed = PreviewInfo { error: Some("hits failed".to_owned()), ..info(&["a"], 1) };
        assert_eq!(counts_line(&failed, None, true, none), "not applied: hits failed (the queue keeps the last result)");
        let other = PreviewInfo { ready: false, ..info(&["a"], 1) };
        assert!(counts_line(&other, None, true, none).starts_with("Filters apply at once"));
        // before any filter change: a missing or old file is no error of the queue
        let missing = PreviewInfo { error: Some("no hits file".to_owned()), ready: false, ..info(&[], 0) };
        assert!(counts_line(&missing, None, false, none).starts_with("Filters apply at once"));
    }

    #[test]
    fn the_counts_line_names_the_last_change_and_the_undo_keys() {
        let last = LastChange { what: Some("Filter: +rock"), undo: true, redo: false };
        assert_eq!(counts_line(&info(&["a", "b"], 3), Some(5), true, last), "Filter: +rock · 2 owned · 1 missing · was 5 · z undoes");
        let both = LastChange { what: Some("Undo: rock cleared"), undo: true, redo: true };
        assert_eq!(counts_line(&info(&["a"], 1), None, true, both), "Undo: rock cleared · 1 owned · 0 missing · z undoes · Z redoes");
        // an album came back: the filters have no result of their own
        let back = LastChange { what: Some("Undo: back to album Low"), undo: false, redo: true };
        let other = PreviewInfo { ready: false, ..info(&["a"], 1) };
        assert_eq!(counts_line(&other, None, true, back), "Undo: back to album Low · Z redoes");
    }

    #[test]
    fn steps_are_taken_once_in_order() {
        request_step(Step::Undo);
        request_step(Step::Redo);
        assert_eq!(take_steps(), vec![Step::Undo, Step::Redo]);
        assert!(take_steps().is_empty());
    }
}
