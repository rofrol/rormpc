//! rormpc: "Pause for…": mpd-player (rormpc-tools) pauses MPD and plays on at a wall-clock deadline, also with
//! rormpc closed. Commands go as `pause start|extend|resume|cancel` on the "rormpc" channel; pause.json (written only
//! by mpd-player) holds the deadline, which the volume slider shows as a countdown.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rmpc_mpd::mpd_client::MpdClient;
use serde::Deserialize;

use crate::{
    ctx::Ctx,
    shared::macros::{modal, status_error, status_info, status_warn},
    ui::{
        modals::{input_modal::InputModal, menu::modal::MenuModal},
        rormpc_player::{CHANNEL, state_path},
    },
};

/// how long to wait for mpd-player to write pause.json after a command (it answers within milliseconds when it runs)
const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);
/// longest pause that can be asked for (mpd-player refuses more)
const MAX_SECONDS: u64 = 24 * 3600;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PauseState {
    /// wall-clock Unix time when playback goes on; None: no timed pause by mpd-player
    #[serde(default)]
    pub deadline: Option<f64>,
    /// +1 for every command mpd-player handled
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub error: Option<String>,
}

/// pause.json, read again only when the file changed (the slider looks at it on every render).
pub fn pause_state() -> PauseState {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<(Option<SystemTime>, PauseState)>> = OnceLock::new();
    let p = state_path("pause");
    let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
    let cache = CACHE.get_or_init(|| Mutex::new((None, PauseState::default())));
    let Ok(mut c) = cache.lock() else { return PauseState::default() };
    if c.0 != mtime || mtime.is_none() {
        c.1 = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        c.0 = mtime;
    }
    c.1.clone()
}

/// Seconds until playback goes on (0 once the deadline passed and mpd-player has not answered yet), or None
/// without a timed pause.
pub fn remaining() -> Option<f64> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs_f64();
    pause_state().deadline.map(|d| (d - now).max(0.0))
}

