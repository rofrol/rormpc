//! rormpc: Deleted pane. The songs deleted with Ctrl-x (`musicdb deletions --json --all`): when, how (Trash or
//! permanent, history kept or deleted), what happened to their ListenBrainz listens and YouTube playlist entries,
//! and failed steps. Enter or the context menu restores a song still in the Trash (`musicdb undo --id`) or
//! retries failed steps. Nothing here deletes: the irreversible actions stay in the Ctrl-x menu.
//! A deleted song is never downloaded again (rormpc-tools `deleted.py`): the Download column says "blocked", and the
//! menu allows a re-download (`musicdb deletions allow ID`) or blocks it again.
//! Music shows the pane as an overlay (`gd`, or its `Deleted ! N` badge while steps failed or are unresolved);
//! `Pane(Deleted())` stays loadable as a tab in explicit configs.

use std::{
    process::Command,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    prelude::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState, Wrap},
};
use serde::Deserialize;

use super::Pane;
use crate::{
    config::keys::CommonAction,
    ctx::Ctx,
    shared::{
        events::AppEvent,
        keys::ActionEvent,
        macros::{modal, status_error, status_info, status_warn},
        mouse_event::{MouseEvent, MouseEventKind},
    },
    ui::{UiEvent, dirstack::DirState, modals::menu::modal::MenuModal},
};

const MUSICDB: &str = "musicdb";

#[derive(Debug, Clone, Deserialize)]
struct Deleted {
    id: String,
    file: String,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    title: String,
    mode: String,
    history: String,
    deleted_at: String,
    #[serde(default)]
    restorable: bool,
    #[serde(default)]
    plays: u32,
    #[serde(default)]
    lb_listens: u32,
    #[serde(default)]
    lb_deleted: u32,
    #[serde(default)]
    youtube_removed: Vec<String>,
    #[serde(default)]
    ytid: Option<String>,
    #[serde(default)]
    ops: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    error: Option<String>,
    /// how the deletion gates the downloaders; missing from an older musicdb
    #[serde(default)]
    download: Option<Download>,
}

#[derive(Debug, Clone, Deserialize)]
struct Download {
    /// "blocked" or "allowed"
    state: String,
    /// what it matches: the video, the recording, the chart song (main artist|title)
    #[serde(default)]
    ytid: Option<String>,
    #[serde(default)]
    mbid: Option<String>,
    #[serde(default)]
    chart_key: Option<String>,
}

impl Deleted {
    fn blocked(&self) -> Option<bool> {
        self.download.as_ref().map(|d| d.state == "blocked")
    }

    /// A step failed or is unresolved: an error, or a step whose outcome says "failed".
    fn needs_attention(&self) -> bool {
        self.error.is_some() || self.ops.values().any(|o| o.starts_with("failed"))
    }
}

/// State shared with the background `musicdb` runs.
#[derive(Debug, Default)]
struct Job {
    loading: bool,
    /// a newer result to take on the next render
    rows: Option<Vec<Deleted>>,
    error: Option<String>,
    /// a restore or retry finished: load again
    reload: bool,
}

#[derive(Debug)]
pub struct DeletedPane {
    rows: Vec<Deleted>,
    state: DirState<TableState>,
    table_area: Rect,
    job: Arc<Mutex<Job>>,
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

impl DeletedPane {
    pub fn new() -> Self {
        // the pane is built at startup: say once if deletion cleanup steps are failing, even when it is not shown
        std::thread::spawn(|| {
            #[derive(Deserialize)]
            struct Pending {
                #[serde(default)]
                error: Option<String>,
            }
            let failed = run(&["deletions", "--json"])
                .ok()
                .and_then(|out| serde_json::from_str::<Vec<Pending>>(&out).ok())
                .map_or(0, |rows| rows.iter().filter(|r| r.error.is_some()).count());
            if failed > 0 {
                status_warn!("cleanup: {failed} deletions have failed steps (gd in Music; retried hourly)");
            }
        });
        Self {
            rows: Vec::new(),
            state: DirState::default(),
            table_area: Rect::default(),
            job: Arc::new(Mutex::new(Job::default())),
        }
    }

    /// Read the journal in a background thread; newest deletions first.
    fn load(&self, ctx: &Ctx) {
        let mut job = self.job.lock().expect("deleted job lock");
        if job.loading {
            return;
        }
        job.loading = true;
        drop(job);
        let (job, sender) = (Arc::clone(&self.job), ctx.app_event_sender.clone());
        std::thread::spawn(move || {
            let result = run(&["deletions", "--json", "--all"])
                .and_then(|out| serde_json::from_str::<Vec<Deleted>>(&out).map_err(|e| e.to_string()));
            let mut j = job.lock().expect("deleted job lock");
            j.loading = false;
            match result {
                Ok(mut rows) => {
                    rows.sort_by(|a, b| b.deleted_at.cmp(&a.deleted_at));
                    j.rows = Some(rows);
                    j.error = None;
                }
                Err(err) => j.error = Some(err),
            }
            drop(j);
            let _ = sender.send(AppEvent::RequestRender);
        });
    }

