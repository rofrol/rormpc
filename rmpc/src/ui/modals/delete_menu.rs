//! rormpc: the delete menu (Ctrl-x and the context menus). Two independent choices in four items: the file
//! goes to the Trash or is removed for good, and the song's history stays or is deleted (its ListenBrainz
//! listens, irreversibly, plus the video in my YouTube playlists). `musicdb delete --preview` fills in what a
//! deletion would touch in a background thread; `musicdb delete` does the work, also in the background.

use std::{
    process::Command,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use serde::Deserialize;

use super::{
    Modal,
    confirm_modal::{Action, ConfirmModal},
};
use crate::{
    config::keys::CommonAction,
    ctx::Ctx,
    shared::{
        events::AppEvent,
        id::{self, Id},
        keys::ActionEvent,
        macros::{modal, status_error, status_info},
        mouse_event::{MouseEvent, MouseEventKind},
    },
};

const MUSICDB: &str = "musicdb";

#[derive(Debug, Clone, Deserialize)]
struct Playlist {
    title: String,
}

#[derive(Debug, Clone, Deserialize)]
struct YouTube {
    #[serde(default)]
    playlists: Vec<Playlist>,
    #[serde(default)]
    error: Option<String>,
    /// the login expired and the answer comes from yt-playlist's cached index of this time
    #[serde(default)]
    cached_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Preview {
    artist: String,
    title: String,
    #[serde(default)]
    ytid: Option<String>,
    #[serde(default)]
    lb_listens: u32,
    /// the file is still in the music directory
    #[serde(default = "yes")]
    exists: bool,
    /// other library files with the same recording: their listens stay on ListenBrainz
    #[serde(default)]
    shared: Vec<String>,
    /// only from `--preview --youtube`
    #[serde(default)]
    youtube: Option<YouTube>,
}

/// `musicdb delete --preview`'s JSON: `{"version": 1, "songs": [...]}`. Older rormpc-tools printed the
/// bare list; it is read the same way so an older install keeps working.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PreviewOutput {
    Versioned { version: u32, songs: Vec<Preview> },
    Legacy(Vec<Preview>),
}

const PREVIEW_VERSION: u32 = 1;

fn yes() -> bool {
    true
}

fn parse_preview(out: &str) -> Result<Vec<Preview>, String> {
    match serde_json::from_str(out).map_err(|e| format!("musicdb delete --preview: {e}"))? {
        PreviewOutput::Versioned { version: PREVIEW_VERSION, songs } | PreviewOutput::Legacy(songs) => Ok(songs),
        PreviewOutput::Versioned { version, .. } => Err(format!(
            "musicdb delete --preview speaks version {version}, this rormpc reads {PREVIEW_VERSION}: install the \
             matching rormpc-tools ({}) with scripts/rormpc_install.sh companions",
            *crate::shared::dependencies::RORMPC_TOOLS_TAG
        )),
    }
}

#[derive(Debug, Default)]
struct Job {
    songs: Option<Vec<Preview>>,
    youtube_done: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Choice {
    Trash,
    TrashHistory,
    Permanent,
    PermanentHistory,
}

const CHOICES: [Choice; 4] =
    [Choice::Trash, Choice::TrashHistory, Choice::Permanent, Choice::PermanentHistory];

impl Choice {
    fn permanent(self) -> bool {
        matches!(self, Choice::Permanent | Choice::PermanentHistory)
    }

    fn history(self) -> bool {
        matches!(self, Choice::TrashHistory | Choice::PermanentHistory)
    }

    fn args(self, files: &[String]) -> Vec<String> {
        let mut args = vec!["delete".to_owned()];
        if self.permanent() {
            args.push("--permanent".to_owned());
        }
        if self.history() {
            args.push("--listenbrainz".to_owned());
        }
        args.push("--".to_owned());
        args.extend(files.iter().cloned());
        args
    }
}

#[derive(Debug)]
pub struct DeleteMenu {
    popup_area: Option<Rect>,
    id: Id,
    files: Vec<String>,
    sel: usize,
    job: Arc<Mutex<Job>>,
    list_area: ratatui::layout::Rect,
}

fn run(args: &[String]) -> Result<String, String> {
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

fn preview(files: &[String], youtube: bool) -> Result<Vec<Preview>, String> {
    let mut args = vec!["delete".to_owned(), "--preview".to_owned()];
    if youtube {
        args.push("--youtube".to_owned());
    }
    args.push("--".to_owned());
    args.extend(files.iter().cloned());
    run(&args).and_then(|out| parse_preview(&out))
}

/// The local preview first (fast), then again with the live YouTube lookup (a network call).
fn load(job: &Arc<Mutex<Job>>, files: Vec<String>, sender: crossbeam::channel::Sender<AppEvent>) {
    let job = Arc::clone(job);
    std::thread::spawn(move || {
        for youtube in [false, true] {
            let result = preview(&files, youtube);
            {
                let mut j = job.lock().expect("delete menu job lock");
                j.youtube_done = youtube;
                match result {
                    Ok(songs) => j.songs = Some(songs),
                    Err(err) => j.error = Some(err),
                }
            }
            let _ = sender.send(AppEvent::RequestRender);
        }
    });
}

/// Check the preview again (the library may have changed since the menu opened), then delete; the status bar
/// gets musicdb's last line when it has finished, or why it failed.
fn delete(choice: Choice, files: &[String]) {
    match preview(files, false) {
        Err(err) => return status_error!("musicdb delete: {err}; nothing deleted"),
        Ok(songs) => {
            if let Some(gone) = songs.iter().find(|s| !s.exists) {
                return status_error!("{} - {} is no longer in the library; nothing deleted", gone.artist, gone.title);
            }
        }
    }
    match run(&choice.args(files)) {
        Ok(out) => status_info!("{}", out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("musicdb delete: done")),
        Err(err) => status_error!("musicdb delete: {err}"),
    }
}

/// Deletable ListenBrainz listens over all songs (a shared recording keeps its listens).
fn listens(songs: &[Preview]) -> u32 {
    songs.iter().filter(|s| s.shared.is_empty()).map(|s| s.lb_listens).sum()
}

/// One line per song with a video: the playlists it will be removed from, or why that is not known.
fn youtube_lines(songs: &[Preview], done: bool) -> Vec<String> {
    songs
        .iter()
        .filter(|s| s.ytid.is_some())
        .map(|s| {
            let what = match (&s.youtube, done) {
                (Some(YouTube { error: Some(err), .. }), _) => {
                    format!("unknown ({err}); run `yt-playlist auth` in a terminal")
                }
                (Some(yt), _) => {
                    let found = if yt.playlists.is_empty() {
                        "in none of my playlists".to_owned()
                    } else {
                        yt.playlists.iter().map(|p| p.title.as_str()).collect::<Vec<_>>().join(", ")
                    };
                    match &yt.cached_at {
                        Some(at) => format!("{found} (cached {}, login expired)", at.get(..16).unwrap_or(at).replace('T', " ")),
                        None => found,
                    }
                }
                (_, false) => "checking…".to_owned(),
                _ => "unknown".to_owned(),
            };
            format!("{} - {}: {what}", s.artist, s.title)
        })
        .collect()
}

impl DeleteMenu {
    pub fn new(ctx: &Ctx, files: Vec<String>) -> Self {
        let menu = Self {
            popup_area: None,
            id: id::new(),
            files,
            sel: 0,
            job: Arc::new(Mutex::new(Job::default())),
            list_area: ratatui::layout::Rect::default(),
        };
        load(&menu.job, menu.files.clone(), ctx.app_event_sender.clone());
        menu
    }

    fn label(&self, choice: Choice, songs: Option<&[Preview]>) -> String {
        let file = if choice.permanent() { "Delete permanently" } else { "Move to Trash" };
        if !choice.history() {
            return file.to_owned();
        }
        let n = songs.map_or_else(|| "…".to_owned(), |s| listens(s).to_string());
        format!("{file} + delete {n} ListenBrainz listens and YouTube playlist entries…")
    }

    fn choose(&mut self, ctx: &mut Ctx) -> Result<()> {
        let choice = CHOICES[self.sel];
        let (songs, youtube_done) = {
            let j = self.job.lock().expect("delete menu job lock");
            (j.songs.clone(), j.youtube_done)
        };
        let names = songs.as_ref().map_or_else(
            || self.files.join("\n"),
            |s| s.iter().map(|p| format!("{} - {}", p.artist, p.title)).collect::<Vec<_>>().join("\n"),
        );
        let files = self.files.clone();
        let go = move |ctx: &Ctx| -> Result<()> {
            let sender = ctx.app_event_sender.clone();
            std::thread::spawn(move || {
                delete(choice, &files);
                // the groups (Versions, the Queue's ≋) change, also after a partial failure
                crate::ui::rormpc_versions::changed(&sender);
            });
            Ok(())
        };
        if choice == Choice::Trash {
            self.hide(ctx)?;
            return go(ctx);
        }
        let mut message = vec![];
        if choice.history() {
            // the confirmation must name what is deleted, so wait for the lookups
            let Some(songs) = songs.filter(|_| youtube_done) else {
                status_info!("Still checking ListenBrainz and YouTube, try again in a moment");
                return Ok(());
            };
            message.push(format!("{}?\n\n{names}", self.label(choice, Some(&songs)).trim_end_matches('…')));
            message.push(format!(
                "\nListenBrainz: {} listens are deleted for good (this cannot be undone).",
                listens(&songs)
            ));
            let kept: Vec<_> = songs.iter().filter(|s| !s.shared.is_empty()).collect();
            for s in kept {
                message.push(format!(
                    "{} - {}: listens kept, {} has the same recording.",
                    s.artist, s.title, s.shared[0]
                ));
            }
            let yt = youtube_lines(&songs, true);
            if !yt.is_empty() {
                message.push(format!("\nRemoved from my YouTube playlists:\n{}", yt.join("\n")));
            }
        } else {
            message.push(format!("Delete permanently?\n\n{names}"));
        }
        message.push(if choice.permanent() {
            "\nThe file is removed, not moved to the Trash: this cannot be undone.".to_owned()
        } else {
            "\nCtrl-y restores the file, not the deleted history.".to_owned()
        });
        message.push("\nh/l select · Enter activates the selected button · Esc cancels".to_owned());
        let button = match choice {
            Choice::TrashHistory => "Trash + delete history",
            Choice::PermanentHistory => "Delete file + history",
            _ => "Delete permanently",
        };
        self.hide(ctx)?;
        // "Cancel" is the first button, so a stray Enter cancels
        modal!(
            ctx,
            ConfirmModal::builder()
                .ctx(ctx)
                .message(message)
                .action(Action::CustomButtons {
                    buttons: vec![("Cancel", Box::new(|_: &Ctx| Ok(()))), (button, Box::new(go))],
                })
                .build()
        );
        Ok(())
    }
}

impl Modal for DeleteMenu {
    fn id(&self) -> Id {
        self.id
    }

    fn area(&self) -> Option<Rect> {
        self.popup_area
    }

    fn render(&mut self, frame: &mut Frame, ctx: &mut Ctx) -> Result<()> {
        let area = frame.area().centered(Constraint::Percentage(80), Constraint::Percentage(60));
        self.popup_area = Some(area);
        frame.render_widget(Clear, area);
        if let Some(bg) = ctx.config.theme.modal_background_color {
            frame.render_widget(Block::default().style(Style::default().bg(bg)), area);
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(ctx.config.as_border_style())
            .title(" Delete ");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let (songs, youtube_done, error) = {
            let j = self.job.lock().expect("delete menu job lock");
            (j.songs.clone(), j.youtube_done, j.error.clone())
        };
        let label = ctx.config.theme.preview_label_style;
        let dim = Style::default().add_modifier(Modifier::DIM);
        let mut head: Vec<Line> = match &songs {
            Some(s) => s.iter().map(|p| Line::from(format!("{} - {}", p.artist, p.title))).collect(),
            None => self.files.iter().map(|f| Line::from(f.as_str())).collect(),
        };
        head.push(Line::default());
        let [top, list, info, footer] = Layout::vertical([
            Constraint::Length(u16::try_from(head.len()).unwrap_or(u16::MAX)),
            Constraint::Length(4),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(inner);
        frame.render_widget(Paragraph::new(head).wrap(Wrap { trim: false }), top);

        self.list_area = list;
        let items: Vec<Line> = CHOICES
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let text = format!("{} {}", if i == self.sel { "›" } else { " " }, self.label(*c, songs.as_deref()));
                if i == self.sel {
                    Line::from(Span::styled(text, label.add_modifier(Modifier::BOLD)))
                } else {
                    Line::from(text)
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(items), list);

        let mut lines = vec![Line::default()];
        match (&songs, &error) {
            (_, Some(err)) => lines.push(Line::from(Span::styled(format!("musicdb: {err}"), label))),
            (None, None) => lines.push(Line::from("counting ListenBrainz listens…")),
            (Some(s), None) => {
                for p in s.iter().filter(|p| !p.shared.is_empty()) {
                    lines.push(Line::from(format!(
                        "{} - {}: its listens stay, {} has the same recording",
                        p.artist, p.title, p.shared[0]
                    )));
                }
                let yt = youtube_lines(s, youtube_done);
                if !yt.is_empty() {
                    lines.push(Line::from(Span::styled("YouTube playlists (removed only with history):", label)));
                    lines.extend(yt.into_iter().map(Line::from));
                }
            }
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), info);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "j/k move · Enter choose · Esc close · Ctrl-y restores the last trashed file",
                dim,
            ))),
            footer,
        );
        Ok(())
    }

    fn handle_key(&mut self, key: &mut ActionEvent, ctx: &mut Ctx) -> Result<()> {
        let Some(action) = key.claim_common().cloned() else { return Ok(()) };
        match action {
            CommonAction::Down => self.sel = (self.sel + 1).min(CHOICES.len() - 1),
            CommonAction::Up => self.sel = self.sel.saturating_sub(1),
            CommonAction::Confirm => return self.choose(ctx),
            CommonAction::Close => return self.hide(ctx),
            _ => return Ok(()),
        }
        ctx.render()?;
        Ok(())
    }

    fn handle_mouse_event(&mut self, event: MouseEvent, ctx: &mut Ctx) -> Result<()> {
        if self.list_area.contains(event.into()) {
            let idx = usize::from(event.y.saturating_sub(self.list_area.y));
            if idx < CHOICES.len() {
                self.sel = idx;
                if matches!(event.kind, MouseEventKind::DoubleClick) {
                    return self.choose(ctx);
                }
                ctx.render()?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::parse_preview;

    // the shape rormpc-tools' tests/test_rormpc_contract.py checks on its side
    const SONG: &str = r#"{"file": "yt/001--Rick_Astley--dQw4w9WgXcQ--20091025.mp3", "artist": "Rick Astley",
        "title": "Never Gonna Give You Up", "ytid": "dQw4w9WgXcQ", "exists": true, "plays": 3, "lb_listens": 2,
        "shared": []}"#;

    #[test]
    fn reads_the_versioned_preview() {
        let songs = parse_preview(&format!(r#"{{"version": 1, "songs": [{SONG}]}}"#)).unwrap();
        assert_eq!(songs.len(), 1);
        assert_eq!((songs[0].artist.as_str(), songs[0].lb_listens), ("Rick Astley", 2));
        assert_eq!(songs[0].ytid.as_deref(), Some("dQw4w9WgXcQ"));
        assert!(songs[0].youtube.is_none());
    }

    #[test]
    fn reads_the_youtube_lookup_and_its_error() {
        let found = SONG.replace(
            r#""shared": []"#,
            r#""shared": ["cd/01.flac"], "youtube": {"video": "dQw4w9WgXcQ", "playlists":
                [{"id": "PL1", "title": "Favourites", "item": "x"}], "cached_at": "2026-10-06T12:00:00"}"#,
        );
        let failed = SONG.replace(r#""shared": []"#, r#""shared": [], "youtube": {"error": "login expired"}"#);
        let songs = parse_preview(&format!(r#"{{"version": 1, "songs": [{found}, {failed}]}}"#)).unwrap();
        let yt = songs[0].youtube.as_ref().unwrap();
        assert_eq!(yt.playlists[0].title, "Favourites");
        assert_eq!(yt.cached_at.as_deref(), Some("2026-10-06T12:00:00"));
        assert_eq!(songs[0].shared, vec!["cd/01.flac".to_owned()]);
        assert_eq!(songs[1].youtube.as_ref().unwrap().error.as_deref(), Some("login expired"));
    }

    #[test]
    fn reads_the_bare_list_of_older_rormpc_tools() {
        assert_eq!(parse_preview(&format!("[{SONG}]")).unwrap().len(), 1);
    }

    #[test]
    fn refuses_another_version_and_garbage() {
        let err = parse_preview(&format!(r#"{{"version": 2, "songs": [{SONG}]}}"#)).unwrap_err();
        assert!(err.contains("version 2") && err.contains("rormpc_install.sh"), "{err}");
        assert!(parse_preview("usage: musicdb [-h]").is_err());
    }
}
