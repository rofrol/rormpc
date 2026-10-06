//! rormpc: "Playing from" a source, plus a short "Up next" list that plays before the rest (TODO "Source + Up
//! next"). The whole queue stays in MPD, so phones, media keys and mpc keep working.
//!
//! - the source: the whole library or a saved playlist, put in the queue by "Sources…" (it replaces the queue,
//!   explicitly, and keeps Up next). rormpc remembers it in `$XDG_STATE_HOME/rormpc/source.json`; when the queue no
//!   longer has the source's length, the header says "modified": another client changed it, and nothing is
//!   rebuilt behind its back.
//! - Up next belongs to mpd-player (rormpc-tools), so it works with rormpc closed: rormpc sends it commands over
//!   MPD messages (`upnext add FILE`, see rormpc_player) and shows its `upnext.json`. Without mpd-player, Enter
//!   still plays a song (added after the current one, not removed afterwards) and Play next reports it.

use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

use anyhow::Result;
use crossbeam::channel::Sender;
use notify_debouncer_full::notify::{self, RecommendedWatcher, RecursiveMode, Watcher};
use rmpc_mpd::{
    commands::status::OnOffOneshot,
    mpd_client::MpdClient,
    proto_client::ProtoClient,
    queue_position::QueuePosition,
};
use serde::{Deserialize, Serialize};

use crate::{
    ctx::Ctx,
    shared::{events::AppEvent, macros::{modal, status_error, status_info, status_warn}},
    ui::{
        modals::{
            confirm_modal::{Action, ConfirmModal},
            menu::modal::MenuModal,
        },
        rormpc_player,
    },
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    source: Option<Source>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Source {
    /// "library", "playlist" or "hits" (a Hits result, played as a snapshot of its owned songs)
    kind: String,
    name: String,
    /// songs the source put in the queue (without the added Up next songs)
    len: usize,
    /// a Hits snapshot: its files in ranking order (mpd-player plays it in rounds)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    files: Vec<String>,
}

/// The source being played: (kind, name, snapshot files for a Hits source).
pub fn source_info() -> Option<(String, String, Vec<String>)> {
    let s = state().lock().ok()?;
    s.source.as_ref().map(|src| (src.kind.clone(), src.name.clone(), src.files.clone()))
}

/// Hits "Play these results": the owned rows replace the queue as the source, after a confirmation.
pub fn play_hits_source(ctx: &Ctx, name: String, files: Vec<String>) {
    confirm_replace_with(ctx, "hits".into(), name, files);
}

fn path() -> PathBuf {
    let base = std::env::var("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state")
    });
    base.join("rormpc/source.json")
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default())
    })
}

fn save(s: &State) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("tmp");
    if let Ok(text) = serde_json::to_string_pretty(s)
        && std::fs::write(&tmp, text).is_ok()
    {
        let _ = std::fs::rename(&tmp, &p); // atomic: a crash never leaves half a file
    }
}

/// One Up next entry from mpd-player's upnext.json.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Waiting {
    pub id: u32,
    pub file: String,
    /// added to the queue only for Up next (leaves it after playing)
    pub added: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct UpNextFile {
    #[serde(default)]
    entries: Vec<Waiting>,
    #[serde(default)]
    playing: Option<Waiting>,
    #[serde(default)]
    error: Option<String>,
}

static UP_NEXT_CACHE: OnceLock<Mutex<(Option<SystemTime>, UpNextFile)>> = OnceLock::new();

/// Watch the directory, not the replaced inode: errors must redraw even while playback is paused or stopped.
pub fn watch(tx: Sender<AppEvent>) -> Result<RecommendedWatcher> {
    watch_path(rormpc_player::state_path("upnext"), tx)
}