    /// Take a finished load (keeping the selected row) and start the reload a restore or retry asked for.
    /// Returns whether a load runs and its error.
    pub fn refresh(&mut self, ctx: &Ctx) -> (bool, Option<String>) {
        let (fresh, reload, loading, error) = {
            let mut j = self.job.lock().expect("deleted job lock");
            (j.rows.take(), std::mem::take(&mut j.reload), j.loading, j.error.clone())
        };
        if reload {
            self.load(ctx);
        }
        if let Some(rows) = fresh {
            let keep = self.selected().map(|r| r.id.clone());
            self.rows = rows;
            self.state.set_content_and_viewport_len(self.rows.len(), self.table_area.height.saturating_sub(1).into());
            let idx = keep.and_then(|id| self.rows.iter().position(|r| r.id == id)).unwrap_or(0);
            self.state.select((!self.rows.is_empty()).then_some(idx), 0);
        }
        (loading, error)
    }

    /// Deletions with failed or unresolved steps (Music's `Deleted ! N` badge), as of the last load.
    pub fn attention(&self) -> usize {
        self.rows.iter().filter(|r| r.needs_attention()).count()
    }

    fn selected(&self) -> Option<&Deleted> {
        self.state.get_selected().and_then(|i| self.rows.get(i))
    }

    fn open_menu(&self, ctx: &Ctx) {
        let Some(r) = self.selected().cloned() else { return };
        let job = Arc::clone(&self.job);
        let restore = r.restorable.then(|| r.id.clone());
        let retry = r.error.is_some();
        let gate = r.blocked().map(|blocked| (blocked, r.id.clone()));
        let menu = MenuModal::new(ctx)
            .list_section(ctx, move |mut section| {
                if let Some(id) = restore {
                    let job = Arc::clone(&job);
                    section.add_item("Restore to the library", move |ctx| {
                        run_then_reload(ctx, job, vec!["undo".into(), "--id".into(), id], "Restored");
                        Ok(())
                    });
                }
                if let Some((blocked, id)) = gate {
                    let job = Arc::clone(&job);
                    let (label, verb) = if blocked {
                        ("Allow downloading it again", "allow")
                    } else {
                        ("Block downloading it again", "block")
                    };
                    section.add_item(label, move |ctx| {
                        run_then_reload(ctx, job, vec!["deletions".into(), verb.into(), id], "Changed");
                        Ok(())
                    });
                }
                if retry {
                    section.add_item("Retry failed steps", move |ctx| {
                        run_then_reload(ctx, job, vec!["deletions".into(), "--retry".into()], "Retried");
                        Ok(())
                    });
                }
                Some(section)
            })
            .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
            .build();
        modal!(ctx, menu);
    }

    fn details(&self, ctx: &Ctx) -> Vec<Line<'static>> {
        let Some(r) = self.selected() else {
            return vec![Line::from("Nothing deleted yet.")];
        };
        let key = ctx.config.theme.preview_label_style;
        let dim = Style::default().add_modifier(Modifier::DIM);
        let field = |name: &str, value: String| Line::from(vec![Span::styled(format!("{name}: "), key), Span::raw(value)]);
        let mut lines = vec![
            Line::from(Span::styled(r.title.clone(), Style::default().add_modifier(Modifier::BOLD))),
            Line::from(r.artist.clone()),
            Line::default(),
            field("Deleted", r.deleted_at.replace('T', " ")),
            field("File", file_state(r).to_owned()),
            field("Path", r.file.clone()),
            field("Plays", r.plays.to_string()),
        ];
        if r.history == "delete" {
            lines.push(field("ListenBrainz", format!("{} of {} listens deleted", r.lb_deleted, r.lb_listens)));
            let youtube = match (&r.ytid, r.youtube_removed.is_empty()) {
                (None, _) => "not a YouTube download".to_owned(),
                (Some(_), true) => "in none of my playlists".to_owned(),
                (Some(_), false) => format!("removed from {}", r.youtube_removed.join(", ")),
            };
            lines.push(field("YouTube", youtube));
        } else {
            lines.push(field("History", "kept (ListenBrainz, YouTube playlists)".to_owned()));
        }
        if !r.ops.is_empty() {
            lines.push(Line::default());
            for (step, outcome) in &r.ops {
                lines.push(field(step, outcome.clone()));
            }
        }
        if let Some(d) = &r.download {
            let what: Vec<String> = [
                d.ytid.as_ref().map(|y| format!("video {y}")),
                d.mbid.as_ref().map(|m| format!("recording {m}")),
                d.chart_key.as_ref().map(|k| format!("chart song {k}")),
            ]
            .into_iter()
            .flatten()
            .collect();
            let gate = if d.state == "blocked" { "never downloaded again" } else { "allowed to be downloaded again" };
            lines.push(Line::default());
            lines.push(field("Download", gate.to_owned()));
            lines.push(Line::from(Span::styled(
                if what.is_empty() { "matches nothing (no video id, recording or artist known)".to_owned() } else { what.join(" · ") },
                dim,
            )));
        }
        if let Some(err) = &r.error {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(format!("Failed: {err}"), Style::default().add_modifier(Modifier::BOLD))));
        }
        lines.push(Line::default());
        let hint = match (r.restorable, r.error.is_some(), r.download.is_some()) {
            (true, true, _) => "Enter: restore or retry",
            (true, false, _) => "Enter: restore to the library",
            (false, true, _) => "Enter: retry failed steps",
            (false, false, true) => "Enter: allow or block downloading it again",
            (false, false, false) => "",
        };
        lines.push(Line::from(Span::styled(hint, dim)));
        lines
    }
}

