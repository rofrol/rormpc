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
    collections::{HashMap, HashSet},
    fmt::Write as _,
    path::PathBuf,
    sync::{
        Arc,
        Mutex,
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

use anyhow::Result;
use crossbeam::channel::Sender;
use notify_debouncer_full::notify::{self, RecommendedWatcher, RecursiveMode, Watcher};
use rmpc_mpd::{
    commands::status::OnOffOneshot,
    errors::MpdError,
    mpd_client::{MpdClient, MpdCommand as _},
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
        rormpc_undo::Departed,
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
    /// Play's Apply: the `args` of the `hits` result it played (loaded back into Play's filters) and their
    /// canonical hash, mpd-player's round key (the same rules keep the round, other rules start a new one)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rules: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rules_hash: Option<String>,
    /// songs appended by hand to a Hits source (Play's Browse `a`): they join its round in mpd-player and the
    /// source line says "+N added"; the next Apply drops them like any hand edit
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    added: Vec<String>,
    /// a filter source's identity, mpd-player's round key: a new one when the queue starts holding a filter
    /// source, kept by every later filter change (plans/instant-filtering.md, "Rounds belong to the source")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_id: Option<String>,
    /// how many generations of this filter source reached MPD (1 for the first), counted across rormpc runs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generation: Option<u64>,
    /// queue entries a filter source put there that its newest rules left out but a reconcile could not delete
    /// (the playing song, an Up next request): not members, but the next reconcile may still remove them
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    held: Vec<String>,
}

impl Source {
    /// The identity and generation a new filter source of `kind` takes after `previous`: a filter source going
    /// on keeps its id and counts one more generation, anything else starts a new id at generation 1.
    fn next_identity(kind: &str, previous: Option<&Source>) -> (Option<String>, Option<u64>) {
        if kind != "hits" {
            return (None, None);
        }
        match previous {
            Some(p) if p.kind == "hits" && p.source_id.is_some() => {
                (p.source_id.clone(), Some(p.generation.unwrap_or(0) + 1))
            }
            _ => (Some(new_source_id()), Some(1)),
        }
    }

    /// Count `appended` (just added to the end of the queue) into the source: its length follows them so the
    /// header does not call it modified, and on a Hits source the files it does not hold join it (`added`).
    /// Returns how many files joined.
    fn append(&mut self, appended: &[String]) -> usize {
        self.len += appended.len();
        if self.kind != "hits" {
            return 0;
        }
        let mut joined = 0;
        for f in appended {
            if !self.files.contains(f) && !self.added.contains(f) {
                self.added.push(f.clone());
                joined += 1;
            }
        }
        joined
    }
}

/// A random UUID (version 4) from the system's random device; the clock and pid when it cannot be read.
fn new_source_id() -> String {
    use std::io::Read as _;
    let mut b = [0u8; 16];
    let read = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    if read.is_err() {
        let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        b = (nanos ^ (u128::from(std::process::id()) << 96)).to_le_bytes();
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex = b.iter().fold(String::with_capacity(32), |mut hex, x| {
        let _ = write!(hex, "{x:02x}");
        hex
    });
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// The source being played: (kind, name, snapshot files for a Hits source).
pub fn source_info() -> Option<(String, String, Vec<String>)> {
    let s = state().lock().ok()?;
    s.source.as_ref().map(|src| (src.kind.clone(), src.name.clone(), src.files.clone()))
}

/// The rules Play applied for the source being played, with their hash (None for any other source).
pub fn source_rules() -> Option<(serde_json::Value, String)> {
    let s = state().lock().ok()?;
    let src = s.source.as_ref()?;
    Some((src.rules.clone()?, src.rules_hash.clone()?))
}

/// One generation of Music's filters: the `hits` result's owned files in rank order, with its rules.
#[derive(Debug, Clone)]
pub struct HitsSource {
    pub name: String,
    pub files: Vec<String>,
    pub rules: serde_json::Value,
    pub rules_hash: String,
}

/// The generation counters of Music's filters: `latest` is the newest intent, `reached` the newest generation
/// whose queue writes started on the MPD command thread (the undo history's entries are the ones that did).
#[derive(Debug, Clone, Default)]
pub struct Generations {
    pub latest: Arc<AtomicU64>,
    pub reached: Arc<AtomicU64>,
}

impl Generations {
    /// Whether `generation` is still the newest: then it counts as reached and its writes go ahead.
    fn start(&self, generation: u64) -> bool {
        if self.latest.load(Ordering::SeqCst) != generation {
            return false; // a newer generation follows: it changes the queue from the state it is in then
        }
        self.reached.fetch_max(generation, Ordering::SeqCst);
        true
    }
}

/// Music's filters apply at once (plans/instant-filtering.md): generation `generation` reconciles the queue with
/// `src` on the MPD command thread, unless a newer generation was asked for by then (`gens.latest`): a backlog of
/// reconciles collapses to the newest, and one that has started finishes its command list. `in_order` (weighted
/// shuffle off) also moves the source's songs after the playing one into rank order.
pub fn reconcile_hits_source(ctx: &Ctx, src: HitsSource, generation: u64, gens: Generations, in_order: bool) {
    ctx.command(move |_, client| {
        if !gens.start(generation) {
            return Ok(());
        }
        reconcile(client, src, in_order)
    });
}

/// Whether the queue holds Music's filter source (else another kind of source, which a filter change replaces).
pub fn holds_filter_source() -> bool {
    state().lock().is_ok_and(|g| g.source.as_ref().is_some_and(|s| s.kind == "hits" && s.source_id.is_some()))
}

/// The source in the queue as undo brings it back: source.json's, with its files in order; a queue without one
/// (or a source that kept no files) is taken as it is, without the Up next requests.
pub fn departed(ctx: &Ctx) -> Departed {
    let src = state().lock().ok().and_then(|g| g.source.clone());
    let up_next = up_next_ids();
    let queue_files = || ctx.queue.iter().filter(|s| !up_next.contains(&s.id)).map(|s| s.file.clone()).collect();
    match src {
        Some(s) if matches!(s.kind.as_str(), "library" | "playlist") => Departed { kind: s.kind, name: s.name, files: Vec::new() },
        Some(s) if !s.files.is_empty() => Departed { kind: s.kind, name: s.name, files: s.files },
        Some(s) => Departed { kind: s.kind, name: s.name, files: queue_files() },
        None => Departed { kind: "selection".to_owned(), name: "the queue before the filters".to_owned(), files: queue_files() },
    }
}

/// Undo past the first filter touch: `d` comes back as the source, in its order around the playing song (the
/// songs before it in `d` go before it), Up next stays; generation-tagged like a reconcile.
pub fn restore_departed(ctx: &Ctx, d: Departed, generation: u64, gens: Generations) {
    ctx.command(move |_, client| {
        if !gens.start(generation) {
            return Ok(());
        }
        restore(client, d)
    });
}

/// What a restore does: every entry but the playing song and the Up next requests leaves; `files` come back
/// with the ones before the playing song's file (when it is one of them) in front of it, the rest at the end.
fn plan_restore(queue: &[(u32, &str)], playing: Option<u32>, up_next: &[u32], files: &[String]) -> Vec<Cmd> {
    let kept = |id: u32| Some(id) == playing || up_next.contains(&id);
    let mut cmds: Vec<Cmd> = queue.iter().filter(|(id, _)| !kept(*id)).map(|(id, _)| Cmd::Delete(*id)).collect();
    let playing_file = queue.iter().find(|(id, _)| Some(*id) == playing).map(|(_, f)| *f);
    let at = playing_file.and_then(|p| files.iter().position(|f| f == p));
    for (i, f) in files.iter().enumerate() {
        match at {
            Some(k) if i < k => cmds.push(Cmd::Add(f.clone(), Some(i))),
            Some(k) if i == k => {}
            _ => cmds.push(Cmd::Add(f.clone(), None)),
        }
    }
    cmds
}

/// One restore (see `plan_restore`): source.json first, then one command list; a file MPD no longer has is
/// left out and the rest planned again from the queue as it then is.
fn restore(client: &mut rmpc_mpd::client::Client<'_>, d: Departed) -> Result<()> {
    let Departed { kind, name, mut files } = d;
    match kind.as_str() {
        "playlist" => files = client.list_playlist(&name)?.0,
        "library" => files = vec!["/".to_owned()],
        _ => {}
    }
    let members = if kind == "library" { Vec::new() } else { files.clone() };
    let mut src = Source {
        kind,
        name,
        len: members.len(),
        files: members,
        rules: None,
        rules_hash: None,
        added: Vec::new(),
        source_id: None,
        generation: None,
        held: Vec::new(),
    };
    publish(Some(src.clone()));
    let mut gone = 0;
    loop {
        let status = client.get_status()?;
        let queue = client.playlist_info()?.unwrap_or_default();
        let entries: Vec<(u32, &str)> = queue.iter().map(|s| (s.id, s.file.as_str())).collect();
        let cmds = plan_restore(&entries, status.songid, &up_next_ids(), &files);
        if cmds.is_empty() {
            break;
        }
        client.send_start_cmd_list()?;
        for cmd in &cmds {
            match cmd {
                Cmd::Delete(id) => client.send_delete_id(*id)?,
                Cmd::Add(f, at) => client.send_add(f, at.map(QueuePosition::Absolute))?,
                Cmd::Move(id, to) => client.send_move_id(*id, QueuePosition::Absolute(*to))?,
            }
        }
        client.send_execute_cmd_list()?;
        match client.read_ok() {
            Ok(()) => break,
            Err(MpdError::Mpd(e)) if e.is_no_exist() => {
                let Some(Cmd::Add(f, _)) = cmds.get(e.command_list_index) else { return Err(MpdError::Mpd(e).into()) };
                let f = f.clone();
                files.retain(|t| *t != f);
                src.files.retain(|t| *t != f);
                gone += 1;
            }
            Err(err) => return Err(err.into()),
        }
    }
    let len = client.playlist_info()?.map_or(0, |q| q.len());
    let up = upnext_file();
    src.len = len.saturating_sub(up.entries.iter().chain(up.playing.iter()).filter(|e| e.added).count());
    let what = match src.kind.as_str() {
        "library" => "the whole library".to_owned(),
        "playlist" => format!("playlist {}", src.name),
        kind => format!("{} {}", kind_label(kind), src.name),
    };
    publish(Some(src));
    status_info!("Back to {what}{}", left_out_note(gone));
    Ok(())
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

/// Make `source` the one being played: in memory and in source.json (atomically). Music's filters left for
/// another kind of source: Previous sources remembers their last rules (`rormpc_smartlists::remember`).
fn publish(source: Option<Source>) {
    let st = State { source };
    let old = state().lock().ok().and_then(|mut g| std::mem::replace(&mut *g, st.clone()).source);
    save(&st);
    if let Some(old) = old.filter(|o| left_filters(o, st.source.as_ref()))
        && let (Some(rules), Some(hash)) = (&old.rules, &old.rules_hash)
    {
        crate::ui::rormpc_smartlists::remember(&old.name, rules, hash);
    }
}

/// Whether going from `old` to `new` leaves Music's filters for another kind of source (or none).
fn left_filters(old: &Source, new: Option<&Source>) -> bool {
    old.kind == "hits" && old.rules.is_some() && new.is_none_or(|n| n.kind != "hits")
}

/// Previous sources (decided by the user 2026-10-11): the filters' rule set is remembered once a song of it has
/// played. `file` started playing: when it is one of the filter source's songs, its rules and their hash (the
/// caller remembers them once per rule set).
pub fn played_rules(file: &str) -> Option<(String, serde_json::Value, String)> {
    let s = state().lock().ok()?;
    let src = s.source.as_ref().filter(|src| src.kind == "hits" && src.files.iter().any(|f| f == file))?;
    Some((src.name.clone(), src.rules.clone()?, src.rules_hash.clone()?))
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
    let shuffle_target = parent.join("shuffle.json");
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        match event {
            Ok(event) if event.need_rescan() || event.paths.is_empty()
                || event.paths.iter().any(|p| p == &target || p == &shuffle_target) => {
                // An atomic replacement can have the same mtime: the event, not a timestamp, invalidates it.
                if let Some(cache) = UP_NEXT_CACHE.get() && let Ok(mut cache) = cache.lock() {
                    *cache = (None, UpNextFile::default()); // a deleted file also has mtime None
                }
                rormpc_player::invalidate_shuffle_state();
                let _ = tx.send(AppEvent::RequestRender);
            }
            Ok(_) => {}
            Err(err) => log::warn!(error:? = err; "Up next state watcher failed"),
        }
    })?;
    watcher.watch(&parent, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

#[cfg(test)]
thread_local! {
    /// What `upnext_file()` returns in this test thread: (waiting entries, error). Default: nothing waiting.
    pub static TEST_UPNEXT: std::cell::RefCell<(Vec<Waiting>, Option<String>)> = std::cell::RefCell::default();
}

/// mpd-player's upnext.json, cached until its mtime changes or its watcher invalidates it.
fn upnext_file() -> UpNextFile {
    #[cfg(test)]
    return TEST_UPNEXT.with(|t| {
        let (entries, error) = t.borrow().clone();
        UpNextFile { entries, playing: None, error }
    });
    #[cfg(not(test))]
    upnext_file_from_disk()
}

#[cfg(not(test))]
fn upnext_file_from_disk() -> UpNextFile {
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
            "album" | "artist" | "directory" | "selection" => format!("{} {}", kind_label(&src.kind), src.name),
            _ => src.name.clone(),
        };
        let added = if src.added.is_empty() { String::new() } else { format!(" +{} added", src.added.len()) };
        format!("Playing from: {name}{added}{}", if modified { " (modified)" } else { "" })
    });
    match (src, n) {
        (None, 0) => None,
        (None, n) => Some(format!(" Up next {n} ")),
        (Some(src), 0) => Some(format!(" {src} ")),
        (Some(src), n) => Some(format!(" {src} · Up next {n} ")),
    }
}

/// How a source kind is named in the header and the confirmations.
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "library" => "the whole library",
        "hits" => "Hits",
        "playlist" => "playlist",
        "album" => "album",
        "artist" => "artist",
        "directory" => "folder",
        "selection" => "selection",
        _ => "source",
    }
}