fn watch_path(path: PathBuf, tx: Sender<AppEvent>) -> Result<RecommendedWatcher> {
    let parent = path.parent().ok_or_else(|| anyhow::anyhow!("Up next state has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    // notify reports canonical paths (on macOS /tmp is /private/tmp); compare in the same namespace.
    let parent = std::fs::canonicalize(parent)?;
    let target = parent.join(path.file_name().ok_or_else(|| anyhow::anyhow!("Up next state has no filename"))?);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        match event {
            Ok(event) if event.need_rescan() || event.paths.is_empty()
                || event.paths.iter().any(|p| p == &target) => {
                // An atomic replacement can have the same mtime: the event, not a timestamp, invalidates it.
                if let Some(cache) = UP_NEXT_CACHE.get() && let Ok(mut cache) = cache.lock() {
                    cache.0 = None;
                }
                let _ = tx.send(AppEvent::RequestRender);
            }
            Ok(_) => {}
            Err(err) => log::warn!(error:? = err; "Up next state watcher failed"),
        }
    })?;
    watcher.watch(&parent, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

/// mpd-player's upnext.json, cached until its mtime changes or its watcher invalidates it.
fn upnext_file() -> UpNextFile {
    let p = rormpc_player::state_path("upnext");
    let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
    let cache = UP_NEXT_CACHE.get_or_init(|| Mutex::new((None, UpNextFile::default())));
    let Ok(mut c) = cache.lock() else { return UpNextFile::default() };
    if c.0 != mtime || mtime.is_none() {
        c.1 = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        c.0 = mtime;
    }
    c.1.clone()
}

/// The songs waiting in Up next, in play order (also used by the Shuffle timeline).
pub fn waiting(_ctx: &Ctx) -> Vec<Waiting> {
    upnext_file().entries
}

/// One coherent snapshot of the waiting list and its command error.
pub fn waiting_and_error(_ctx: &Ctx) -> (Vec<Waiting>, Option<String>) {
    let state = upnext_file();
    (state.entries, state.error)
}

/// Queue ids that are waiting in Up next, in play order (for the badge).
pub fn up_next_ids() -> Vec<u32> {
    upnext_file().entries.iter().map(|e| e.id).collect()
}

/// "Playing from: Hits 1980s top100 · Up next 2", or None before any source was chosen.
pub fn header(ctx: &Ctx) -> Option<String> {
    let s = state().lock().ok()?;
    let up = upnext_file();
    let n = up.entries.len();
    let src = s.source.as_ref().map(|src| {
        let added = up.entries.iter().chain(up.playing.iter()).filter(|e| e.added).count();
        let modified = ctx.queue.len().saturating_sub(added) != src.len;
        let name = match src.kind.as_str() {
            "library" => "Whole library".to_owned(),
            "hits" => format!("Hits · {}", src.name),
            _ => src.name.clone(),
        };
        format!("Playing from: {name}{}", if modified { " (modified)" } else { "" })
    });
    match (src, n) {
        (None, 0) => None,
        (None, n) => Some(format!(" Up next {n} ")),
        (Some(src), 0) => Some(format!(" {src} ")),
        (Some(src), n) => Some(format!(" {src} · Up next {n} ")),
    }
}

/// Send Up next commands to mpd-player, in order; say so when it is not running.
fn send(ctx: &Ctx, msgs: Vec<String>) {
    ctx.command(move |_, client| {
        if !client.channels()?.0.iter().any(|c| c == rormpc_player::CHANNEL) {
            status_error!("Up next needs mpd-player, which is not running (rormpc_install.sh companions starts it)");
            return Ok(());
        }
        for m in &msgs {
            client.send_message(rormpc_player::CHANNEL, m)?;
        }
        Ok(())
    });
}

/// Put songs into Up next (marked rows, else the cursor row), after the ones already waiting; a song already
/// waiting moves to the top.
pub fn play_next(ctx: &Ctx, files: Vec<String>) {
    if files.is_empty() {
        return;
    }
    if !matches!(ctx.status.consume, OnOffOneshot::Off) {
        return status_warn!("Up next needs consume off (consume would delete the source as it plays). c turns it off");
    }
    let waiting = up_next_ids().len();
    let n = files.len();
    let already = files.iter().filter(|f| upnext_file().entries.iter().any(|e| &e.file == *f)).count();
    send(ctx, files.into_iter().map(|f| format!("upnext add {f}")).collect());
    if already == n {
        status_info!("Already in Up next: moved to next");
    } else {
        status_info!("Up next: {} added, {} waiting", n - already, waiting + n - already);
    }
}

/// Enter on a song in a browser pane: play it now without touching the rest of the queue. A song already in the
/// queue plays from its entry; another one is added right after the current song, played, and leaves the queue
/// again after it played (mpd-player), so the source stays as it was.
pub fn play_now(ctx: &Ctx, file: String) {
    if crate::ui::rormpc_actions::use_existing_entry(ctx, &file, true) {
        return;
    }
    let after_current = ctx.current_song().is_some();
    ctx.command(move |_, client| {
        if client.channels()?.0.iter().any(|c| c == rormpc_player::CHANNEL) {
            client.send_message(rormpc_player::CHANNEL, &format!("upnext playnow {file}"))?;
            return Ok(());
        }
        // no mpd-player: play it anyway, it just stays in the queue
        client.add(&file, after_current.then_some(QueuePosition::RelativeAdd(0)))?;
        let queue = client.playlist_info()?.unwrap_or_default();
        if let Some(id) = queue.iter().rev().find(|s| s.file == file).map(|s| s.id) {
            client.play_id(id)?;
        }
        status_warn!("mpd-player is not running: the song stays in the queue after it played");
        Ok(())
    });
}

/// Move the waiting entry `id` by `delta` places (negative: earlier).
pub fn move_entry(ctx: &Ctx, id: u32, delta: isize) {
    send(ctx, vec![format!("upnext move {id} {delta}")]);
}

/// Make the waiting entry `id` the next one.
pub fn make_next(ctx: &Ctx, id: u32) {
    send(ctx, vec![format!("upnext first {id}")]);
}

/// Drop the waiting entry `id` from Up next.
pub fn remove(ctx: &Ctx, id: u32) {
    send(ctx, vec![format!("upnext remove {id}")]);
}

/// Drop every waiting entry.
pub fn clear(ctx: &Ctx) {
    send(ctx, vec!["upnext clear".to_owned()]);
}

/// Play the waiting entry `id` now.
pub fn play_entry(ctx: &Ctx, id: u32) {
    send(ctx, vec![format!("upnext play {id}")]);
}

/// "Sources…": the whole library or a saved playlist, played now (it replaces the queue; Up next stays).
pub fn open_sources(ctx: &Ctx) {
    let lookup = ctx.query_sync(|client| {
        let mut out = Vec::new();
        for pl in client.list_playlists()? {
            let n = client.list_playlist(&pl.name)?.0.len();
            out.push((pl.name, n));
        }
        Ok(out)
    });
    let mut playlists = match lookup {
        Ok(p) => p,
        Err(err) => return status_error!("Cannot read the playlists: {err}"),
    };
    playlists.retain(|(_, n)| *n > 0); // an empty one (e.g. "Not finished" with nothing left) is no source
    playlists.sort_by_key(|(name, _)| name.to_lowercase());
    let current = state().lock().ok().and_then(|s| s.source.clone());
    let mark = move |kind: &str, name: &str| {
        current.as_ref().is_some_and(|c| c.kind == kind && (kind == "library" || c.name == name))
    };
    let library_mark = if mark("library", "") { "▶" } else { " " };
    let marks: Vec<bool> = playlists.iter().map(|(n, _)| mark("playlist", n)).collect();
    let menu = MenuModal::new(ctx)
        .width(60)
        .list_section(ctx, move |mut section| {
            section.add_item(format!("{library_mark} Whole library"), |ctx| {
                confirm_replace(ctx, "library".into(), String::new());
                Ok(())
            });
            for ((name, n), on) in playlists.into_iter().zip(marks) {
                let label = format!("{} {name}  ({n})", if on { "▶" } else { " " });
                section.add_item(label, move |ctx| {
                    confirm_replace(ctx, "playlist".into(), name.clone());
                    Ok(())
                });
            }
            Some(section)
        })
        .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
        .build();
    modal!(ctx, menu);
}

fn confirm_replace(ctx: &Ctx, kind: String, name: String) {
    confirm_replace_with(ctx, kind, name, Vec::new());
}

fn confirm_replace_with(ctx: &Ctx, kind: String, name: String, files: Vec<String>) {
    let what = match kind.as_str() {
        "library" => "the whole library".to_owned(),
        "hits" => format!("{} songs of {name} (shuffled in rounds: each once)", files.len()),
        _ => format!("playlist {name}"),
    };
    let waiting = upnext_file().entries;
    let up = waiting.len();
    let message = vec![format!(
        "Play {what}?\n\nThe queue is replaced by it; the song playing now goes on, then the new source plays.{}",
        if up > 0 { format!("\nUp next ({up}) is kept and plays first.") } else { String::new() }
    )];
    let go = move |ctx: &Ctx| -> anyhow::Result<()> {
        let (kind, name, files) = (kind.clone(), name.clone(), files.clone());
        ctx.command(move |_, client| {
            let started = std::time::Instant::now();
            let status = client.get_status()?;
            let current = status.songid;
            // everything but the playing song leaves (two range deletes around it); Up next follows its files
            // (mpd-player re-finds the requests in the new queue)
            match status.song {
                Some(pos) if current.is_some() => {
                    let len = client.playlist_info()?.map_or(0, |q| q.len());
                    if pos + 1 < len {
                        client.execute(&format!("delete {}:", pos + 1))?;
                        client.read_ok()?;
                    }
                    if pos > 0 {
                        client.execute(&format!("delete 0:{pos}"))?;
                        client.read_ok()?;
                    }
                }
                _ => client.clear()?,
            }
            match kind.as_str() {
                "library" => client.add("/", None)?,
                "hits" => {
                    for f in &files {
                        client.add(f, None)?;
                    }
                }
                _ => client.load_playlist(&name, None)?,
            }
            // the playing song was kept: its copy from the new source would make it play twice
            let queue = client.playlist_info()?.unwrap_or_default();
            if let Some(cur) = current.and_then(|id| queue.iter().find(|s| s.id == id)).map(|s| s.file.clone()) {
                for s in queue.iter().filter(|s| s.file == cur && Some(s.id) != current) {
                    client.delete_id(s.id)?;
                }
            }
            let len = client.playlist_info()?.map_or(0, |q| q.len());
            let st = State { source: Some(Source { kind, name, len, files }) };
            if let Ok(mut g) = state().lock() {
                *g = st.clone();
            }
            save(&st);
            if current.is_none() {
                client.play()?; // nothing was playing: start (Up next and the shuffle's plan come first)
            }
            status_info!(
                "Playing from {} · {len} songs, prepared in {:.1} s",
                match st.source.as_ref().map(|s| s.kind.as_str()) {
                    Some("library") => "the whole library",
                    Some("hits") => "the Hits result",
                    _ => "the playlist",
                },
                started.elapsed().as_secs_f64()
            );
            Ok(())
        });
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

    #[test]
    fn error_snapshot_keeps_waiting_entries_and_accepts_older_json() {
        let state: UpNextFile = serde_json::from_str(r#"{
            "entries": [{"id": 17, "file": "song.flac", "added": true}],
            "playing": null, "error": "Cannot play song.flac: No such song"
        }"#).unwrap();
        assert_eq!(state.entries[0].id, 17);
        assert!(state.error.unwrap().contains("Cannot play song.flac"));
        let old: UpNextFile = serde_json::from_str(r#"{"entries": [], "playing": null}"#).unwrap();
        assert!(old.error.is_none());
    }

    #[test]
    fn atomic_state_replacements_wake_ui_without_playback_events() {
        let dir = std::env::temp_dir().join(format!("rormpc-upnext-watch-{}-{}",
            std::process::id(), SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()));
        let path = dir.join("upnext.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "old inode").unwrap();
        let (tx, rx) = crossbeam::channel::unbounded();
        let watcher = watch_path(path.clone(), tx).unwrap();
        let tmp = dir.join(".upnext.json.tmp");
        std::fs::write(&tmp, "replacement publication").unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        // External test deadline for OS/FSEvents delivery, not a delay or retry in the UI.
        assert!(matches!(rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap(), AppEvent::RequestRender));
        drop(watcher);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
