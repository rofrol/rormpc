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
