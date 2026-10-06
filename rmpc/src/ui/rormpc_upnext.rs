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

use rmpc_mpd::{commands::status::OnOffOneshot, mpd_client::MpdClient, queue_position::QueuePosition};
use serde::{Deserialize, Serialize};

use crate::{
    ctx::Ctx,
    shared::macros::{modal, status_error, status_info, status_warn},
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
    /// "library" or "playlist"
    kind: String,
    name: String,
    /// songs the source put in the queue (without the added Up next songs)
    len: usize,
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
}

/// mpd-player's upnext.json, read again only when the file changed (it is looked at on every render).
fn upnext_file() -> UpNextFile {
    static CACHE: OnceLock<Mutex<(Option<SystemTime>, UpNextFile)>> = OnceLock::new();
    let p = rormpc_player::state_path("upnext");
    let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
    let cache = CACHE.get_or_init(|| Mutex::new((None, UpNextFile::default())));
    let Ok(mut c) = cache.lock() else { return UpNextFile::default() };
    if c.0 != mtime || mtime.is_none() {
        c.1 = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        c.0 = mtime;
    }
    c.1.clone()
}

/// The songs waiting in Up next, in play order.
pub fn waiting(_ctx: &Ctx) -> Vec<Waiting> {
    upnext_file().entries
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
        let name = if src.kind == "library" { "Whole library".to_owned() } else { src.name.clone() };
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
                    if n == 0 {
                        status_warn!("{name} is empty");
                    } else {
                        confirm_replace(ctx, "playlist".into(), name.clone());
                    }
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
    let what = if kind == "library" { "the whole library".to_owned() } else { format!("playlist {name}") };
    let waiting = upnext_file().entries;
    let up = waiting.len();
    let message = vec![format!(
        "Play {what} now?\n\nThe queue is replaced by it and playback starts (the current song is cut).{}",
        if up > 0 { format!("\nUp next ({up}) is kept and plays first.") } else { String::new() }
    )];
    let go = move |ctx: &Ctx| -> anyhow::Result<()> {
        let (kind, name) = (kind.clone(), name.clone());
        let first = waiting.first().map(|e| e.file.clone());
        ctx.command(move |_, client| {
            client.clear()?;
            if kind == "library" {
                client.add("/", None)?;
            } else {
                client.load_playlist(&name, None)?;
            }
            let len = client.playlist_info()?.map_or(0, |q| q.len());
            let st = State { source: Some(Source { kind, name, len }) };
            if let Ok(mut g) = state().lock() {
                *g = st.clone();
            }
            save(&st);
            // mpd-player keeps Up next across the replaced queue (same files, new ids); its first song plays first
            match first {
                Some(f) if client.channels()?.0.iter().any(|c| c == rormpc_player::CHANNEL) => {
                    client.send_message(rormpc_player::CHANNEL, &format!("upnext playnow {f}"))?;
                }
                _ => client.play()?,
            }
            status_info!("Playing from {}", if st.source.as_ref().is_some_and(|s| s.kind == "library") { "the whole library" } else { "the playlist" });
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