/// 12:34, or 1:02:03 from an hour on (rounded up, so it never shows 0:00 while still paused).
pub fn fmt_remaining(secs: f64) -> String {
    let s = secs.ceil() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// "5" (minutes), "2.5", "90s", "5m", "1h", "1h30" (minutes after the hours), "1h30m", "1:30" (h:mm).
pub fn parse_duration(text: &str) -> Option<u64> {
    let t = text.trim().to_lowercase().replace(',', ".").replace(' ', "");
    if t.is_empty() {
        return None;
    }
    let secs = if let Some((h, m)) = t.split_once(':') {
        let m: u64 = m.parse().ok().filter(|m| *m < 60)?;
        h.parse::<u64>().ok()? * 3600 + m * 60
    } else if let Some(s) = t.strip_suffix('s') {
        s.parse::<f64>().ok()?.round() as u64
    } else if let Some((h, rest)) = t.split_once('h') {
        let h: f64 = h.parse().ok()?;
        let m: f64 = match rest.strip_suffix('m').unwrap_or(rest) {
            "" => 0.0,
            m => m.parse().ok()?,
        };
        (h * 3600.0 + m * 60.0).round() as u64
    } else {
        t.strip_suffix('m').unwrap_or(&t).parse::<f64>().ok().filter(|m| m.is_finite() && *m >= 0.0).map(|m| (m * 60.0).round() as u64)?
    };
    (1..=MAX_SECONDS).contains(&secs).then_some(secs)
}

/// "5 min", "1 h 30 min", "90 s".
fn fmt_duration(secs: u64) -> String {
    match (secs / 3600, secs / 60 % 60, secs % 60) {
        (0, 0, s) => format!("{s} s"),
        (0, m, 0) => format!("{m} min"),
        (h, 0, 0) => format!("{h} h"),
        (h, m, 0) => format!("{h} h {m} min"),
        _ => format!("{} min {} s", secs / 60, secs % 60),
    }
}

/// Send one `pause …` command, then report once mpd-player handled it (pause.json's generation moved on): its error,
/// or `ok`.
fn send(ctx: &Ctx, msg: String, ok: impl Fn(&PauseState) -> String + Send + 'static) {
    let before = pause_state().generation;
    ctx.command(move |_, client| {
        if !client.channels()?.0.iter().any(|c| c == CHANNEL) {
            status_error!("mpd-player is not running (rormpc_install.sh companions starts it): not paused");
            return Ok(());
        }
        client.send_message(CHANNEL, &msg)?;
        std::thread::spawn(move || {
            let start = Instant::now();
            loop {
                let st = pause_state();
                if st.generation != before {
                    if let Some(e) = &st.error {
                        status_warn!("Pause: {e}");
                    } else {
                        status_info!("{}", ok(&st));
                    }
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

fn start(ctx: &Ctx, secs: u64) {
    send(ctx, format!("pause start {secs}"), move |_| format!("Paused for {}: plays on at the end", fmt_duration(secs)));
}

fn extend(ctx: &Ctx, secs: u64) {
    send(ctx, format!("pause extend {secs}"), |_| {
        format!("Still paused: plays on in {}", remaining().map_or_else(String::new, fmt_remaining))
    });
}

/// "Pause for…" (presets and a custom duration), or while paused for a while: play now, longer, or stay paused
/// without a timer.
pub fn open_pause_menu(ctx: &Ctx) {
    let menu = if let Some(left) = remaining() {
        MenuModal::new(ctx)
            .width(50)
            .list_section(ctx, move |mut section| {
                section.add_item(format!("Paused: plays on in {}", fmt_remaining(left)), |_| Ok(()));
                Some(section)
            })
            .list_section(ctx, |mut section| {
                section.add_item("Play now", |ctx| {
                    send(ctx, "pause resume".to_owned(), |_| "Playing on".to_owned());
                    Ok(())
                });
                for min in [5, 15, 30] {
                    section.add_item(format!("+{min} min"), move |ctx| {
                        extend(ctx, min * 60);
                        Ok(())
                    });
                }
                section.add_item("Cancel timer, stay paused", |ctx| {
                    send(ctx, "pause cancel".to_owned(), |_| "Timer cancelled: stays paused".to_owned());
                    Ok(())
                });
                Some(section)
            })
    } else {
        MenuModal::new(ctx)
            .width(50)
            .list_section(ctx, |mut section| {
                section.add_item("Pause for… (then plays on by itself)", |_| Ok(()));
                Some(section)
            })
            .list_section(ctx, |mut section| {
                for min in [1, 5, 15, 30, 60] {
                    section.add_item(fmt_duration(min * 60), move |ctx| {
                        start(ctx, min * 60);
                        Ok(())
                    });
                }
                section.add_item("Other…", |ctx| {
                    modal!(
                        ctx,
                        InputModal::new(ctx)
                            .title("Pause for")
                            .input_label("Minutes, or 90s, 1h30, 1:30:")
                            .confirm_label("Pause")
                            .on_confirm(|ctx, value| {
                                if let Some(secs) = parse_duration(value) {
                                    start(ctx, secs);
                                } else {
                                    status_warn!("Not a duration from 1 s to 24 h: {value}");
                                }
                                Ok(())
                            })
                    );
                    Ok(())
                });
                Some(section)
            })
    };
    let menu = menu.list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(())))).build();
    modal!(ctx, menu);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("5"), Some(300));
        assert_eq!(parse_duration("2,5"), Some(150));
        assert_eq!(parse_duration("90s"), Some(90));
        assert_eq!(parse_duration("5m"), Some(300));
        assert_eq!(parse_duration("1h"), Some(3600));
        assert_eq!(parse_duration("1h30"), Some(5400));
        assert_eq!(parse_duration("1h30m"), Some(5400));
        assert_eq!(parse_duration(" 1:30 "), Some(5400));
        assert_eq!(parse_duration("0"), None);
        assert_eq!(parse_duration("25h"), None);
        assert_eq!(parse_duration("1:75"), None);
        assert_eq!(parse_duration("soon"), None);
        assert_eq!(parse_duration("-5"), None);
    }

    #[test]
    fn countdown() {
        assert_eq!(fmt_remaining(754.2), "12:35");
        assert_eq!(fmt_remaining(0.1), "0:01");
        assert_eq!(fmt_remaining(3723.0), "1:02:03");
        assert_eq!(fmt_duration(5400), "1 h 30 min");
        assert_eq!(fmt_duration(90), "1 min 30 s");
    }
}
