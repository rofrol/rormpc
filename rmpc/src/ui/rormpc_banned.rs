//! rormpc: a hand removal from the queue bans the song (plans/instant-filtering.md, stage 5; decided by the user
//! 2026-10-11, against both models). With filters applying at once, a one-off removal would come back on the next
//! filter click, so `d` on a queue song (Music's queue body, the plan view and the Queue pane, a middle click or
//! the row menu alike) takes it out of MPD's queue and runs `musicdb ban` on its file in the background: the file
//! stays on disk, `ListenBrainz` is untouched, `hits` and mpd-player never bring it back. The ban is an entry of
//! Music's undo timeline (`z` unbans it, `Z` bans it again); the Banned panel (`gB`) lists and unbans them.
//!
//! Not a ban: `D` (clear the queue), removing an Up next request (it only cancels the request), "Remove repeated
//! entries".

use rmpc_mpd::mpd_client::MpdClient as _;
use serde::Deserialize;

use crate::{
    ctx::Ctx,
    shared::{
        events::AppEvent,
        macros::{status_error, status_info},
    },
    ui::{
        panes::deleted::run,
        rormpc_play::{self, Step},
        rormpc_undo::Ban,
        rormpc_upnext,
    },
};

/// The `musicdb banned --json` this rormpc reads (rormpc-tools `BANNED_VERSION`).
const BANNED_VERSION: u32 = 1;

/// One banned song of `musicdb banned --json`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct BannedSong {
    pub key: String,
    pub file: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub title: String,
    /// when it was banned (ISO local time)
    #[serde(default)]
    pub ts: String,
}

#[derive(Debug, Deserialize)]
struct BannedFile {
    version: u32,
    songs: Vec<BannedSong>,
}

/// The songs of `musicdb banned --json`, newest ban first.
pub fn parse(text: &str) -> Result<Vec<BannedSong>, String> {
    let file: BannedFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if file.version != BANNED_VERSION {
        return Err(format!(
            "musicdb banned version {} (this rormpc reads {BANNED_VERSION}): update rormpc and rormpc-tools together",
            file.version
        ));
    }
    Ok(file.songs)
}

/// What a ban of these titles is called in the status bar and the undo line: "Africa", "Africa, Rosanna, +2".
fn what(titles: &[String]) -> String {
    match titles {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a}, {b}"),
        [a, b, rest @ ..] => format!("{a}, {b}, +{}", rest.len()),
    }
}

/// The queue entries `ids` were taken out by hand: an Up next request only leaves Up next (an added song leaves
/// the queue with it, a source song stays), every other entry leaves MPD's queue and its song is banned.
pub fn remove(ctx: &Ctx, ids: Vec<u32>) {
    let waiting = rormpc_upnext::up_next_ids();
    let (requests, others): (Vec<u32>, Vec<u32>) = ids.into_iter().partition(|id| waiting.contains(id));
    for id in requests {
        rormpc_upnext::remove(ctx, id);
    }
    if others.is_empty() {
        return;
    }
    let (mut files, mut titles) = (Vec::new(), Vec::new());
    for song in ctx.queue.iter().filter(|s| others.contains(&s.id)) {
        if !files.contains(&song.file) {
            files.push(song.file.clone());
            titles.push(song.metadata.get("title").map_or(song.file.as_str(), |t| t.last()).to_owned());
        }
    }
    ctx.command(move |_, client| {
        for id in others {
            client.delete_id(id)?;
        }
        Ok(())
    });
    if !files.is_empty() {
        ban_in_background(ctx, Ban { files, what: what(&titles) });
    }
}

/// `musicdb ban FILE...` in the background; once it banned them, the status bar says so and the ban becomes the
/// newest entry of Music's undo timeline.
fn ban_in_background(ctx: &Ctx, ban: Ban) {
    let sender = ctx.app_event_sender.clone();
    std::thread::spawn(move || {
        let mut args = vec!["ban"];
        args.extend(ban.files.iter().map(String::as_str));
        match run(&args) {
            Ok(out) if out.trim_start().starts_with("Banned") => {
                status_info!("Banned {}: {} · z undoes", ban.files.len(), ban.what);
                rormpc_play::request_step(Step::Banned(ban));
            }
            Ok(out) => status_info!("Removed from the queue · {}", out.trim()), // "Already banned: …"
            Err(err) => status_error!("Removed from the queue, but not banned: musicdb: {err}"),
        }
        let _ = sender.send(AppEvent::RequestRender);
    });
}

/// `z` / `Z` on a ban, or the Banned panel: `musicdb unban FILE...` (`unban`) or `musicdb ban FILE...` in the
/// background; `then` runs once it succeeded (Music reruns its filters, so the queue follows).
pub fn set_banned(ctx: &Ctx, files: Vec<String>, ban: bool, then: impl FnOnce() + Send + 'static) {
    let sender = ctx.app_event_sender.clone();
    std::thread::spawn(move || {
        let mut args = vec![if ban { "ban" } else { "unban" }];
        args.extend(files.iter().map(String::as_str));
        match run(&args) {
            Ok(out) => {
                status_info!("{}", out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("done"));
                then();
            }
            Err(err) => status_error!("musicdb: {err}"),
        }
        let _ = sender.send(AppEvent::RequestRender);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_banned_list_is_read_with_its_version() {
        let text = r#"{"version": 1, "songs": [{"key": "yt:abc", "file": "a/Toto - Africa.mp3", "artist": "Toto",
            "title": "Africa", "ts": "2026-10-11T14:02:11"}]}"#;
        let songs = parse(text).expect("a banned list");
        assert_eq!(songs.len(), 1);
        assert_eq!((songs[0].file.as_str(), songs[0].title.as_str()), ("a/Toto - Africa.mp3", "Africa"));
        let newer = parse(r#"{"version": 2, "songs": []}"#).expect_err("a newer format");
        assert!(newer.contains("update rormpc and rormpc-tools together"), "{newer}");
    }

    #[test]
    fn a_ban_is_named_by_its_titles() {
        let t = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(what(&t(&["Africa"])), "Africa");
        assert_eq!(what(&t(&["Africa", "Rosanna"])), "Africa, Rosanna");
        assert_eq!(what(&t(&["Africa", "Rosanna", "Hold the Line", "99"])), "Africa, Rosanna, +2");
    }
}
