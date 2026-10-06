//! rormpc: lyrics from `musicdb lyrics` (rormpc-tools, LRCLIB). Synced lyrics are ordinary `.lrc` files under
//! `lyrics_dir`, which upstream's Lyrics pane already reads. This adds what upstream lacks: plain lyrics (`.txt`
//! next to where the `.lrc` would be), scrolled along with the song; a note when there are none (instrumental,
//! nothing on LRCLIB, not checked yet) taken from `<lyrics_dir>/index.json`; and "Choose lyrics…", a menu of the
//! LRCLIB entries for a song.

use std::{collections::HashMap, process::Command};

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Text},
    widgets::{Paragraph, Wrap},
};
use serde::Deserialize;

use crate::{
    ctx::Ctx,
    shared::{
        events::AppEvent,
        macros::{modal, status_error, status_info},
    },
    ui::{UiEvent, modals::menu::modal::MenuModal},
};

const MUSICDB: &str = "musicdb";

/// What to show when the current song has no `.lrc`.
#[derive(Debug, Clone)]
pub enum Fallback {
    Plain(Vec<String>),
    Note(String),
}

#[derive(Debug, Deserialize)]
struct IndexEntry {
    state: String,
}

/// Plain lyrics or a note for the playing song; None without lyrics_dir or a song.
pub fn fallback(ctx: &Ctx) -> Option<Fallback> {
    let song = ctx.current_song()?;
    let dir = ctx.config.lyrics_dir.as_ref()?;
    let txt = crate::shared::lrc::get_lrc_path(dir, &song.file).ok()?.with_extension("txt");
    if let Ok(text) = std::fs::read_to_string(&txt) {
        return Some(Fallback::Plain(text.lines().map(str::to_owned).collect()));
    }
    let index: HashMap<String, IndexEntry> = std::fs::read_to_string(std::path::Path::new(dir).join("index.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let note = match index.get(&song.file).map(|e| e.state.as_str()) {
        Some("instrumental") => "Instrumental",
        Some("none") => "No lyrics on LRCLIB within 2 s of this file's length\n\nQueue menu: Choose lyrics…",
        Some("untagged") => "No artist/title tags to look the lyrics up by",
        Some(_) => return None, // synced: the .lrc is being (re)loaded
        None => "Lyrics not checked yet\n\nQueue menu: Choose lyrics…",
    };
    Some(Fallback::Note(note.to_owned()))
}

/// Plain lyrics scrolled in proportion to the song's progress (they have no timestamps); a note centred.
pub fn render(frame: &mut Frame, area: Rect, ctx: &Ctx, fallback: &Fallback) {
    match fallback {
        Fallback::Plain(lines) => {
            let text: Vec<Line> = lines.iter().map(|l| Line::from(l.clone())).collect();
            let wrapped = lines
                .iter()
                .map(|l| textwrap::wrap(l, usize::from(area.width).max(1)).len().max(1))
                .sum::<usize>();
            let hidden = wrapped.saturating_sub(usize::from(area.height));
            let total = ctx.status.duration.as_secs_f64();
            let progress = if total > 0.0 { (ctx.status.elapsed.as_secs_f64() / total).clamp(0.0, 1.0) } else { 0.0 };
            let scroll = (hidden as f64 * progress) as u16;
            let paragraph = Paragraph::new(Text::from(text))
                .alignment(Alignment::Center)
                .style(ctx.config.as_text_style())
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0));
            frame.render_widget(paragraph, area);
        }
        Fallback::Note(note) => {
            let lines: Vec<Line> = note.lines().map(|l| Line::from(l.to_owned())).collect();
            let top = area.height.saturating_sub(lines.len() as u16) / 2;
            let area = Rect { y: area.y + top, height: area.height.saturating_sub(top), ..area };
            let paragraph = Paragraph::new(Text::from(lines))
                .alignment(Alignment::Center)
                .style(Style::default().add_modifier(Modifier::DIM))
                .wrap(Wrap { trim: false });
            frame.render_widget(paragraph, area);
        }
    }
}

#[derive(Debug, Deserialize)]
struct Candidates {
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    id: u64,
    #[serde(default)]
    artist: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    album: Option<String>,
    delta: f64,
    synced: bool,
    plain: bool,
    instrumental: bool,
}

fn run(args: &[&str]) -> Result<String, String> {
    match Command::new(MUSICDB).args(args).output() {
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => Err(String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("musicdb failed")
            .to_owned()),
        Err(err) => Err(crate::shared::dependencies::cannot_run(MUSICDB, &err)),
    }
}

/// Run `musicdb lyrics …` in the background, report its last line, then make the Lyrics pane reload.
fn run_then_reload(ctx: &Ctx, args: Vec<String>) {
    let sender = ctx.app_event_sender.clone();
    std::thread::spawn(move || {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        match run(&args) {
            Ok(out) => status_info!("{}", out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("lyrics updated")),
            Err(err) => status_error!("musicdb lyrics: {err}"),
        }
        let _ = sender.send(AppEvent::UiEvent(UiEvent::LyricsIndexed));
    });
}

/// "Choose lyrics…": the LRCLIB entries for a song, closest length first. Looking them up takes a moment
/// (one LRCLIB search), so the menu opens after it.
pub fn open_chooser(ctx: &Ctx, file: String) {
    let found = run(&["lyrics", "candidates", &file])
        .and_then(|out| serde_json::from_str::<Candidates>(&out).map_err(|e| e.to_string()));
    let candidates = match found {
        Ok(c) => c.candidates,
        Err(err) => return status_error!("musicdb lyrics: {err}"),
    };
    let auto = file.clone();
    let menu = MenuModal::new(ctx)
        .width(90) // "synced  -8 s  Artist - Title  (Album)" needs room
        .list_section(ctx, move |mut section| {
            if candidates.is_empty() {
                section.add_item("LRCLIB has nothing under this artist and title", |_| Ok(()));
            }
            for c in candidates.into_iter().take(15) {
                let kind = if c.synced { "synced" } else if c.instrumental { "instrumental" } else if c.plain { "plain" } else { "empty" };
                let label = format!(
                    "{kind:<12} {:>+4.0} s  {} - {}{}",
                    c.delta,
                    c.artist.unwrap_or_default(),
                    c.title.unwrap_or_default(),
                    c.album.map(|a| format!("  ({a})")).unwrap_or_default()
                );
                let (file, id) = (file.clone(), c.id.to_string());
                section.add_item(label, move |ctx| {
                    run_then_reload(ctx, vec!["lyrics".into(), "use".into(), file, id]);
                    Ok(())
                });
            }
            Some(section)
        })
        .list_section(ctx, move |mut section| {
            section.add_item("Fetch automatically (closest length)", move |ctx| {
                run_then_reload(ctx, vec!["lyrics".into(), "fetch".into(), auto]);
                Ok(())
            });
            Some(section)
        })
        .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
        .build();
    modal!(ctx, menu);
}
