//! rormpc: the scrobbler's view of the playing song. `ro-listenbrainz-mpd`
//! writes `status.json` next to its `listens.jsonl` on each change (song, play
//! state, seek, sent, a manual request's answer); rormpc only shows it
//! (the listen rule lives in the scrobbler's config, never here) and adds MPD's
//! elapsed time since the status was written. "Send to `ListenBrainz` now" is
//! `submit <instance> <play>` on the scrobbler's MPD channel; the status file's
//! `manual` field is the answer.

use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime},
};

use anyhow::Result;
use crossbeam::channel::Sender;
use notify_debouncer_full::notify::{self, RecommendedWatcher, RecursiveMode, Watcher};
use rmpc_mpd::{commands::State, mpd_client::MpdClient};
use serde::Deserialize;

use crate::{
    ctx::Ctx,
    shared::{
        events::AppEvent,
        macros::{modal, status_error, status_info},
    },
    ui::modals::confirm_modal::{Action, ConfirmModal},
};

/// The scrobbler's channel for manual sends.
pub const CHANNEL: &str = "listenbrainz_listen";
/// The `status.json` format this build reads.
const VERSION: u32 = 1;

/// The scrobbler's `status.json`: upstream's data directory, `listenbrainz-mpd`
/// (the fork keeps it).
pub fn status_path() -> PathBuf {
    #[cfg(test)]
    let base = std::env::temp_dir().join(format!("rormpc-test-scrobbler-{}", std::process::id()));
    #[cfg(all(not(test), target_os = "macos"))]
    let base = PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join("Library/Application Support");
    #[cfg(all(not(test), not(target_os = "macos")))]
    let base = std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share")
    });
    base.join("listenbrainz-mpd/status.json")
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rule {
    pub fraction: f64,
    #[serde(default)]
    pub max_s: Option<f64>,
    #[serde(default)]
    pub uninterrupted: bool,
}

/// The answer to the last manual send.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Answer {
    /// counts the answers, so a new one differs from an earlier one with the
    /// same content
    pub n: u64,
    #[serde(default)]
    pub play: Option<u64>,
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScrobbleStatus {
    pub version: u32,
    #[cfg_attr(test, allow(dead_code))] // the process check is left out of tests
    pub pid: u32,
    /// this scrobbler run
    pub instance: String,
    /// the play within the run (a new song, a repeat or playing again after a
    /// stop is a new play)
    pub play: u64,
    #[serde(default)]
    pub id: Option<u32>,
    #[serde(default)]
    pub duration_s: Option<f64>,
    pub rule: Rule,
    /// playtime the listen needs (None: it never counts)
    #[serde(default)]
    pub required_s: Option<f64>,
    /// where the current stretch without a seek began
    #[serde(default)]
    pub segment_start_s: f64,
    /// the playtime counted at `position_s`
    #[serde(default)]
    pub position_s: f64,
    #[serde(default)]
    pub counted_s: f64,
    /// "counting", "impossible", "never" or "sent"
    pub listen: String,
    #[serde(default)]
    pub manual: Option<Answer>,
}

type Cache = Mutex<(Option<SystemTime>, Option<ScrobbleStatus>)>;
static CACHE: OnceLock<Cache> = OnceLock::new();
/// The manual send waiting for its answer: the instance, the play and the
/// answer count before it.
static PENDING: Mutex<Option<(String, u64, u64)>> = Mutex::new(None);

/// status.json, cached until its mtime changes or its watcher invalidates it.
pub fn status() -> Option<ScrobbleStatus> {
    let p = status_path();
    let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
    let cache = CACHE.get_or_init(|| Mutex::new((None, None)));
    let Ok(mut c) = cache.lock() else { return None };
    if c.0 != mtime || mtime.is_none() {
        c.1 = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(&t).ok());
        c.0 = mtime;
    }
    c.1.clone()
}

/// Watch the scrobbler's directory, not the replaced inode: a change while
/// paused (a manual send's answer) must redraw too.
pub fn watch(tx: Sender<AppEvent>) -> Result<RecommendedWatcher> {
    let path = status_path();
    let parent =
        path.parent().ok_or_else(|| anyhow::anyhow!("Scrobbler status has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    // notify reports canonical paths (on macOS /tmp is /private/tmp); compare
    // in the same namespace.
    let parent = std::fs::canonicalize(parent)?;
    let target = parent.join("status.json");
    let mut watcher =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            Ok(event)
                if event.need_rescan()
                    || event.paths.is_empty()
                    || event.paths.iter().any(|p| p == &target) =>
            {
                if let Some(cache) = CACHE.get()
                    && let Ok(mut cache) = cache.lock()
                {
                    *cache = (None, None); // a deleted file also has mtime None
                }
                report_answer(status().as_ref());
                let _ = tx.send(AppEvent::RequestRender);
            }
            Ok(_) => {}
            Err(err) => log::warn!(error:? = err; "Scrobbler status watcher failed"),
        })?;
    watcher.watch(&parent, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

/// The answer to the pending manual send, once the status file has it.
fn answer_for<'a>(st: &'a ScrobbleStatus, pending: &(String, u64, u64)) -> Option<&'a Answer> {
    let (instance, _, before) = pending;
    st.manual.as_ref().filter(|a| st.instance == *instance && a.n > *before)
}