/// Run `musicdb` for a restore or a retry in the background, report its last line, then reload the list.
fn run_then_reload(ctx: &Ctx, job: Arc<Mutex<Job>>, args: Vec<String>, ok: &'static str) {
    let sender = ctx.app_event_sender.clone();
    std::thread::spawn(move || {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        match run(&args) {
            Ok(out) => {
                let last = out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(ok).to_owned();
                status_info!("{last}");
            }
            Err(err) => status_error!("musicdb: {err}"),
        }
        job.lock().expect("deleted job lock").reload = true;
        let _ = sender.send(AppEvent::RequestRender);
    });
}

fn file_state(r: &Deleted) -> &'static str {
    match (r.mode.as_str(), r.restorable) {
        ("permanent", _) => "deleted permanently",
        (_, true) => "in the Trash (restorable)",
        _ => "gone from the Trash",
    }
}

impl Pane for DeletedPane {
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        let (loading, error) = self.refresh(ctx);
        let [main, details] =
            Layout::horizontal([Constraint::Min(40), Constraint::Percentage(35)]).spacing(2).areas(area);
        let [table_area, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(main);
        self.table_area = table_area;
        self.state.set_content_and_viewport_len(self.rows.len(), table_area.height.saturating_sub(1).into());

        let dim = Style::default().add_modifier(Modifier::DIM);
        let rows = self.rows.iter().map(|r| {
            let how = match (r.mode.as_str(), r.restorable) {
                ("permanent", _) => "deleted",
                (_, true) => "Trash",
                _ => "gone",
            };
            let history = if r.history == "delete" { "deleted" } else { "kept" };
            Row::new(vec![
                Cell::from(r.deleted_at.get(..16).unwrap_or(&r.deleted_at).replace('T', " ")),
                Cell::from(how),
                Cell::from(history),
                Cell::from(match r.blocked() {
                    Some(true) => "blocked",
                    Some(false) => "allowed",
                    None => "",
                }),
                Cell::from(if r.needs_attention() { "!" } else { "" }),
                Cell::from(if r.artist.is_empty() { r.title.clone() } else { format!("{} - {}", r.artist, r.title) }),
            ])
            .style(if r.restorable || r.needs_attention() { Style::default() } else { dim })
        });
        let header =
            Row::new(["Deleted", "File", "History", "Download", "", "Song"]).style(ctx.config.theme.preview_label_style);
        let table = Table::new(rows, [
            Constraint::Length(16),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(8),
            Constraint::Length(1),
            Constraint::Min(10),
        ])
        .header(header)
        .column_spacing(1)
        .style(ctx.config.as_text_style())
        .row_highlight_style(ctx.config.theme.current_item_style);
        frame.render_stateful_widget(table, table_area, self.state.as_render_state_ref());

        let in_trash = self.rows.iter().filter(|r| r.restorable).count();
        let failed = self.attention();
        let status = match (loading, error) {
            (_, Some(err)) => Span::styled(format!(" musicdb: {err}"), Style::default().add_modifier(Modifier::BOLD)),
            (true, None) if self.rows.is_empty() => Span::styled(" reading the deletion journal…", dim),
            _ => Span::styled(
                format!(" {} deleted · {in_trash} restorable from the Trash · {failed} with failed steps", self.rows.len()),
                dim,
            ),
        };
        frame.render_widget(Paragraph::new(Line::from(status)), footer);
        frame.render_widget(Paragraph::new(self.details(ctx)).wrap(Wrap { trim: false }), details);
        Ok(())
    }

