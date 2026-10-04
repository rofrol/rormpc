//! rormpc: "Playing from" a source, plus a short "Up next" list that plays before the rest (TODO "Source + Up
//! next", milestone 1). The whole queue stays in MPD, so phones, media keys and mpc keep working; rormpc only
//! remembers what it did, in `$XDG_STATE_HOME/rormpc/source.json`:
//!
//! - the source: the whole library or a saved playlist, put in the queue by "Sources…" (it replaces the queue,
//!   explicitly, and keeps Up next). When the queue no longer has the source's length, the header says
//!   "modified": another client changed it, and nothing is rebuilt behind its back.
//! - Up next: songs asked for with "Play next". With random on they get MPD priorities (distinct, decreasing, so
//!   the first asked plays first; MPD resets a song's priority when it starts); with random off they are moved
//!   right after the current song, in order. A song that was not in the queue is added, and deleted again after
//!   it has played, so the source stays as it was; a song from the source keeps its place. Consume must be off.
//!
//! MPD song ids do not survive an MPD restart: entries whose id is gone are dropped.

use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use rmpc_mpd::{
    commands::status::OnOffOneshot,
    mpd_client::MpdClient,
    proto_client::ProtoClient,
    queue_position::QueuePosition,
};
use serde::{Deserialize, Serialize};

