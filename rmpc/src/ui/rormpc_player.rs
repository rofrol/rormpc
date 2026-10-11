//! rormpc: talking to mpd-player (rormpc-tools), the playback daemon that runs with rormpc closed too. Commands go
//! over MPD's client-to-client messages on channel "rormpc" (`gap set 5`); the daemon alone writes its state to
//! `$XDG_STATE_HOME/rormpc/<module>.json`, which rormpc reads to show it. A command counts as done only when that
//! file shows the new value: an MPD message has no reply, and the daemon may not be running.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use rmpc_mpd::{
    commands::State,
    mpd_client::{MpdClient, ValueChange},
};
use serde::Deserialize;

use crate::{
    ctx::Ctx,
    shared::{
        macros::{modal, status_error, status_info, status_warn},
        mpd_client_ext::MpdClientExt,
    },
    ui::modals::{input_modal::InputModal, menu::modal::MenuModal},
};

pub const CHANNEL: &str = "rormpc";
/// how long to wait for mpd-player to write its state after a command: it answers within milliseconds when it
/// runs; this only bounds the wait when it does not
pub(crate) const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);
/// Two existing mpd-player `MAX_WAIT` (30 s) heartbeats: a missing publication is stale, not a live forecast.
pub const FORECAST_FRESH_SECONDS: f64 = 60.0;

/// Re-read whether mpd-player is subscribed to its channel (on MPD's Subscription event, a reconnect, or
/// entering the Queue plan view). A crash sends no Subscription event; `rormpc_process` sees that one.
pub fn refresh_presence(ctx: &Ctx) {
    let present = std::sync::Arc::clone(&ctx.player_present);
    ctx.command(move |tx, client| {
        let channels = client.channels();
        let subscribed = channels.as_ref().is_ok_and(|c| c.0.iter().any(|c| c == CHANNEL));
        present.store(subscribed, std::sync::atomic::Ordering::Relaxed);
        tx.send(crate::shared::events::AppEvent::RequestRender)?;
        Ok(channels.map(|_| ())?)
    });
}

fn state_base() -> PathBuf {
    // tests never see the machine's mpd-player state (a weighted shuffle left on changed what labels render)
    #[cfg(test)]
    return std::env::temp_dir().join(format!("rormpc-test-state-{}", std::process::id()));
    #[cfg(not(test))]
    std::env::var("XDG_STATE_HOME").map_or_else(
        |_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state"),
        PathBuf::from,
    )
}

pub fn state_path(module: &str) -> PathBuf {
    state_base().join(format!("rormpc/{module}.json"))
}

/// mpd-player's command socket, as `mpd-player socket-path` prints it: `$XDG_RUNTIME_DIR/rormpc/player.sock` when
/// that is set (Linux), else beside its state files. Tests never reach the machine's socket.
pub fn socket_path() -> PathBuf {
    #[cfg(not(test))]
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").filter(|r| !r.is_empty()) {
        return PathBuf::from(runtime).join("rormpc/player.sock");
    }
    state_base().join("rormpc/player.sock")
}

/// One transport command (`next`, `prev`) to mpd-player's command socket: a datagram, fire and forget, never
/// blocking. Err says why it did not go out (no socket, nobody bound, a full buffer); the caller falls back at once.
pub fn send_transport(command: &str) -> Result<(), String> {
    send_transport_to(&socket_path(), command)
}

/// One datagram's payload, mpd-player's v1 JSON (`{"command": "next"}`).
fn transport_payload(command: &str) -> String {
    serde_json::json!({ "command": command }).to_string()
}

