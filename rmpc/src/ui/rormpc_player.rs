//! rormpc: talking to mpd-player (rormpc-tools), the playback daemon that runs with rormpc closed too. Commands go
//! over MPD's client-to-client messages on channel "rormpc" (`gap set 5`); the daemon alone writes its state to
//! `$XDG_STATE_HOME/rormpc/<module>.json`, which rormpc reads to show it. A command counts as done only when that
//! file shows the new value: an MPD message has no reply, and the daemon may not be running.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use rmpc_mpd::mpd_client::MpdClient;
use serde::Deserialize;

use crate::{
    ctx::Ctx,
    shared::macros::{modal, status_error, status_info, status_warn},
    ui::modals::{input_modal::InputModal, menu::modal::MenuModal},
};

pub const CHANNEL: &str = "rormpc";
/// how long to wait for mpd-player to write its state after a command: it answers within milliseconds when it
/// runs; this only bounds the wait when it does not
const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);

pub fn state_path(module: &str) -> PathBuf {
    let base = std::env::var("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state")
    });
    base.join(format!("rormpc/{module}.json"))
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
    #[serde(default)]
    pub nominee: Option<Nominee>,
    #[serde(default)]
    pub round: Option<Round>,
    #[serde(default)]
    pub cooldown: std::collections::HashMap<String, Cooldown>,
    #[serde(default)]
    pub plan: Vec<Planned>,
}

/// shuffle.json, read again only when the file changed (it is looked at while rendering).
pub fn shuffle_state() -> ShuffleState {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<(Option<std::time::SystemTime>, ShuffleState)>> = OnceLock::new();
    let p = state_path("shuffle");
    let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
    let cache = CACHE.get_or_init(|| Mutex::new((None, ShuffleState::default())));
    let Ok(mut c) = cache.lock() else { return ShuffleState::default() };
    if c.0 != mtime || mtime.is_none() {
        c.1 = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        c.0 = mtime;
    }
    c.1.clone()
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

/// A queued song's turn: 0.. for the Up next requests (in order), then 1000 + k for the weighted shuffle's plan.
pub fn next_rank(id: u32) -> Option<usize> {
    if let Some(k) = crate::ui::rormpc_upnext::up_next_ids().iter().position(|i| *i == id) {
        return Some(k);
    }
    let sh = shuffle_state();
    (sh.enabled && sh.active).then(|| sh.plan.iter().position(|p| p.id == id).map(|k| 1000 + k)).flatten()
}

/// The ShuffleNext column: "↑1" for the first Up next request, "1".."10" for the shuffle's plan.
pub fn next_marker(id: u32) -> Option<String> {
    next_rank(id).map(|r| if r >= 1000 { (r - 999).to_string() } else { format!("↑{}", r + 1) })
}

/// The ShuffleNext marker of a library file (Hits rows know files, not queue ids): its first queue entry's turn.
pub fn next_marker_for_file(ctx: &crate::ctx::Ctx, file: &str) -> Option<String> {
    ctx.queue.iter().filter(|s| s.file == file).find_map(|s| next_marker(s.id))
}