/// Songs just appended to the end of the queue by hand (Play's Browse `a`/`A`): source.json counts them, and a
/// Hits source takes them into its files and round ("+N added"). Returns how many joined a Hits source.
pub fn note_appended(files: &[String]) -> usize {
    if files.is_empty() {
        return 0;
    }
    let Ok(mut g) = state().lock() else { return 0 };
    let Some(src) = g.source.as_mut() else { return 0 };
    let joined = src.append(files);
    let st = g.clone();
    drop(g);
    save(&st);
    joined
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

/// "Clear Up next (N)…": every waiting entry goes, after a confirmation.
pub fn confirm_clear(ctx: &Ctx, n: usize) {
    let message = vec![format!(
        "Clear Up next ({n})?\n\nSongs added only for Up next leave the queue; songs of the source stay where they are."
    )];
    modal!(
        ctx,
        ConfirmModal::builder()
            .ctx(ctx)
            .message(message)
            .action(Action::CustomButtons {
                buttons: vec![
                    ("Cancel", Box::new(|_: &Ctx| Ok(()))),
                    ("Clear", Box::new(|ctx: &Ctx| {
                        clear(ctx);
                        Ok(())
                    })),
                ],
            })
            .build()
    );
}

/// The menu of Music's "Up next · N" header row (Enter or the context menu on it).
pub fn open_block_menu(ctx: &Ctx, n: usize) {
    let menu = MenuModal::new(ctx)
        .list_section(ctx, move |mut section| {
            if n > 0 {
                section.add_item(format!("Clear Up next ({n})…"), move |ctx| {
                    confirm_clear(ctx, n);
                    Ok(())
                });
            }
            if rormpc_player::shuffle_state().round.is_some_and(|r| r.done) {
                section.add_item("New round (every song of the source once more)", |ctx| {
                    rormpc_player::new_round(ctx);
                    Ok(())
                });
            }
            Some(section)
        })
        .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
        .build();
    modal!(ctx, menu);
}

/// The label of Music's header row above the waiting requests.
pub fn block_label(n: usize) -> String {
    format!("Up next · {n}")
}

/// mpd-player's last rejected Up next command ("Cannot play …"), kept until its next explicit successful action.
pub fn error() -> Option<String> {
    upnext_file().error
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

/// A queue replacement: the source's songs replace everything but the playing song, and `source.json` names it.
struct Replace {
    kind: String,
    name: String,
    files: Vec<String>,
    rules: Option<serde_json::Value>,
    rules_hash: Option<String>,
    /// Play's Browse `P`: start the new source at once from this file index (the playing song leaves the queue),
    /// with the weighted shuffle and random off so MPD plays it in its order. None: the playing song goes on.
    start: Option<usize>,
}

impl Replace {
    /// `expect_version`: refuse unless MPD's queue is still at that `playlist` version.
    fn run(self, client: &mut rmpc_mpd::client::Client<'_>, expect_version: Option<u32>) -> Result<()> {
        let Replace { kind, name, files, rules, rules_hash, start } = self;
        let started = std::time::Instant::now();
        let status = client.get_status()?;
        if let Some(want) = expect_version
            && status.playlist != Some(want)
        {
            status_warn!("The queue changed, preview again: nothing was replaced (a then plays the new counts)");
            return Ok(());
        }
        let current = status.songid;
        let old_len = status.playlistlength as usize;
        // source.json names the new source before the queue changes: mpd-player plans from `queue ∩ files`, so
        // members not queued yet are harmless, and the idle events of the writes below find the new members
        let previous = state().lock().ok().and_then(|g| g.source.clone());
        let (source_id, generation) = Source::next_identity(&kind, previous.as_ref());
        let mut src = Source {
            kind,
            name,
            len: files.len(),
            files: files.clone(),
            rules,
            rules_hash,
            added: Vec::new(),
            source_id,
            generation,
            held: Vec::new(),
        };
        publish(Some(src.clone()));
        // the new songs go after the old queue first: MPD's database decides which files still exist (a stale
        // preview can name deleted songs), and nothing old leaves unless something new came in
        let (files, start, gone) = match src.kind.as_str() {
            "library" => {
                client.add("/", None)?;
                (files, start, 0)
            }
            _ if !files.is_empty() => {
                let total = files.len();
                let added = match add_known(files, start, |f| client.add(f, None)) {
                    Ok(added) => added,
                    Err(err) => {
                        // not a missing file: take back what came in (best effort, the add's error is the
                        // one to report), the queue and source.json stay as they were
                        if client.get_status().is_ok_and(|s| s.playlistlength as usize > old_len) {
                            let _ = client.execute(&format!("delete {old_len}:")).and_then(|()| client.read_ok());
                        }
                        publish(previous);
                        return Err(err.into());
                    }
                };
                if added.files.is_empty() {
                    publish(previous);
                    status_warn!("None of the {total} songs is in the library any more: the queue stays as it was");
                    return Ok(());
                }
                (added.files, added.start, added.gone)
            }
            _ => {
                client.load_playlist(&src.name, None)?;
                (files, start, 0)
            }
        };
        // everything old but the playing song leaves (two range deletes around it); Up next follows its files
        // (mpd-player re-finds the requests in the new queue)
        match status.song {
            Some(pos) if current.is_some() => {
                if pos + 1 < old_len {
                    client.execute(&format!("delete {}:{old_len}", pos + 1))?;
                    client.read_ok()?;
                }
                if pos > 0 {
                    client.execute(&format!("delete 0:{pos}"))?;
                    client.read_ok()?;
                }
            }
            _ if old_len > 0 => {
                client.execute(&format!("delete 0:{old_len}"))?;
                client.read_ok()?;
            }
            _ => {}
        }
        let queue = client.playlist_info()?.unwrap_or_default();
        if let Some(at) = start {
            // P: the collection starts now in its order; the song that played leaves (it is not part of it)
            if rormpc_player::shuffle_state().enabled
                && client.channels()?.0.iter().any(|c| c == rormpc_player::CHANNEL)
            {
                client.send_message(rormpc_player::CHANNEL, "shuffle off")?;
            }
            client.random(false)?;
            let old = current.and_then(|id| queue.iter().position(|s| s.id == id));
            let first_new = usize::from(old == Some(0));
            if let Some(song) = queue.get(first_new + at) {
                client.play_id(song.id)?;
            }
            if let Some(id) = current {
                client.delete_id(id)?;
            }
        } else if let Some(cur) = current.and_then(|id| queue.iter().find(|s| s.id == id)).map(|s| s.file.clone()) {
            // the playing song was kept: its copy from the new source would make it play twice
            for s in queue.iter().filter(|s| s.file == cur && Some(s.id) != current) {
                client.delete_id(s.id)?;
            }
        }
        let len = client.playlist_info()?.map_or(0, |q| q.len());
        // the members MPD took and the queue's length (the header's "modified" compares it)
        src.files = files;
        src.len = len;
        let kind = src.kind.clone();
        publish(Some(src));
        if current.is_none() && start.is_none() {
            client.play()?; // nothing was playing: start (Up next and the shuffle's plan come first)
        }
        status_info!(
            "Playing from {} · {len} songs{}{}, prepared in {:.1} s",
            match kind.as_str() {
                "library" => "the whole library",
                "hits" => "the Hits result",
                kind @ ("album" | "artist" | "directory" | "selection") => kind_label(kind),
                _ => "the playlist",
            },
            if start.is_some() { " in their order" } else { "" },
            left_out_note(gone),
            started.elapsed().as_secs_f64()
        );
        Ok(())
    }
}

/// One MPD command of a reconcile, in the order of its command list.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Cmd {
    Delete(u32),
    /// a file at a position (None: at the end)
    Add(String, Option<usize>),
    Move(u32, usize),
}

/// What a reconcile does to the queue, and the source's songs it keeps without being able to delete them.
#[derive(Debug, Default, PartialEq, Eq)]
struct Plan {
    cmds: Vec<Cmd>,
    held: Vec<String>,
}

/// A queue entry slot while a plan is simulated: an entry already there, or one the plan adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Id(u32),
    New(usize),
}