use crate::{
    ctx::Ctx,
    shared::macros::{modal, status_error, status_info, status_warn},
    ui::modals::{
        confirm_modal::{Action, ConfirmModal},
        menu::modal::MenuModal,
    },
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    source: Option<Source>,
    #[serde(default)]
    up_next: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Source {
    /// "library" or "playlist"
    kind: String,
    name: String,
    /// songs the source put in the queue (without the added Up next songs)
    len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    id: u32,
    file: String,
    /// added to the queue for Up next (deleted after it played), not part of the source
    added: bool,
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

/// Queue ids that are waiting in Up next, in play order (for the badge).
pub fn up_next_ids() -> Vec<u32> {
    state().lock().map(|s| s.up_next.iter().map(|e| e.id).collect()).unwrap_or_default()
}

/// "Playing from: Hits 1980s top100 · Up next 2", or None before any source was chosen.
pub fn header(ctx: &Ctx) -> Option<String> {
    let s = state().lock().ok()?;
    let up = s.up_next.len();
    let src = s.source.as_ref().map(|src| {
        let added = s.up_next.iter().filter(|e| e.added).count();
        let modified = ctx.queue.len().saturating_sub(added) != src.len;
        let name = if src.kind == "library" { "Whole library".to_owned() } else { src.name.clone() };
        format!("Playing from: {name}{}", if modified { " (modified)" } else { "" })
    });
    match (src, up) {
        (None, 0) => None,
        (None, n) => Some(format!(" Up next {n} ")),
        (Some(src), 0) => Some(format!(" {src} ")),
        (Some(src), n) => Some(format!(" {src} · Up next {n} ")),
    }
}

/// Put songs into Up next (marked rows, else the cursor row), after the ones already waiting.
pub fn play_next(ctx: &Ctx, files: Vec<String>) {
    if files.is_empty() {
        return;
    }
    if !matches!(ctx.status.consume, OnOffOneshot::Off) {
        return status_warn!("Up next needs consume off (consume would delete the source as it plays)");
    }
    ctx.command(move |_, client| {
        let random = client.get_status()?.random;
        let queue = client.playlist_info()?.unwrap_or_default();
        let current = client.get_status()?.songid;
        let mut st = state().lock().map_err(|_| anyhow::anyhow!("Up next state poisoned"))?.clone();
        st.up_next.retain(|e| queue.iter().any(|s| s.id == e.id) && Some(e.id) != current);
        let mut added_now = 0;
        for file in files {
            if st.up_next.iter().any(|e| e.file == file) {
                continue; // already waiting
            }
            let existing = queue.iter().find(|s| s.file == file && Some(s.id) != current).map(|s| s.id);
            let (id, added) = match existing {
                Some(id) => (id, false),
                None => {
                    client.add(&file, None)?;
                    let q = client.playlist_info()?.unwrap_or_default();
                    let Some(song) = q.iter().rev().find(|s| s.file == file) else { continue };
                    (song.id, true)
                }
            };
            st.up_next.push(Entry { id, file, added });
            added_now += 1;
        }
        apply_order(client, &st, random)?;
        let n = st.up_next.len();
        if let Ok(mut g) = state().lock() {
            *g = st.clone();
        }
        save(&st);
        status_info!("Up next: {added_now} added, {n} waiting");
        Ok(())
    });
}

/// Random on: priorities 255, 254, … in Up next order. Random off: the entries right after the current song.
fn apply_order(client: &mut impl ClientLike, st: &State, random: bool) -> anyhow::Result<()> {
    for (k, e) in st.up_next.iter().enumerate() {
        if random {
            client.prio_id(255u32.saturating_sub(k as u32).max(1), e.id)?;
        } else {
            client.move_id(e.id, QueuePosition::RelativeAdd(k))?;
        }
    }
    Ok(())
}

/// The few client calls this module needs, so `prioid` (not in rmpc-mpd) is one raw command.
trait ClientLike {
    fn prio_id(&mut self, prio: u32, id: u32) -> anyhow::Result<()>;
    fn move_id(&mut self, id: u32, to: QueuePosition) -> anyhow::Result<()>;
}

impl<T: MpdClient + ProtoClient> ClientLike for T {
    fn prio_id(&mut self, prio: u32, id: u32) -> anyhow::Result<()> {
        self.execute(&format!("prioid {prio} {id}"))?;
        self.read_ok()?;
        Ok(())
    }

    fn move_id(&mut self, id: u32, to: QueuePosition) -> anyhow::Result<()> {
        MpdClient::move_id(self, id, to)?;
        Ok(())
    }
}

/// Called when the playing song changes: the previous song, if it was an Up next entry, has played; one that
/// was added only for Up next leaves the queue again.
pub fn song_changed(ctx: &Ctx, previous: Option<u32>) {
    let Some(prev) = previous else { return };
    let Ok(mut st) = state().lock() else { return };
    let Some(pos) = st.up_next.iter().position(|e| e.id == prev) else { return };
    let entry = st.up_next.remove(pos);
    save(&st);
    drop(st);
    if entry.added {
        ctx.command(move |_, client| {
            // re-check: another client may have removed or reused it
            if client.playlist_id(entry.id).ok().flatten().is_some_and(|s| s.file == entry.file) {
                client.delete_id(entry.id)?;
            }
            Ok(())
        });
    }
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
    let up = state().lock().map(|s| s.up_next.len()).unwrap_or(0);
    let message = vec![format!(
        "Play {what} now?\n\nThe queue is replaced by it and playback starts (the current song is cut).{}",
        if up > 0 { format!("\nUp next ({up}) is kept and plays first.") } else { String::new() }
    )];
    let go = move |ctx: &Ctx| -> anyhow::Result<()> {
        let (kind, name) = (kind.clone(), name.clone());
        ctx.command(move |_, client| {
            let random = client.get_status()?.random;
            let old = state().lock().map(|s| s.clone()).unwrap_or_default();
            client.clear()?;
            if kind == "library" {
                client.add("/", None)?;
            } else {
                client.load_playlist(&name, None)?;
            }
            let len = client.playlist_info()?.map_or(0, |q| q.len());
            let mut st = State { source: Some(Source { kind, name, len }), up_next: Vec::new() };
            // Up next survives: same files, new ids (the clear dropped the old ones)
            let queue = client.playlist_info()?.unwrap_or_default();
            for e in old.up_next {
                match queue.iter().find(|s| s.file == e.file) {
                    Some(s) => st.up_next.push(Entry { id: s.id, file: e.file, added: false }),
                    None => {
                        client.add(&e.file, None)?;
                        if let Some(s) = client.playlist_info()?.unwrap_or_default().iter().rev().find(|s| s.file == e.file) {
                            st.up_next.push(Entry { id: s.id, file: e.file, added: true });
                        }
                    }
                }
            }
            apply_order(client, &st, random)?;
            match st.up_next.first() {
                Some(e) => client.play_id(e.id)?,
                None => client.play()?,
            }
            if let Ok(mut g) = state().lock() {
                *g = st.clone();
            }
            save(&st);
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