fn report_answer(st: Option<&ScrobbleStatus>) {
    let Some(st) = st else { return };
    let Ok(mut pending) = PENDING.lock() else { return };
    let Some(answer) = pending.as_ref().and_then(|p| answer_for(st, p)) else { return };
    if answer.ok {
        status_info!("Sent to ListenBrainz");
    } else {
        status_error!("Not sent to ListenBrainz: {}", answer.error.as_deref().unwrap_or("refused"));
    }
    *pending = None;
}

/// "1:23"
fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0).ceil() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// What the listen needs: "50%" of the song, or the playtime when the maximum
/// is less than the fraction (or the duration is unknown).
fn needs(st: &ScrobbleStatus, required: f64) -> String {
    match st.duration_s {
        Some(d) if d > 0.0 && st.rule.max_s.is_none_or(|m| m >= st.rule.fraction * d) => {
            format!("{:.0}%", st.rule.fraction * 100.0)
        }
        _ => clock(required),
    }
}

/// The status line text for the playing song, or None when there is nothing to
/// say (stopped, the scrobbler has not seen this song yet, a status file of
/// another format).
pub fn describe(
    st: &ScrobbleStatus,
    song_id: Option<u32>,
    state: State,
    elapsed: Duration,
) -> Option<String> {
    if st.version != VERSION || state == State::Stop || song_id.is_none() || st.id != song_id {
        return None;
    }
    if pending_play().is_some_and(|(instance, play)| instance == st.instance && play == st.play) {
        return Some("sending to ListenBrainz…".to_owned());
    }
    match st.listen.as_str() {
        "sent" => Some("scrobbled ✓".to_owned()),
        "never" => Some("no scrobble: unknown length".to_owned()),
        "impossible" => {
            let required = st.required_s?;
            let at = st.duration_s.filter(|d| *d > 0.0).map_or_else(
                || clock(st.segment_start_s),
                |d| format!("{:.0}%", st.segment_start_s / d * 100.0),
            );
            let how = if st.rule.uninterrupted { " uninterrupted" } else { "" };
            Some(format!("no scrobble: seeked to {at} (needs {}{how})", needs(st, required)))
        }
        "counting" => {
            let required = st.required_s?;
            let counted =
                (st.counted_s + (elapsed.as_secs_f64() - st.position_s).max(0.0)).min(required);
            let progress = if required > 0.0 { counted / required * 100.0 } else { 100.0 };
            Some(format!(
                "scrobble in {} · {:.0}% of {}",
                clock(required - counted),
                progress.floor(),
                needs(st, required)
            ))
        }
        _ => None,
    }
}

fn pending_play() -> Option<(String, u64)> {
    PENDING.lock().ok()?.as_ref().map(|(instance, play, _)| (instance.clone(), *play))
}

/// The Scrobble status property: `describe` for a scrobbler that runs;
/// "scrobbler not running" when its process ended; nothing without a status
/// file (no scrobbler on this machine).
pub fn line(ctx: &Ctx) -> Option<String> {
    let st = status()?;
    #[cfg(not(test))]
    if !crate::ui::rormpc_process::daemon_alive(
        "scrobbler",
        st.pid,
        &st.instance,
        &ctx.app_event_sender,
    ) {
        return (ctx.status.state != State::Stop).then(|| "scrobbler not running".to_owned());
    }
    describe(&st, ctx.status.songid, ctx.status.state, ctx.status.elapsed)
}

/// "Send to `ListenBrainz` now": the playing song's listen goes out now, once;
/// asks first when the rule is not met.
pub fn send_now(ctx: &Ctx) {
    let Some(st) = status() else {
        return status_error!(
            "No scrobbler status: is ro-listenbrainz-mpd running (and new enough)?"
        );
    };
    if ctx.status.state == State::Stop || ctx.status.songid.is_none() {
        return status_info!("No song is playing");
    }
    if st.version != VERSION || st.id != ctx.status.songid {
        return status_error!("The scrobbler has not caught up with the playing song");
    }
    let summary =
        describe(&st, ctx.status.songid, ctx.status.state, ctx.status.elapsed).unwrap_or_default();
    let why = match summary.strip_prefix("no scrobble: ") {
        Some(reason) => format!("The scrobbler will not send this song: {reason}."),
        None => format!("The scrobbler has not sent this song yet: {summary}."),
    };
    let request = (st.instance.clone(), st.play, st.manual.as_ref().map_or(0, |a| a.n));
    match st.listen.as_str() {
        "sent" => status_info!("Already scrobbled"),
        "counting" | "impossible" | "never" => {
            let go = move |ctx: &Ctx| {
                send(ctx, request.clone());
                Ok(())
            };
            modal!(
                ctx,
                ConfirmModal::builder()
                    .ctx(ctx)
                    .message(vec![format!("{why}\nSend it to ListenBrainz now anyway?")])
                    .action(Action::CustomButtons {
                        buttons: vec![
                            ("Cancel", Box::new(|_: &Ctx| Ok(()))),
                            ("Send", Box::new(go))
                        ]
                    })
                    .build()
            );
        }
        other => status_error!("Unknown scrobbler state: {other}"),
    }
}