/// Reconcile `queue` (id, file in queue order) with `target` (files in rank order) without replacing it:
/// - only entries of `base` (the files the previous generation put there; None: the queue held another kind of
///   source, which the filters replace) that are not in the target leave, never the playing song (`playing`) or an
///   Up next request (`up_next`, in play order); those stay in `held`. An entry nobody can attribute (a phone's
///   or mpc's add) stays;
/// - each target file keeps one entry (the playing one first, then the first in the queue), so mpd-player's
///   planned entries keep their ids; the base's other copies of it leave; files without an entry are added;
/// - `in_order`: after the playing song come the Up next requests, then the target in rank order, then the entries
///   nobody owns, in their order. Without it the new files go at the end (the weighted shuffle orders them).
fn plan_reconcile(
    queue: &[(u32, &str)],
    playing: Option<u32>,
    up_next: &[u32],
    base: Option<&HashSet<&str>>,
    target: &[String],
    in_order: bool,
) -> Plan {
    let wanted: HashSet<&str> = target.iter().map(String::as_str).collect();
    let protected = |id: u32| Some(id) == playing || up_next.contains(&id);
    let ours = |file: &str| base.is_none_or(|b| b.contains(file));
    // each target file's entry: the playing song's, else the first in the queue
    let mut chosen: HashMap<&str, u32> = HashMap::new();
    if let Some((id, f)) = queue.iter().find(|(id, _)| Some(*id) == playing)
        && wanted.contains(f)
    {
        chosen.insert(f, *id);
    }
    for (id, f) in queue {
        if wanted.contains(f) {
            chosen.entry(f).or_insert(*id);
        }
    }
    let mut plan = Plan::default();
    let mut kept: Vec<u32> = Vec::with_capacity(queue.len());
    for (id, f) in queue {
        let in_target = chosen.get(f) == Some(id);
        if in_target || !ours(f) {
            kept.push(*id);
        } else if protected(*id) {
            kept.push(*id);
            if !wanted.contains(f) && !plan.held.iter().any(|h| h == f) {
                plan.held.push((*f).to_owned());
            }
        } else {
            plan.cmds.push(Cmd::Delete(*id));
        }
    }
    let new: Vec<usize> = (0..target.len()).filter(|&i| !chosen.contains_key(target[i].as_str())).collect();
    if !in_order {
        plan.cmds.extend(new.into_iter().map(|i| Cmd::Add(target[i].clone(), None)));
        return plan;
    }
    // the order after the playing song (from the top when nothing plays)
    let anchor = playing.filter(|id| kept.contains(id));
    let mut sim: Vec<Slot> = kept.iter().map(|id| Slot::Id(*id)).collect();
    let mut desired: Vec<Slot> = up_next.iter().filter(|id| kept.contains(id) && Some(**id) != anchor).map(|id| Slot::Id(*id)).collect();
    for (i, f) in target.iter().enumerate() {
        match chosen.get(f.as_str()) {
            Some(id) if Some(*id) == anchor || up_next.contains(id) => {}
            Some(id) => desired.push(Slot::Id(*id)),
            None => desired.push(Slot::New(i)),
        }
    }
    let after = anchor.and_then(|a| kept.iter().position(|id| *id == a)).map_or(0, |p| p + 1);
    for id in &kept[after..] {
        if Some(*id) != anchor && !desired.contains(&Slot::Id(*id)) {
            desired.push(Slot::Id(*id));
        }
    }
    let mut at = after;
    for slot in desired {
        match slot {
            Slot::New(i) => {
                sim.insert(at, slot);
                plan.cmds.push(Cmd::Add(target[i].clone(), Some(at)));
                at += 1;
            }
            Slot::Id(id) => {
                let Some(from) = sim.iter().position(|s| *s == slot) else { continue };
                // an entry from before the placed block moves to its end, which shifts the block left
                let to = if from < at { at - 1 } else { at };
                if from != to {
                    sim.remove(from);
                    sim.insert(to, slot);
                    plan.cmds.push(Cmd::Move(id, to));
                }
                if from >= at {
                    at += 1;
                }
            }
        }
    }
    plan
}