    fn before_show(&mut self, ctx: &Ctx) -> Result<()> {
        self.load(ctx);
        Ok(())
    }

    fn on_event(&mut self, event: &mut UiEvent, is_visible: bool, ctx: &Ctx) -> Result<()> {
        // a Ctrl-x or Ctrl-y elsewhere changes the library, and so the journal
        if matches!(event, UiEvent::Database | UiEvent::Reconnected) && is_visible {
            self.load(ctx);
        }
        Ok(())
    }

    fn handle_mouse_event(&mut self, event: MouseEvent, ctx: &Ctx) -> Result<()> {
        if !self.table_area.contains(event.into()) {
            return Ok(());
        }
        let row = usize::from(event.y.saturating_sub(self.table_area.y + 1)); // +1: header row
        match event.kind {
            MouseEventKind::LeftClick | MouseEventKind::DoubleClick => {
                if let Some(idx) = self.state.get_at_rendered_row(row) {
                    self.state.select(Some(idx), ctx.config.scrolloff);
                    if matches!(event.kind, MouseEventKind::DoubleClick) {
                        self.open_menu(ctx);
                    }
                }
            }
            MouseEventKind::ScrollUp => self.state.scroll_up(ctx.config.scroll_amount, ctx.config.scrolloff),
            MouseEventKind::ScrollDown => self.state.scroll_down(ctx.config.scroll_amount, ctx.config.scrolloff),
            _ => return Ok(()),
        }
        ctx.render()?;
        Ok(())
    }

    fn handle_action(&mut self, event: &mut ActionEvent, ctx: &mut Ctx) -> Result<()> {
        let Some(action) = event.claim_common().cloned() else {
            return Ok(());
        };
        let (scrolloff, wrap) = (ctx.config.scrolloff, ctx.config.wrap_navigation);
        match action {
            CommonAction::Down => self.state.next(scrolloff, wrap),
            CommonAction::Up => self.state.prev(scrolloff, wrap),
            CommonAction::DownHalf => self.state.next_half_viewport(scrolloff),
            CommonAction::UpHalf => self.state.prev_half_viewport(scrolloff),
            CommonAction::PageDown => self.state.next_viewport(scrolloff),
            CommonAction::PageUp => self.state.prev_viewport(scrolloff),
            CommonAction::Top => self.state.first(),
            CommonAction::Bottom => self.state.last(),
            CommonAction::Confirm | CommonAction::ContextMenu => self.open_menu(ctx),
            _ => {
                event.abandon(); // not ours: let global keys (tabs, playback) handle it
                return Ok(());
            }
        }
        ctx.render()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Deleted;

    #[test]
    fn journal_row_with_and_without_the_download_gate() {
        let row = r#"{"id": "i", "file": "a.mp3", "mode": "permanent", "history": "delete",
            "deleted_at": "2026-10-10T14:40:06", "download": {"state": "blocked", "ytid": "vid", "mbid": null,
            "chart_key": "artist|song"}}"#;
        let r: Deleted = serde_json::from_str(row).expect("a journal row");
        assert_eq!(r.blocked(), Some(true));
        assert_eq!(r.download.as_ref().and_then(|d| d.chart_key.as_deref()), Some("artist|song"));
        // an older musicdb writes no gate: nothing to allow or block
        let old: Deleted = serde_json::from_str(r#"{"id": "i", "file": "a.mp3", "mode": "trash", "history": "keep",
            "deleted_at": "2026-10-03T12:00:00"}"#).expect("an older journal row");
        assert_eq!(old.blocked(), None);
        assert!(!old.needs_attention());
    }

    #[test]
    fn failed_steps_need_attention() {
        let failed: Deleted = serde_json::from_str(r#"{"id": "i", "file": "a.mp3", "mode": "trash",
            "history": "delete", "deleted_at": "2026-10-10T12:00:00",
            "ops": {"listenbrainz": "failed: 401", "local": "done"}}"#).expect("a journal row");
        assert!(failed.needs_attention());
        let errored: Deleted = serde_json::from_str(r#"{"id": "i", "file": "a.mp3", "mode": "trash",
            "history": "delete", "deleted_at": "2026-10-10T12:00:00", "error": "youtube: login expired"}"#)
            .expect("a journal row");
        assert!(errored.needs_attention());
        let done: Deleted = serde_json::from_str(r#"{"id": "i", "file": "a.mp3", "mode": "trash",
            "history": "delete", "deleted_at": "2026-10-10T12:00:00", "ops": {"listenbrainz": "done"}}"#)
            .expect("a journal row");
        assert!(!done.needs_attention());
    }
}