fn send(ctx: &Ctx, request: (String, u64, u64)) {
    ctx.command(move |_, client| {
        if !client.channels()?.0.iter().any(|c| c == CHANNEL) {
            status_error!(
                "The scrobbler does not take manual sends: not running, or older than this rormpc"
            );
            return Ok(());
        }
        let message = format!("submit {} {}", request.0, request.1);
        if let Ok(mut pending) = PENDING.lock() {
            *pending = Some(request); // before sending: the answer can arrive before send_message returns
        }
        client.send_message(CHANNEL, &message)?;
        Ok(())
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// 200 s song, 50% (100 s) needed, uninterrupted, counted 30 s at 30 s.
    fn counting() -> ScrobbleStatus {
        serde_json::from_str(
            r#"{"version": 1, "pid": 1, "instance": "1-2", "play": 3, "id": 7, "file": "a.mp3",
                "duration_s": 200.0, "state": "play",
                "rule": {"fraction": 0.5, "max_s": null, "uninterrupted": true},
                "required_s": 100.0, "segment_start_s": 0.0, "position_s": 30.0, "counted_s": 30.0,
                "listen": "counting", "sent": null, "manual": null, "updated_at": 1.0}"#,
        )
        .unwrap()
    }

    fn at(st: &ScrobbleStatus, elapsed: u64) -> Option<String> {
        describe(st, Some(7), State::Play, Duration::from_secs(elapsed))
    }

    #[test]
    fn countdown_and_progress_toward_the_rule() {
        let st = counting();
        assert_eq!(at(&st, 30).as_deref(), Some("scrobble in 1:10 · 30% of 50%"));
        // MPD's elapsed moves on between status writes
        assert_eq!(at(&st, 92).as_deref(), Some("scrobble in 0:08 · 92% of 50%"));
        assert_eq!(at(&st, 150).as_deref(), Some("scrobble in 0:00 · 100% of 50%"));
        // an elapsed behind the status (a seek the scrobbler has not reported
        // yet) never counts backwards
        assert_eq!(at(&st, 10).as_deref(), Some("scrobble in 1:10 · 30% of 50%"));
    }

    #[test]
    fn a_maximum_below_the_fraction_shows_the_playtime() {
        let mut st = counting();
        st.rule.max_s = Some(60.0);
        st.required_s = Some(60.0);
        assert_eq!(at(&st, 30).as_deref(), Some("scrobble in 0:30 · 50% of 1:00"));
        st.duration_s = None;
        assert_eq!(at(&st, 30).as_deref(), Some("scrobble in 0:30 · 50% of 1:00"));
    }

    #[test]
    fn seek_too_late_says_why_there_is_no_scrobble() {
        let mut st = counting();
        st.listen = "impossible".to_owned();
        st.segment_start_s = 140.0;
        assert_eq!(
            at(&st, 141).as_deref(),
            Some("no scrobble: seeked to 70% (needs 50% uninterrupted)")
        );
        st.rule.uninterrupted = false;
        assert_eq!(at(&st, 141).as_deref(), Some("no scrobble: seeked to 70% (needs 50%)"));
    }

    #[test]
    fn sent_never_and_another_song() {
        let mut st = counting();
        st.listen = "sent".to_owned();
        assert_eq!(at(&st, 120).as_deref(), Some("scrobbled ✓"));
        st.listen = "never".to_owned();
        assert_eq!(at(&st, 1).as_deref(), Some("no scrobble: unknown length"));
        // the scrobbler has not seen the new song yet, MPD is stopped, or the
        // file is of another format
        assert_eq!(describe(&st, Some(8), State::Play, Duration::ZERO), None);
        assert_eq!(describe(&st, Some(7), State::Stop, Duration::ZERO), None);
        st.version = 2;
        assert_eq!(at(&st, 1), None);
    }

    #[test]
    fn a_manual_send_waits_for_its_own_answer() {
        let mut st = counting();
        st.manual =
            Some(Answer { n: 4, play: Some(3), ok: false, error: Some("already sent".to_owned()) });
        let pending = ("1-2".to_owned(), 3, 4);
        assert_eq!(answer_for(&st, &pending), None, "an earlier answer with the same content");
        st.manual.as_mut().unwrap().n = 5;
        assert_eq!(
            answer_for(&st, &pending).map(|a| a.error.as_deref()),
            Some(Some("already sent"))
        );
        st.instance = "9-9".to_owned();
        assert_eq!(answer_for(&st, &pending), None, "another scrobbler run");
    }
}