#[cfg(unix)]
fn send_transport_to(path: &std::path::Path, command: &str) -> Result<(), String> {
    let sock = std::os::unix::net::UnixDatagram::unbound().map_err(|e| e.to_string())?;
    sock.set_nonblocking(true).map_err(|e| e.to_string())?;
    match sock.send_to(transport_payload(command).as_bytes(), path) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(format!("no socket at {}", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            Err(format!("nobody bound at {}", path.display()))
        }
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[cfg(not(unix))]
fn send_transport_to(path: &std::path::Path, _command: &str) -> Result<(), String> {
    Err(format!("no Unix sockets here ({})", path.display()))
}

/// Whether the missing socket was already said in the status bar; a send that goes out again resets it.
static SOCKET_MISSING_SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Send `command` to the socket; true when it went out. A failure is said once per outage, then stays in the log.
fn via_socket(command: &str, said: &std::sync::atomic::AtomicBool) -> bool {
    use std::sync::atomic::Ordering;
    match send_transport(command) {
        Ok(()) => {
            said.store(false, Ordering::Relaxed);
            true
        }
        Err(why) => {
            if said.swap(true, Ordering::Relaxed) {
                log::debug!("mpd-player's command socket: {why}");
            } else {
                status_warn!(
                    "mpd-player's command socket is not bound ({why}): Next and Previous go to MPD"
                );
            }
            false
        }
    }
}

#[cfg(test)]
thread_local! {
    /// What `shuffle_state()` returns in this test thread (default: no shuffle state, as on a fresh machine).
    pub static TEST_SHUFFLE: std::cell::RefCell<ShuffleState> = std::cell::RefCell::default();
}

#[derive(Debug, Deserialize)]
struct GapState {
    seconds: f64,
}

/// The silence between songs that mpd-player applies, from its state file (None: never ran).
pub fn gap_seconds() -> Option<f64> {
    let text = std::fs::read_to_string(state_path("gap")).ok()?;
    serde_json::from_str::<GapState>(&text).ok().map(|g| g.seconds)
}

fn fmt_seconds(s: f64) -> String {
    if s <= 0.0 {
        "off".to_owned()
    } else if s.fract() == 0.0 {
        format!("{s:.0} s")
    } else {
        format!("{s} s")
    }
}

/// Send `gap set N`, then report once gap.json shows N (or that mpd-player did not answer).
fn set_gap(ctx: &Ctx, seconds: f64) {
    ctx.command(move |_, client| {
        let alive = client.channels()?.0.iter().any(|c| c == CHANNEL);
        if !alive {
            status_error!("mpd-player is not running (rormpc_install.sh companions starts it): silence not changed");
            return Ok(());
        }
        client.send_message(CHANNEL, &format!("gap set {seconds}"))?;
        std::thread::spawn(move || {
            let start = Instant::now();
            loop {
                if gap_seconds().is_some_and(|s| (s - seconds).abs() < 1e-6) {
                    status_info!("Silence between songs: {}", fmt_seconds(seconds));
                    return;
                }
                if start.elapsed() > ANSWER_TIMEOUT {
                    status_error!("mpd-player did not confirm the new silence ({})", fmt_seconds(seconds));
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        Ok(())
    });
}

/// "Silence between songs…": presets and a custom value, the current one marked.
pub fn open_gap_menu(ctx: &Ctx) {
    let current = gap_seconds();
    let mark = move |s: f64| if current.is_some_and(|c| (c - s).abs() < 1e-6) { "▶" } else { " " };
    let presets: Vec<(String, f64)> =
        [0.0, 1.0, 2.0, 3.0, 5.0, 8.0, 10.0].into_iter().map(|s| (format!("{} {}", mark(s), fmt_seconds(s)), s)).collect();
    let title = match current {
        Some(c) => format!("Silence between songs (now {})", fmt_seconds(c)),
        None => "Silence between songs (mpd-player has not run yet)".to_owned(),
    };
    let menu = MenuModal::new(ctx)
        .width(50)
        .list_section(ctx, move |mut section| {
            section.add_item(title.clone(), |_| Ok(()));
            Some(section)
        })
        .list_section(ctx, move |mut section| {
            for (label, s) in presets {
                section.add_item(label, move |ctx| {
                    set_gap(ctx, s);
                    Ok(())
                });
            }
            section.add_item("  Other…", |ctx| {
                modal!(
                    ctx,
                    InputModal::new(ctx)
                        .title("Silence between songs")
                        .input_label("Seconds (0-60, 0 = off):")
                        .confirm_label("Set")
                        .on_confirm(|ctx, value| {
                            match value.trim().trim_end_matches('s').trim().replace(',', ".").parse::<f64>() {
                                Ok(s) if (0.0..=60.0).contains(&s) => set_gap(ctx, s),
                                _ => status_warn!("Not a number of seconds from 0 to 60: {value}"),
                            }
                            Ok(())
                        })
                );
                Ok(())
            });
            Some(section)
        })
        .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
        .build();
    modal!(ctx, menu);
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Nominee {
    pub id: u32,
    pub file: String,
    #[serde(default)]
    pub why: String,
}

/// A song the weighted shuffle drew ahead (the plan, in play order; the first one has the MPD priority).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Planned {
    pub id: u32,
    pub file: String,
    #[serde(default)]
    pub why: String,
    /// the round it is drawn for: 0 the current one, 1 the next (a Hits source's rounds)
    #[serde(default)]
    pub round: u32,
    /// drawn although resting: nothing else was eligible ("rest relaxed")
    #[serde(default)]
    pub relaxed: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Round {
    #[serde(default)]
    pub heard: Vec<String>,
    #[serde(default)]
    pub total: usize,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Cooldown {
    pub until: f64,
}

/// mpd-player's shuffle.json: the weighted shuffle's state.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ShuffleState {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub reason: String,
    /// why the plan holds fewer than ten songs ("4 ahead · 12 resting"), empty when it is full
    #[serde(default)]
    pub ahead_note: String,
    #[serde(default)]
    pub nominee: Option<Nominee>,
    #[serde(default)]
    pub round: Option<Round>,
    #[serde(default)]
    pub cooldown: std::collections::HashMap<String, Cooldown>,
    #[serde(default)]
    pub plan: Vec<Planned>,
    #[serde(default)]
    pub history: Vec<Past>,
    #[serde(default)]
    pub plan_version: String,
    #[serde(default)]
    pub pid: u32,
    #[serde(default)]
    pub updated_at: f64,
    #[serde(default)]
    pub ack: Option<PlanAck>,
    #[serde(default)]
    pub publish_error: Option<String>,
    /// the played-songs trail behind `shuffle prev`; only an mpd-player that handles `shuffle prev` writes it
    #[serde(default)]
    pub trail: Option<serde::de::IgnoredAny>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PlanAck {
    pub token: String,
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// A song mpd-player saw play, with how it ended.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Past {
    /// the queue entry that played (the same file may be queued twice)
    #[serde(default)]
    pub id: Option<u32>,
    pub file: String,
    /// "finished", "early" or "late" (skips)
    #[serde(default)]
    pub kind: String,
}

/// shuffle.json, read again only when the file changed (it is looked at while rendering).
pub fn shuffle_state() -> ShuffleState {
    #[cfg(test)]
    return TEST_SHUFFLE.with(|s| s.borrow().clone());
    #[cfg(not(test))]
    shuffle_state_from_file()
}

#[cfg(not(test))]
type ShuffleCache = std::sync::Mutex<(Option<std::time::SystemTime>, ShuffleState)>;
#[cfg(not(test))]
static SHUFFLE_CACHE: std::sync::OnceLock<ShuffleCache> = std::sync::OnceLock::new();

/// Forget the cached shuffle.json: an atomic replacement can keep the old mtime.
pub fn invalidate_shuffle_state() {
    #[cfg(not(test))]
    if let Some(cache) = SHUFFLE_CACHE.get()
        && let Ok(mut cache) = cache.lock()
    {
        // None also means a deleted file; never reuse its old forecast
        *cache = (None, ShuffleState::default());
    }
}

#[cfg(not(test))]
fn shuffle_state_from_file() -> ShuffleState {
    let p = state_path("shuffle");
    let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
    let cache =
        SHUFFLE_CACHE.get_or_init(|| std::sync::Mutex::new((None, ShuffleState::default())));
    let Ok(mut c) = cache.lock() else { return ShuffleState::default() };
    if c.0 != mtime || mtime.is_none() {
        c.1 = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        c.0 = mtime;
    }
    c.1.clone()
}

/// Where Previous goes.
#[derive(Debug, PartialEq, Eq)]
pub enum PrevRoute {
    /// `rewind_to_start_sec` reached: seek the playing song to 0:00
    Rewind,
    /// `prev` went out on mpd-player's command socket, the path media keys and Now Playing take too
    Socket,
    /// `shuffle prev` to mpd-player: back through the songs that really played, no skip counted
    Daemon,
    /// MPD's own `previous` (with random on it follows MPD's random order, not what played)
    Mpd,
}

/// Previous rewinds first, then goes to mpd-player's command socket (`socket` sends `prev`, false when nobody is
/// bound). Without the socket it goes through MPD's channel only while mpd-player is subscribed and its
/// shuffle.json has the `trail` (`daemon`; an mpd-player older than `shuffle prev` ignores the message, so
/// Previous would do nothing), else to MPD itself.
pub fn prev_route(
    rewind_to_start: Option<u64>,
    elapsed_sec: u64,
    socket: impl FnOnce() -> bool,
    daemon: impl FnOnce() -> anyhow::Result<bool>,
) -> anyhow::Result<PrevRoute> {
    Ok(match rewind_to_start {
        Some(value) if elapsed_sec >= value => PrevRoute::Rewind,
        _ if socket() => PrevRoute::Socket,
        _ if daemon()? => PrevRoute::Daemon,
        _ => PrevRoute::Mpd,
    })
}

/// Previous (the key, `rmpc prev`): see `prev_route`. Runs on the MPD command thread, so the subscription is
/// asked of MPD now, not taken from the cached presence (a crashed daemon sends no event).
pub fn previous<C: MpdClient + MpdClientExt>(
    client: &mut C,
    rewind_to_start: Option<u64>,
    elapsed_sec: u64,
    keep_state: bool,
    state: State,
) -> anyhow::Result<()> {
    let route = prev_route(
        rewind_to_start,
        elapsed_sec,
        || via_socket("prev", &SOCKET_MISSING_SAID),
        || {
            Ok(shuffle_state().trail.is_some()
                && client.channels()?.0.iter().any(|c| c == CHANNEL))
        },
    )?;
    match route {
        PrevRoute::Rewind => client.seek_current(ValueChange::Set(0))?,
        PrevRoute::Socket => {}
        PrevRoute::Daemon => client.send_message(CHANNEL, "shuffle prev")?,
        PrevRoute::Mpd => client.prev_keep_state(keep_state, state)?,
    }
    Ok(())
}

/// Where Next goes.
#[derive(Debug, PartialEq, Eq)]
pub enum NextRoute {
    /// `next` went out on mpd-player's command socket
    Socket,
    /// MPD's own `next`
    Mpd,
}

/// Next goes to mpd-player's command socket (`socket` sends `next`, false when nobody is bound), else to MPD.
pub fn next_route(socket: impl FnOnce() -> bool) -> NextRoute {
    if socket() { NextRoute::Socket } else { NextRoute::Mpd }
}

/// Next (the key, `rmpc next`): see `next_route`.
pub fn next<C: MpdClient + MpdClientExt>(
    client: &mut C,
    keep_state: bool,
    state: State,
) -> anyhow::Result<()> {
    if next_route(|| via_socket("next", &SOCKET_MISSING_SAID)) == NextRoute::Mpd {
        client.next_keep_state(keep_state, state)?;
    }
    Ok(())
}

/// Days left of a "heard enough" cooldown (None: not cooling down).
pub fn cooldown_days(file: &str) -> Option<f64> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64();
    let c = shuffle_state().cooldown.get(file).cloned()?;
    (c.until > now).then(|| (c.until - now) / 86400.0)
}

/// Send one command, then report `ok()` once `done()` holds (mpd-player wrote its state), or that it did not answer.
fn send_and_confirm(
    ctx: &Ctx,
    msg: String,
    done: impl Fn() -> bool + Send + 'static,
    ok: impl Fn() -> String + Send + 'static,
) {
    ctx.command(move |_, client| {
        if !client.channels()?.0.iter().any(|c| c == CHANNEL) {
            status_error!("mpd-player is not running (rormpc_install.sh companions starts it)");
            return Ok(());
        }
        client.send_message(CHANNEL, &msg)?;
        std::thread::spawn(move || {
            let start = Instant::now();
            loop {
                if done() {
                    status_info!("{}", ok());
                    return;
                }
                if start.elapsed() > ANSWER_TIMEOUT {
                    status_error!("mpd-player did not confirm: {msg}");
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        Ok(())
    });
}

/// Turn the weighted shuffle on or off.
pub fn toggle_shuffle(ctx: &Ctx) {
    let on = !shuffle_state().enabled;
    send_and_confirm(
        ctx,
        format!("shuffle {}", if on { "on" } else { "off" }),
        move || shuffle_state().enabled == on,
        move || {
            if on {
                let s = shuffle_state();
                if s.active {
                    "Weighted shuffle on: the next song is drawn by plays and likes".to_owned()
                } else {
                    format!("Weighted shuffle on, waiting: {}", s.reason)
                }
            } else {
                "Weighted shuffle off: the queue plays in order (x for plain random)".to_owned()
            }
        },
    );
}

/// x while the weighted shuffle owns MPD's random: plain random instead (random stays on, the shuffle turns off).
pub fn release_shuffle(ctx: &Ctx) {
    send_and_confirm(
        ctx,
        "shuffle release".to_owned(),
        || !shuffle_state().enabled,
        || "Plain random (weighted shuffle off)".to_owned(),
    );
}

/// "Heard enough" for a song (the playing one also skips to the next): the weighted shuffle leaves it out for
/// 1, 3, 7, then 14 days; Enter and Play next still play it.
pub fn heard_enough(ctx: &Ctx, file: String, title: String) {
    let before = cooldown_days(&file).unwrap_or(0.0);
    let f = file.clone();
    send_and_confirm(
        ctx,
        format!("shuffle heardenough {file}"),
        move || cooldown_days(&f).is_some_and(|d| d > before + 0.5),
        move || {
            let days = cooldown_days(&file).unwrap_or(0.0).round();
            format!("Heard enough: {title} rests {days:.0} days (Enter and Play next still play it)")
        },
    );
}

/// Undo "heard enough".
pub fn unheard_enough(ctx: &Ctx, file: String) {
    let f = file.clone();
    send_and_confirm(
        ctx,
        format!("shuffle unheardenough {file}"),
        move || cooldown_days(&f).is_none(),
        || "Back in the weighted shuffle".to_owned(),
    );
}

/// Start the next round of a Hits source (every song once more).
pub fn new_round(ctx: &Ctx) {
    send_and_confirm(
        ctx,
        "shuffle newround".to_owned(),
        || shuffle_state().round.is_none_or(|r| !r.done),
        || "New round: every song of the source once more".to_owned(),
    );
}

static PLAYING: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(u32::MAX);

/// The playing song's queue id, kept for the ShuffleNext column (song properties are rendered without ctx).
pub fn set_playing(id: Option<u32>) {
    PLAYING.store(id.unwrap_or(u32::MAX), std::sync::atomic::Ordering::Relaxed);
}

/// how many past plays the Next column numbers (-1 the last one)
const PAST_SHOWN: usize = 10;

/// A queued song's turn on the timeline: -k the k-th most recent play (the entry stays in the queue), 0 the playing
/// song, then 1.. the Up next requests, then the weighted shuffle's plan.
pub fn next_rank(id: u32) -> Option<i64> {
    if id == PLAYING.load(std::sync::atomic::Ordering::Relaxed) {
        return Some(0);
    }
    let requests = crate::ui::rormpc_upnext::up_next_ids();
    if let Some(k) = requests.iter().position(|i| *i == id) {
        return Some(1 + k as i64);
    }
    let sh = shuffle_state();
    if sh.enabled && sh.active
        && let Some(k) = sh.plan.iter().position(|p| p.id == id)
    {
        return Some(1 + (requests.len() + k) as i64);
    }
    // its latest play among the last ones (an entry played twice shows the newer one)
    sh.history.iter().rev().take(PAST_SHOWN).position(|p| p.id == Some(id)).map(|k| -(k as i64) - 1)
}

/// The ShuffleNext column: "-1" the last play, "0" the playing song, "↑1" a request, then the plan numbered on.
pub fn next_marker(id: u32) -> Option<String> {
    let requests = crate::ui::rormpc_upnext::up_next_ids().len() as i64;
    next_rank(id).map(|r| match r {
        r if r >= 1 && r <= requests => format!("↑{r}"),
        r => r.to_string(),
    })
}

/// The ShuffleNext marker of a library file (Hits rows know files, not queue ids): its first queue entry's turn.
pub fn next_marker_for_file(ctx: &crate::ctx::Ctx, file: &str) -> Option<String> {
    ctx.queue.iter().filter(|s| s.file == file).find_map(|s| next_marker(s.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prev(rewind: Option<u64>, elapsed: u64, socket: bool, daemon: bool) -> PrevRoute {
        prev_route(rewind, elapsed, || socket, || Ok(daemon)).unwrap()
    }

    #[test]
    fn previous_goes_to_the_socket_then_through_mpd_player_only_when_it_handles_shuffle_prev() {
        assert_eq!(prev(None, 30, true, true), PrevRoute::Socket);
        assert_eq!(prev(None, 30, true, false), PrevRoute::Socket);
        // no socket: `shuffle prev` while mpd-player is subscribed and has the trail
        assert_eq!(prev(None, 30, false, true), PrevRoute::Daemon);
        assert_eq!(prev(None, 30, false, false), PrevRoute::Mpd);
        // rewinding the playing song comes first, below the threshold the song changes
        assert_eq!(prev(Some(5), 30, true, true), PrevRoute::Rewind);
        assert_eq!(prev(Some(5), 5, false, false), PrevRoute::Rewind);
        assert_eq!(prev(Some(5), 4, true, true), PrevRoute::Socket);
        assert_eq!(prev(Some(5), 4, false, true), PrevRoute::Daemon);
        assert_eq!(prev(Some(5), 4, false, false), PrevRoute::Mpd);
    }

    #[test]
    fn previous_tries_nothing_it_does_not_need() {
        // a rewind sends nothing; a sent `prev` asks MPD nothing
        let r = prev_route(Some(5), 9, || panic!("socket"), || panic!("daemon")).unwrap();
        assert_eq!(r, PrevRoute::Rewind);
        let r = prev_route(None, 9, || true, || panic!("daemon")).unwrap();
        assert_eq!(r, PrevRoute::Socket);
        // the subscription query failing is an error, not a silent fallback
        assert!(prev_route(None, 9, || false, || Err(anyhow::anyhow!("mpd gone"))).is_err());
    }

    #[test]
    fn next_goes_to_the_socket_else_to_mpd() {
        assert_eq!(next_route(|| true), NextRoute::Socket);
        assert_eq!(next_route(|| false), NextRoute::Mpd);
    }

    #[test]
    fn a_send_to_no_socket_fails_at_once() {
        let dir = std::env::temp_dir().join(format!("rormpc-test-sock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("player.sock");
        let _ = std::fs::remove_file(&path);
        #[cfg(unix)]
        assert!(send_transport_to(&path, "next").unwrap_err().contains("no socket"));
        #[cfg(not(unix))]
        assert!(send_transport_to(&path, "next").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_datagram_is_mpd_players_canonical_command() {
        assert_eq!(transport_payload("next"), r#"{"command":"next"}"#);
        assert_eq!(transport_payload("prev"), r#"{"command":"prev"}"#);
    }

    #[test]
    fn a_missing_socket_is_said_once_per_outage() {
        use std::sync::atomic::{AtomicBool, Ordering};
        // tests' socket_path() is in their own temp dir, where nobody listens
        let said = AtomicBool::new(false);
        assert!(!via_socket("next", &said));
        assert!(said.load(Ordering::Relaxed));
        assert!(!via_socket("next", &said));
        assert!(said.load(Ordering::Relaxed));
    }

    #[test]
    fn trail_key_marks_an_mpd_player_that_handles_shuffle_prev() {
        let old: ShuffleState = serde_json::from_str(r#"{"enabled": true, "pid": 7}"#).unwrap();
        assert!(old.trail.is_none());
        let new: ShuffleState =
            serde_json::from_str(r#"{"enabled": true, "trail": {"ids": [3, 9], "cursor": null}}"#)
                .unwrap();
        assert!(new.trail.is_some());
        let list: ShuffleState = serde_json::from_str(r#"{"trail": []}"#).unwrap();
        assert!(list.trail.is_some());
    }
}