/// One generation's reconcile (see `plan_reconcile`): source.json first, then one command list. A file MPD's
/// database no longer has stops the list at its `add` (MPD's ACK names it): it is left out and the rest is
/// planned again from the queue as MPD then has it, once per such file.
fn reconcile(client: &mut rmpc_mpd::client::Client<'_>, src: HitsSource, in_order: bool) -> Result<()> {
    let started = std::time::Instant::now();
    let HitsSource { name, files, rules, rules_hash } = src;
    let previous = state().lock().ok().and_then(|g| g.source.clone());
    let (source_id, generation) = Source::next_identity("hits", previous.as_ref());
    // the queue held this filter source: only its own entries may go; any other source is replaced
    let (base, added): (Option<HashSet<String>>, Vec<String>) = match &previous {
        Some(p) if p.kind == "hits" && p.source_id.is_some() => {
            (Some(p.files.iter().chain(&p.added).chain(&p.held).cloned().collect()), p.added.clone())
        }
        _ => (None, Vec::new()),
    };
    let was_other = base.is_none();
    let mut target = files;
    for f in &added {
        if !target.contains(f) {
            target.push(f.clone());
        }
    }
    let mut gone: Vec<String> = Vec::new();
    let mut src = Source {
        kind: "hits".to_owned(),
        name,
        len: 0,
        files: target.iter().filter(|f| !added.contains(f)).cloned().collect(),
        rules: Some(rules),
        rules_hash: Some(rules_hash),
        added,
        source_id,
        generation,
        held: Vec::new(),
    };
    publish(Some(src.clone()));
    let (current, deleted, added_n) = loop {
        let status = client.get_status()?;
        let queue = client.playlist_info()?.unwrap_or_default();
        let entries: Vec<(u32, &str)> = queue.iter().map(|s| (s.id, s.file.as_str())).collect();
        let up_next = up_next_ids();
        let base_ref: Option<HashSet<&str>> = base.as_ref().map(|b| b.iter().map(String::as_str).collect());
        let plan = plan_reconcile(&entries, status.songid, &up_next, base_ref.as_ref(), &target, in_order);
        src.held.clone_from(&plan.held);
        if plan.cmds.is_empty() {
            break (status.songid, 0, 0);
        }
        client.send_start_cmd_list()?;
        for cmd in &plan.cmds {
            match cmd {
                Cmd::Delete(id) => client.send_delete_id(*id)?,
                Cmd::Add(f, at) => client.send_add(f, at.map(QueuePosition::Absolute))?,
                Cmd::Move(id, to) => client.send_move_id(*id, QueuePosition::Absolute(*to))?,
            }
        }
        client.send_execute_cmd_list()?;
        match client.read_ok() {
            Ok(()) => {
                let deleted = plan.cmds.iter().filter(|c| matches!(c, Cmd::Delete(_))).count();
                let added_n = plan.cmds.iter().filter(|c| matches!(c, Cmd::Add(..))).count();
                break (status.songid, deleted, added_n);
            }
            Err(MpdError::Mpd(e)) if e.is_no_exist() => {
                // the commands before it ran; the file is left out and the rest planned again
                let Some(Cmd::Add(f, _)) = plan.cmds.get(e.command_list_index) else { return Err(MpdError::Mpd(e).into()) };
                target.retain(|t| t != f);
                src.files.retain(|t| t != f);
                src.added.retain(|t| t != f);
                gone.push(f.clone());
            }
            Err(err) => return Err(err.into()),
        }
    };
    let len = client.playlist_info()?.map_or(0, |q| q.len());
    let up = upnext_file();
    src.len = len.saturating_sub(up.entries.iter().chain(up.playing.iter()).filter(|e| e.added).count());
    let members = src.files.len() + src.added.len();
    publish(Some(src));
    if current.is_none() && members > 0 {
        client.play()?; // nothing was playing: start (Up next and the shuffle's plan come first)
    }
    let what = if was_other { "Filters replaced the queue" } else { "Filters applied" };
    status_info!(
        "{what}: {members} songs (+{added_n} −{deleted}){}, in {:.2} s",
        left_out_note(gone.len()),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// What a replacement put in the queue: the files MPD took, the start index among them, and how many it did
/// not know.
#[derive(Debug, PartialEq, Eq)]
struct Added {
    files: Vec<String>,
    start: Option<usize>,
    gone: usize,
}

/// Add `files` in order with `add`; a file MPD's database no longer has (ACK "No such directory") is left out
/// and counted, any other error stops. `start` (an index in `files`) moves to the same song among the added
/// ones, or to the next added one when its own song is gone.
fn add_known(
    files: Vec<String>,
    start: Option<usize>,
    mut add: impl FnMut(&str) -> Result<(), MpdError>,
) -> Result<Added, MpdError> {
    let mut kept = Vec::with_capacity(files.len());
    let (mut new_start, mut gone) = (None, 0);
    for (i, f) in files.into_iter().enumerate() {
        match add(&f) {
            Ok(()) => {
                if start.is_some_and(|s| i >= s) && new_start.is_none() {
                    new_start = Some(kept.len());
                }
                kept.push(f);
            }
            Err(MpdError::Mpd(e)) if e.is_no_exist() => gone += 1,
            Err(err) => return Err(err),
        }
    }
    // every song from the start on is gone: start at the last one MPD took
    let start = start.map(|_| new_start.unwrap_or(kept.len().saturating_sub(1)));
    Ok(Added { files: kept, start, gone })
}

/// " · 7 songs no longer in the library were left out" after the song count of a replacement's status.
fn left_out_note(gone: usize) -> String {
    match gone {
        0 => String::new(),
        1 => " · 1 song no longer in the library was left out".to_owned(),
        n => format!(" · {n} songs no longer in the library were left out"),
    }
}

/// What Play's Browse `P` replaces the queue with: a collection in its own order, started at once.
#[derive(Debug, Clone)]
pub struct Collection {
    /// "album", "artist", "directory", "playlist" or "selection"
    pub kind: String,
    pub name: String,
    pub files: Vec<String>,
    /// the index in `files` to start from
    pub start: usize,
}

/// Browse `P`: the queue becomes `col` and plays it from `col.start` now, in its order (weighted shuffle and
/// random off). The queue must still be at `version` (the one the confirmation was judged on).
pub fn play_collection(ctx: &Ctx, col: Collection, version: Option<u32>) {
    let Collection { kind, name, files, start } = col;
    let replace = Replace { kind, name, files, rules: None, rules_hash: None, start: Some(start) };
    ctx.command(move |_, client| replace.run(client, version));
}

/// A stored playlist played as the source (Play's list picker), after the same confirmation as "Sources…".
pub fn play_playlist(ctx: &Ctx, name: String) {
    confirm_replace(ctx, "playlist".into(), name);
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
        let replace = Replace {
            kind: kind.clone(),
            name: name.clone(),
            files: files.clone(),
            rules: None,
            rules_hash: None,
            start: None,
        };
        ctx.command(move |_, client| replace.run(client, None));
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

    fn source(kind: &str, files: &[&str]) -> Source {
        Source {
            kind: kind.to_owned(),
            name: "n".to_owned(),
            len: files.len(),
            files: files.iter().map(|f| (*f).to_owned()).collect(),
            rules: None,
            rules_hash: Some("h".to_owned()),
            added: Vec::new(),
            source_id: None,
            generation: None,
            held: Vec::new(),
        }
    }

    #[test]
    fn previous_sources_remember_the_filters_when_another_kind_of_source_replaces_them() {
        let filters = Source { rules: Some(serde_json::json!({"sort": "plays"})), ..source("hits", &["a"]) };
        assert!(left_filters(&filters, Some(&source("album", &["b"]))));
        assert!(left_filters(&filters, None));
        // another generation of the filters is no leaving: a song of it playing records it instead
        assert!(!left_filters(&filters, Some(&filters)));
        // the old Hits "Play these results" snapshot has no rules to remember
        assert!(!left_filters(&source("hits", &["a"]), Some(&source("album", &["b"]))));
        assert!(!left_filters(&source("album", &["b"]), Some(&filters)));
    }

    #[test]
    fn songs_appended_to_a_hits_source_join_it_once_and_keep_its_length_right() {
        let mut src = source("hits", &["a", "b"]);
        let appended = vec!["c".to_owned(), "a".to_owned(), "c".to_owned()];
        // "a" is a member already and "c" joins once; the length counts every queue entry added
        assert_eq!(src.append(&appended), 1);
        assert_eq!((src.added.clone(), src.len), (vec!["c".to_owned()], 5));
        let json = serde_json::to_value(&src).expect("source.json");
        assert_eq!(json["added"], serde_json::json!(["c"]));
        assert_eq!(json["rules_hash"], "h"); // the round key stays: mpd-player keeps the round
        // another source kind only counts them (it plays the whole queue anyway)
        let mut album = source("album", &["a"]);
        assert_eq!(album.append(&appended), 0);
        assert!(album.added.is_empty() && album.len == 4);
        assert!(serde_json::to_value(&album).expect("source.json").get("added").is_none());
    }

    #[test]
    fn a_filter_source_keeps_its_id_and_counts_generations_another_source_starts_a_new_one() {
        let id = new_source_id();
        assert_eq!(id.len(), 36);
        assert_eq!(id.as_bytes()[14], b'4'); // version 4
        assert_ne!(id, new_source_id());
        // the first filter source after an album, or after nothing: a new id, generation 1
        let (first, g) = Source::next_identity("hits", Some(&source("album", &["a"])));
        assert!(first.is_some());
        assert_eq!(g, Some(1));
        assert!(Source::next_identity("hits", None).0.is_some_and(|other| Some(&other) != first.as_ref()));
        // a filter change keeps it and counts one more
        let mut hits = source("hits", &["a"]);
        hits.source_id.clone_from(&first);
        hits.generation = Some(4);
        assert_eq!(Source::next_identity("hits", Some(&hits)), (first, Some(5)));
        // an older source.json without an id: a new one
        assert!(Source::next_identity("hits", Some(&source("hits", &["a"]))).0.is_some());
        // other kinds carry neither
        assert_eq!(Source::next_identity("album", Some(&hits)), (None, None));
        let json = serde_json::to_value(&hits).expect("source.json");
        assert_eq!(json["generation"], 4);
        assert!(json["source_id"].is_string());
        assert!(serde_json::to_value(source("album", &["a"])).expect("source.json").get("source_id").is_none());
    }

    /// Run a plan's commands the way MPD does on `queue` (new entries get ids from 100): the files in order.
    fn after(queue: &[(u32, &str)], plan: &Plan) -> Vec<String> {
        let mut q: Vec<(u32, String)> = queue.iter().map(|(id, f)| (*id, (*f).to_owned())).collect();
        let mut next = 100;
        for cmd in &plan.cmds {
            match cmd {
                Cmd::Delete(id) => q.retain(|(i, _)| i != id),
                Cmd::Add(f, at) => {
                    q.insert(at.unwrap_or(q.len()), (next, f.clone()));
                    next += 1;
                }
                Cmd::Move(id, to) => {
                    let from = q.iter().position(|(i, _)| i == id).expect("a moved id is queued");
                    let e = q.remove(from);
                    q.insert(*to, e);
                }
            }
        }
        q.into_iter().map(|(_, f)| f).collect()
    }

    fn base<'a>(files: &[&'a str]) -> HashSet<&'a str> {
        files.iter().copied().collect()
    }

    #[test]
    fn a_reconcile_changes_only_what_the_filters_changed_and_keeps_shared_ids() {
        // the previous generation put a b c d; the phone added x; c plays
        let q = [(1, "a"), (2, "b"), (3, "c"), (4, "x"), (5, "d")];
        let prev = base(&["a", "b", "c", "d"]);
        let target = strings(&["b", "e", "d"]);
        let plan = plan_reconcile(&q, Some(3), &[], Some(&prev), &target, false);
        // a leaves; c plays on outside the result (held for the next generation); x is nobody's: it stays
        assert_eq!(plan.cmds, vec![Cmd::Delete(1), Cmd::Add("e".to_owned(), None)]);
        assert_eq!(plan.held, strings(&["c"]));
        assert_eq!(after(&q, &plan), strings(&["b", "c", "x", "d", "e"]));
    }

    #[test]
    fn normal_mode_puts_the_result_after_the_playing_song_in_rank_order() {
        let q = [(1, "a"), (2, "b"), (3, "c"), (4, "x"), (5, "d")];
        let prev = base(&["a", "b", "c", "d"]);
        let target = strings(&["d", "e", "a", "c"]);
        let plan = plan_reconcile(&q, Some(3), &[], Some(&prev), &target, true);
        // b leaves; c plays (it is in the result: no copy); then d e a in rank order; x (nobody's) after them
        assert_eq!(after(&q, &plan), strings(&["c", "d", "e", "a", "x"]));
        assert!(plan.cmds.contains(&Cmd::Delete(2)));
        assert!(plan.held.is_empty());
        // the same rules again change nothing
        let q2 = [(3, "c"), (5, "d"), (100, "e"), (1, "a"), (4, "x")];
        assert_eq!(plan_reconcile(&q2, Some(3), &[], Some(&base(&["d", "e", "a", "c"])), &target, true), Plan::default());
    }

    #[test]
    fn up_next_requests_stay_first_and_are_never_removed() {
        // 7 is an Up next request of a source song the new rules leave out, 8 one added only for Up next
        let q = [(1, "a"), (7, "b"), (8, "r"), (2, "c")];
        let prev = base(&["a", "b", "c"]);
        let target = strings(&["c", "d"]);
        let plan = plan_reconcile(&q, Some(1), &[8, 7], Some(&prev), &target, true);
        assert_eq!(after(&q, &plan), strings(&["a", "r", "b", "c", "d"]));
        assert_eq!(plan.held, strings(&["a", "b"])); // the playing song and the request stay, outside the result
        // weighted: priorities order them, nothing moves
        let plan = plan_reconcile(&q, Some(1), &[8, 7], Some(&prev), &target, false);
        assert_eq!(plan.cmds, vec![Cmd::Add("d".to_owned(), None)]);
    }

    #[test]
    fn another_source_is_replaced_and_zero_songs_leave_only_the_playing_one() {
        // an album played with P: every entry is its own, the first filter touch replaces it
        let q = [(1, "t1"), (2, "t2"), (3, "t3")];
        let plan = plan_reconcile(&q, Some(2), &[], None, &strings(&["t3", "h1"]), true);
        assert_eq!(after(&q, &plan), strings(&["t2", "t3", "h1"]));
        // 0 owned: the upcoming songs go, it stops after the playing song; an entry nobody owns stays
        let q = [(1, "a"), (2, "b"), (4, "x"), (3, "c")];
        let plan = plan_reconcile(&q, Some(2), &[], Some(&base(&["a", "b", "c"])), &[], true);
        assert_eq!(after(&q, &plan), strings(&["b", "x"]));
        assert_eq!(plan.held, strings(&["b"]));
        // nothing plays: the result from the top, in order
        let q = [(1, "a"), (2, "b")];
        let plan = plan_reconcile(&q, None, &[], Some(&base(&["a", "b"])), &strings(&["b", "c", "a"]), true);
        assert_eq!(after(&q, &plan), strings(&["b", "c", "a"]));
    }

    #[test]
    fn copies_of_a_result_song_keep_one_entry_the_playing_one_first() {
        // the playing song (3) and an older copy (1) of the same file: the playing one stays, the copy goes
        let q = [(1, "a"), (2, "b"), (3, "a")];
        let plan = plan_reconcile(&q, Some(3), &[], Some(&base(&["a", "b"])), &strings(&["a", "b"]), true);
        assert_eq!(after(&q, &plan), strings(&["a", "b"]));
        assert!(plan.cmds.contains(&Cmd::Delete(1)));
        // a phone's copy of a result song is nobody's: it stays, and the result uses the first entry
        let q = [(1, "a"), (2, "x"), (3, "a")];
        let plan = plan_reconcile(&q, Some(2), &[], Some(&base(&["a"])), &strings(&["a"]), false);
        assert_eq!(plan.cmds, vec![Cmd::Delete(3)]);
    }

    #[test]
    fn songs_before_the_playing_one_move_after_it_without_losing_order() {
        let q = [(1, "e"), (2, "d"), (3, "p"), (4, "c"), (5, "b"), (6, "a")];
        let all = base(&["a", "b", "c", "d", "e", "p"]);
        let target = strings(&["a", "b", "c", "d", "e"]);
        let plan = plan_reconcile(&q, Some(3), &[], Some(&all), &target, true);
        assert_eq!(after(&q, &plan), strings(&["p", "a", "b", "c", "d", "e"]));
    }

    /// An `add` as MPD answers it: files in `missing` are not in its database.
    fn mpd_add<'a>(missing: &'a [&str], log: &'a mut Vec<String>) -> impl FnMut(&str) -> Result<(), MpdError> + 'a {
        move |f| {
            log.push(f.to_owned());
            if missing.contains(&f) {
                return Err(MpdError::Mpd(rmpc_mpd::errors::MpdFailureResponse {
                    code: rmpc_mpd::errors::ErrorCode::NoExist,
                    command_list_index: 0,
                    command: "add".to_owned(),
                    message: "No such directory".to_owned(),
                }));
            }
            Ok(())
        }
    }

    fn strings(files: &[&str]) -> Vec<String> {
        files.iter().map(|f| (*f).to_owned()).collect()
    }

    #[test]
    fn files_mpd_no_longer_has_are_left_out_and_counted() {
        let mut log = Vec::new();
        let added = add_known(strings(&["a", "gone1", "b", "gone2", "c"]), None, mpd_add(&["gone1", "gone2"], &mut log))
            .expect("missing files are no error");
        // every file was tried, in order; source.json's members are only the ones MPD took
        assert_eq!(log, strings(&["a", "gone1", "b", "gone2", "c"]));
        assert_eq!(added, Added { files: strings(&["a", "b", "c"]), start: None, gone: 2 });
        assert_eq!(left_out_note(added.gone), " · 2 songs no longer in the library were left out");
        assert_eq!(left_out_note(1), " · 1 song no longer in the library was left out");
        assert_eq!(left_out_note(0), "");
    }

    #[test]
    fn nothing_known_adds_nothing() {
        let mut log = Vec::new();
        let added = add_known(strings(&["x", "y"]), None, mpd_add(&["x", "y"], &mut log)).expect("no error");
        // Replace refuses before deleting anything when nothing came in
        assert!(added.files.is_empty());
        assert_eq!(added.gone, 2);
    }

    #[test]
    fn another_add_error_stops_the_replacement() {
        let mut calls = 0;
        let err = add_known(strings(&["a", "b", "c"]), None, |_| {
            calls += 1;
            if calls == 2 { Err(MpdError::Generic("broken pipe".to_owned())) } else { Ok(()) }
        });
        assert!(matches!(err, Err(MpdError::Generic(_))));
        assert_eq!(calls, 2);
    }

    #[test]
    fn the_start_follows_its_song_or_the_next_one_mpd_took() {
        let files = &["a", "gone", "b", "c"];
        let at = |start: usize, missing: &[&str]| {
            let mut log = Vec::new();
            add_known(strings(files), Some(start), mpd_add(missing, &mut log)).expect("no error").start
        };
        assert_eq!(at(2, &["gone"]), Some(1)); // "b" moved up one place
        assert_eq!(at(1, &["gone"]), Some(1)); // its own song is gone: "b" starts
        assert_eq!(at(0, &["gone"]), Some(0));
        assert_eq!(at(2, &["gone", "b", "c"]), Some(0)); // nothing from the start on: the last one MPD took
    }

    #[test]
    fn a_restored_album_comes_back_in_its_order_around_the_playing_song() {
        // the filters' songs x, y and a phone's z go; track 2 plays on, Up next 9 stays
        let q = [(1, "x"), (2, "t2"), (9, "req"), (3, "y"), (4, "z")];
        let cmds = plan_restore(&q, Some(2), &[9], &strings(&["t1", "t2", "t3"]));
        assert_eq!(cmds, vec![
            Cmd::Delete(1),
            Cmd::Delete(3),
            Cmd::Delete(4),
            Cmd::Add("t1".into(), Some(0)),
            Cmd::Add("t3".into(), None),
        ]);
        // the playing song is not the album's: it plays on and the album follows Up next
        let cmds = plan_restore(&[(1, "x"), (2, "y")], Some(1), &[], &strings(&["t1", "t2"]));
        assert_eq!(cmds, vec![Cmd::Delete(2), Cmd::Add("t1".into(), None), Cmd::Add("t2".into(), None)]);
        // nothing plays: the whole queue is the album again
        let cmds = plan_restore(&[(1, "x")], None, &[], &strings(&["t1"]));
        assert_eq!(cmds, vec![Cmd::Delete(1), Cmd::Add("t1".into(), None)]);
    }

    #[test]
    fn a_generation_counts_as_reached_only_while_it_is_the_newest() {
        let gens = Generations::default();
        gens.latest.store(3, Ordering::SeqCst);
        assert!(!gens.start(2), "a stale generation never writes");
        assert_eq!(gens.reached.load(Ordering::SeqCst), 0);
        assert!(gens.start(3));
        assert_eq!(gens.reached.load(Ordering::SeqCst), 3);
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
